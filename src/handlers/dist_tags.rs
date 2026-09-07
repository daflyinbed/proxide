use crate::error::{WebError, WebResult};
use crate::handlers::publish::{
    ManifestCandidateParams, ManifestCommitChanges, commit_manifest_candidate,
    prepare_manifest_candidate,
};
use crate::handlers::{ensure_local_package, load_tag_map, lock_package};
use crate::middleware::auth::{
    RequireAuth, ensure_package_readable, ensure_package_readable_with_auth,
    ensure_package_write_access,
};
use crate::npm::types::PublishResponse;
use crate::state::{AppState, LockOwner};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use std::collections::HashMap;

const MAX_TAG_LEN: usize = 214;

pub(crate) fn validate_dist_tag(tag: &str) -> WebResult<()> {
    if tag.is_empty() {
        return Err(WebError::BadRequest("tag is empty".to_string()));
    }
    if tag.len() > MAX_TAG_LEN {
        return Err(WebError::BadRequest(
            "tag cannot exceed 214 characters".to_string(),
        ));
    }
    if semver::VersionReq::parse(tag).is_ok() {
        return Err(WebError::BadRequest(format!(
            "tag \"{tag}\" must not be a valid semver range"
        )));
    }
    Ok(())
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
    headers: HeaderMap,
    Path(fullname): Path<String>,
) -> WebResult<Json<HashMap<String, String>>> {
    let fullname = fullname.trim();
    let pkg = state
        .repo
        .get_package_by_name(fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    ensure_package_readable(&state, &headers, &pkg).await?;

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
    RequireAuth(auth): RequireAuth,
    Path((fullname, tag)): Path<(String, String)>,
    Json(version): Json<String>,
) -> WebResult<Json<PublishResponse>> {
    let fullname = fullname.trim().to_string();
    let tag = tag.trim().to_string();
    let version = version.trim().to_string();

    validate_dist_tag(&tag)?;
    if semver::Version::parse(&version).is_err() {
        return Err(WebError::BadRequest(format!("invalid version: {version}")));
    }

    let _unlock = lock_package(&state, &fullname, LockOwner::Publish)?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;
    ensure_package_readable_with_auth(&state, &auth, &pkg).await?;
    ensure_package_write_access(&state, &auth, &pkg).await?;
    ensure_local_package(pkg.source.as_deref(), &fullname)?;

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

    let manifests = prepare_manifest_candidate(
        &state,
        ManifestCandidateParams {
            package: Some(&pkg),
            fullname: &fullname,
            description: pkg.description.as_deref(),
            dist_tags: &tags,
            added_version: None,
            removed_version: None,
            maintainers: None,
        },
    )
    .await?;
    commit_manifest_candidate(
        &state,
        &pkg,
        ManifestCommitChanges {
            tags,
            maintainers: None,
            delete_version_id: None,
        },
        manifests,
    )
    .await?;

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
    RequireAuth(auth): RequireAuth,
    Path((fullname, tag)): Path<(String, String)>,
) -> WebResult<Json<PublishResponse>> {
    let fullname = fullname.trim().to_string();
    let tag = tag.trim().to_string();

    validate_dist_tag(&tag)?;

    if tag == "latest" {
        return Err(WebError::Forbidden(
            "Can't remove the \"latest\" tag".to_string(),
        ));
    }

    let _unlock = lock_package(&state, &fullname, LockOwner::Publish)?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;
    ensure_package_readable_with_auth(&state, &auth, &pkg).await?;
    ensure_package_write_access(&state, &auth, &pkg).await?;
    ensure_local_package(pkg.source.as_deref(), &fullname)?;

    let mut tags = load_tag_map(&state, pkg.id).await?;
    if tags.remove(&tag).is_none() {
        return Ok(Json(PublishResponse {
            ok: true,
            rev: format!("{}-{}", pkg.id, tag),
        }));
    }

    let manifests = prepare_manifest_candidate(
        &state,
        ManifestCandidateParams {
            package: Some(&pkg),
            fullname: &fullname,
            description: pkg.description.as_deref(),
            dist_tags: &tags,
            added_version: None,
            removed_version: None,
            maintainers: None,
        },
    )
    .await?;
    commit_manifest_candidate(
        &state,
        &pkg,
        ManifestCommitChanges {
            tags,
            maintainers: None,
            delete_version_id: None,
        },
        manifests,
    )
    .await?;

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
