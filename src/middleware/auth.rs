use crate::error::{WebError, WebResult};
use crate::repository::{TokenRow, UserRow};
use crate::state::AppState;
use axum::http::HeaderMap;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let auth = headers.get("authorization")?.to_str().ok()?;
    let token = auth.strip_prefix("Bearer ")?;
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn hash_token(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub async fn validate_auth(state: &AppState, headers: &HeaderMap) -> WebResult<AuthContext> {
    let ctx = validate_auth_any(state, headers).await?;

    if ctx.token.is_readonly {
        return Err(WebError::Forbidden(
            "Read-only token cannot perform this operation".to_string(),
        ));
    }

    Ok(ctx)
}

pub async fn validate_auth_any(state: &AppState, headers: &HeaderMap) -> WebResult<AuthContext> {
    let raw_token = extract_bearer_token(headers)
        .ok_or_else(|| WebError::Unauthorized("Login first".to_string()))?;

    let token_key = hash_token(&raw_token);

    let token_row = state
        .repo
        .find_token_by_key(&token_key)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::Unauthorized("Invalid token".to_string()))?;

    // TODO: enforce cidr_whitelist — resolve client IP (X-Forwarded-For / ConnectInfo)
    // and match against token_row.cidr_whitelist. Mirrors cnpmcore which also stores
    // the field without enforcing; enabling here would be an enhancement over both.
    if let Some(expired) = token_row.expired_at
        && expired < chrono::Utc::now().naive_utc()
    {
        return Err(WebError::Unauthorized("Token expired".to_string()));
    }

    let user = state
        .repo
        .get_user_by_id(token_row.user_id)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::Unauthorized("User not found".to_string()))?;

    if let Err(e) = state.repo.touch_token(token_row.id).await {
        tracing::warn!("failed to update token last_used_at: {e:#}");
    }

    Ok(AuthContext {
        user,
        token: token_row,
    })
}

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user: UserRow,
    pub token: TokenRow,
}

pub fn is_admin(user: &UserRow, admins: &[String]) -> bool {
    admins.contains(&user.name)
}

pub fn check_scope_access(
    scope: Option<&str>,
    allow_scopes: &[String],
    allow_publish_non_scope: bool,
) -> WebResult<()> {
    if allow_publish_non_scope {
        return Ok(());
    }
    let Some(scope) = scope else {
        return Err(WebError::Forbidden(format!(
            "Package scope required, legal scopes: \"{}\"",
            allow_scopes.join(", ")
        )));
    };
    if !allow_scopes.iter().any(|s| s == scope) {
        return Err(WebError::Forbidden(format!(
            "Scope \"{}\" not match legal scopes: \"{}\"",
            scope,
            allow_scopes.join(", ")
        )));
    }
    Ok(())
}

pub fn generate_salt() -> String {
    let mut buf = [0u8; 30];
    getrandom::fill(&mut buf).expect("failed to generate random salt");
    hex::encode(buf)
}

pub fn compute_password_integrity(salt: &str, password: &str) -> String {
    let mut hasher = sha2::Sha512::new();
    hasher.update(salt.as_bytes());
    hasher.update(password.as_bytes());
    let hash = hasher.finalize();
    format!("sha512-{}", BASE64.encode(hash))
}

pub fn verify_password(salt: &str, integrity: &str, password: &str) -> bool {
    compute_password_integrity(salt, password) == integrity
}
