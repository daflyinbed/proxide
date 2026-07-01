use crate::error::{WebError, WebResult};
use crate::middleware::auth::{check_scope_access, is_admin, validate_auth};
use crate::npm::split_scope_name;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
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
    Path(fullname): Path<String>,
) -> WebResult<Json<serde_json::Value>> {
    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::Forbidden("Forbidden".to_string()))?;

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

#[utoipa::path(
    get,
    tag = "auth",
    path = "/-/org/{username}/package",
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
    Path(username): Path<String>,
) -> WebResult<Json<serde_json::Value>> {
    let user = state
        .repo
        .get_user_by_name(&username)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("User \"{username}\" not found")))?;

    let pkgs = state
        .repo
        .list_packages_by_user_id(user.id)
        .await
        .map_err(WebError::CustomApiError)?;

    let mut res: BTreeMap<String, String> = BTreeMap::new();
    for pkg in pkgs {
        res.insert(pkg.name, "write".to_string());
    }
    Ok(Json(serde_json::to_value(res).unwrap()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct VisibilityResponse {
    public: bool,
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
    Path(fullname): Path<String>,
) -> WebResult<Json<VisibilityResponse>> {
    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::Forbidden("Forbidden".to_string()))?;

    let is_public = pkg.scope.is_none() || pkg.access == "public";
    Ok(Json(VisibilityResponse { public: is_public }))
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

    let auth = validate_auth(&state, &headers).await?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    if pkg.source.is_some() {
        return Err(WebError::Forbidden(format!(
            "package {fullname} was synced from upstream, access mutation is not allowed"
        )));
    }

    let (scope, _name) = split_scope_name(&fullname);
    if !is_admin(&auth.user, &state.config.auth.admins) {
        check_scope_access(
            scope,
            &state.config.auth.allow_scopes,
            state.config.auth.allow_publish_non_scope_package,
        )?;
        let is_maintainer = state
            .repo
            .is_maintainer(pkg.id, auth.user.id)
            .await
            .map_err(WebError::CustomApiError)?;
        if !is_maintainer {
            return Err(WebError::Forbidden(format!(
                "\"{}\" not authorized to modify {fullname}, please contact maintainers",
                auth.user.name
            )));
        }
    }

    state
        .repo
        .set_package_access(pkg.id, normalized)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(
        action = "set_access";
        "name={fullname} access={normalized} user={}",
        auth.user.name
    );

    Ok(Json(AccessResponse { ok: true }))
}
