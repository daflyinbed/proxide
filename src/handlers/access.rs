use crate::error::{WebError, WebResult};
use crate::handlers::orgs::require_org_manager;
use crate::middleware::auth::{
    OptionalAuth, RequireAuth, ensure_package_readable, ensure_package_readable_with_auth,
    ensure_package_write_access, is_admin,
};
use crate::npm::types::Packument;
use crate::state::{AppState, LockOwner, UnlockGuard};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

#[utoipa::path(
    get,
    tag = "registry",
    path = "/-/package/{fullname}/collaborators",
    params(
        ("fullname" = String, Path, description = "Package full name, e.g. lodash or @babel/core"),
    ),
    responses(
        (status = OK, description = "Map of collaborator name to access level", body = serde_json::Value),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn list_collaborators(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(fullname): Path<String>,
) -> WebResult<Json<serde_json::Value>> {
    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    ensure_package_readable(&state, &headers, &pkg).await?;

    let maintainers = state
        .repo
        .list_maintainers(pkg.id)
        .await
        .map_err(WebError::CustomApiError)?;

    let mut res: BTreeMap<String, String> = BTreeMap::new();
    for m in maintainers {
        res.insert(m.name, "write".to_string());
    }
    Ok(Json(serde_json::to_value(res).unwrap()))
}

pub async fn build_user_packages(
    state: &AppState,
    auth: Option<&crate::middleware::auth::AuthContext>,
    username: &str,
) -> WebResult<Json<serde_json::Value>> {
    let user = state
        .repo
        .get_user_by_name(username)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("User \"{username}\" not found")))?;

    let is_admin_viewer = auth.is_some_and(|a| is_admin(&a.user, &state.config.auth.admins));
    let is_self = auth.is_some_and(|a| a.user.id == user.id);

    let pkgs = if is_self || is_admin_viewer {
        state
            .repo
            .list_packages_by_user_id(user.id)
            .await
            .map_err(WebError::CustomApiError)?
    } else if let Some(a) = auth {
        state
            .repo
            .list_packages_by_user_id_readable(user.id, a.user.id)
            .await
            .map_err(WebError::CustomApiError)?
    } else {
        state
            .repo
            .list_packages_by_user_id_readable(user.id, 0)
            .await
            .map_err(WebError::CustomApiError)?
    };

    let mut res: BTreeMap<String, String> = BTreeMap::new();
    for pkg in pkgs {
        res.insert(pkg.name, "write".to_string());
    }
    Ok(Json(serde_json::to_value(res).unwrap()))
}

