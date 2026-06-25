use axum::{
    Router,
    extract::{Path, State},
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::{TypedHeader, headers};
use bytes::Bytes;
use http::HeaderMap;
use lru::LruCache;
use regex::Regex;
use reqwest::{Client, StatusCode, header};
use std::{
    borrow::Cow,
    num::NonZeroUsize,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

const LISTEN_ADDR: &str = "0.0.0.0:3000";
const UPSTREAM_PREFIX: &str = "https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/";
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);
const CACHE_CAP: NonZeroUsize = NonZeroUsize::new(32).unwrap();

struct CachedResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

type Cached = Arc<CachedResponse>;

impl IntoResponse for &CachedResponse {
    fn into_response(self) -> Response {
        (self.status, self.headers.clone(), self.body.clone()).into_response()
    }
}

#[derive(Clone)]
struct AppState {
    client: Client,
    cache: Arc<Mutex<LruCache<String, Cached>>>,
}

static PLAYER_PATCH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?-u)(`p2pPath`)|(forceLowResolution:!!\w+\.dab)|&&([\w$]+\.createElement\([\w$]+,\{confirmHandler:\w+=>\{\w+\.isTrusted)"#,
    )
    .expect("player patch regex should compile")
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

    let client = Client::builder().timeout(UPSTREAM_TIMEOUT).build()?;
    let state = AppState {
        client,
        cache: Arc::new(Mutex::new(LruCache::new(CACHE_CAP))),
    };

    let app = Router::new()
        .route("/{player_link}", get(chzzk))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = TcpListener::bind(LISTEN_ADDR).await?;
    tracing::info!("listening on {}", listener.local_addr()?);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn chzzk(
    State(state): State<AppState>,
    Path(player_link): Path<String>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> Result<Response, AppError> {
    tracing::debug!(%player_link, has_user_agent = user_agent.is_some(), "handling request");

    if !player_link.ends_with(".js") {
        tracing::warn!(%player_link, "rejecting invalid player link");
        return Ok((StatusCode::BAD_REQUEST, "invalid player link").into_response());
    }

    let cached = {
        let mut cache = state.cache.lock().unwrap();
        cache.get(&player_link).cloned()
    };
    if let Some(cached) = cached {
        tracing::debug!(%player_link, bytes = cached.body.len(), "serving cached response");
        return Ok(cached.into_response());
    }

    let url = format!("{UPSTREAM_PREFIX}{player_link}");
    let should_patch = player_link.starts_with("index-");
    tracing::debug!(%player_link, should_patch, "cache miss, fetching from upstream");

    let req = state.client.get(&url);
    let req = if let Some(user_agent) = user_agent {
        req.header(header::USER_AGENT, user_agent.as_str())
    } else {
        req
    };

    let res = req.send().await?;
    let status = res.status();

    let header_keys = [header::CONTENT_TYPE, header::CACHE_CONTROL, header::EXPIRES];
    let headers = HeaderMap::from_iter(header_keys.into_iter().filter_map(|key| {
        res.headers()
            .get(&key)
            .map(|header_value| (key, header_value.clone()))
    }));

    let body = res.bytes().await?;
    tracing::debug!(%status, bytes = body.len(), "upstream response");

    let body = match (should_patch, status.is_success()) {
        (true, true) => patch_player_bundle(&player_link, body),
        (true, false) => {
            tracing::warn!(%player_link, %status, "skipping patch, non-success status");
            body
        }
        _ => body,
    };

    let cached: Cached = Arc::new(CachedResponse {
        status,
        headers,
        body,
    });
    if status.is_success() {
        state
            .cache
            .lock()
            .unwrap()
            .put(player_link.clone(), cached.clone());
        tracing::debug!(%player_link, %status, bytes = cached.body.len(), "served and cached");
    } else {
        tracing::warn!(%player_link, %status, "served without caching, non-success status");
    }

    Ok(cached.into_response())
}

fn patch_player_bundle(player_link: &str, body: Bytes) -> Bytes {
    let Ok(text) = std::str::from_utf8(&body) else {
        tracing::warn!(%player_link, "body is not valid utf-8, skipping patch");
        return body;
    };

    let (mut p2p, mut low_res, mut popup) = (0u32, 0u32, 0u32);
    let patched = PLAYER_PATCH_PATTERN.replace_all(text, |caps: &regex::Captures| {
        match (caps.get(1), caps.get(2), caps.get(3)) {
            (Some(_), _, _) => {
                p2p += 1;
                "`p2p`".into()
            }
            (_, Some(_), _) => {
                low_res += 1;
                "forceLowResolution:!1".into()
            }
            (_, _, Some(rest)) => {
                popup += 1;
                format!("&&!1&&{}", rest.as_str())
            }
            _ => {
                tracing::warn!("unexpected match without a capture group");
                caps[0].into()
            }
        }
    });

    match patched {
        Cow::Borrowed(_) => {
            tracing::warn!(%player_link, "no patterns matched, bundle may have changed");
            body
        }
        Cow::Owned(s) => {
            tracing::info!(%player_link, p2p, low_res, popup, "applied patches");
            s.into()
        }
    }
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
