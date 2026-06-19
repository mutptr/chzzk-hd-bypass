use axum::{
    Router,
    extract::{Path, State},
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::{TypedHeader, headers};
use http::{HeaderMap, HeaderName};
use lru::LruCache;
use regex::Regex;
use reqwest::{Client, StatusCode, header};
use std::{
    num::NonZeroUsize,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

type CachedResponse = (StatusCode, HeaderMap, String);

#[derive(Clone)]
struct AppState {
    client: Client,
    cache: Arc<Mutex<LruCache<String, CachedResponse>>>,
}

static PLAYER_PATCH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?-u)(`p2pPath`)|(forceLowResolution:!!\w+\.dab)|&&([\w$]+\.createElement\([\w$]+,\{confirmHandler:\w+=>\{\w+\.isTrusted)"#,
    )
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
                let directives = if cfg!(debug_assertions) {
                    concat!("info,", env!("CARGO_PKG_NAME"), "=debug")
                } else {
                    "info"
                };
                tracing_subscriber::EnvFilter::new(directives)
            }),
        )
        .with_timer(tracing_subscriber::fmt::time::ChronoLocal::new(
            "%Y-%m-%d %H:%M:%S".to_owned(),
        ))
        .init();

    let client = Client::builder().timeout(Duration::from_secs(10)).build()?;
    let state = AppState {
        client,
        cache: Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(32).unwrap()))),
    };

    let app = Router::new()
        .route("/{player_link}", get(chzzk))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    tracing::info!("listening on 0.0.0.0:3000");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn chzzk(
    State(state): State<AppState>,
    Path(player_link): Path<String>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> Result<Response, AppError> {
    tracing::debug!(%player_link, has_user_agent = user_agent.is_some(), "handling request");

    if !PLAYER_LINK_PATTERN.is_match(&player_link) {
        tracing::warn!(%player_link, "rejecting invalid player link");
        return Ok((StatusCode::BAD_REQUEST, "invalid player link").into_response());
    }

    let cached = {
        let mut cache = state.cache.lock().unwrap();
        cache.get(&player_link).cloned()
    };
    if let Some(cached) = cached {
        tracing::debug!(%player_link, bytes = cached.2.len(), "serving cached response");
        return Ok(cached.into_response());
    }

    let url =
        format!("https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/{player_link}");
    let header_keys = [header::CONTENT_TYPE, header::CACHE_CONTROL, header::EXPIRES];
    let should_patch = player_link.starts_with("index-");
    tracing::debug!(%player_link, should_patch, "cache miss, fetching from upstream");

    let log_link = player_link.clone();
    let (status, headers, content) = process(
        state.client,
        url,
        user_agent,
        header_keys,
        move |content| {
            if !should_patch {
                tracing::debug!(player_link = %log_link, "passing through, not an index bundle");
                return content;
            }

            let (mut p2p, mut low_res, mut popup) = (0u32, 0u32, 0u32);
            let patched = PLAYER_PATCH_PATTERN
                .replace_all(&content, |caps: &regex::Captures| {
                    if caps.get(1).is_some() {
                        p2p += 1;
                        "`p2p`".to_string()
                    } else if caps.get(2).is_some() {
                        low_res += 1;
                        "forceLowResolution:!1".to_string()
                    } else if let Some(rest) = caps.get(3) {
                        popup += 1;
                        format!("&&!1&&{}", rest.as_str())
                    } else {
                        tracing::warn!("unexpected match without a capture group");
                        caps[0].to_string()
                    }
                })
                .to_string();

            if p2p == 0 && low_res == 0 && popup == 0 {
                tracing::warn!(player_link = %log_link, "no patterns matched, bundle may have changed");
            } else {
                tracing::info!(player_link = %log_link, p2p, low_res, popup, "applied patches");
            }
            patched
        },
    )
    .await?;

    if status.is_success() {
        let mut cache = state.cache.lock().unwrap();
        cache.put(
            player_link.clone(),
            (status, headers.clone(), content.clone()),
        );
        tracing::debug!(%player_link, %status, bytes = content.len(), "served and cached");
    } else {
        tracing::warn!(%player_link, %status, "served without caching, non-success status");
    }

    Ok((status, headers, content).into_response())
}

async fn process<F>(
    client: Client,
    url: String,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    header_keys: impl IntoIterator<Item = HeaderName>,
    f: F,
) -> Result<CachedResponse, AppError>
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
    tracing::debug!(
        %status,
        is_success,
        is_javascript,
        bytes = content.len(),
        "upstream response"
    );

    let content = if is_success && is_javascript {
        f(content)
    } else {
        tracing::warn!(is_success, is_javascript, "skipping patch");
        content
    };

    Ok((status, headers, content))
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
