use crate::error::{WebError, WebResult};
use crate::middleware::auth::ensure_package_readable;
use crate::repository::PackageVersionRow;
use crate::state::{AppState, TarballInflight, TarballInflightError};
use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::Response;
use bytes::Bytes;
use futures::{StreamExt, stream};
use reqwest::StatusCode;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const TARBALL_CONTENT_TYPE: &str = "application/octet-stream";
const CACHE_READ_CHUNK_SIZE: usize = 64 * 1024;
const MULTIPART_UPLOAD_CONCURRENCY: usize = 4;

fn tarball_response(body: Body, content_length: Option<u64>) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", TARBALL_CONTENT_TYPE);

    if let Some(content_length) = content_length {
        builder = builder.header("content-length", content_length);
    }

    builder.body(body).unwrap()
}

pub(crate) fn extract_version(fullname: &str, filename: &str) -> Option<String> {
    let name = fullname
        .rsplit_once('/')
        .map(|(_, n)| n)
        .unwrap_or(fullname);
    let target = filename.strip_suffix(".tgz").unwrap_or(filename);
    target
        .strip_prefix(name)
        .and_then(|s| s.strip_prefix('-'))
        .map(|s| s.to_string())
}

fn tarball_cache_path(cache_dir: &str, storage_key: &str) -> PathBuf {
    Path::new(cache_dir).join(storage_key)
}

fn inflight_error_to_web(error: TarballInflightError) -> WebError {
    match error {
        TarballInflightError::NotFound(message) => WebError::NotFound(message),
        TarballInflightError::Internal(message) => {
            WebError::CustomApiError(anyhow::anyhow!(message))
        }
    }
}

fn inflight_error_to_io(error: TarballInflightError) -> io::Error {
    match error {
        TarballInflightError::NotFound(message) => io::Error::new(io::ErrorKind::NotFound, message),
        TarballInflightError::Internal(message) => io::Error::other(message),
    }
}

async fn stream_storage_tarball(state: &AppState, storage_key: &str) -> WebResult<Response> {
    let result = state
        .repo
        .storage_get_result(storage_key)
        .await
        .map_err(WebError::CustomApiError)?;
    let content_length = result.meta.size;
    let body = Body::from_stream(result.into_stream());
    Ok(tarball_response(body, Some(content_length)))
}

async fn wait_for_inflight_ready(inflight: &TarballInflight) -> WebResult<Option<u64>> {
    let mut rx = inflight.subscribe();
    loop {
        let snapshot = inflight.snapshot();

        if let Some(error) = snapshot.error {
            return Err(inflight_error_to_web(error));
        }

        if snapshot.ready {
            return Ok(snapshot.content_length);
        }

        if rx.changed().await.is_err() {
            return Err(WebError::CustomApiError(anyhow::anyhow!(
                "tarball inflight sender dropped"
            )));
        }
    }
}

struct CacheStreamCleanup {
    inflight: Arc<TarballInflight>,
}

impl Drop for CacheStreamCleanup {
    fn drop(&mut self) {
        self.inflight.remove_reader();
        let inflight = self.inflight.clone();
        tokio::spawn(async move {
            cleanup_completed_cache_file(inflight).await;
        });
    }
}

struct LocalCacheStream<S> {
    inner: Pin<Box<S>>,
    _cleanup: CacheStreamCleanup,
}

