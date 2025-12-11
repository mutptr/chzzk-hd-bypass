use axum::{
    Router,
    extract::{Path, State},
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::{TypedHeader, headers};
use http::{HeaderMap, HeaderName};
use regex::Regex;
use reqwest::{Client, StatusCode, header};
use tokio::net::TcpListener;
use tracing::Level;
use tracing_subscriber::{EnvFilter, fmt::time::ChronoLocal};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(Level::INFO.into()))
        .with_timer(ChronoLocal::new("%Y-%m-%d %H:%M:%S".to_owned()))
        .init();

    let client = Client::new();

    let app = Router::new()
        .route("/{player_link}", get(chzzk))
        .with_state(client);

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}

async fn chzzk(
    State(client): State<Client>,
    Path(player_link): Path<String>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> Result<impl IntoResponse, AppError> {
    let url =
        format!("https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/{player_link}");
    let header_keys = vec![header::CONTENT_TYPE, header::CACHE_CONTROL, header::EXPIRES];

    let combined_pattern = Regex::new(
        r#"("p2pPath")|(.\.forceLowResolution)|(var .=.\.exposureAdBlockPopup(?:.*?)\)\},)"#,
    )
    .unwrap();

    process(client, url, user_agent, header_keys, move |content| {
        combined_pattern
            .replace_all(&content, |caps: &regex::Captures| {
                if caps.get(1).is_some() {
                    "\"p2p\""
                } else if caps.get(2).is_some() {
                    "false"
                } else if caps.get(3).is_some() {
                    "},"
                } else {
                    tracing::warn!("Pattern");
                    ""
                }
            })
            .to_string()
    })
    .await
}

async fn process<F>(
    client: Client,
    url: String,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    header_keys: Vec<HeaderName>,
    f: F,
) -> Result<impl IntoResponse, AppError>
where
    F: FnOnce(String) -> String,
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
        f(content)
    } else {
        tracing::error!(is_success, is_javascript);
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
