use crate::{
    config::BinaryConfig,
    error::{WebError, WebResult},
    repository::{Binary, Repository},
};
use crate::{config::UpstreamConfig, state::AppState};
use axum::{
    Json, debug_handler,
    extract::{Path as AxumPath, State},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BinaryListItem {
    category: String,
    description: String,
    #[serde(rename = "type")]
    kind: String,
    url: String,
    upstream: Vec<UpstreamConfig>,
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BinaryItem {
    id: u64,
    name: String,
    date: Option<chrono::NaiveDateTime>,
    #[serde(rename = "type")]
    item_type: String,
    size: Option<u64>,
    url: String,
    modified: chrono::NaiveDateTime,
}

#[debug_handler]
#[utoipa::path(
    get,
    path = "/-/binary/",
    responses(
        (status = 200, description = "List of available binaries", body = [BinaryListItem])
    )
)]
pub async fn list_binaries(State(state): State<AppState>) -> Json<Vec<BinaryListItem>> {
    let registry_url = state.config.server.root_url.clone();

    let binaries = state
        .config
        .binary
        .iter()
        .map(
            |BinaryConfig {
                 category,
                 description,
                 upstreams,
             }| BinaryListItem {
                category: format!("{}/", &category),
                description: description.clone(),
                kind: "dir".to_string(),
                url: format!("{}/-/binary/{}/", registry_url, category),
                upstream: upstreams.clone(),
            },
        )
        .collect::<Vec<_>>();

    Json(binaries)
}

fn validate_binary_name(binary_name: &str) -> WebResult<()> {
    let valid = binary_name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '@');

    if !valid || binary_name.is_empty() || binary_name.len() > 220 {
        return WebResult::Err(WebError::BadRequest(format!(
            "invalid binary name: {}",
            binary_name
        )));
    }

    Ok(())
}

fn format_items(binaries: Vec<Binary>, registry_url: &str) -> Vec<BinaryItem> {
    binaries
        .into_iter()
        .map(
            |Binary {
                 id,
                 category,
                 parent,
                 name,
                 is_dir,
                 size,
                 date,
                 updated_at,
             }| {
                BinaryItem {
                    id,
                    name: name.clone(),
                    date: Some(date),
                    item_type: if is_dir {
                        "dir".to_string()
                    } else {
                        "file".to_string()
                    },
                    size,
                    url: format!("{}/-/binary/{}{}{}", registry_url, category, parent, name),
                    modified: updated_at,
                }
            },
        )
        .collect()
}

#[debug_handler]
#[utoipa::path(
    get,
    path = "/-/binary/{binary_name}",
    responses(
        (status = 200, description = "List of binary files in root directory", body = [BinaryItem]),
        (status = 404, description = "Binary not found")
    )
)]
pub async fn show_binary_index(
    State(state): State<AppState>,
    AxumPath(binary_name): AxumPath<String>,
) -> WebResult<Json<Vec<BinaryItem>>> {
    // Validate binary_name
    validate_binary_name(&binary_name)?;

    let registry_url = state.config.server.root_url.clone();

    let items = state
        .repo
        .list_binaries(&binary_name, "/")
        .await
        .map_err(|err| WebError::CustomApiError(err.into()))?;

    Ok(Json(format_items(items, &registry_url)))
}

#[debug_handler]
#[utoipa::path(
    get,
    path = "/-/binary/{binary_name}/{*subpath}",
    responses(
        (status = 200, description = "Binary file or directory listing"),
        (status = 404, description = "Binary not found")
    )
)]
pub async fn show_binary(
    State(state): State<AppState>,
    AxumPath((binary_name, subpath)): AxumPath<(String, String)>,
) -> WebResult<Response> {
    // Validate binary_name
    validate_binary_name(&binary_name)?;

    let registry_url = state.config.server.root_url.clone();

    if subpath == "/" || subpath.is_empty() {
        let items = state
            .repo
            .list_binaries(&binary_name, "/")
            .await
            .map_err(|err| WebError::CustomApiError(err.into()))?;

        return Ok(Json(format_items(items, &registry_url)).into_response());
    }
    match subpath.rsplit_once("/") {
        Some((parent, name)) => {
            if !parent.is_empty() && !name.is_empty() {
                let item = state
                    .repo
                    .find_binary(&binary_name, parent, name)
                    .await
                    .map_err(|err| WebError::CustomApiError(err.into()))?
                    .ok_or(WebError::NotFound)?;
                if item.is_dir {
                    let items = state
                        .repo
                        .list_binaries(&binary_name, &subpath)
                        .await
                        .map_err(|err| WebError::CustomApiError(err.into()))?;
                    return Ok(Json(format_items(items, &registry_url)).into_response());
                } else {
                    todo!("download")
                }
            }
            let parent = if parent.is_empty() { name } else { parent };
            let items = state
                .repo
                .list_binaries(&binary_name, parent)
                .await
                .map_err(|err| WebError::CustomApiError(err.into()))?;

            return Ok(Json(format_items(items, &registry_url)).into_response());
        }
        None => {
            let items = state
                .repo
                .list_binaries(&binary_name, &subpath)
                .await
                .map_err(|err| WebError::CustomApiError(err.into()))?;

            return Ok(Json(format_items(items, &registry_url)).into_response());
        }
    }
}
