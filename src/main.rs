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
use std::{sync::LazyLock, time::Duration};
use tokio::net::TcpListener;

static PLAYER_PATCH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"("p2pPath")|(.\.forceLowResolution)|(var .=.\.exposureAdBlockPopup(?:.*?)\)\},)"#)
        .expect("player patch regex should compile")
});

static PLAYER_LINK_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^[A-Za-z0-9._-]+\.js$"#).expect("player link regex should compile")
});

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new(tracing::Level::INFO.to_string())
            }),
        )
        .with_timer(tracing_subscriber::fmt::time::ChronoLocal::new(
            "%Y-%m-%d %H:%M:%S".to_owned(),
        ))
        .init();

    let client = Client::builder().timeout(Duration::from_secs(10)).build()?;

    let app = Router::new()
        .route("/{player_link}", get(chzzk))
        .with_state(client);

    let listener = TcpListener::bind("0.0.0.0:3000").await?;

    axum::serve(listener, app).await?;
    Ok(())
}

async fn chzzk(
    State(client): State<Client>,
    Path(player_link): Path<String>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> Result<Response, AppError> {
    if !PLAYER_LINK_PATTERN.is_match(&player_link) {
        return Ok((StatusCode::BAD_REQUEST, "invalid player link").into_response());
    }

    let url =
        format!("https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/{player_link}");
    let header_keys = [header::CONTENT_TYPE, header::CACHE_CONTROL, header::EXPIRES];

    process(client, url, user_agent, header_keys, move |content| {
        PLAYER_PATCH_PATTERN
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
    header_keys: impl IntoIterator<Item = HeaderName>,
    f: F,
) -> Result<Response, AppError>
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
        tracing::warn!(is_success, is_javascript);
        content
    };

    Ok((status, headers, content).into_response())
}

struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("{:?}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
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
