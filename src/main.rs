use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use http_cache_reqwest::{Cache, CacheMode, HttpCache, HttpCacheOptions, MokaManager};
use regex::Regex;
use reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use tokio::time::Instant;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() {
    let client = ClientBuilder::new(Client::new())
        .with(Cache(HttpCache {
            mode: CacheMode::Default,
            manager: MokaManager::default(),
            options: HttpCacheOptions::default(),
        }))
        .build();

    let app = Router::new()
        .route("/:player_link", get(handler))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(client);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handler(
    State(client): State<ClientWithMiddleware>,
    Path(player_link): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let user_agent = headers.get(header::USER_AGENT);

    let start_request = Instant::now();
    let req = client.get(format!(
        "https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/{player_link}"
    ));

    let req = if let Some(user_agent) = user_agent {
        req.header(header::USER_AGENT, user_agent)
    } else {
        req
    };

    let res = req.send().await?;

    println!("request {:#?}", start_request.elapsed());

    let start = Instant::now();

    let header_keys = [
        header::CONTENT_TYPE,
        header::AGE,
        header::CACHE_CONTROL,
        header::DATE,
        header::EXPIRES,
        header::LAST_MODIFIED,
    ];
    let headers = HeaderMap::from_iter(header_keys.into_iter().filter_map(|key| {
        res.headers()
            .get(reqwest::header::HeaderName::from_bytes(key.as_ref()).unwrap())
            .map(|header_value| (key, HeaderValue::from_bytes(header_value.as_ref()).unwrap()))
    }));

    let is_javascript = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|header_value| header_value.to_str().ok())
        .map(|x| x == "application/javascript")
        .unwrap_or(false);

    let status = res.status();
    let mut content = res.text().await?;

    if status.is_success() && is_javascript {
        // a(!0),y(null),l(t),
        let regex = Regex::new(r"(.\(!0\),.\(null\)),.\(.\),.*?case 6").unwrap();
        content = regex.replace(&content, "$1,e.next=6;case 6").to_string();
    }

    println!("parse: {:#?}", start.elapsed());

    Ok((
        StatusCode::from_u16(status.as_u16()).unwrap_or_default(),
        headers,
        content,
    ))
}

struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
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
