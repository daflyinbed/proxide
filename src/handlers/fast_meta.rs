use crate::error::{WebError, WebResult};
use crate::npm::types::{FastMetaFull, FastMetaResolved, FastMetaVersions, VersionMeta};
use crate::repository::PackageRow;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use semver::VersionReq;
use serde_json::Value;
use std::collections::HashMap;

fn parse_specifier(pkg: &str) -> (String, String) {
    if let Some(at_pos) = pkg.rfind('@')
        && at_pos > 0
    {
        return (pkg[..at_pos].to_string(), pkg[at_pos + 1..].to_string());
    }
    (pkg.to_string(), "latest".to_string())
}

async fn load_packument(
    state: &AppState,
    fullname: &str,
    use_full: bool,
) -> WebResult<(PackageRow, Value)> {
    let pkg = state
        .repo
        .get_package_by_name(fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let dist_id = if use_full {
        pkg.full_dist_id
    } else {
        pkg.abbreviated_dist_id
    }
    .ok_or_else(|| WebError::CustomApiError(anyhow::anyhow!("manifest not synced")))?;

    let (data, _) = state
        .repo
        .get_content(dist_id)
        .await
        .map_err(WebError::CustomApiError)?;

    let packument: Value =
        serde_json::from_slice(&data).map_err(|e| WebError::CustomApiError(e.into()))?;

    Ok((pkg, packument))
}

pub async fn resolve_version(
    State(state): State<AppState>,
    Path(pkg): Path<String>,
) -> WebResult<Json<FastMetaResolved>> {
    let (fullname, specifier) = parse_specifier(&pkg);
    let (_, packument) = load_packument(&state, &fullname, false).await?;

    let dist_tags = extract_dist_tags(&packument);
    let versions = extract_version_list(&packument);

    let resolved_version = resolve_specifier(&specifier, &dist_tags, &versions)
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{specifier} not resolved")))?;

    let published_at = get_time_field(packument.get("time"), &resolved_version);

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
    let (_, packument) = load_packument(&state, &fullname, false).await?;

    let dist_tags = extract_dist_tags(&packument);
    let all_versions = extract_version_list(&packument);

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
    let (_, packument) = load_packument(&state, &fullname, true).await?;

    let dist_tags = extract_dist_tags(&packument);
    let versions_meta = extract_versions_meta(&packument);
    let time_obj = packument.get("time");
    let time_created = get_time_field(time_obj, "created");
    let time_modified = get_time_field(time_obj, "modified");

    Ok(Json(FastMetaFull {
        name: fullname,
        dist_tags,
        versions_meta,
        time_created,
        time_modified,
        last_synced: None,
    }))
}

fn extract_dist_tags(packument: &Value) -> HashMap<String, String> {
    packument
        .get("dist-tags")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn extract_version_list(packument: &Value) -> Vec<String> {
    packument
        .get("versions")
        .and_then(|v| v.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

fn extract_versions_meta(packument: &Value) -> HashMap<String, VersionMeta> {
    let Some(versions) = packument.get("versions").and_then(|v| v.as_object()) else {
        return HashMap::new();
    };
    versions
        .iter()
        .filter_map(|(ver, data)| Some((ver.clone(), build_version_meta(packument, ver, data)?)))
        .collect()
}

fn build_version_meta(packument: &Value, ver: &str, data: &Value) -> Option<VersionMeta> {
    let time = packument
        .get("time")
        .and_then(|t| t.get(ver))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let engines = data
        .get("engines")
        .and_then(|e| serde_json::from_value(e.clone()).ok());
    let deprecated = data
        .get("deprecated")
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());
    let integrity = data
        .get("dist")
        .and_then(|d| d.get("integrity"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(VersionMeta {
        time,
        engines,
        deprecated,
        integrity,
        provenance: None,
    })
}

fn resolve_specifier(
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

fn get_time_field(time_obj: Option<&Value>, key: &str) -> Option<String> {
    time_obj
        .and_then(|t| t.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
