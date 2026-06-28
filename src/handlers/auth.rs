use crate::error::{WebError, WebResult};
use crate::middleware::auth::{
    compute_password_integrity, generate_salt, hash_token, verify_password,
};
use crate::npm::types::{LoginPayload, LoginResponse};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

#[utoipa::path(
    put,
    tag = "auth",
    path = "/npm/-/user/org.couchdb.user:{name}",
    request_body = LoginPayload,
    params(
        ("name" = String, Path, description = "CouchDB user name"),
    ),
    responses(
        (status = CREATED, description = "Login successful", body = LoginResponse),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = INTERNAL_SERVER_ERROR, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn login(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(payload): Json<LoginPayload>,
) -> WebResult<(StatusCode, Json<LoginResponse>)> {
    if state.config.auth.is_cas_enabled() {
        return Err(WebError::Forbidden(
            "Legacy login is disabled, please use `npm login` for web authentication".to_string(),
        ));
    }

    if payload.name != name {
        return Err(WebError::BadRequest(
            "Name in URL does not match name in body".to_string(),
        ));
    }

    if payload.name.is_empty() || payload.password.is_empty() {
        return Err(WebError::BadRequest(
            "Username and password are required".to_string(),
        ));
    }

    let user = state
        .repo
        .get_user_by_name(&payload.name)
        .await
        .map_err(WebError::CustomApiError)?;

    let user_id = if let Some(u) = user {
        let (salt, integrity) = u
            .password_salt
            .as_deref()
            .zip(u.password_integrity.as_deref())
            .ok_or_else(|| WebError::Unauthorized("Please use CAS login".to_string()))?;

        if !verify_password(salt, integrity, &payload.password) {
            return Err(WebError::Unauthorized("Invalid password".to_string()));
        }

        u.id
    } else {
        let salt = generate_salt();
        let integrity = compute_password_integrity(&salt, &payload.password);
        state
            .repo
            .create_user(
                &payload.name,
                payload.email.as_deref(),
                Some(&salt),
                Some(&integrity),
            )
            .await
            .map_err(WebError::CustomApiError)?
    };

    let raw_token = uuid::Uuid::new_v4().to_string();
    let token_key = hash_token(&raw_token);

    state
        .repo
        .create_token(&token_key, "default", user_id, false, None, None)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(action = "login"; "user={}", payload.name);

    Ok((
        StatusCode::CREATED,
        Json(LoginResponse {
            ok: true,
            id: "org.couchdb.user:undefined".to_string(),
            rev: "_we_dont_use_revs_any_more".to_string(),
            token: raw_token,
        }),
    ))
}
