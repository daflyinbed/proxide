pub mod content_type;

use crate::error::{WebError, WebResult};
use crate::repository::{NewVersionFile, PackageVersionRow};
use crate::state::AppState;
use anyhow::Result;
use base64::Engine;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::io::Read;
use tar::Archive;

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

    let entries = {
        let tarball_bytes = tarball_bytes.clone();
        tokio::task::spawn_blocking(move || extract_entries(tarball_bytes))
            .await
            .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("extraction join failed: {e}")))?
            .map_err(WebError::CustomApiError)?
    };

    let version_id = version.id;
    let mut new_files = Vec::with_capacity(entries.len());
    for f in entries {
        let storage_key = format!(
            "packages/{fullname}/{}/unpacked/{}",
            version.version, f.filepath
        );
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

    state
        .repo
        .insert_version_files(version_id, &new_files)
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
        let result = state
            .repo
            .storage_get_result(&storage_key)
            .await
            .map_err(WebError::CustomApiError)?;
        let bytes = result
            .bytes()
            .await
            .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("{e:#}")))?
            .to_vec();
        return Ok(bytes);
    }

    let url = format!(
        "{}/{fullname}/-/{tarball_filename}",
        state.config.worker.upstream_registry
    );
    let mut request = state.http.get(&url);
    if !state.config.worker.upstream_auth_token.is_empty() {
        request = request.bearer_auth(&state.config.worker.upstream_auth_token);
    }
    let resp = request
        .send()
        .await
        .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("tarball fetch failed: {e:#}")))?;
    if !resp.status().is_success() {
        return Err(WebError::NotFound(format!(
            "upstream tarball {url} returned status {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("tarball read failed: {e:#}")))?
        .to_vec();

    state
        .repo
        .put_storage(&storage_key, bytes.clone())
        .await
        .map_err(WebError::CustomApiError)?;

    let dist_id = state
        .repo
        .create_dist(
            tarball_filename,
            &storage_key,
            bytes.len() as i64,
            None,
            None,
        )
        .await
        .map_err(WebError::CustomApiError)?;
    state
        .repo
        .update_version_dists(
            version.id,
            version.abbrev_dist_id,
            version.manifest_dist_id,
            Some(dist_id),
            version.readme_dist_id,
        )
        .await
        .map_err(WebError::CustomApiError)?;

    Ok(bytes)
}

fn extract_entries(tarball_bytes: Vec<u8>) -> Result<Vec<ExtractedFile>> {
    let decoder = GzDecoder::new(Cursor::new(tarball_bytes));
    let mut archive = Archive::new(decoder);
    let mut out = Vec::new();

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
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        let size = bytes.len() as i64;
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&bytes));
        let content_type = content_type::guess(rel);
        out.push(ExtractedFile {
            filepath: rel.to_string(),
            bytes,
            size,
            hash,
            content_type,
        });
    }

    Ok(out)
}
