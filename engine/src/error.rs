use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// No valid session (accounts on), or wrong credentials.
    #[error("{0}")]
    Unauthorized(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("{0}")]
    NotFound(&'static str),
    /// Rate limited; how long until trying again makes sense.
    #[error("too many attempts; try again in {} s", .0.as_secs().max(1))]
    TooManyRequests(std::time::Duration),
    #[error("{0}")]
    UnsupportedMediaType(&'static str),
    #[error("{0}")]
    BadRequest(String),
    #[error("run not found")]
    RunNotFound,
    #[error("{0}")]
    Conflict(&'static str),
    #[error("{0}")]
    Internal(String),
    #[error("confirm you own or are authorised to load-test `{0}` first")]
    HostNotConfirmed(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // The UI keys its confirmation prompt off `code`, not the message.
        if let Self::HostNotConfirmed(ref host) = self {
            let body = json!({ "error": self.to_string(), "code": "hostNotConfirmed", "host": host });
            return (StatusCode::CONFLICT, Json(body)).into_response();
        }
        if let Self::TooManyRequests(wait) = self {
            let retry_after = [(header::RETRY_AFTER, wait.as_secs().max(1).to_string())];
            return (StatusCode::TOO_MANY_REQUESTS, retry_after, Json(json!({ "error": self.to_string() })))
                .into_response();
        }
        let status = match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::UnsupportedMediaType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::RunNotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Internal(ref msg) => {
                tracing::error!("{msg}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::HostNotConfirmed(_) | Self::TooManyRequests(_) => unreachable!("handled above"),
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
