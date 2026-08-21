use crate::error::{WebError, WebResult};
use crate::handlers::jsdelivr_util::{
    ensure_version_files_single_flight, parse_pkg_spec_path, resolve_version,
};
use crate::state::AppState;
use crate::unpacked::validate_filepath;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use reqwest::StatusCode;

const CACHE_FILE: &str = "public, max-age=31536000, immutable";
const CACHE_FILE_SCOPED: &str = "public, max-age=300, must-revalidate";
const CACHE_FILE_PRIVATE: &str = "private, no-store";
const SERVE_ATTEMPTS: usize = 3;
const STREAM_BUFFER_SIZE: usize = 64 * 1024;

#[utoipa::path(
    get,
    tag = "cdn",
    path = "/{*rest}",
    params(
        ("rest" = String, Path, description = "Package spec and file path, e.g. lodash@4.17.21/lodash.js"),
    ),
    responses(
        (status = OK, description = "File content (binary stream)"),
        (status = TEMPORARY_REDIRECT, description = "Redirect to resolved version"),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
        (status = INTERNAL_SERVER_ERROR, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn serve_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(rest): Path<String>,
) -> WebResult<Response> {
    if !state.config.cdn.enabled {
        return Err(WebError::NotFound("cdn is disabled".to_string()));
    }

    let (fullname, spec, filepath) = parse_pkg_spec_path(&rest);
    if fullname.is_empty() {
        return Err(WebError::NotFound("package name is empty".to_string()));
    }

    let resolved = resolve_version(&state, &headers, &fullname, &spec).await?;

    if spec != resolved.resolved {
        let location = format!(
            "/jsdelivr/npm/{fullname}@{}/{}",
            resolved.resolved, filepath
        );
        return Ok(redirect_response(&location));
    }

    if filepath.is_empty() {
        return Ok(redirect_response(&format!(
            "/jsdelivr/api/npm/{fullname}@{}",
            resolved.resolved
        )));
    }

    let filepath_query = validate_filepath(&filepath).ok_or_else(|| {
        WebError::BadRequest(format!("invalid file path: /{filepath}"))
    })?;

    let version_id = resolved.version_row.id;

    for _ in 0..SERVE_ATTEMPTS {
        ensure_version_files_single_flight(&state, &fullname, &resolved).await?;

        let Some(manifest) = state.unpacked.get(version_id) else {
            continue;
        };
        let Some(file) = manifest.find(&filepath_query) else {
            return Err(WebError::NotFound(format!("/{filepath} not found")));
        };

        let path = state.unpacked.file_path(version_id, &filepath_query);
        match tokio::fs::File::open(&path).await {
            Ok(file_handle) => {
                let metadata = file_handle
                    .metadata()
                    .await
                    .map_err(|e| WebError::CustomApiError(e.into()))?;
                let cache_control = if !resolved.is_public {
                    CACHE_FILE_PRIVATE
                } else if resolved.is_scoped {
                    CACHE_FILE_SCOPED
                } else {
                    CACHE_FILE
                };
                let stream =
                    tokio_util::io::ReaderStream::with_capacity(file_handle, STREAM_BUFFER_SIZE);
                let mut builder = Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", &file.content_type)
                    .header("content-length", metadata.len())
                    .header("cache-control", cache_control)
                    .header("cross-origin-resource-policy", "cross-origin");
                if !file.hash.is_empty() {
                    builder = builder.header("x-content-hash", &file.hash);
                }
                return Ok(builder.body(Body::from_stream(stream)).unwrap());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                state.unpacked.remove(version_id);
                continue;
            }
            Err(e) => return Err(WebError::CustomApiError(e.into())),
        }
    }

    Err(WebError::CustomApiError(anyhow::anyhow!(
        "unpacked files for {fullname}@{} keep disappearing",
        resolved.resolved
    )))
}

fn redirect_response(location: &str) -> Response {
    Redirect::temporary(location).into_response()
}
