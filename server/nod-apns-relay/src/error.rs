use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Upstream(String),
    #[error("{0}")]
    Delivery(DeliveryFailure),
}

#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct DeliveryFailure {
    pub message: String,
    pub retryable: bool,
    pub invalid_token: bool,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
            Self::Delivery(failure) if failure.retryable => StatusCode::BAD_GATEWAY,
            Self::Delivery(_) => StatusCode::UNPROCESSABLE_ENTITY,
        };
        if let Self::Delivery(failure) = self {
            return (status, Json(failure)).into_response();
        }
        let body = Json(ErrorResponse {
            error: status.canonical_reason().unwrap_or("error"),
            message: self.to_string(),
        });
        (status, body).into_response()
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    error: &'static str,
    message: String,
}

/// Upstream rejection bodies are diagnostics, never an unbounded response stream.
pub async fn bounded_error_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Option<T> {
    const MAX_ERROR_BYTES: usize = 4096;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len().saturating_add(chunk.len()) > MAX_ERROR_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).ok()
}
