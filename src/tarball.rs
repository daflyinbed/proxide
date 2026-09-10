use crate::error::{WebError, WebResult};
use crate::npm::validate_tarball_digests;
use crate::repository::{AttachDistOutcome, PackageVersionRow};
use crate::state::{AppState, TarballInflight, TarballInflightError};
use bytes::Bytes;
use futures::{StreamExt, stream, stream::BoxStream};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const CACHE_READ_CHUNK_SIZE: usize = 64 * 1024;
const CACHE_VALIDATION_TAIL_SIZE: u64 = 64 * 1024;

pub(crate) struct Tarball {
    pub content_length: Option<u64>,
    pub stream: BoxStream<'static, WebResult<Bytes>>,
}

impl Tarball {
    pub async fn into_bytes(mut self, limit: u64) -> WebResult<Vec<u8>> {
        let size_error =
            || WebError::BadRequest(format!("tarball exceeds cdn.maxTarballSize ({limit})"));
        if self.content_length.is_some_and(|size| size > limit) {
            return Err(size_error());
        }

        let mut bytes = Vec::new();
        while let Some(chunk) = self.stream.next().await {
            let chunk = chunk?;
            if (bytes.len() as u64).saturating_add(chunk.len() as u64) > limit {
                return Err(size_error());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

fn tarball_cache_path(cache_dir: &str) -> PathBuf {
    Path::new(cache_dir).join(format!("{}.tmp", uuid::Uuid::new_v4().simple()))
}

fn inflight_error_to_web(error: TarballInflightError) -> WebError {
    match error {
        TarballInflightError::NotFound(message) => WebError::NotFound(message),
        TarballInflightError::SizeLimitExceeded(message) => WebError::BadRequest(message),
        TarballInflightError::Internal(message) => {
            WebError::CustomApiError(anyhow::anyhow!(message))
        }
    }
}

fn inflight_error_to_io(error: TarballInflightError) -> io::Error {
    match error {
        TarballInflightError::NotFound(message) => io::Error::new(io::ErrorKind::NotFound, message),
        TarballInflightError::SizeLimitExceeded(message) => {
            io::Error::new(io::ErrorKind::FileTooLarge, message)
        }
        TarballInflightError::Internal(message) => io::Error::other(message),
    }
}

async fn stream_storage_tarball(state: &AppState, dist_id: i64) -> WebResult<Tarball> {
    let dist = state
        .repo
        .get_dist(dist_id)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::ServiceUnavailable(anyhow::anyhow!("dist {dist_id} not found")))?;
    let result = state
        .repo
        .storage_get_result(&dist.path)
        .await
        .map_err(WebError::ServiceUnavailable)?;
    let content_length = u64::try_from(dist.stored_size).map_err(|_| {
        WebError::ServiceUnavailable(anyhow::anyhow!(
            "dist {dist_id} has invalid stored size {}",
            dist.stored_size
        ))
    })?;
    if result.meta.size != content_length {
        return Err(WebError::ServiceUnavailable(anyhow::anyhow!(
            "dist {dist_id} stored size mismatch: expected {content_length}, got {}",
            result.meta.size
        )));
    }
    Ok(Tarball {
        content_length: Some(content_length),
        stream: result
            .into_stream()
            .map(|chunk| chunk.map_err(|e| WebError::ServiceUnavailable(e.into())))
            .boxed(),
    })
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
            cleanup_cache_file(inflight).await;
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

                if offset < snapshot.available_bytes {
                    let remaining = snapshot.available_bytes - offset;
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

                if snapshot.completed && offset >= snapshot.available_bytes {
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

async fn stream_inflight_tarball(
    inflight: Arc<TarballInflight>,
    cleanup: CacheStreamCleanup,
) -> WebResult<Tarball> {
    let content_length = wait_for_inflight_ready(&inflight).await?;
    let stream = stream_local_cache(inflight.clone())
        .await
        .map_err(|e| WebError::CustomApiError(e.into()))?;
    let stream = LocalCacheStream {
        inner: Box::pin(stream),
        _cleanup: cleanup,
    };
    Ok(Tarball {
        content_length,
        stream: stream
            .map(|chunk| {
                chunk.map_err(|e| match e.kind() {
                    io::ErrorKind::NotFound => WebError::NotFound(e.to_string()),
                    io::ErrorKind::FileTooLarge => WebError::BadRequest(e.to_string()),
                    _ => WebError::CustomApiError(e.into()),
                })
            })
            .boxed(),
    })
}

async fn cleanup_cache_file(inflight: Arc<TarballInflight>) {
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

fn build_tarball_request(
    http: &reqwest::Client,
    config: &crate::config::Config,
    fullname: &str,
    filename: &str,
) -> reqwest::RequestBuilder {
    let mut request = http.get(format!(
        "{}/{fullname}/-/{filename}",
        config.worker.upstream_registry
    ));
    if !config.worker.upstream_auth_token.is_empty() {
        request = request.bearer_auth(&config.worker.upstream_auth_token);
    }
    request
}

async fn run_tarball_producer(
    state: AppState,
    inflight: Arc<TarballInflight>,
    inflight_key: String,
    fullname: String,
    version: PackageVersionRow,
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

        let request = build_tarball_request(
            &state.http,
            &state.config,
            &fullname,
            &filename,
        );

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
        let max_tarball_size = state.config.cdn.max_tarball_size;
        if content_length.is_some_and(|size| size > max_tarball_size) {
            return Err(TarballInflightError::SizeLimitExceeded(format!(
                "tarball for {fullname}/-/{filename} exceeds cdn.maxTarballSize ({max_tarball_size})"
            )));
        }

        let mut cache_file = File::options()
            .write(true)
            .create_new(true)
            .open(&file_path)
            .await
            .map_err(|e| {
                TarballInflightError::Internal(format!(
                    "failed to create cache file {}: {e}",
                    file_path.display()
                ))
            })?;
        inflight.mark_ready(content_length);

        let mut total_bytes = 0u64;
        let mut sha1 = Sha1::new();
        let mut sha256 = Sha256::new();
        let mut sha512 = Sha512::new();
        let mut upstream_stream = upstream_resp.bytes_stream();

        while let Some(chunk) = upstream_stream.next().await {
            let chunk = chunk.map_err(|e| {
                TarballInflightError::Internal(format!(
                    "failed reading upstream tarball stream for {fullname}/-/{filename}: {e}"
                ))
            })?;

            let next_size = total_bytes.checked_add(chunk.len() as u64).ok_or_else(|| {
                TarballInflightError::Internal(format!(
                    "tarball for {fullname}/-/{filename} is too large"
                ))
            })?;
            if next_size > max_tarball_size {
                return Err(TarballInflightError::SizeLimitExceeded(format!(
                    "tarball for {fullname}/-/{filename} exceeds cdn.maxTarballSize ({max_tarball_size})"
                )));
            }

            cache_file.write_all(&chunk).await.map_err(|e| {
                TarballInflightError::Internal(format!(
                    "failed writing cache file {}: {e}",
                    file_path.display()
                ))
            })?;

            sha1.update(&chunk);
            sha256.update(&chunk);
            sha512.update(&chunk);
            total_bytes = next_size;
            inflight.advance(total_bytes.saturating_sub(CACHE_VALIDATION_TAIL_SIZE));
        }

        cache_file.flush().await.map_err(|e| {
            TarballInflightError::Internal(format!(
                "failed flushing cache file {}: {e}",
                file_path.display()
            ))
        })?;

        if content_length.is_some_and(|expected| expected != total_bytes) {
            return Err(TarballInflightError::Internal(format!(
                "upstream content length mismatch for {fullname}/-/{filename}"
            )));
        }

        let sha1_digest = sha1.finalize();
        let storage_sha256 = sha256.finalize().into();
        let sha512_digest = sha512.finalize();
        validate_tarball_digests(
            &sha1_digest,
            &sha512_digest,
            version.tar_shasum.as_deref(),
            version.tar_integrity.as_deref(),
        )
        .map_err(|e| {
            TarballInflightError::Internal(format!(
                "tarball checksum validation failed for {fullname}@{}: {e:#}",
                version.version
            ))
        })?;
        let stored_size = i64::try_from(total_bytes).map_err(|_| {
            TarballInflightError::Internal(format!(
                "tarball for {fullname}/-/{filename} is too large"
            ))
        })?;
        let prepared = state
            .repo
            .prepare_raw_dist_file(&file_path, storage_sha256, stored_size)
            .await
            .map_err(|e| {
                TarballInflightError::Internal(format!(
                    "failed preparing tar dist for {fullname}@{}: {e:#}",
                    version.version
                ))
            })?;
        let outcome = state
            .repo
            .attach_tar_dist(
                version.id,
                &prepared,
                stored_size,
                &sha1_digest,
                &sha512_digest,
            )
            .await
            .map_err(|e| {
                TarballInflightError::Internal(format!(
                    "failed attaching tar dist for {fullname}@{}: {e:#}",
                    version.version
                ))
            })?;
        if outcome == AttachDistOutcome::VersionDeleted {
            return Err(TarballInflightError::NotFound(format!(
                "{fullname}@{} not found",
                version.version
            )));
        }

        inflight.finish(total_bytes);

        Ok::<(), TarballInflightError>(())
    }
    .await;

    match result {
        Ok(()) => {
            state.tarball_downloads.remove(&inflight_key);
            cleanup_cache_file(inflight).await;
        }
        Err(error_kind) => {
            inflight.fail(error_kind.clone());
            state.tarball_downloads.remove(&inflight_key);
            cleanup_cache_file(inflight).await;

            if let TarballInflightError::Internal(message) = error_kind {
                log::error!("tarball background download failed for {inflight_key}: {message}");
            }
        }
    }
}

pub(crate) async fn acquire(
    state: &AppState,
    fullname: &str,
    version: &PackageVersionRow,
    filename: &str,
) -> WebResult<Tarball> {
    if let Some(dist_id) = version.tar_dist_id {
        return stream_storage_tarball(state, dist_id).await;
    }

    let inflight_key = format!("{fullname}@{}", version.version);
    let cache_file_path = tarball_cache_path(&state.config.server.tarball_cache_dir);
    let (inflight, is_leader) = state
        .tarball_downloads
        .get_or_insert(&inflight_key, cache_file_path);
    inflight.add_reader();
    let cleanup = CacheStreamCleanup {
        inflight: inflight.clone(),
    };

    if is_leader {
        let version = version.clone();
        let state = state.clone();
        let inflight = inflight.clone();
        let inflight_key = inflight_key.clone();
        let fullname = fullname.to_string();
        let filename = filename.to_string();
        tokio::spawn(async move {
            run_tarball_producer(state, inflight, inflight_key, fullname, version, filename).await;
        });
    }

    stream_inflight_tarball(inflight, cleanup).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tarball_cache_path_uses_unique_temp_files() {
        let cache_dir = "/tmp/proxide-test-cache";
        let first = tarball_cache_path(cache_dir);
        let second = tarball_cache_path(cache_dir);

        assert_eq!(first.parent(), Some(Path::new(cache_dir)));
        assert_eq!(
            first.extension().and_then(|value| value.to_str()),
            Some("tmp")
        );
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn buffered_tarball_enforces_size_limit_with_and_without_content_length() {
        for content_length in [None, Some(6)] {
            for limit in [5, 6] {
                let tarball = Tarball {
                    content_length,
                    stream: stream::iter([
                        Ok(Bytes::from_static(b"abc")),
                        Ok(Bytes::from_static(b"def")),
                    ])
                    .boxed(),
                };
                let result = tarball.into_bytes(limit).await;
                if limit == 6 {
                    assert_eq!(result.unwrap(), b"abcdef");
                } else {
                    assert!(matches!(result, Err(WebError::BadRequest(_))));
                }
            }
        }
    }

    #[tokio::test]
    async fn buffered_tarball_propagates_failure_after_receiving_bytes() {
        let tarball = Tarball {
            content_length: Some(3),
            stream: stream::iter([
                Ok(Bytes::from_static(b"abc")),
                Err(WebError::ServiceUnavailable(anyhow::anyhow!(
                    "checksum mismatch"
                ))),
            ])
            .boxed(),
        };
        assert!(matches!(
            tarball.into_bytes(3).await,
            Err(WebError::ServiceUnavailable(_))
        ));
    }
}
