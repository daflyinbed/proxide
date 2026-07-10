use crate::error::{WebError, WebResult};
use crate::state::{AppState, LoginSession};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const SESSION_TTL_SECS: i64 = 300;

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebLoginResponse {
    pub login_url: String,
    pub done_url: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct LoginRequestBody {
    pub hostname: Option<String>,
}

#[utoipa::path(
    post,
    tag = "auth",
    path = "/-/v1/login",
    request_body = LoginRequestBody,
    responses(
        (status = OK, description = "Login session created", body = WebLoginResponse),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn init_login(
    State(state): State<AppState>,
    Json(_body): Json<LoginRequestBody>,
) -> WebResult<Json<WebLoginResponse>> {
    if !state.config.auth.is_cas_enabled() {
        return Err(WebError::BadRequest("Web login is not enabled".to_string()));
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let expired_at = chrono::Utc::now().naive_utc() + chrono::Duration::seconds(SESSION_TTL_SECS);

    state.login_sessions.insert(
        session_id.clone(),
        LoginSession {
            token: None,
            user_id: None,
            expired_at,
        },
    );

    let root_url = &state.config.server.root_url;
    let cas_url = &state.config.auth.cas_url;
    let service_url = format!("{root_url}/api/auth/cas/callback/session/{session_id}");

    Ok(Json(WebLoginResponse {
        login_url: format!("{cas_url}/cas/login?service={service_url}"),
        done_url: format!("{root_url}/npm/-/v1/login/done/session/{session_id}"),
    }))
}

#[utoipa::path(
    get,
    tag = "auth",
    path = "/-/v1/login/done/session/{sessionId}",
    params(
        ("sessionId" = String, Path, description = "Login session ID"),
    ),
    responses(
        (status = OK, description = "Login completed, returns token"),
        (status = ACCEPTED, description = "Login still processing"),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn poll_done(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> WebResult<Response> {
    let session = state
        .login_sessions
        .get(&session_id)
        .ok_or_else(|| WebError::Unauthorized("Session not found".to_string()))?;

    if session.expired_at < chrono::Utc::now().naive_utc() {
        state.login_sessions.remove(&session_id);
        return Err(WebError::Unauthorized("Session expired".to_string()));
    }

    match session.token {
        None => Ok((
            StatusCode::ACCEPTED,
            [("retry-after", "5")],
            Json(serde_json::json!({ "message": "processing" })),
        )
            .into_response()),
        Some(token) => {
            state.login_sessions.remove(&session_id);
            Ok(Json(serde_json::json!({ "token": token })).into_response())
        }
    }
}
