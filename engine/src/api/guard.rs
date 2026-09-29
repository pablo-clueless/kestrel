//! Every `/api` request passes through here. Binding to 127.0.0.1 keeps other machines out, but
//! not web pages in the user's own browser, so we also check the token, `Host`, `Origin` and
//! content type. See HANDOFF.md → Safety rails → The engine API.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use subtle::ConstantTimeEq;

use super::AppState;
use crate::{config::Config, error::ApiError};

pub const TOKEN_HEADER: &str = "x-kestrel-token";

pub async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    match check(&state.config, &req) {
        Ok(()) => next.run(req).await,
        Err(err) => {
            tracing::warn!(method = %req.method(), path = %req.uri().path(), "rejected: {err}");
            err.into_response()
        }
    }
}

fn check(cfg: &Config, req: &Request) -> Result<(), ApiError> {
    let headers = req.headers();

    let host = header_str(headers, header::HOST).ok_or(ApiError::Forbidden("missing Host header"))?;
    if !cfg.allowed_hosts.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        tracing::warn!(
            "Host `{host}` is not allowed. If that's this deployment's own hostname, add it to KESTREL_ALLOWED_HOSTS."
        );
        return Err(ApiError::Forbidden("Host not allowed (see KESTREL_ALLOWED_HOSTS)"));
    }

    if let Some(origin) = headers.get(header::ORIGIN) {
        let allowed = origin.to_str().is_ok_and(|o| cfg.allowed_origins.iter().any(|a| a == o));
        if !allowed {
            return Err(ApiError::Forbidden("Origin not allowed"));
        }
    }

    let provided = header_str(headers, TOKEN_HEADER).or_else(|| query_token(req));
    let valid = provided.is_some_and(|t| bool::from(t.as_bytes().ct_eq(cfg.token.as_bytes())));
    if !valid {
        return Err(ApiError::Forbidden("missing or invalid token"));
    }

    if has_body(headers) && !is_allowed_content_type(req.uri().path(), headers) {
        return Err(ApiError::UnsupportedMediaType("request body must be application/json"));
    }

    Ok(())
}

/// `EventSource` can't set headers, so only the SSE endpoint accepts `?token=`.
fn query_token(req: &Request) -> Option<&str> {
    if req.method() != Method::GET || !req.uri().path().ends_with("/events") {
        return None;
    }
    req.uri().query()?.split('&').find_map(|pair| pair.strip_prefix("token="))
}

fn has_body(headers: &HeaderMap) -> bool {
    let content_length = header_str(headers, header::CONTENT_LENGTH).and_then(|v| v.parse::<u64>().ok());
    content_length.is_some_and(|n| n > 0) || headers.contains_key(header::TRANSFER_ENCODING)
}

fn is_allowed_content_type(path: &str, headers: &HeaderMap) -> bool {
    // Uploads are the raw file, whatever its type. The token check above still applies.
    if path.ends_with("/files") {
        return true;
    }
    let Some(ct) = header_str(headers, header::CONTENT_TYPE) else { return false };
    let mime = ct.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    mime == "application/json" || (path.ends_with("/import") && mime == "multipart/form-data")
}

fn header_str(headers: &HeaderMap, name: impl header::AsHeaderName) -> Option<&str> {
    headers.get(name)?.to_str().ok()
}