impl<S> futures::Stream for LocalCacheStream<S>
where
    S: futures::Stream<Item = io::Result<Bytes>>,
{
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

async fn stream_local_cache(
    inflight: Arc<TarballInflight>,
) -> io::Result<impl futures::Stream<Item = io::Result<Bytes>>> {
    let file = open_cache_reader(&inflight.file_path, 0).await?;
    let rx = inflight.subscribe();

    Ok(stream::try_unfold(
        (file, 0u64, inflight, rx),
        |(mut file, mut offset, inflight, mut rx)| async move {
            loop {
                let snapshot = inflight.snapshot();

                if offset < snapshot.bytes_written {
                    let remaining = snapshot.bytes_written - offset;
                    let read_len = remaining.min(CACHE_READ_CHUNK_SIZE as u64) as usize;
                    let mut buffer = vec![0; read_len];
                    let bytes_read = file.read(&mut buffer).await?;

                    if bytes_read > 0 {
                        buffer.truncate(bytes_read);
                        offset += bytes_read as u64;
                        return Ok(Some((Bytes::from(buffer), (file, offset, inflight, rx))));
                    }

                    file = open_cache_reader(&inflight.file_path, offset).await?;
                    continue;
                }

                if let Some(error) = snapshot.error {
                    return Err(inflight_error_to_io(error));
                }

                if snapshot.completed && offset >= snapshot.bytes_written {
                    return Ok(None);
                }

                if rx.changed().await.is_err() {
                    return Err(io::Error::other("tarball inflight sender dropped"));
                }
                file = open_cache_reader(&inflight.file_path, offset).await?;
            }
        },
    ))
}

async fn open_cache_reader(file_path: &Path, offset: u64) -> io::Result<File> {
    let mut file = File::open(file_path).await?;
    file.seek(io::SeekFrom::Start(offset)).await?;
    Ok(file)
}

async fn stream_inflight_tarball(inflight: Arc<TarballInflight>) -> WebResult<Response> {
    wait_for_inflight_ready(&inflight).await?;
    inflight.add_reader();
    let stream = stream_local_cache(inflight.clone()).await.map_err(|e| {
        inflight.remove_reader();
        WebError::CustomApiError(e.into())
    })?;
    let stream = LocalCacheStream {
        inner: Box::pin(stream),
        _cleanup: CacheStreamCleanup { inflight },
    };
    Ok(tarball_response(Body::from_stream(stream), None))
}

async fn cleanup_failed_cache_file(file_path: &Path) {
    if let Err(e) = fs::remove_file(file_path).await
        && e.kind() != io::ErrorKind::NotFound
    {
        log::error!(
            "failed to remove tarball cache file {}: {e}",
            file_path.display()
        );
    }
}

async fn cleanup_completed_cache_file(inflight: Arc<TarballInflight>) {
    if !inflight.try_start_cleanup() {
        return;
    }

    if let Err(e) = fs::remove_file(&inflight.file_path).await
        && e.kind() != io::ErrorKind::NotFound
    {
        log::error!(
            "failed to remove tarball cache file {}: {e}",
            inflight.file_path.display()
        );
    }
}

async fn ensure_tarball_dist_link(
    state: &AppState,
    fullname: &str,
    filename: &str,
    storage_key: &str,
    version: &PackageVersionRow,
) -> WebResult<()> {
    if version.tar_dist_id.is_some() {
        return Ok(());
    }

    let dist = state
        .repo
        .get_dist_by_path(storage_key)
        .await
        .map_err(WebError::CustomApiError)?;

    let dist_id = if let Some(dist) = dist {
        dist.id
    } else {
        let size = state
            .repo
            .storage_get_result(storage_key)
            .await
            .map_err(WebError::CustomApiError)?
            .meta
            .size;

        state
            .repo
            .create_dist(filename, storage_key, size as i64, None, None)
            .await
            .map_err(WebError::CustomApiError)?
    };

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
        .map_err(|e| {
            WebError::CustomApiError(anyhow::anyhow!(
                "failed to update tar dist for {fullname}@{}: {e:#}",
                version.version
            ))
        })?;

    Ok(())
}

async fn run_tarball_producer(
    state: AppState,
    inflight: Arc<TarballInflight>,
    storage_key: String,
    fullname: String,
    package_id: i64,
    version_name: String,
    filename: String,
) {
    let result = async {
        let file_path = inflight.file_path.clone();
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                TarballInflightError::Internal(format!(
                    "failed to create cache directory {}: {e}",
                    parent.display()
                ))
            })?;
        }

        let mut request = state.http.get(format!(
            "{}/{fullname}/-/{filename}",
            state.config.worker.upstream_registry
        ));

        if !state.config.worker.upstream_auth_token.is_empty() {
            request = request.bearer_auth(&state.config.worker.upstream_auth_token);
        }

        let upstream_resp = request.send().await.map_err(|e| {
            TarballInflightError::Internal(format!(
                "failed to request upstream tarball for {fullname}/-/{filename}: {e}"
            ))
        })?;

        if upstream_resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(TarballInflightError::NotFound(format!(
                "\"{filename}\" not found"
            )));
        }

        if !upstream_resp.status().is_success() {
            return Err(TarballInflightError::Internal(format!(
                "upstream returned status {} for {fullname}/-/{filename}",
                upstream_resp.status()
            )));
        }

        let content_length = upstream_resp.content_length();
        let mut cache_file = File::create(&file_path).await.map_err(|e| {
            TarballInflightError::Internal(format!(
                "failed to create cache file {}: {e}",
                file_path.display()
            ))
        })?;

        let mut upload = Some(
            state
                .repo
                .storage_put_multipart(&storage_key)
                .await
                .map_err(|e| {
                    TarballInflightError::Internal(format!(
                        "failed to start multipart upload for {storage_key}: {e:#}"
                    ))
                })?,
        );

        inflight.mark_ready(content_length);

        let mut bytes_written = 0u64;
        let mut upstream_stream = upstream_resp.bytes_stream();

        while let Some(chunk) = upstream_stream.next().await {
            let chunk = chunk.map_err(|e| {
                TarballInflightError::Internal(format!(
                    "failed reading upstream tarball stream for {fullname}/-/{filename}: {e}"
                ))
            })?;

            cache_file.write_all(&chunk).await.map_err(|e| {
                TarballInflightError::Internal(format!(
                    "failed writing cache file {}: {e}",
                    file_path.display()
                ))
            })?;

            upload
                .as_mut()
                .expect("multipart upload should exist while streaming")
                .wait_for_capacity(MULTIPART_UPLOAD_CONCURRENCY)
                .await
                .map_err(|e| {
                    TarballInflightError::Internal(format!(
                        "failed waiting for multipart upload capacity on {storage_key}: {e:#}"
                    ))
                })?;
            upload
                .as_mut()
                .expect("multipart upload should exist while streaming")
                .put(chunk.clone());

            bytes_written += chunk.len() as u64;
            let max_tarball_size = state.config.cdn.max_tarball_size;
            if bytes_written > max_tarball_size {
                cache_file.flush().await.ok();
                drop(std::mem::take(&mut upload));
                let _ = fs::remove_file(&file_path).await;
                return Err(TarballInflightError::Internal(format!(
                    "tarball for {fullname}/-/{filename} exceeds cdn.maxTarballSize ({max_tarball_size})"
                )));
            }
            inflight.advance(bytes_written);
        }

        cache_file.flush().await.map_err(|e| {
            TarballInflightError::Internal(format!(
                "failed flushing cache file {}: {e}",
                file_path.display()
            ))
        })?;

        upload
            .take()
            .expect("multipart upload should exist before completion")
            .finish()
            .await
            .map_err(|e| {
                TarballInflightError::Internal(format!(
                    "failed completing multipart upload for {storage_key}: {e:#}"
                ))
            })?;

        let latest_version = state
            .repo
            .get_version(package_id, &version_name)
            .await
            .map_err(|e| {
                TarballInflightError::Internal(format!(
                    "failed to reload version {fullname}@{version_name}: {e:#}"
                ))
            })?;

        if latest_version
            .as_ref()
            .and_then(|v| v.tar_dist_id)
            .is_none()
        {
            let dist_id = state
                .repo
                .create_dist(
                    filename.as_str(),
                    &storage_key,
                    bytes_written as i64,
                    None,
                    None,
                )
                .await
                .map_err(|e| {
                    TarballInflightError::Internal(format!(
                        "failed to insert dist metadata for {storage_key}: {e:#}"
                    ))
                })?;

            let version = latest_version.ok_or_else(|| {
                TarballInflightError::NotFound(format!("{fullname}@{version_name} not found"))
            })?;

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
                .map_err(|e| {
                    TarballInflightError::Internal(format!(
                        "failed to update tar dist for {fullname}@{version_name}: {e:#}"
                    ))
                })?;
        }

        inflight.finish();

        Ok::<(), TarballInflightError>(())
    }
    .await;

    match result {
        Ok(()) => {
            state.tarball_downloads.remove(&storage_key);
            cleanup_completed_cache_file(inflight).await;
        }
        Err(error_kind) => {
            let download_completed = inflight.snapshot().completed;

            if !download_completed {
                inflight.fail(error_kind.clone());
                cleanup_failed_cache_file(&inflight.file_path).await;
            }

            state.tarball_downloads.remove(&storage_key);
            if download_completed {
                cleanup_completed_cache_file(inflight).await;
            }

            if let TarballInflightError::Internal(message) = error_kind {
                log::error!("tarball background download failed for {storage_key}: {message}");
            }
        }
    }
}

