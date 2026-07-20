pub mod content_type;

use crate::error::{WebError, WebResult};
use crate::repository::{NewVersionFile, PackageVersionRow};
use crate::state::AppState;
use anyhow::Result;
use base64::Engine;
use flate2::read::GzDecoder;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::io::Read;
use tar::Archive;
use tokio::sync::mpsc;

struct ExtractedFile {
    filepath: String,
    bytes: Vec<u8>,
    size: i64,
    hash: String,
    content_type: String,
}

pub async fn ensure_version_files(
    state: &AppState,
    fullname: &str,
    version: &PackageVersionRow,
    tarball_filename: &str,
) -> WebResult<()> {
    let tarball_bytes = acquire_tarball(state, fullname, version, tarball_filename).await?;

    let limit = state.config.cdn.max_tarball_size;
    if tarball_bytes.len() as u64 > limit {
        return Err(WebError::BadRequest(format!(
            "tarball for {fullname}@{} exceeds cdn.maxTarballSize ({limit})",
            version.version
        )));
    }

    let max_unpacked_size = state.config.cdn.max_unpacked_size;
    let (tx, mut rx) = mpsc::channel::<ExtractedFile>(1);

    let extract_handle = tokio::task::spawn_blocking(move || -> Result<()> {
        extract_entries_streaming(tarball_bytes, max_unpacked_size, tx)
    });

    let version_str = &version.version;
    let mut new_files = Vec::new();
    while let Some(f) = rx.recv().await {
        let storage_key = format!("packages/{fullname}/{version_str}/unpacked/{}", f.filepath);
        let actual_key = state
            .repo
            .put_storage_compressed(&storage_key, f.bytes)
            .await
            .map_err(WebError::CustomApiError)?;
        new_files.push(NewVersionFile {
            storage_key: actual_key,
            size: f.size,
            shasum: Some(f.hash),
            filepath: f.filepath,
            content_type: f.content_type,
        });
    }

    extract_handle
        .await
        .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("extraction join failed: {e}")))?
        .map_err(WebError::CustomApiError)?;

    state
        .repo
        .insert_version_files(version.id, &new_files)
        .await
        .map_err(WebError::CustomApiError)?;

    Ok(())
}

async fn acquire_tarball(
    state: &AppState,
    fullname: &str,
    version: &PackageVersionRow,
    tarball_filename: &str,
) -> WebResult<Vec<u8>> {
    if let Some(tar_dist_id) = version.tar_dist_id {
        let (data, _) = state
            .repo
            .get_content(tar_dist_id)
            .await
            .map_err(WebError::CustomApiError)?;
        return Ok(data);
    }

    let storage_key = format!("packages/{fullname}/{}/{tarball_filename}", version.version);

    if state
        .repo
        .storage_exists(&storage_key)
        .await
        .map_err(WebError::CustomApiError)?
    {
        return read_tarball_from_storage(state, &storage_key).await;
    }

    if let Some(inflight) = state.tarball_downloads.get_inflight(&storage_key) {
        if inflight.wait_for_completion().await.is_ok() {
            return read_tarball_from_storage(state, &storage_key).await;
        }
    }

    let request = crate::handlers::tarball::build_tarball_request(
        &state.http,
        &state.config,
        fullname,
        tarball_filename,
    );
    let resp = request
        .send()
        .await
        .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("tarball fetch failed: {e:#}")))?;
    if !resp.status().is_success() {
        return Err(WebError::NotFound(format!(
            "upstream tarball for {fullname}/-/{tarball_filename} returned status {}",
            resp.status()
        )));
    }

    let limit = state.config.cdn.max_tarball_size;
    if let Some(len) = resp.content_length()
        && len > limit
    {
        return Err(WebError::BadRequest(format!(
            "tarball for {fullname}/-/{tarball_filename} exceeds cdn.maxTarballSize ({len} > {limit})"
        )));
    }

    let mut bytes = Vec::new();
    let mut total: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result
            .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("tarball read failed: {e:#}")))?;
        total += chunk.len() as u64;
        if total > limit {
            return Err(WebError::BadRequest(format!(
                "tarball for {fullname}/-/{tarball_filename} exceeds cdn.maxTarballSize ({limit})"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }

    state
        .repo
        .put_storage(&storage_key, bytes.clone())
        .await
        .map_err(WebError::CustomApiError)?;

    crate::handlers::tarball::ensure_tarball_dist_link(
        state,
        fullname,
        tarball_filename,
        &storage_key,
        version,
    )
    .await?;

    Ok(bytes)
}

async fn read_tarball_from_storage(state: &AppState, storage_key: &str) -> WebResult<Vec<u8>> {
    let result = state
        .repo
        .storage_get_result(storage_key)
        .await
        .map_err(WebError::CustomApiError)?;
    result
        .bytes()
        .await
        .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("{e:#}")))
        .map(|b| b.to_vec())
}

fn extract_entries_streaming(
    tarball_bytes: Vec<u8>,
    max_unpacked_size: u64,
    tx: mpsc::Sender<ExtractedFile>,
) -> Result<()> {
    let decoder = GzDecoder::new(Cursor::new(tarball_bytes));
    let mut archive = Archive::new(decoder);
    let mut total: u64 = 0;

    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?;
        let path_str = path.to_string_lossy().into_owned();
        let rel = path_str.strip_prefix("package/").unwrap_or(&path_str);
        if rel.is_empty() || rel.starts_with('/') {
            continue;
        }
        if rel.split('/').any(|seg| seg == ".." || seg.is_empty()) {
            continue;
        }
        let declared_size = entry.header().size().unwrap_or(0);
        if total.saturating_add(declared_size) > max_unpacked_size {
            anyhow::bail!(
                "unpacked size exceeds cdn.maxUnpackedSize ({max_unpacked_size}) at {rel}"
            );
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        total = total.saturating_add(bytes.len() as u64);
        if total > max_unpacked_size {
            anyhow::bail!(
                "unpacked size exceeds cdn.maxUnpackedSize ({max_unpacked_size}) at {rel}"
            );
        }
        let size = bytes.len() as i64;
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&bytes));
        let content_type = content_type::guess(rel);

        if tx
            .blocking_send(ExtractedFile {
                filepath: rel.to_string(),
                bytes,
                size,
                hash,
                content_type,
            })
            .is_err()
        {
            break;
        }
    }

    Ok(())
}
