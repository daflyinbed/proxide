use axum::{Json, response::IntoResponse};
use reqwest::StatusCode;
use serde::Serialize;

#[derive(thiserror::Error, Debug)]
pub enum WebError {
    /// 501 Internal Server Error
    #[error("Internal Server Error:\n{0}")]
    CustomApiError(anyhow::Error),

    /// 404 Not Found
    #[error("Not found")]
    NotFound,

    /// 400 Bad Request
    #[error("Bad Request: {0}")]
    BadRequest(String),
}

#[derive(Debug, Serialize)]
pub struct ApiErrorDetail {
    detail: String,
}

impl From<WebError> for ApiErrorDetail {
    fn from(value: WebError) -> Self {
        Self {
            detail: value.to_string(),
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
            err @ Self::NotFound => {
                (StatusCode::NOT_FOUND, Json(ApiErrorDetail::from(err))).into_response()
            }
            err @ Self::BadRequest(..) => {
                (StatusCode::BAD_REQUEST, Json(ApiErrorDetail::from(err))).into_response()
            }
        }
    }
}

pub type WebResult<T> = std::result::Result<T, WebError>;