pub async fn download_tarball_inner(
    state: &AppState,
    headers: &HeaderMap,
    fullname: &str,
    filename: &str,
) -> WebResult<Response> {
    if !filename.ends_with(".tgz") {
        return Err(WebError::BadRequest(format!(
            "{filename} not a tarball file"
        )));
    }

    let version_name = extract_version(fullname, filename).ok_or_else(|| {
        WebError::NotFound(format!("{fullname}: invalid tarball filename {filename}"))
    })?;

    let pkg = state
        .repo
        .get_package_by_name(fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    ensure_package_readable(state, headers, &pkg).await?;

    let version = state
        .repo
        .get_version(pkg.id, &version_name)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{version_name} not found")))?;

    state
        .download_counters
        .entry(version.id)
        .or_insert(AtomicU64::new(0))
        .fetch_add(1, Ordering::Relaxed);

    let storage_key = format!("packages/{fullname}/{version_name}/{filename}");
    if state
        .repo
        .storage_exists(&storage_key)
        .await
        .map_err(WebError::CustomApiError)?
    {
        ensure_tarball_dist_link(state, fullname, filename, &storage_key, &version).await?;
        return stream_storage_tarball(state, &storage_key).await;
    }

    let cache_file_path = tarball_cache_path(&state.config.server.tarball_cache_dir, &storage_key);
    let (inflight, is_leader) = state
        .tarball_downloads
        .get_or_insert(&storage_key, cache_file_path);

    if is_leader {
        let state = state.clone();
        let inflight = inflight.clone();
        let storage_key = storage_key.clone();
        let fullname = fullname.to_string();
        let version_name = version_name.clone();
        let filename = filename.to_string();
        tokio::spawn(async move {
            run_tarball_producer(
                state,
                inflight,
                storage_key,
                fullname,
                pkg.id,
                version_name,
                filename,
            )
            .await;
        });
    }

    stream_inflight_tarball(inflight).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_version_simple_package() {
        assert_eq!(
            extract_version("lodash", "lodash-4.17.21.tgz"),
            Some("4.17.21".into())
        );
    }

    #[test]
    fn extract_version_hyphenated_package() {
        assert_eq!(
            extract_version("core-js", "core-js-3.36.0.tgz"),
            Some("3.36.0".into())
        );
    }

    #[test]
    fn extract_version_scoped_package() {
        assert_eq!(
            extract_version("@babel/core", "core-7.24.0.tgz"),
            Some("7.24.0".into())
        );
    }

    #[test]
    fn extract_version_scoped_hyphenated_package() {
        assert_eq!(
            extract_version("@core-js/pure", "pure-3.36.0.tgz"),
            Some("3.36.0".into())
        );
    }

    #[test]
    fn extract_version_prerelease() {
        assert_eq!(
            extract_version("foo", "foo-1.0.0-beta.1.tgz"),
            Some("1.0.0-beta.1".into())
        );
    }

    #[test]
    fn extract_version_name_prefix_mismatch() {
        assert_eq!(extract_version("bar", "baz-1.0.0.tgz"), None);
    }

    #[test]
    fn extract_version_missing_separator() {
        assert_eq!(extract_version("foo", "foo1.0.0.tgz"), None);
    }

    #[test]
    fn extract_version_long_hyphenated_name() {
        assert_eq!(
            extract_version(
                "@cnpmcore/test-sync-package-has-two-versions",
                "test-sync-package-has-two-versions-2.0.0.tgz"
            ),
            Some("2.0.0".into())
        );
    }

    #[test]
    fn extract_version_cnpmcore_deprecated() {
        assert_eq!(
            extract_version(
                "cnpmcore-test-sync-deprecated",
                "cnpmcore-test-sync-deprecated-0.0.0.tgz"
            ),
            Some("0.0.0".into())
        );
    }
}
