pub mod content_type;

use crate::error::{WebError, WebResult};
use crate::repository::PackageVersionRow;
use crate::state::AppState;
use crate::unpacked::{
    ManifestFile, VersionManifest, manifest_from_entries, validate_filepath, version_disk_usage,
};
use anyhow::{Context, Result};
use base64::Engine;
use flate2::read::GzDecoder;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tar::Archive;
use tokio::io::AsyncWriteExt;

const COPY_BUFFER_SIZE: usize = 64 * 1024;

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
    let token = uuid::Uuid::new_v4().simple().to_string();
    let staging = state.unpacked.staging_dir(version.id, &token);

    let extract_result = tokio::task::spawn_blocking({
        let staging = staging.clone();
        move || extract_to_dir(&tarball_bytes, &staging, max_unpacked_size)
    })
    .await
    .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("extraction join failed: {e}")))?;

    let manifest = match extract_result {
        Ok(manifest) => manifest,
        Err(e) => {
            let staging = staging.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = std::fs::remove_dir_all(staging);
            })
            .await;
            return Err(WebError::CustomApiError(e));
        }
    };

    let final_dir = state.unpacked.version_dir(version.id);
    if let Err(e) = commit_staging(state, version.id, &staging, &final_dir, &manifest).await {
        let staging = staging.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = std::fs::remove_dir_all(staging);
        })
        .await;
        return Err(WebError::CustomApiError(e));
    }

    let disk_size = tokio::task::spawn_blocking({
        let dir = final_dir.clone();
        let manifest_path = state.unpacked.manifest_path(version.id);
        move || version_disk_usage(&dir, &manifest_path)
    })
    .await
    .map_err(|e| WebError::CustomApiError(anyhow::anyhow!("disk usage join failed: {e}")))?;

    state.unpacked.insert(version.id, Arc::new(manifest), disk_size);
    Ok(())
}

async fn commit_staging(
    state: &AppState,
    version_id: i64,
    staging: &Path,
    final_dir: &Path,
    manifest: &VersionManifest,
) -> Result<()> {
    if let Some(parent) = final_dir.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    match tokio::fs::remove_dir_all(final_dir).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("failed to remove old unpacked dir"),
    }

    tokio::fs::rename(staging, final_dir)
        .await
        .with_context(|| {
            format!(
                "failed to move staging dir into place: {} -> {}",
                staging.display(),
                final_dir.display()
            )
        })?;

    let manifest_path = state.unpacked.manifest_path(version_id);
    let tmp_path = manifest_path.with_file_name(format!(
        "{}.tmp",
        manifest_path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let bytes = serde_json::to_vec(manifest)?;
    let mut file = tokio::fs::File::create(&tmp_path).await?;
    file.write_all(&bytes).await?;
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp_path, &manifest_path).await?;
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

fn extract_to_dir(
    tarball_bytes: &[u8],
    staging: &Path,
    max_unpacked_size: u64,
) -> Result<VersionManifest> {
    std::fs::create_dir_all(staging)
        .with_context(|| format!("failed to create staging dir {}", staging.display()))?;

    let decoder = GzDecoder::new(Cursor::new(tarball_bytes));
    let mut archive = Archive::new(decoder);
    let mut files: BTreeMap<String, ManifestFile> = BTreeMap::new();
    let mut total: u64 = 0;
    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];

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
        let Some(rel) = validate_filepath(rel) else {
            continue;
        };
        let declared_size = entry.header().size().unwrap_or(0);
        if total.saturating_add(declared_size) > max_unpacked_size {
            anyhow::bail!(
                "unpacked size exceeds cdn.maxUnpackedSize ({max_unpacked_size}) at {rel}"
            );
        }

        let dest = staging.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create dir {}", parent.display()))?;
        }
        let mut file = std::fs::File::create(&dest)
            .with_context(|| format!("failed to create file {}", dest.display()))?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;
        loop {
            let n = entry.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            file.write_all(&buffer[..n])?;
            written += n as u64;
            if total.saturating_add(written) > max_unpacked_size {
                anyhow::bail!(
                    "unpacked size exceeds cdn.maxUnpackedSize ({max_unpacked_size}) at {rel}"
                );
            }
        }

        total = total.saturating_add(written);
        let size = written;
        let hash = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());
        let content_type = content_type::guess(&rel);
        if let Some(prev) = files.insert(rel.clone(), ManifestFile {
            path: rel,
            size,
            hash,
            content_type,
        }) {
            total = total.saturating_sub(prev.size);
        }
    }

    Ok(manifest_from_entries(files))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::fs;

    fn build_tgz(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = tar::Builder::new(&mut encoder);
            for (name, content) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, name, content.as_bytes()).unwrap();
            }
            tar.into_inner().unwrap();
        }
        encoder.finish().unwrap()
    }

    #[test]
    fn extract_writes_files_and_builds_manifest() {
        let base = std::env::temp_dir().join(format!("proxide-extract-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let tgz = build_tgz(&[
            ("package/index.js", "console.log(1);"),
            ("package/lib/deep/nested.txt", "hello"),
            ("package/package.json", "{}"),
            ("package-ignored", "no package prefix is kept as-is"),
            ("./package/./dot.txt", "dot segment filtered"),
            ("C:/Windows/system.ini", "drive path filtered"),
            (r"package\..\escape.txt", "backslash path filtered"),
        ]);

        let manifest = extract_to_dir(&tgz, &base, 1024 * 1024).unwrap();
        assert_eq!(
            manifest.total_size,
            manifest.files.iter().map(|f| f.size).sum::<u64>()
        );

        let names: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert!(names.contains(&"index.js"));
        assert!(names.contains(&"lib/deep/nested.txt"));
        assert!(names.contains(&"package.json"));
        assert!(names.contains(&"package-ignored"));
        assert!(!names.iter().any(|n| n.contains("..")));
        assert!(!names.iter().any(|n| n.contains("Windows")));
        assert!(!names.iter().any(|n| n.contains("escape")));

        let file = manifest.find("lib/deep/nested.txt").unwrap();
        assert_eq!(file.size, 5);
        assert_eq!(file.content_type, "text/plain");
        assert_eq!(
            fs::read_to_string(base.join("lib/deep/nested.txt")).unwrap(),
            "hello"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn extract_enforces_unpacked_limit() {
        let base = std::env::temp_dir().join(format!("proxide-extract-limit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let tgz = build_tgz(&[("package/big.bin", "0123456789")]);
        let err = extract_to_dir(&tgz, &base, 5).unwrap_err();
        assert!(format!("{err:#}").contains("maxUnpackedSize"), "got: {err:#}");

        let _ = fs::remove_dir_all(&base);
    }
}
