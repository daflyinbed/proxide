use axum::{response::IntoResponse, Json};
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

impl IntoResponse for WebError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!(error.msg = %self,error.details = ?self,"controller_error");
        match self {
            err @ Self::CustomApiError(..) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorDetail::from(err)),
            )
                .into_response(),
            err @ Self::NotFound(..) => {
                (StatusCode::NOT_FOUND, Json(ApiErrorDetail::from(err))).into_response()
            }
            err @ Self::BadRequest(..) => {
                (StatusCode::BAD_REQUEST, Json(ApiErrorDetail::from(err))).into_response()
            }
            err @ Self::Unauthorized(..) => {
                (StatusCode::UNAUTHORIZED, Json(ApiErrorDetail::from(err))).into_response()
            }
            err @ Self::Forbidden(..) => {
                (StatusCode::FORBIDDEN, Json(ApiErrorDetail::from(err))).into_response()
            }
            err @ Self::Conflict(..) => {
                (StatusCode::CONFLICT, Json(ApiErrorDetail::from(err))).into_response()
            }
        }
    }
}

pub type WebResult<T> = std::result::Result<T, WebError>;
