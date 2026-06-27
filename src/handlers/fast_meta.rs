use crate::error::{WebError, WebResult};
use crate::npm::types::{
    AbbreviatedPackument, FastMetaFull, FastMetaResolved, FastMetaVersions, Packument, VersionMeta,
};
use crate::repository::PackageRow;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use semver::VersionReq;
use std::collections::HashMap;

fn parse_specifier(pkg: &str) -> (String, String) {
    if let Some(at_pos) = pkg.rfind('@')
        && at_pos > 0
    {
        return (pkg[..at_pos].to_string(), pkg[at_pos + 1..].to_string());
    }
    (pkg.to_string(), "latest".to_string())
}

pub(crate) async fn load_abbreviated_packument(
    state: &AppState,
    fullname: &str,
) -> WebResult<(PackageRow, AbbreviatedPackument)> {
    let pkg = state
        .repo
        .get_package_by_name(fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let dist_id = pkg
        .abbreviated_dist_id
        .ok_or_else(|| WebError::CustomApiError(anyhow::anyhow!("manifest not synced")))?;

    let (data, _) = state
        .repo
        .get_content(dist_id)
        .await
        .map_err(WebError::CustomApiError)?;

    let packument: AbbreviatedPackument =
        serde_json::from_slice(&data).map_err(|e| WebError::CustomApiError(e.into()))?;

    Ok((pkg, packument))
}

pub(crate) async fn load_full_packument(
    state: &AppState,
    fullname: &str,
) -> WebResult<(PackageRow, Packument)> {
    let pkg = state
        .repo
        .get_package_by_name(fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let dist_id = pkg
        .full_dist_id
        .ok_or_else(|| WebError::CustomApiError(anyhow::anyhow!("manifest not synced")))?;

    let (data, _) = state
        .repo
        .get_content(dist_id)
        .await
        .map_err(WebError::CustomApiError)?;

    let packument: Packument =
        serde_json::from_slice(&data).map_err(|e| WebError::CustomApiError(e.into()))?;

    Ok((pkg, packument))
}

pub async fn resolve_version(
    State(state): State<AppState>,
    Path(pkg): Path<String>,
) -> WebResult<Json<FastMetaResolved>> {
    let (fullname, specifier) = parse_specifier(&pkg);
    let (_, packument) = load_abbreviated_packument(&state, &fullname).await?;

    let versions: Vec<String> = packument.versions.keys().cloned().collect();
    let resolved_version = resolve_specifier(&specifier, &packument.dist_tags, &versions)
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{specifier} not resolved")))?;

    let published_at = packument
        .time
        .as_ref()
        .and_then(|t| t.get(&resolved_version))
        .cloned();

    Ok(Json(FastMetaResolved {
        name: fullname,
        specifier,
        version: resolved_version,
        published_at,
        last_synced: None,
    }))
}

pub async fn get_versions(
    State(state): State<AppState>,
    Path(pkg): Path<String>,
) -> WebResult<Json<FastMetaVersions>> {
    let (fullname, specifier) = parse_specifier(&pkg);
    let (_, packument) = load_abbreviated_packument(&state, &fullname).await?;

    let dist_tags = packument.dist_tags.clone();
    let all_versions: Vec<String> = packument.versions.keys().cloned().collect();

    let filtered = if specifier == "*" || specifier == "latest" {
        all_versions
    } else {
        filter_versions_by_range(&specifier, &all_versions)
    };

    Ok(Json(FastMetaVersions {
        name: fullname,
        specifier,
        dist_tags,
        versions: filtered,
        last_synced: None,
    }))
}

pub async fn get_full(
    State(state): State<AppState>,
    Path(pkg): Path<String>,
) -> WebResult<Json<FastMetaFull>> {
    let (fullname, _) = parse_specifier(&pkg);
    let (_, packument) = load_full_packument(&state, &fullname).await?;

    let dist_tags = packument.dist_tags.clone();
    let versions_meta = extract_versions_meta(&packument);
    let time_created = packument.time.get("created").cloned();
    let time_modified = packument.time.get("modified").cloned();

    Ok(Json(FastMetaFull {
        name: fullname,
        dist_tags,
        versions_meta,
        time_created,
        time_modified,
        last_synced: None,
    }))
}

fn extract_versions_meta(packument: &Packument) -> HashMap<String, VersionMeta> {
    packument
        .versions
        .iter()
        .map(|(ver, pv)| {
            (
                ver.clone(),
                VersionMeta {
                    time: packument.time.get(ver).cloned(),
                    engines: pv.engines.clone(),
                    deprecated: pv.deprecated.clone(),
                    integrity: pv.dist.integrity.clone(),
                    provenance: None,
                },
            )
        })
        .collect()
}

pub(crate) fn resolve_specifier(
    specifier: &str,
    dist_tags: &HashMap<String, String>,
    versions: &[String],
) -> Option<String> {
    if specifier == "latest" || specifier == "*" {
        return dist_tags.get("latest").cloned();
    }
    if let Some(tag_version) = dist_tags.get(specifier) {
        return Some(tag_version.clone());
    }
    if let Ok(req) = VersionReq::parse(specifier) {
        let mut best: Option<semver::Version> = None;
        for v in versions {
            if let Ok(sv) = semver::Version::parse(v)
                && req.matches(&sv)
                && best.as_ref().is_none_or(|b| sv > *b)
            {
                best = Some(sv);
            }
        }
        return best.map(|v| v.to_string());
    }
    if versions.contains(&specifier.to_string()) {
        return Some(specifier.to_string());
    }
    None
}

fn filter_versions_by_range(range: &str, versions: &[String]) -> Vec<String> {
    if let Ok(req) = VersionReq::parse(range) {
        versions
            .iter()
            .filter(|v| {
                semver::Version::parse(v)
                    .map(|sv| req.matches(&sv))
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    } else {
        versions.to_vec()
    }
}
