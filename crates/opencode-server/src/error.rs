use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Provider error: {0}")]
    ProviderError(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Internal error: {0}")]
    InternalError(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message, error_type) = match self {
            ApiError::SessionNotFound(id) => (
                StatusCode::NOT_FOUND,
                format!("Session not found: {}", id),
                "session_not_found",
            ),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg, "not_found"),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg, "bad_request"),
            ApiError::ProviderError(msg) => (
                StatusCode::BAD_GATEWAY,
                format!("Provider error: {}", msg),
                "provider_error",
            ),
            ApiError::InvalidRequest(msg) => (
                StatusCode::BAD_REQUEST,
                format!("Invalid request: {}", msg),
                "invalid_request",
            ),
            ApiError::InternalError(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Internal error: {}", msg),
                "internal_error",
            ),
        };

        let body = Json(json!({
            "error": {
                "message": message,
                "type": error_type
            }
        }));

        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;
