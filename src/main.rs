use std::{borrow::Cow, time::Duration};

use axum::{
    extract::{Path, Request, State},
    http::{HeaderMap, HeaderName},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use axum_extra::{headers, TypedHeader};
use listenfd::ListenFd;
use regex::Regex;
use reqwest::{header, Client, StatusCode};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::Span;
use tracing_subscriber::{fmt::time::ChronoLocal, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with_timer(ChronoLocal::rfc_3339())
        .init();

    let client = Client::new();

    let app = Router::new()
        .route("/chzzk/:player_link", get(chzzk))
        .route("/soop/liveplayer.js", get(soop))
        .route_layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    tracing::info_span!(
                        "request",
                        method = %request.method(),
                        uri = %request.uri(),
                    )
                })
                .on_response(|_response: &Response, _latency: Duration, _span: &Span| {
                    tracing::debug!(?_latency);
                }),
        )
        .with_state(client);

    let mut listenfd = ListenFd::from_env();
    let listener = match listenfd.take_tcp_listener(0)? {
        // if we are given a tcp listener on listen fd 0, we use that one
        Some(listener) => {
            listener.set_nonblocking(true)?;
            TcpListener::from_std(listener)?
        }
        // otherwise fall back to local listening
        None => TcpListener::bind("0.0.0.0:3000").await?,
    };

    axum::serve(listener, app).await?;

    Ok(())
}

async fn chzzk(
    State(client): State<Client>,
    Path(player_link): Path<String>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> Result<impl IntoResponse, AppError> {
    let url =
        format!("https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/{player_link}");
    let header_keys = vec![header::CONTENT_TYPE, header::CACHE_CONTROL, header::EXPIRES];

    let patterns = [
        (
            r"(.\(!0\),.\(null\)),.\(.\),.*?case 6",
            "$1,e.next=6;case 6",
        ),
        (r".\.forceLowResolution", "false"),
        (r"var .=.\.exposureAdBlockPopup(.*?)\)\},", "},"),
    ];

    let patterns = patterns.map(|pattern| (Regex::new(pattern.0).unwrap(), pattern.1));

    process(&client, &url, user_agent, header_keys, move |content| {
        let content = patterns
            .iter()
            .fold(Cow::Borrowed(&content), |content, pattern| {
                match pattern.0.replace(&content, pattern.1) {
                    Cow::Borrowed(_) => {
                        tracing::warn!(pattern = pattern.0.to_string());
                        content
                    }
                    Cow::Owned(replaced) => Cow::Owned(replaced),
                }
            })
            .to_string();
        // for a in arr {
        //     content = match patterns.replace(&content, a.1) {
        //         Cow::Borrowed(_) => content,
        //         Cow::Owned(replaced) => Cow::Owned(replaced),
        //     };
        // }

        Ok(content)
    })
    .await
}

async fn soop(
    State(client): State<Client>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> Result<impl IntoResponse, AppError> {
    let url = "https://static.sooplive.co.kr/asset/app/liveplayer/player/dist/LivePlayer.js";
    let header_keys = vec![header::CONTENT_TYPE, header::CACHE_CONTROL];
    let regex_pattern = r"shouldConnectToAgentForHighQuality\(\)\{.*?\},";
    let replacement = "shouldConnectToAgentForHighQuality(){return!1},";

    process(&client, url, user_agent, header_keys, move |content| {
        let regex = Regex::new(regex_pattern)?;
        Ok(regex.replace(&content, replacement).to_string())
    })
    .await
}

async fn process<F>(
    client: &Client,
    url: &str,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    header_keys: Vec<HeaderName>,
    f: F,
) -> Result<impl IntoResponse, AppError>
where
    F: FnOnce(String) -> Result<String, AppError>,
{
    let req = client.get(url);
    let req = if let Some(user_agent) = user_agent {
        req.header(header::USER_AGENT, user_agent.as_str())
    } else {
        req
    };

    let res = req.send().await?;
    let status = res.status();

    let headers = HeaderMap::from_iter(header_keys.into_iter().filter_map(|key| {
        res.headers()
            .get(&key)
            .map(|header_value| (key, header_value.clone()))
    }));

    let is_success = status.is_success();
    let is_javascript = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|header_value| header_value.to_str().ok())
        .map(|x| x.contains("javascript"))
        .unwrap_or(false);

    let content = res.text().await?;

    let content = if is_success && is_javascript {
        f(content)?
    } else {
        tracing::warn!(is_success, is_javascript);
        content
    };

    Ok((status, headers, content))
}

struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("{:?}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()).into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
