use crate::error::{WebError, WebResult};
use crate::middleware::auth::validate_auth_any;
use crate::npm::types::UserProfile;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;

fn to_iso(dt: chrono::NaiveDateTime) -> String {
    dt.and_utc().to_rfc3339()
}

#[utoipa::path(
    get,
    tag = "auth",
    path = "/-/npm/v1/user",
    responses(
        (status = OK, description = "Current user profile", body = UserProfile),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn get_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> WebResult<Json<UserProfile>> {
    let auth = validate_auth_any(&state, &headers).await?;
    let created = to_iso(auth.user.created_at);
    Ok(Json(UserProfile {
        name: auth.user.name,
        email: auth.user.email,
        email_verified: false,
        created: created.clone(),
        updated: created,
    }))
}

#[utoipa::path(
    post,
    tag = "auth",
    path = "/-/npm/v1/user",
    responses(
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail, description = "Profile updates are not allowed"),
    ),
)]
pub async fn update_profile() -> WebResult<Json<serde_json::Value>> {
    Err(WebError::Forbidden(
        "npm profile set is not allowed".to_string(),
    ))
}
