use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Open in an updated Nod app to answer this request.")]
    ClientUpdateRequired,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
    message: String,
}

impl ApiError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::ClientUpdateRequired => StatusCode::CONFLICT,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status();
        let code = match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict(_) => "conflict",
            Self::ClientUpdateRequired => "client_update_required",
            Self::Internal(_) => "internal",
        };

        let body = Json(ErrorBody {
            error: code,
            message: self.to_string(),
        });
        (status, body).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(value: sqlx::Error) -> Self {
        tracing::error!(error = %value, "database error");
        Self::Internal("database error".to_string())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(value: serde_json::Error) -> Self {
        Self::BadRequest(value.to_string())
    }
}
