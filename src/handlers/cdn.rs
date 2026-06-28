use crate::error::{WebError, WebResult};
use crate::handlers::jsdelivr_util::{
    ensure_version_files_single_flight, parse_pkg_spec_path, resolve_version,
};
use crate::state::AppState;
use async_compression::tokio::bufread::ZstdDecoder;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use futures::StreamExt;
use reqwest::StatusCode;

const CACHE_FILE: &str = "public, max-age=31536000";
const ZSTD_SUFFIX: &str = ".zst";

#[utoipa::path(
    get,
    tag = "cdn",
    path = "/jsdelivr/npm/{rest}",
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
    Path(rest): Path<String>,
) -> WebResult<Response> {
    if !state.config.cdn.enabled {
        return Err(WebError::NotFound("cdn is disabled".to_string()));
    }

    let (fullname, spec, filepath) = parse_pkg_spec_path(&rest);
    if fullname.is_empty() {
        return Err(WebError::NotFound("package name is empty".to_string()));
    }

    let resolved = resolve_version(&state, &fullname, &spec).await?;

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

    ensure_version_files_single_flight(&state, &fullname, &resolved).await?;

    let filepath_query = filepath.trim_start_matches('/').to_string();
    let file = state
        .repo
        .get_version_file(resolved.version_row.id, &filepath_query)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("/{filepath} not found")))?;

    let result = state
        .repo
        .storage_get_result(&file.storage_path)
        .await
        .map_err(WebError::CustomApiError)?;

    let stream = result
        .into_stream()
        .map(|r| r.map_err(std::io::Error::other));
    let body = if file.storage_path.ends_with(ZSTD_SUFFIX) {
        let reader = tokio_util::io::StreamReader::new(stream);
        let buf = tokio::io::BufReader::new(reader);
        let decoder = ZstdDecoder::new(buf);
        Body::from_stream(tokio_util::io::ReaderStream::new(decoder))
    } else {
        Body::from_stream(stream)
    };

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", &file.content_type)
        .header("content-length", file.size)
        .header("cache-control", CACHE_FILE)
        .header("cross-origin-resource-policy", "cross-origin");
    if let Some(hash) = &file.shasum {
        builder = builder.header("x-content-hash", hash);
    }
    Ok(builder.body(body).unwrap())
}

fn redirect_response(location: &str) -> Response {
    Redirect::temporary(location).into_response()
}
