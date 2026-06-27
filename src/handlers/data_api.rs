use crate::error::{WebError, WebResult};
use crate::handlers::jsdelivr_util::{
    ensure_version_files_single_flight, parse_pkg_spec_path, resolve_version,
};
use crate::repository::VersionFileRow;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;

const CACHE_META: &str = "public, s-maxage=600, max-age=60";

#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
pub struct StructureQuery {
    pub structure: Option<String>,
}

#[utoipa::path(
    get,
    path = "/jsdelivr/api/npm/{rest}",
    tag = "cdn",
    params(
        ("rest" = String, Path, description = "`{pkg}@{version}` — e.g. `lodash@4.17.21`"),
        StructureQuery,
    ),
    responses(
        (status = OK, description = "File tree (or flat list with `?structure=flat`)", content_type = "application/json"),
        (status = MOVED_PERMANENTLY, description = "Redirect to resolved version"),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail, description = "Package not found"),
    )
)]
pub async fn version_files(
    State(state): State<AppState>,
    Path(rest): Path<String>,
    Query(query): Query<StructureQuery>,
) -> WebResult<Response> {
    if !state.config.cdn.enabled {
        return Err(WebError::NotFound("cdn is disabled".to_string()));
    }

    let (fullname, spec, _) = parse_pkg_spec_path(&rest);
    if fullname.is_empty() {
        return Err(WebError::NotFound("package name is empty".to_string()));
    }

    let resolved = resolve_version(&state, &fullname, &spec).await?;

    if spec != resolved.resolved {
        let location = format!("/jsdelivr/api/npm/{fullname}@{}", resolved.resolved);
        return Ok(Redirect::temporary(&location).into_response());
    }

    ensure_version_files_single_flight(&state, &fullname, &resolved).await?;

    let files = state
        .repo
        .list_version_files(resolved.version_row.id)
        .await
        .map_err(WebError::CustomApiError)?;

    let flat = matches!(query.structure.as_deref(), Some("flat"));

    let body = if flat {
        serde_json::json!({
            "type": "npm",
            "name": fullname,
            "version": resolved.resolved,
            "files": files.iter().map(flat_file).collect::<Vec<_>>(),
        })
    } else {
        serde_json::json!({
            "type": "npm",
            "name": fullname,
            "version": resolved.resolved,
            "files": build_tree(&files),
        })
    };

    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert("cache-control", CACHE_META.parse().unwrap());
    Ok(response)
}

fn flat_file(f: &VersionFileRow) -> FlatFile {
    FlatFile {
        name: format!("/{}", f.filepath),
        hash: f.shasum.clone().unwrap_or_default(),
        size: f.size,
    }
}

#[derive(Serialize, ToSchema)]
struct FlatFile {
    name: String,
    hash: String,
    size: i64,
}

#[derive(Serialize, ToSchema)]
struct TreeNode {
    #[serde(rename = "type")]
    node_type: &'static str,
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files: Vec<TreeNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<i64>,
}

enum BuilderNode {
    Dir(BTreeMap<String, BuilderNode>),
    File(String, i64),
}

fn build_tree(files: &[VersionFileRow]) -> Vec<TreeNode> {
    let mut root: BTreeMap<String, BuilderNode> = BTreeMap::new();
    for f in files {
        let segments: Vec<&str> = f.filepath.split('/').collect();
        let hash = f.shasum.clone().unwrap_or_default();
        insert(&mut root, &segments, &hash, f.size);
    }
    serialize_children(&root)
}

fn insert(map: &mut BTreeMap<String, BuilderNode>, segments: &[&str], hash: &str, size: i64) {
    if segments.is_empty() {
        return;
    }
    let head = segments[0];
    if segments.len() == 1 {
        map.insert(head.to_string(), BuilderNode::File(hash.to_string(), size));
        return;
    }
    let child = map
        .entry(head.to_string())
        .or_insert_with(|| BuilderNode::Dir(BTreeMap::new()));
    if let BuilderNode::Dir(children) = child {
        insert(children, &segments[1..], hash, size);
    }
}

fn serialize_children(map: &BTreeMap<String, BuilderNode>) -> Vec<TreeNode> {
    map.iter()
        .map(|(name, node)| match node {
            BuilderNode::Dir(children) => TreeNode {
                node_type: "directory",
                name: name.clone(),
                files: serialize_children(children),
                hash: None,
                size: None,
            },
            BuilderNode::File(hash, size) => TreeNode {
                node_type: "file",
                name: name.clone(),
                files: Vec::new(),
                hash: Some(hash.clone()),
                size: Some(*size),
            },
        })
        .collect()
}
