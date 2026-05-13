use crate::error::{WebError, WebResult};
use crate::npm::types::RegistryInfo;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;

const ABBREVIATED_ACCEPT: &str = "application/vnd.npm.install-v1+json";

fn is_abbreviated_request(headers: &HeaderMap) -> bool {
    headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains(ABBREVIATED_ACCEPT))
        .unwrap_or(false)
}

async fn load_manifest_json(
    state: &AppState,
    dist_id: i64,
) -> WebResult<(serde_json::Value, Option<String>)> {
    let (data, dist) = state
        .repo
        .get_content(dist_id)
        .await
        .map_err(WebError::CustomApiError)?;

    let shasum = dist.shasum.clone();

    let json: serde_json::Value =
        serde_json::from_slice(&data).map_err(|e| WebError::CustomApiError(e.into()))?;

    Ok((json, shasum))
}

pub async fn registry_root(State(state): State<AppState>) -> WebResult<Json<RegistryInfo>> {
    let count = state
        .repo
        .count_packages()
        .await
        .map_err(WebError::CustomApiError)?;
    Ok(Json(RegistryInfo {
        db_name: "registry".to_string(),
        doc_count: count,
    }))
}

pub async fn get_package(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(fullname): Path<String>,
) -> WebResult<axum::response::Response> {
    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let dist_id = if is_abbreviated_request(&headers) {
        pkg.abbreviated_dist_id
    } else {
        pkg.full_dist_id
    }
    .ok_or_else(|| WebError::CustomApiError(anyhow::anyhow!("package manifest not yet synced")))?;

    let (mut json, shasum) = load_manifest_json(&state, dist_id).await?;

    rewrite_tarball_urls(&mut json, &state.config.server.root_url, &fullname);

    if let Some(shasum) = shasum {
        let etag = format!("W/\"{shasum}\"");
        return Ok((
            [
                ("etag", etag),
                ("cache-control", "max-age=300".to_string()),
            ],
            Json(json),
        )
            .into_response());
    }

    Ok(Json(json).into_response())
}

pub async fn get_package_version(
    State(state): State<AppState>,
    Path((fullname, version)): Path<(String, String)>,
) -> WebResult<Json<serde_json::Value>> {
    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let ver = state
        .repo
        .get_version(pkg.id, &version)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{version} not found")))?;

    let dist_id = ver
        .manifest_dist_id
        .ok_or_else(|| WebError::CustomApiError(anyhow::anyhow!("version manifest not synced")))?;

    let (mut json, _) = load_manifest_json(&state, dist_id).await?;

    rewrite_tarball_urls(&mut json, &state.config.server.root_url, &fullname);

    Ok(Json(json))
}

fn rewrite_tarball_urls(json: &mut serde_json::Value, root_url: &str, fullname: &str) {
    let Some(versions) = json.get_mut("versions").and_then(|v| v.as_object_mut()) else {
        return;
    };
    for obj in versions.values_mut() {
        let Some(dist) = obj.get_mut("dist") else { continue };
        let Some(tarball) = dist.get_mut("tarball") else { continue };
        let Some(url) = tarball.as_str() else { continue };
        if let Some(filename) = extract_tarball_filename(url) {
            *tarball = serde_json::Value::String(format!(
                "{root_url}/npm/{fullname}/-/{filename}"
            ));
        }
    }
}

fn extract_tarball_filename(url: &str) -> Option<String> {
    let last_segment = url.rsplit('/').next()?;
    if last_segment.ends_with(".tgz") {
        Some(last_segment.to_string())
    } else {
        None
    }
}
