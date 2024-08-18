use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderName, Request},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use axum_extra::{headers, TypedHeader};
use http_cache_reqwest::{CACacheManager, Cache, CacheMode, HttpCache, HttpCacheOptions};
use mimalloc::MiMalloc;
use regex::Regex;
use reqwest::{header, Client, StatusCode};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use tower_http::{compression::CompressionLayer, trace::TraceLayer};
use tracing::Span;
use tracing_subscriber::{fmt::time::ChronoLocal, EnvFilter};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let name = env!("CARGO_PKG_NAME").replace("-", "_");
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(format!("{name}=debug").parse()?)
                .from_env_lossy(),
        )
        .with_timer(ChronoLocal::rfc_3339())
        .init();

    let client = ClientBuilder::new(Client::new())
        .with(Cache(HttpCache {
            mode: CacheMode::Default,
            manager: CACacheManager::default(),
            options: HttpCacheOptions::default(),
        }))
        .build();

    let app = Router::new()
        .route("/chzzk/:player_link", get(chzzk))
        .route("/afreecatv/liveplayer.js", get(afreecatv))
        .layer(CompressionLayer::new())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    tracing::info_span!(
                        "request",
                        method = %request.method(),
                        uri = %request.uri(),
                    )
                })
                .on_response(|_response: &Response, _latency: Duration, _span: &Span| {
                    tracing::debug!(parent: _span, _latency = ?_latency);
                }),
        )
        .with_state(client);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn chzzk(
    State(client): State<ClientWithMiddleware>,
    Path(player_link): Path<String>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> Result<impl IntoResponse, AppError> {
    let url =
        format!("https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/{player_link}");
    let header_keys = [
        header::CONTENT_TYPE,
        header::AGE,
        header::CACHE_CONTROL,
        header::DATE,
        header::EXPIRES,
        header::LAST_MODIFIED,
    ];
    let regex_pattern = r"(.\(!0\),.\(null\)),.\(.\),.*?case 6";
    let replacement = "$1,e.next=6;case 6";

    process(
        &client,
        url,
        user_agent,
        header_keys,
        regex_pattern,
        replacement,
    )
    .await
}

async fn afreecatv(
    State(client): State<ClientWithMiddleware>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> Result<impl IntoResponse, AppError> {
    let url = "https://static.afreecatv.com/asset/app/liveplayer/player/dist/LivePlayer.js";
    let header_keys = [header::ETAG, header::CONTENT_TYPE, header::CACHE_CONTROL];
    let regex_pattern = r"shouldConnectToAgentForHighQuality:function\(\)\{.*?\},";
    let replacement = "shouldConnectToAgentForHighQuality:function(){return!1},";

    process(
        &client,
        url,
        user_agent,
        header_keys,
        regex_pattern,
        replacement,
    )
    .await
}

async fn process<const N: usize>(
    client: &ClientWithMiddleware,
    url: impl AsRef<str>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    header_keys: [HeaderName; N],
    regex_pattern: impl AsRef<str>,
    replacement: impl AsRef<str>,
) -> Result<impl IntoResponse, AppError> {
    let req = client.get(url.as_ref());
    let req = if let Some(user_agent) = user_agent {
        tracing::debug!(user_agent = user_agent.as_str());
        req.header(header::USER_AGENT, user_agent.as_str())
    } else {
        req
    };

    tracing::debug!(url = url.as_ref(), "request");
    let res = req.send().await?;
    let status = res.status();
    tracing::debug!(status = %status, headers = ?res.headers(), "response");

    let headers = HeaderMap::from_iter(header_keys.into_iter().filter_map(|key| {
        res.headers()
            .get(&key)
            .map(|header_value| (key, header_value.clone()))
    }));
    tracing::trace!(?headers);

    let is_success = status.is_success();
    let is_javascript = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|header_value| header_value.to_str().ok())
        .map(|x| x == "text/javascript")
        .unwrap_or(false);

    tracing::trace!("res.text");
    let content = res.text().await?;

    let content = if is_success && is_javascript {
        tracing::trace!(regex_pattern = regex_pattern.as_ref(), "Regex::new");
        let regex = Regex::new(regex_pattern.as_ref())?;
        tracing::trace!(replacement = replacement.as_ref(), "regex.replace");
        regex.replace(&content, replacement.as_ref()).to_string()
    } else {
        tracing::warn!(is_success, is_javascript);
        content
    };

    tracing::trace!("response to client");
    Ok((status, headers, content))
}

struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("{}", self.0);
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
