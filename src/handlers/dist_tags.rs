use crate::error::{WebError, WebResult};
use crate::handlers::publish::refresh_manifests;
use crate::middleware::auth::{AuthContext, check_scope_access, is_admin, validate_auth};
use crate::npm::split_scope_name;
use crate::npm::types::PublishResponse;
use crate::state::{AppState, LockOwner, UnlockGuard};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use std::collections::HashMap;

async fn ensure_tag_write_access(
    state: &AppState,
    auth: &AuthContext,
    fullname: &str,
    package_id: i64,
) -> WebResult<()> {
    if is_admin(&auth.user, &state.config.auth.admins) {
        return Ok(());
    }
    let (scope, _name) = split_scope_name(fullname);
    check_scope_access(
        scope,
        &state.config.auth.allow_scopes,
        state.config.auth.allow_publish_non_scope_package,
    )?;
    let is_maintainer = state
        .repo
        .is_maintainer(package_id, auth.user.id)
        .await
        .map_err(WebError::CustomApiError)?;
    if !is_maintainer {
        return Err(WebError::Forbidden(format!(
            "\"{}\" not authorized to modify {fullname}, please contact maintainers",
            auth.user.name
        )));
    }
    Ok(())
}

fn ensure_local_package(source: Option<&str>, fullname: &str) -> WebResult<()> {
    if let Some(s) = source {
        return Err(WebError::Forbidden(format!(
            "package {fullname} was synced from upstream ({s}), dist-tag mutation is not allowed"
        )));
    }
    Ok(())
}

fn lock_package<'a>(state: &'a AppState, fullname: &str) -> WebResult<UnlockGuard<'a>> {
    if !state.package_lock.try_lock(fullname, LockOwner::Publish) {
        let owner = state.package_lock.get_owner(fullname);
        return Err(WebError::Conflict(format!(
            "package {fullname} is currently being {}",
            owner.map(|o| o.to_string()).unwrap_or_default()
        )));
    }
    Ok(UnlockGuard::new(&state.package_lock, fullname.to_string()))
}

async fn load_tag_map(state: &AppState, package_id: i64) -> WebResult<HashMap<String, String>> {
    let tags = state
        .repo
        .list_tags(package_id)
        .await
        .map_err(WebError::CustomApiError)?;
    Ok(tags.into_iter().map(|t| (t.tag, t.version)).collect())
}

#[utoipa::path(
    get,
    tag = "registry",
    path = "/-/package/{fullname}/dist-tags",
    params(
        ("fullname" = String, Path, description = "Package full name, e.g. lodash or @babel/core"),
    ),
    responses(
        (status = OK, description = "Dist-tags map (tag -> version)", body = HashMap<String, String>),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn list_dist_tags(
    State(state): State<AppState>,
    Path(fullname): Path<String>,
) -> WebResult<Json<HashMap<String, String>>> {
    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let map = load_tag_map(&state, pkg.id).await?;
    Ok(Json(map))
}

#[utoipa::path(
    put,
    tag = "registry",
    path = "/-/package/{fullname}/dist-tags/{tag}",
    params(
        ("fullname" = String, Path, description = "Package full name"),
        ("tag" = String, Path, description = "Tag name"),
    ),
    request_body = String,
    responses(
        (status = OK, description = "Tag set", body = PublishResponse),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
        (status = CONFLICT, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn set_dist_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((fullname, tag)): Path<(String, String)>,
    Json(version): Json<String>,
) -> WebResult<Json<PublishResponse>> {
    let fullname = fullname.trim().to_string();
    let tag = tag.trim().to_string();
    let version = version.trim().to_string();

    if tag.is_empty() {
        return Err(WebError::BadRequest("tag is empty".to_string()));
    }
    if tag.len() > 214 {
        return Err(WebError::BadRequest("tag cannot exceed 214 characters".to_string()));
    }
    if semver::Version::parse(&version).is_err() {
        return Err(WebError::BadRequest(format!("invalid version: {version}")));
    }

    let auth = validate_auth(&state, &headers).await?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    ensure_tag_write_access(&state, &auth, &fullname, pkg.id).await?;
    ensure_local_package(pkg.source.as_deref(), &fullname)?;

    let _unlock = lock_package(&state, &fullname)?;

    let version_exists = state
        .repo
        .get_version(pkg.id, &version)
        .await
        .map_err(WebError::CustomApiError)?
        .is_some();
    if !version_exists {
        return Err(WebError::NotFound(format!(
            "{fullname}@{version} not found"
        )));
    }

    let mut tags = load_tag_map(&state, pkg.id).await?;
    if tags.get(&tag) == Some(&version) {
        return Ok(Json(PublishResponse {
            ok: true,
            rev: format!("{}-{}", pkg.id, version),
        }));
    }
    tags.insert(tag.clone(), version.clone());

    let full_manifest = refresh_manifests(
        &state,
        pkg.id,
        &fullname,
        pkg.description.as_deref(),
        &tags,
    )
    .await?;

    if let Some(idx) = &state.search {
        crate::search::upsert_search_document(&*state.repo, idx, pkg.id, &full_manifest).await;
    }

    log::info!(
        action = "dist_tag_set";
        "name={fullname} tag={tag} version={version} user={}",
        auth.user.name
    );

    Ok(Json(PublishResponse {
        ok: true,
        rev: format!("{}-{}", pkg.id, version),
    }))
}

#[utoipa::path(
    delete,
    tag = "registry",
    path = "/-/package/{fullname}/dist-tags/{tag}",
    params(
        ("fullname" = String, Path, description = "Package full name"),
        ("tag" = String, Path, description = "Tag name"),
    ),
    responses(
        (status = OK, description = "Tag removed", body = PublishResponse),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
        (status = CONFLICT, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn remove_dist_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((fullname, tag)): Path<(String, String)>,
) -> WebResult<Json<PublishResponse>> {
    let fullname = fullname.trim().to_string();
    let tag = tag.trim().to_string();

    if tag == "latest" {
        return Err(WebError::Forbidden(
            "Can't remove the \"latest\" tag".to_string(),
        ));
    }

    if tag.is_empty() {
        return Err(WebError::BadRequest("tag is empty".to_string()));
    }

    let auth = validate_auth(&state, &headers).await?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    ensure_tag_write_access(&state, &auth, &fullname, pkg.id).await?;
    ensure_local_package(pkg.source.as_deref(), &fullname)?;

    let _unlock = lock_package(&state, &fullname)?;

    let mut tags = load_tag_map(&state, pkg.id).await?;
    if tags.remove(&tag).is_none() {
        return Ok(Json(PublishResponse {
            ok: true,
            rev: format!("{}-{}", pkg.id, tag),
        }));
    }

    let full_manifest = refresh_manifests(
        &state,
        pkg.id,
        &fullname,
        pkg.description.as_deref(),
        &tags,
    )
    .await?;

    if let Some(idx) = &state.search {
        crate::search::upsert_search_document(&*state.repo, idx, pkg.id, &full_manifest).await;
    }

    log::info!(
        action = "dist_tag_rm";
        "name={fullname} tag={tag} user={}",
        auth.user.name
    );

    Ok(Json(PublishResponse {
        ok: true,
        rev: format!("{}-{}", pkg.id, tag),
    }))
}
