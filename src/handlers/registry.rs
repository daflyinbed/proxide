use crate::error::{WebError, WebResult};
use crate::npm::types::RegistryInfo;
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
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

#[utoipa::path(
    get,
    tag = "registry",
    path = "/",
    responses(
        (status = OK, description = "Registry info", body = RegistryInfo),
        (status = INTERNAL_SERVER_ERROR, body = crate::error::ApiErrorDetail),
    ),
)]
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

pub async fn get_package_inner(
    state: &AppState,
    headers: &HeaderMap,
    fullname: &str,
) -> WebResult<axum::response::Response> {
    let pkg = state
        .repo
        .get_package_by_name(fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let abbreviated = is_abbreviated_request(headers);
    let dist_id = if abbreviated {
        pkg.abbreviated_dist_id
    } else {
        pkg.full_dist_id
    }
    .ok_or_else(|| WebError::CustomApiError(anyhow::anyhow!("package manifest not yet synced")))?;

    let (json, shasum) = load_manifest_json(state, dist_id).await?;
    // todo(review): compare cache-control with cnpmcore
    let mut response = if let Some(shasum) = shasum {
        let etag = format!("W/\"{shasum}\"");
        (
            [("etag", etag), ("cache-control", "max-age=300".to_string())],
            Json(json),
        )
            .into_response()
    } else {
        Json(json).into_response()
    };

    if abbreviated {
        response.headers_mut().insert(
            "content-type",
            ABBREVIATED_ACCEPT.parse().unwrap(),
        );
    }

    Ok(response)
}

pub async fn get_package_version_inner(
    state: &AppState,
    fullname: &str,
    version: &str,
) -> WebResult<Json<serde_json::Value>> {
    let pkg = state
        .repo
        .get_package_by_name(fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let ver = state
        .repo
        .get_version(pkg.id, version)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{version} not found")))?;

    let dist_id = ver
        .manifest_dist_id
        .ok_or_else(|| WebError::CustomApiError(anyhow::anyhow!("version manifest not synced")))?;

    let (json, _) = load_manifest_json(state, dist_id).await?;
    // todo(review): compare cache-control with cnpmcore
    Ok(Json(json))
}
