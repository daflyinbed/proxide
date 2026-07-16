use crate::error::{WebError, WebResult};
use crate::repository::{PackageRow, TokenRow, UserRow};
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::HeaderMap;
use axum::http::request::Parts;
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

pub async fn ensure_package_readable(
    state: &AppState,
    headers: &HeaderMap,
    pkg: &PackageRow,
) -> WebResult<()> {
    if pkg.is_public() {
        return Ok(());
    }
    match validate_auth_any(state, headers).await {
        Ok(auth) => ensure_package_readable_with_auth(state, &auth, pkg).await,
        Err(WebError::Unauthorized(_)) => {
            Err(WebError::NotFound(format!("{} not found", pkg.name)))
        }
        Err(e) => Err(e),
    }
}

pub async fn ensure_package_readable_with_auth(
    state: &AppState,
    auth: &AuthContext,
    pkg: &PackageRow,
) -> WebResult<()> {
    if pkg.is_public() {
        return Ok(());
    }
    if is_admin(&auth.user, &state.config.auth.admins) {
        return Ok(());
    }
    if state
        .repo
        .is_maintainer(pkg.id, auth.user.id)
        .await
        .map_err(WebError::CustomApiError)?
    {
        return Ok(());
    }
    if pkg.source.is_none()
        && state
            .repo
            .user_has_team_access(pkg.id, auth.user.id, "read")
            .await
            .map_err(WebError::CustomApiError)?
    {
        return Ok(());
    }
    if pkg.source.is_none()
        && let Some(scope) = &pkg.scope
        && state
            .repo
            .user_is_org_manager_for_scope(scope, auth.user.id)
            .await
            .map_err(WebError::CustomApiError)?
    {
        return Ok(());
    }
    Err(WebError::NotFound(format!("{} not found", pkg.name)))
}

pub async fn ensure_package_write_access(
    state: &AppState,
    auth: &AuthContext,
    pkg: &PackageRow,
) -> WebResult<()> {
    if is_admin(&auth.user, &state.config.auth.admins) {
        return Ok(());
    }
    let scope = pkg.scope.as_deref();
    if pkg.source.is_none()
        && let Some(scope) = scope
        && let Some(org) = state
            .repo
            .get_org_by_name(scope)
            .await
            .map_err(WebError::CustomApiError)?
        && state
            .repo
            .get_org_member(org.id, auth.user.id)
            .await
            .map_err(WebError::CustomApiError)?
            .is_some()
    {
        return Ok(());
    }
    if pkg.source.is_none()
        && let Some(scope) = scope
        && state
            .repo
            .get_org_by_name(scope)
            .await
            .map_err(WebError::CustomApiError)?
            .is_some()
    {
        return Err(WebError::Forbidden(format!(
            "\"{}\" is not a member of organization \"{scope}\"",
            auth.user.name
        )));
    }
    check_scope_access(
        scope,
        &state.config.auth.allow_scopes,
        state.config.auth.allow_publish_non_scope_package,
    )?;
    if state
        .repo
        .is_maintainer(pkg.id, auth.user.id)
        .await
        .map_err(WebError::CustomApiError)?
    {
        return Ok(());
    }
    if pkg.source.is_none()
        && let Some(scope) = scope
        && state
            .repo
            .user_is_org_manager_for_scope(scope, auth.user.id)
            .await
            .map_err(WebError::CustomApiError)?
    {
        return Ok(());
    }
    if pkg.source.is_none()
        && state
            .repo
            .user_has_team_access(pkg.id, auth.user.id, "write")
            .await
            .map_err(WebError::CustomApiError)?
    {
        return Ok(());
    }
    Err(WebError::Forbidden(format!(
        "\"{}\" not authorized to modify {}, please contact maintainers",
        auth.user.name, pkg.name
    )))
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

pub struct RequireAuth(pub AuthContext);

impl FromRequestParts<AppState> for RequireAuth {
    type Rejection = WebError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        Ok(Self(validate_auth(state, &parts.headers).await?))
    }
}

pub struct RequireAnyAuth(pub AuthContext);

impl FromRequestParts<AppState> for RequireAnyAuth {
    type Rejection = WebError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        Ok(Self(validate_auth_any(state, &parts.headers).await?))
    }
}

pub struct OptionalAuth(pub Option<AuthContext>);

impl FromRequestParts<AppState> for OptionalAuth {
    type Rejection = WebError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        match validate_auth_any(state, &parts.headers).await {
            Ok(auth) => Ok(Self(Some(auth))),
            Err(WebError::Unauthorized(_)) => Ok(Self(None)),
            Err(e) => Err(e),
        }
    }
}
