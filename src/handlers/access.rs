use crate::error::{WebError, WebResult};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use std::collections::BTreeMap;

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
