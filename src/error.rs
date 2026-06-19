use axum::{Json, response::IntoResponse};
use reqwest::StatusCode;
use serde::Serialize;

#[derive(thiserror::Error, Debug)]
pub enum WebError {
    #[error("[INTERNAL_SERVER_ERROR] {0}")]
    CustomApiError(anyhow::Error),

    #[error("[NOT_FOUND] {0}")]
    NotFound(String),

    #[error("[BAD_REQUEST] {0}")]
    BadRequest(String),

    #[error("[UNAUTHORIZED] {0}")]
    Unauthorized(String),

    #[error("[FORBIDDEN] {0}")]
    Forbidden(String),

    #[error("[CONFLICT] {0}")]
    Conflict(String),

    #[error("[NOT_IMPLEMENTED] {0}")]
    NotImplemented(String),
}

#[derive(Debug, Serialize)]
pub struct ApiErrorDetail {
    error: String,
}

impl From<WebError> for ApiErrorDetail {
    fn from(value: WebError) -> Self {
        Self {
            error: value.to_string(),
        }
    }
}

impl WebError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::CustomApiError(..) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotFound(..) => StatusCode::NOT_FOUND,
            Self::BadRequest(..) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(..) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(..) => StatusCode::FORBIDDEN,
            Self::Conflict(..) => StatusCode::CONFLICT,
            Self::NotImplemented(..) => StatusCode::NOT_IMPLEMENTED,
        }
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!(error.msg = %self,error.details = ?self,"controller_error");
        (self.status_code(), Json(ApiErrorDetail::from(self))).into_response()
    }
}

pub type WebResult<T> = std::result::Result<T, WebError>;
