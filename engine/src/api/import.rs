use std::time::Duration;

use axum::Json;

use crate::{
    error::ApiError,
    import::{
        self, ImportRequest, ImportResult, ImportSource,
        curl::{CurlParseRequest, CurlParseResult},
    },
};

const MAX_SPEC_BYTES: usize = 10 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Parses a spec into a collection. Nothing is saved: the UI adds the collection to the workspace.
pub async fn import(Json(req): Json<ImportRequest>) -> Result<Json<ImportResult>, ApiError> {
    let text = match req.source {
        ImportSource::Text { content } => content,
        ImportSource::Url { url } => fetch(&url).await.map_err(ApiError::BadRequest)?,
    };
    if text.len() > MAX_SPEC_BYTES {
        return Err(ApiError::BadRequest("the spec is larger than 10 MB".into()));
    }
    // Big specs take a moment to walk; keep it off the async workers.
    let name = req.name;
    tokio::task::spawn_blocking(move || import::import(&text, name.as_deref()))
        .await
        .map_err(|e| ApiError::Internal(format!("import task failed: {e}")))?
        .map(Json)
        .map_err(ApiError::BadRequest)
}

/// Reads one curl command into an endpoint, for pasting into a request. Nothing is saved.
pub async fn curl(Json(req): Json<CurlParseRequest>) -> Result<Json<CurlParseResult>, ApiError> {
    if req.command.len() > MAX_SPEC_BYTES {
        return Err(ApiError::BadRequest("the command is larger than 10 MB".into()));
    }
    import::curl::parse_one(&req.command).map(Json).map_err(ApiError::BadRequest)
}

async fn fetch(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url.trim()).map_err(|e| format!("invalid URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("only http and https URLs can be imported".into());
    }
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| format!("couldn't build HTTP client: {e}"))?;
    let res = client.get(parsed).send().await.map_err(|e| format!("couldn't fetch the spec: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("fetching the spec returned HTTP {}", res.status().as_u16()));
    }
    if res.content_length().is_some_and(|n| n as usize > MAX_SPEC_BYTES) {
        return Err("the spec is larger than 10 MB".into());
    }
    let bytes = res.bytes().await.map_err(|e| format!("couldn't read the spec: {e}"))?;
    String::from_utf8(bytes.to_vec()).map_err(|_| "the spec isn't UTF-8 text".into())
}
