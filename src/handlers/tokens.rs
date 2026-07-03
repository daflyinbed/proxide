use crate::error::{WebError, WebResult};
use crate::middleware::auth::{RequireAnyAuth, RequireAuth, hash_token, verify_password};
use crate::npm::types::{
    OkResponse, TokenCreateRequest, TokenListResponse, TokenObject, WhoAmIResponse,
};
use crate::repository::TokenRow;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

fn mask_token(key: &str) -> String {
    let prefix = key.chars().take(7).collect::<String>();
    format!("{prefix}...")
}

fn parse_cidrs(raw: &Option<String>) -> Vec<String> {
    raw.as_ref()
        .and_then(|v| serde_json::from_str::<Vec<String>>(v).ok())
        .unwrap_or_default()
}

fn to_iso(dt: chrono::NaiveDateTime) -> String {
    dt.and_utc().to_rfc3339()
}

fn to_iso_opt(dt: Option<chrono::NaiveDateTime>) -> Option<String> {
    dt.map(to_iso)
}

fn token_object(row: &TokenRow, masked: bool) -> TokenObject {
    TokenObject {
        token: if masked {
            mask_token(&row.token_key)
        } else {
            row.token_key.clone()
        },
        key: row.token_key.clone(),
        cidr_whitelist: parse_cidrs(&row.cidr_whitelist),
        readonly: row.is_readonly,
        created: Some(to_iso(row.created_at)),
        updated: Some(to_iso(row.updated_at)),
        last_used_at: to_iso_opt(row.last_used_at),
    }
}

#[utoipa::path(
    get,
    tag = "auth",
    path = "/-/whoami",
    responses(
        (status = OK, description = "Current authenticated user", body = WhoAmIResponse),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn whoami(
    RequireAnyAuth(auth): RequireAnyAuth,
) -> WebResult<Json<WhoAmIResponse>> {
    Ok(Json(WhoAmIResponse {
        username: auth.user.name,
    }))
}

#[utoipa::path(
    delete,
    tag = "auth",
    path = "/-/user/token/{token}",
    params(
        ("token" = String, Path, description = "The plaintext token to revoke (must match the bearer token)"),
    ),
    responses(
        (status = OK, description = "Logged out", body = OkResponse),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn logout(
    State(state): State<AppState>,
    RequireAnyAuth(auth): RequireAnyAuth,
    Path(token): Path<String>,
) -> WebResult<(StatusCode, Json<OkResponse>)> {
    let path_key = hash_token(&token);
    if path_key != auth.token.token_key {
        return Err(WebError::BadRequest("invalid token".to_string()));
    }

    state
        .repo
        .delete_token_by_id(auth.token.id)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(action = "logout"; "user={}", auth.user.name);

    Ok((StatusCode::OK, Json(OkResponse { ok: true })))
}

#[utoipa::path(
    get,
    tag = "auth",
    path = "/-/npm/v1/tokens",
    responses(
        (status = OK, description = "List of tokens for the authenticated user", body = TokenListResponse),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn list_tokens(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
) -> WebResult<Json<TokenListResponse>> {

    let rows = state
        .repo
        .list_tokens_by_user(auth.user.id)
        .await
        .map_err(WebError::CustomApiError)?;

    let total = rows.len() as u64;
    let objects = rows.iter().map(|r| token_object(r, true)).collect();

    Ok(Json(TokenListResponse {
        objects,
        total,
        urls: serde_json::json!({}),
    }))
}

#[utoipa::path(
    post,
    tag = "auth",
    path = "/-/npm/v1/tokens",
    request_body = TokenCreateRequest,
    responses(
        (status = OK, description = "Token created", body = TokenObject),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn create_token(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Json(body): Json<TokenCreateRequest>,
) -> WebResult<Json<TokenObject>> {

    let (Some(salt), Some(integrity)) = (auth.user.password_salt.as_deref(), auth.user.password_integrity.as_deref())
        else { return Err(WebError::Forbidden("Password verification unavailable for this account".to_string())); };

    let password = body.password.as_deref().unwrap_or("");
    if !verify_password(salt, integrity, password) {
        return Err(WebError::Unauthorized("Invalid password".to_string()));
    }

    let cidr = body.cidr_whitelist.unwrap_or_default();
    if cidr.len() > 10 {
        return Err(WebError::BadRequest(
            "cidr_whitelist can contain at most 10 entries".to_string(),
        ));
    }
    let cidr_json = if cidr.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&cidr).map_err(|e| WebError::CustomApiError(e.into()))?)
    };

    let raw_token = uuid::Uuid::new_v4().to_string();
    let token_key = hash_token(&raw_token);
    let name = if body.automation {
        "automation"
    } else if body.readonly {
        "read-only"
    } else {
        "publish"
    };

    state
        .repo
        .create_token(
            &token_key,
            name,
            auth.user.id,
            body.readonly,
            None,
            cidr_json.as_deref(),
            None,
        )
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(action = "token_create"; "user={}", auth.user.name);

    Ok(Json(TokenObject {
        token: raw_token,
        key: token_key.clone(),
        cidr_whitelist: cidr,
        readonly: body.readonly,
        created: Some(to_iso(chrono::Utc::now().naive_utc())),
        updated: Some(to_iso(chrono::Utc::now().naive_utc())),
        last_used_at: None,
    }))
}

#[utoipa::path(
    delete,
    tag = "auth",
    path = "/-/npm/v1/tokens/token/{key}",
    params(
        ("key" = String, Path, description = "The token key (hash) to revoke"),
    ),
    responses(
        (status = OK, description = "Token revoked", body = OkResponse),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn revoke_token(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path(key): Path<String>,
) -> WebResult<(StatusCode, Json<OkResponse>)> {

    let token = state
        .repo
        .find_token_by_key(&key)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound("token not found".to_string()))?;

    if token.user_id != auth.user.id {
        return Err(WebError::Forbidden(
            "cannot revoke a token owned by another user".to_string(),
        ));
    }

    state
        .repo
        .delete_token_by_id(token.id)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(action = "token_revoke"; "user={}", auth.user.name);

    Ok((StatusCode::OK, Json(OkResponse { ok: true })))
}