#[utoipa::path(
    get,
    tag = "auth",
    path = "/-/user/{username}/package",
    params(
        ("username" = String, Path, description = "User name"),
    ),
    responses(
        (status = OK, description = "Map of package full name to access level", body = serde_json::Value),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn list_packages_by_user(
    State(state): State<AppState>,
    OptionalAuth(auth): OptionalAuth,
    Path(username): Path<String>,
) -> WebResult<Json<serde_json::Value>> {
    build_user_packages(&state, auth.as_ref(), &username).await
}

#[utoipa::path(
    get,
    tag = "registry",
    path = "/-/package/{fullname}/visibility",
    params(
        ("fullname" = String, Path, description = "Package full name, e.g. lodash or @babel/core"),
    ),
    responses(
        (status = OK, description = "Package visibility", body = VisibilityResponse),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn get_visibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(fullname): Path<String>,
) -> WebResult<Json<VisibilityResponse>> {
    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    ensure_package_readable(&state, &headers, &pkg).await?;

    Ok(Json(VisibilityResponse {
        public: pkg.is_public(),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct VisibilityResponse {
    pub public: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AccessRequest {
    #[serde(default)]
    pub access: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AccessResponse {
    ok: bool,
}

#[utoipa::path(
    post,
    tag = "registry",
    path = "/-/package/{fullname}/access",
    params(
        ("fullname" = String, Path, description = "Package full name, e.g. lodash or @babel/core"),
    ),
    request_body = AccessRequest,
    responses(
        (status = OK, description = "Access updated", body = AccessResponse),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn set_access(
    State(state): State<AppState>,
    headers: HeaderMap,
    RequireAuth(auth): RequireAuth,
    Path(fullname): Path<String>,
    Json(body): Json<AccessRequest>,
) -> WebResult<Json<AccessResponse>> {
    let fullname = fullname.trim().to_string();
    let access = body
        .access
        .as_deref()
        .ok_or_else(|| WebError::BadRequest("missing access".to_string()))?;

    let normalized = match access {
        "public" => "public",
        "restricted" | "private" => "restricted",
        other => {
            return Err(WebError::BadRequest(format!(
                "invalid access: {other}"
            )))
        }
    };

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    ensure_package_readable(&state, &headers, &pkg).await?;

    if pkg.source.is_some() {
        return Err(WebError::Forbidden(format!(
            "package {fullname} was synced from upstream, access mutation is not allowed"
        )));
    }

    if pkg.scope.is_none() && normalized != "public" {
        return Err(WebError::BadRequest(
            "unscoped packages are always public; restricted access requires a scope".to_string(),
        ));
    }

    ensure_package_write_access(&state, &auth, &fullname, pkg.id).await?;

    if !state
        .package_lock
        .try_lock(&fullname, LockOwner::Access)
    {
        let owner = state
            .package_lock
            .get_owner(&fullname)
            .map(|o| o.to_string())
            .unwrap_or_else(|| "modified by another request".to_string());
        return Err(WebError::Conflict(format!(
            "package {fullname} is currently being {owner}"
        )));
    }
    let _unlock = UnlockGuard::new(&state.package_lock, fullname.clone());

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let old_access = pkg.access.as_str();
    let pkg_id = pkg.id;
    let full_dist_id = pkg.full_dist_id;

    state
        .repo
        .set_package_access(pkg_id, normalized)
        .await
        .map_err(WebError::CustomApiError)?;

    if let Some(idx) = &state.search
        && let Some(full_dist_id) = full_dist_id
    {
        let reindex_result: Result<(), anyhow::Error> = async {
            let (bytes, _) = state.repo.get_content(full_dist_id).await?;
            let packument: Packument = serde_json::from_slice(&bytes)?;
            crate::search::upsert_search_document_and_wait(
                &*state.repo,
                idx,
                pkg_id,
                normalized,
                &packument,
            )
            .await?;
            Ok(())
        }
        .await;
        if let Err(e) = reindex_result {
            if old_access != normalized {
                if let Err(rb_err) = state.repo.set_package_access(pkg_id, old_access).await {
                    log::error!(
                        action = "set_access_rollback_failed";
                        "name={fullname} failed to roll back access from {normalized} to {old_access}: {rb_err}"
                    );
                }
            }
            return Err(WebError::CustomApiError(e));
        }
    }

    log::info!(
        action = "set_access";
        "name={fullname} access={normalized} user={}",
        auth.user.name
    );

    Ok(Json(AccessResponse { ok: true }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TeamPackageRequest {
    pub package: String,
    #[serde(default)]
    pub permissions: Option<String>,
}

async fn resolve_team_for_handler(
    state: &AppState,
    scope: &str,
    team: &str,
) -> WebResult<(crate::repository::OrganizationRow, crate::repository::TeamRow)> {
    let org = state
        .repo
        .get_org_by_name(scope)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Organization \"{scope}\" not found")))?;
    let team_row = state
        .repo
        .get_team_by_org_name(org.id, team)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Team \"{scope}:{team}\" not found")))?;
    Ok((org, team_row))
}

async fn require_team_pkg_manager(
    state: &AppState,
    auth: &crate::middleware::auth::AuthContext,
    scope: &str,
    package_fullname: &str,
    package_id: i64,
) -> WebResult<()> {
    if require_org_manager(state, auth, scope).await.is_ok() {
        return Ok(());
    }
    if state
        .repo
        .is_maintainer(package_id, auth.user.id)
        .await
        .map_err(WebError::CustomApiError)?
    {
        return Ok(());
    }
    Err(WebError::Forbidden(format!(
        "\"{}\" is not an org manager of \"{scope}\" nor a maintainer of \"{package_fullname}\"",
        auth.user.name
    )))
}

#[utoipa::path(
    get,
    tag = "registry",
    path = "/-/team/{scope}/{team}/package",
    params(
        ("scope" = String, Path, description = "Organization scope (without @)"),
        ("team" = String, Path, description = "Team name"),
    ),
    responses(
        (status = OK, description = "Map of package name to permission", body = serde_json::Value),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn list_team_packages(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path((scope, team)): Path<(String, String)>,
) -> WebResult<Json<serde_json::Value>> {
    let (_org, team_row) = resolve_team_for_handler(&state, &scope, &team).await?;
    let pkgs = state
        .repo
        .list_packages_for_team(team_row.id)
        .await
        .map_err(WebError::CustomApiError)?;

    let mut res: BTreeMap<String, String> = BTreeMap::new();
    for (pkg, perm) in pkgs {
        let readable = if pkg.is_public() {
            true
        } else {
            ensure_package_readable_with_auth(&state, &auth, &pkg).await.is_ok()
        };
        if readable {
            res.insert(pkg.name, perm);
        }
    }
    Ok(Json(serde_json::to_value(res).unwrap()))
}

#[utoipa::path(
    put,
    tag = "registry",
    path = "/-/team/{scope}/{team}/package",
    params(
        ("scope" = String, Path, description = "Organization scope (without @)"),
        ("team" = String, Path, description = "Team name"),
    ),
    request_body = TeamPackageRequest,
    responses(
        (status = OK, description = "Permission granted", body = AccessResponse),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn grant_team_package(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path((scope, team)): Path<(String, String)>,
    Json(body): Json<TeamPackageRequest>,
) -> WebResult<Json<AccessResponse>> {
    let permission = match body.permissions.as_deref() {
        Some("read-only") | Some("read") => "read",
        Some("read-write") | Some("write") => "write",
        Some(other) => {
            return Err(WebError::BadRequest(format!(
                "invalid permission: \"{other}\", must be one of: read-only, read-write"
            )));
        }
        None => {
            return Err(WebError::BadRequest(
                "permissions field is required".to_string(),
            ));
        }
    };

    let (_org, team_row) = resolve_team_for_handler(&state, &scope, &team).await?;

    let pkg = state
        .repo
        .get_package_by_name(&body.package)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Package \"{}\" not found", body.package)))?;

    if pkg.source.is_some() {
        return Err(WebError::Forbidden(format!(
            "package \"{}\" is synced from upstream, team permissions are not applicable",
            body.package
        )));
    }

    if pkg.scope.as_deref() != Some(scope.as_str()) {
        return Err(WebError::Forbidden(format!(
            "package \"{}\" does not belong to scope \"{scope}\"",
            body.package
        )));
    }

    require_team_pkg_manager(&state, &auth, &scope, &body.package, pkg.id).await?;

    state
        .repo
        .grant_team_permission(pkg.id, team_row.id, permission)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(
        action = "team_grant_package";
        "scope={scope} team={team} pkg={} perm={permission} actor={}",
        body.package, auth.user.name
    );

    Ok(Json(AccessResponse { ok: true }))
}

#[utoipa::path(
    delete,
    tag = "registry",
    path = "/-/team/{scope}/{team}/package",
    params(
        ("scope" = String, Path, description = "Organization scope (without @)"),
        ("team" = String, Path, description = "Team name"),
    ),
    request_body = TeamPackageRequest,
    responses(
        (status = NO_CONTENT, description = "Permission revoked"),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn revoke_team_package(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path((scope, team)): Path<(String, String)>,
    Json(body): Json<TeamPackageRequest>,
) -> WebResult<StatusCode> {
    let (_org, team_row) = resolve_team_for_handler(&state, &scope, &team).await?;

    let pkg = state
        .repo
        .get_package_by_name(&body.package)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Package \"{}\" not found", body.package)))?;

    if pkg.source.is_some() {
        return Err(WebError::Forbidden(format!(
            "package \"{}\" is synced from upstream, team permissions are not applicable",
            body.package
        )));
    }

    if pkg.scope.as_deref() != Some(scope.as_str()) {
        return Err(WebError::Forbidden(format!(
            "package \"{}\" does not belong to scope \"{scope}\"",
            body.package
        )));
    }

    require_team_pkg_manager(&state, &auth, &scope, &body.package, pkg.id).await?;

    state
        .repo
        .revoke_team_permission(pkg.id, team_row.id)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(
        action = "team_revoke_package";
        "scope={scope} team={team} pkg={} actor={}",
        body.package, auth.user.name
    );

    Ok(StatusCode::NO_CONTENT)
}
