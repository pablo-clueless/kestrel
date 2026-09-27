use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    Forbidden(&'static str),
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
        let status = match self {
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::UnsupportedMediaType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::RunNotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Internal(ref msg) => {
                tracing::error!("{msg}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::HostNotConfirmed(_) => unreachable!("handled above"),
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
