use crate::config::StorageConfig;
use anyhow::{Context, Result};
use bytes::Bytes;
use futures::stream::{self, BoxStream};
use futures::{StreamExt, TryStreamExt};
use object_store::ObjectStore;
use object_store::PutPayload;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{GetResult, GetResultPayload, MultipartUpload, ObjectMeta, PutPayloadMut};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::io::ReaderStream;

const ZSTD_DICTIONARY: &[u8] = include_bytes!("../../assets/zstd-dictionary.bin");
const ZSTD_DICTIONARY_MAX_SIZE: usize = 10 * 1024 * 1024;
const MULTIPART_THRESHOLD: usize = 10 * 1024 * 1024;
const MULTIPART_PART_SIZE: usize = 5 * 1024 * 1024;
const MULTIPART_CONCURRENCY: usize = 4;

#[derive(Debug, Clone)]
pub struct Storage {
    inner: Arc<dyn ObjectStore>,
    compress_json: bool,
    zstd_level: i32,
    zstd_dict: Arc<ZstdDict>,
}

struct ZstdDict {
    encoder: zstd::dict::EncoderDictionary<'static>,
    decoder: zstd::dict::DecoderDictionary<'static>,
    dict_id: u32,
    sha256: String,
}

pub struct EncodedObject {
    pub path: String,
    pub storage_sha256: [u8; 32],
    pub stored_size: i64,
    bytes: Vec<u8>,
}

pub struct EncodedFile {
    pub path: String,
    pub storage_sha256: [u8; 32],
    pub stored_size: i64,
    file_path: PathBuf,
}

enum StoredCodec<'a> {
    Raw,
    ZstdPlain,
    ZstdDict(&'a str),
}

struct MultipartUploadGuard {
    upload: Option<Box<dyn MultipartUpload>>,
    tasks: JoinSet<object_store::Result<()>>,
}

impl MultipartUploadGuard {
    fn abort(&mut self) -> Option<JoinHandle<()>> {
        let mut upload = self.upload.take()?;
        let mut tasks = std::mem::take(&mut self.tasks);
        Some(tokio::spawn(async move {
            tasks.shutdown().await;
            if let Err(error) = upload.abort().await {
                log::warn!(action = "multipart_abort_failed"; "failed to abort multipart upload: {error:#}");
            }
        }))
    }
}

impl Drop for MultipartUploadGuard {
    fn drop(&mut self) {
        drop(self.abort());
    }
}

async fn upload_stream(
    upload: Box<dyn MultipartUpload>,
    mut stream: impl futures::Stream<Item = Result<Bytes>> + Unpin,
) -> Result<()> {
    let mut guard = MultipartUploadGuard {
        upload: Some(upload),
        tasks: JoinSet::new(),
    };
    let upload = guard.upload.as_mut().unwrap();
    let tasks = &mut guard.tasks;
    let result = async {
        let mut buffer = PutPayloadMut::new();
        while let Some(mut bytes) = stream.try_next().await? {
            while !bytes.is_empty() {
                let len = bytes
                    .len()
                    .min(MULTIPART_PART_SIZE - buffer.content_length());
                buffer.push(bytes.split_to(len));
                if buffer.content_length() == MULTIPART_PART_SIZE {
                    if tasks.len() >= MULTIPART_CONCURRENCY {
                        tasks.join_next().await.unwrap()??;
                    }
                    tasks.spawn(upload.put_part(std::mem::take(&mut buffer).into()));
                }
            }
        }
        if !buffer.is_empty() {
            if tasks.len() >= MULTIPART_CONCURRENCY {
                tasks.join_next().await.unwrap()??;
            }
            tasks.spawn(upload.put_part(buffer.into()));
        }
        while let Some(result) = tasks.join_next().await {
            result??;
        }
        upload.complete().await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        guard.abort().unwrap().await.ok();
    } else {
        drop(guard.upload.take());
    }
    result
}

struct CasVerifyState {
    stream: BoxStream<'static, object_store::Result<Bytes>>,
    hasher: Sha256,
    pending: Option<Bytes>,
    expected_hash: [u8; 32],
    key: String,
    finished: bool,
}

impl std::fmt::Debug for ZstdDict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZstdDict")
            .field("dict_id", &self.dict_id)
            .finish()
    }
}

impl Storage {
    pub fn new(config: &StorageConfig) -> Result<Self> {
        let (store, compress_json, zstd_level) = match config {
            StorageConfig::S3(s3_cfg) => {
                let mut builder = AmazonS3Builder::new()
                    .with_endpoint(&s3_cfg.endpoint)
                    .with_bucket_name(&s3_cfg.bucket_name)
                    .with_access_key_id(&s3_cfg.access_key_id)
                    .with_secret_access_key(&s3_cfg.secret_access_key)
                    .with_allow_http(true);

                if !s3_cfg.region.is_empty() {
                    builder = builder.with_region(&s3_cfg.region);
                }
                if s3_cfg.with_virtual_hosted_style_request {
                    builder = builder.with_virtual_hosted_style_request(true);
                }

                let store = builder.build().context("failed to build S3 client")?;
                (
                    Arc::new(store) as Arc<dyn ObjectStore>,
                    s3_cfg.compress_json,
                    s3_cfg.zstd_level,
                )
            }
            StorageConfig::Local(local_cfg) => {
                let store = LocalFileSystem::new_with_prefix(&local_cfg.directory)
                    .context("failed to create local storage")?;
                (
                    Arc::new(store) as Arc<dyn ObjectStore>,
                    local_cfg.compress_json,
                    local_cfg.zstd_level,
                )
            }
        };
        let dict_id = zstd::zstd_safe::get_dict_id_from_dict(ZSTD_DICTIONARY)
            .context("embedded zstd dictionary has no dictionary id")?
            .get();
        log::info!(
            action = "zstd_dict_loaded";
            "loaded embedded zstd dictionary ({} bytes, id {dict_id})",
            ZSTD_DICTIONARY.len()
        );
        let zstd_dict = Arc::new(ZstdDict {
            encoder: zstd::dict::EncoderDictionary::copy(ZSTD_DICTIONARY, zstd_level),
            decoder: zstd::dict::DecoderDictionary::copy(ZSTD_DICTIONARY),
            dict_id,
            sha256: hex::encode(Sha256::digest(ZSTD_DICTIONARY)),
        });
        Ok(Self {
            inner: store,
            compress_json,
            zstd_level,
            zstd_dict,
        })
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let (codec, _) = parse_cas_path(key)?;
        let result = self.get_result(key).await?;
        let bytes = result
            .bytes()
            .await
            .with_context(|| format!("failed to read object body: {key}"))?;
        let bytes = bytes.to_vec();
        match codec {
            StoredCodec::Raw => Ok(bytes),
            StoredCodec::ZstdPlain => zstd::decode_all(bytes.as_slice())
                .with_context(|| format!("failed to zstd-decompress object: {key}")),
            StoredCodec::ZstdDict(dictionary_sha256) => {
                if dictionary_sha256 != self.zstd_dict.sha256 {
                    anyhow::bail!(
                        "object {key} requires unavailable zstd dictionary {dictionary_sha256}"
                    );
                }
                let ctx = || format!("failed to zstd-decompress object: {key}");
                let mut decoder = zstd::Decoder::with_prepared_dictionary(
                    bytes.as_slice(),
                    &self.zstd_dict.decoder,
                )
                .with_context(ctx)?;
                let mut out = Vec::new();
                decoder.read_to_end(&mut out).with_context(ctx)?;
                Ok(out)
            }
        }
    }

    pub fn encode_raw(&self, bytes: Vec<u8>) -> Result<EncodedObject> {
        encoded_object("raw", bytes)
    }

    pub fn encode_json(&self, data: Vec<u8>) -> Result<EncodedObject> {
        if !self.compress_json {
            return self.encode_raw(data);
        }
        if data.len() > ZSTD_DICTIONARY_MAX_SIZE {
            let bytes = zstd::encode_all(data.as_slice(), self.zstd_level)
                .context("failed to zstd-compress object")?;
            return encoded_object("zstd/plain", bytes);
        }
        let mut compressor =
            zstd::bulk::Compressor::with_prepared_dictionary(&self.zstd_dict.encoder)
                .context("failed to create zstd dictionary compressor")?;
        let bytes = compressor
            .compress(&data)
            .context("failed to zstd-compress object with dictionary")?;
        encoded_object(&format!("zstd/dict-{}", self.zstd_dict.sha256), bytes)
    }

    pub async fn put_encoded(&self, object: &EncodedObject) -> Result<()> {
        self.put(&object.path, object.bytes.clone()).await
    }

    pub fn encode_raw_file(
        &self,
        file_path: &FsPath,
        storage_sha256: [u8; 32],
        stored_size: i64,
    ) -> EncodedFile {
        EncodedFile {
            path: cas_path("raw", &storage_sha256),
            storage_sha256,
            stored_size,
            file_path: file_path.to_path_buf(),
        }
    }

    pub async fn put_encoded_file(&self, object: &EncodedFile) -> Result<()> {
        let mut file = tokio::fs::File::open(&object.file_path)
            .await
            .with_context(|| {
                format!("failed to open object file: {}", object.file_path.display())
            })?;
        let size = file.metadata().await?.len();
        if size <= MULTIPART_THRESHOLD as u64 {
            let mut bytes = Vec::with_capacity(size as usize);
            (&mut file)
                .take(MULTIPART_THRESHOLD as u64 + 1)
                .read_to_end(&mut bytes)
                .await
                .with_context(|| {
                    format!("failed to read object file: {}", object.file_path.display())
                })?;
            if bytes.len() as u64 != size {
                anyhow::bail!("object file size changed: {}", object.file_path.display());
            }
            return self.put(&object.path, bytes).await;
        }
        let stream = ReaderStream::with_capacity(file, 1024 * 1024).map_err(anyhow::Error::from);
        self.put_stream(&object.path, stream).await
    }

    pub async fn get_result(&self, key: &str) -> Result<GetResult> {
        let (_, expected_hash) = parse_cas_path(key)?;
        let path = Path::from(key);
        let result = self
            .inner
            .get(&path)
            .await
            .with_context(|| format!("failed to get object: {key}"))?;
        let meta = result.meta.clone();
        let range = result.range.clone();
        let attributes = result.attributes.clone();
        let state = CasVerifyState {
            stream: result.into_stream(),
            hasher: Sha256::new(),
            pending: None,
            expected_hash,
            key: key.to_string(),
            finished: false,
        };
        let stream = stream::try_unfold(state, |mut state| async move {
            if state.finished {
                return Ok(None);
            }
            loop {
                match state.stream.next().await.transpose()? {
                    Some(chunk) => {
                        if chunk.is_empty() {
                            continue;
                        }
                        state.hasher.update(&chunk);
                        if let Some(pending) = state.pending.replace(chunk) {
                            return Ok(Some((pending, state)));
                        }
                    }
                    None => {
                        let actual_hash = std::mem::take(&mut state.hasher).finalize();
                        if actual_hash.as_slice() != state.expected_hash.as_slice() {
                            return Err(object_store::Error::Generic {
                                store: "CAS",
                                source: std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    format!(
                                        "object {} sha256 does not match its CAS path",
                                        state.key
                                    ),
                                )
                                .into(),
                            });
                        }
                        state.finished = true;
                        return Ok(state.pending.take().map(|pending| (pending, state)));
                    }
                }
            }
        })
        .boxed();
        Ok(GetResult {
            payload: GetResultPayload::Stream(stream),
            meta,
            range,
            attributes,
        })
    }

    pub async fn put(&self, key: &str, data: impl Into<PutPayload>) -> Result<()> {
        let path = Path::from(key);
        let data = data.into();
        if data.content_length() <= MULTIPART_THRESHOLD {
            self.inner
                .put(&path, data)
                .await
                .with_context(|| format!("failed to put object: {key}"))?;
            return Ok(());
        }
        self.put_stream(key, stream::iter(data.into_iter().map(Ok)))
            .await
    }

    async fn put_stream(
        &self,
        key: &str,
        stream: impl futures::Stream<Item = Result<Bytes>> + Unpin,
    ) -> Result<()> {
        let upload = self
            .inner
            .put_multipart(&Path::from(key))
            .await
            .with_context(|| format!("failed to start multipart upload: {key}"))?;
        upload_stream(upload, stream)
            .await
            .with_context(|| format!("multipart upload failed: {key}"))
    }

    pub async fn health_check(&self) -> bool {
        let mut stream = self.inner.list(None);
        match stream.next().await {
            None => true,
            Some(Ok(_)) => true,
            Some(Err(e)) => {
                log::warn!(action = "storage_health"; "storage list failed: {e:#}");
                false
            }
        }
    }

    pub async fn delete_many(&self, keys: &[String]) -> Vec<(String, Result<()>)> {
        futures::stream::iter(keys.iter().cloned())
            .map(|key| async move {
                let path = Path::from(key.as_str());
                let result = match self.inner.delete(&path).await {
                    Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
                    Err(error) => {
                        Err(error).with_context(|| format!("failed to delete object: {key}"))
                    }
                };
                (key, result)
            })
            .buffer_unordered(16)
            .collect()
            .await
    }

    pub fn list_meta(&self, prefix: &str) -> BoxStream<'static, Result<ObjectMeta>> {
        let path = Path::from(prefix);
        let prefix = prefix.to_string();
        self.inner
            .list(Some(&path))
            .map_err(move |error| {
                anyhow::Error::from(error)
                    .context(format!("failed to list objects with prefix: {prefix}"))
            })
            .boxed()
    }
}

fn encoded_object(namespace: &str, bytes: Vec<u8>) -> Result<EncodedObject> {
    let digest = Sha256::digest(&bytes);
    let storage_sha256: [u8; 32] = digest.into();
    let path = cas_path(namespace, &storage_sha256);
    let stored_size = i64::try_from(bytes.len()).context("object is too large")?;
    Ok(EncodedObject {
        path,
        storage_sha256,
        stored_size,
        bytes,
    })
}

fn cas_path(namespace: &str, storage_sha256: &[u8; 32]) -> String {
    let hash = hex::encode(storage_sha256);
    format!(
        "objects/{namespace}/sha256/{}/{}/{hash}",
        &hash[..2],
        &hash[2..4]
    )
}

pub fn is_valid_cas_path(path: &str) -> bool {
    parse_cas_path(path).is_ok()
}

fn parse_cas_path(path: &str) -> Result<(StoredCodec<'_>, [u8; 32])> {
    let parts: Vec<&str> = path.split('/').collect();
    let (codec, shard_a, shard_b, hash) = match parts.as_slice() {
        ["objects", "raw", "sha256", a, b, hash] => (StoredCodec::Raw, *a, *b, *hash),
        ["objects", "zstd", "plain", "sha256", a, b, hash] => {
            (StoredCodec::ZstdPlain, *a, *b, *hash)
        }
        ["objects", "zstd", dictionary, "sha256", a, b, hash]
            if dictionary.starts_with("dict-") =>
        {
            let dictionary_sha256 = &dictionary[5..];
            if dictionary_sha256.len() != 64 || hex::decode(dictionary_sha256).is_err() {
                anyhow::bail!("invalid CAS dictionary sha256: {path}");
            }
            (StoredCodec::ZstdDict(dictionary_sha256), *a, *b, *hash)
        }
        _ => anyhow::bail!("invalid CAS object path: {path}"),
    };
    if hash.len() != 64 || shard_a != &hash[..2] || shard_b != &hash[2..4] {
        anyhow::bail!("invalid CAS object path: {path}");
    }
    let decoded = hex::decode(hash).context("invalid CAS sha256")?;
    let storage_sha256: [u8; 32] = decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid CAS sha256 length"))?;
    Ok((codec, storage_sha256))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LocalConfig, StorageConfig};
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Notify;

    fn local_config(dir: &Path) -> StorageConfig {
        StorageConfig::Local(LocalConfig {
            directory: dir.to_string_lossy().into_owned(),
            compress_json: true,
            zstd_level: 3,
        })
    }

    #[test]
    fn embedded_zstd_dict_roundtrip() {
        let original =
            b"{\"name\":\"pkg-999\",\"dependencies\":{\"lodash\":\"4.99.0\"},\"integrity\":\"sha512-abcdef999\"}";

        let dict_id = zstd::zstd_safe::get_dict_id_from_dict(ZSTD_DICTIONARY).unwrap();
        let mut compressor = zstd::bulk::Compressor::with_dictionary(3, ZSTD_DICTIONARY).unwrap();
        let compressed = compressor.compress(original).unwrap();
        assert_eq!(
            zstd::zstd_safe::get_dict_id_from_frame(&compressed),
            Some(dict_id)
        );

        let mut decoder = zstd::Decoder::with_dictionary(&compressed[..], ZSTD_DICTIONARY).unwrap();
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).unwrap();
        assert_eq!(out, original);
    }

    #[derive(Debug, Default)]
    struct UploadStatus {
        abort_count: usize,
        completed: bool,
        part_sizes: Vec<usize>,
    }

    #[derive(Debug)]
    struct TestUpload {
        fail_part: bool,
        fail_complete: bool,
        status: Arc<Mutex<UploadStatus>>,
    }

    #[async_trait::async_trait]
    impl MultipartUpload for TestUpload {
        fn put_part(&mut self, data: PutPayload) -> object_store::UploadPart {
            let size = data.content_length();
            self.status.lock().unwrap().part_sizes.push(size);
            let fail = self.fail_part && size < MULTIPART_PART_SIZE;
            Box::pin(async move {
                if fail {
                    return Err(object_store::Error::Generic {
                        store: "test",
                        source: std::io::Error::other("part failed").into(),
                    });
                }
                Ok(())
            })
        }

        async fn complete(&mut self) -> object_store::Result<object_store::PutResult> {
            self.status.lock().unwrap().completed = true;
            if self.fail_complete {
                return Err(object_store::Error::Generic {
                    store: "test",
                    source: std::io::Error::other("complete failed").into(),
                });
            }
            Ok(object_store::PutResult {
                e_tag: None,
                version: None,
            })
        }

        async fn abort(&mut self) -> object_store::Result<()> {
            self.status.lock().unwrap().abort_count += 1;
            Ok(())
        }
    }

    #[tokio::test]
    async fn multipart_completion_and_errors() {
        for (fail_part, fail_read, fail_complete) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
            (false, false, false),
        ] {
            let status = Arc::new(Mutex::new(UploadStatus::default()));
            let upload = TestUpload {
                fail_part,
                fail_complete,
                status: status.clone(),
            };
            let mut chunks = vec![Ok(Bytes::from(vec![0; MULTIPART_THRESHOLD + 1]))];
            if fail_read {
                chunks.push(Err(anyhow::anyhow!("read failed")));
            }
            let result = upload_stream(Box::new(upload), stream::iter(chunks)).await;
            tokio::task::yield_now().await;
            let status = status.lock().unwrap();
            let failed = fail_part || fail_read || fail_complete;
            assert_eq!(status.abort_count, usize::from(failed));
            assert_eq!(status.completed, !fail_part && !fail_read);
            if failed {
                let error = result.unwrap_err();
                let expected = if fail_read {
                    "read failed"
                } else if fail_part {
                    "part failed"
                } else {
                    "complete failed"
                };
                assert!(error.to_string().contains(expected));
            } else {
                result.unwrap();
            }
            if fail_read {
                assert_eq!(
                    status.part_sizes,
                    [MULTIPART_PART_SIZE, MULTIPART_PART_SIZE]
                );
            } else {
                assert_eq!(
                    status.part_sizes,
                    [MULTIPART_PART_SIZE, MULTIPART_PART_SIZE, 1]
                );
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum PauseAt {
        Part,
        Complete,
        Abort,
    }

    #[derive(Debug)]
    struct PausedUpload {
        pause_at: PauseAt,
        entered: Arc<Notify>,
        aborted: Arc<Notify>,
        resume_abort: Arc<Notify>,
        active_parts: Arc<AtomicUsize>,
        abort_count: Arc<AtomicUsize>,
    }

    struct ActivePart(Arc<AtomicUsize>);

    impl Drop for ActivePart {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl MultipartUpload for PausedUpload {
        fn put_part(&mut self, _data: PutPayload) -> object_store::UploadPart {
            let pause_at = self.pause_at;
            let entered = self.entered.clone();
            let active_parts = self.active_parts.clone();
            Box::pin(async move {
                active_parts.fetch_add(1, Ordering::SeqCst);
                let _active = ActivePart(active_parts);
                if matches!(pause_at, PauseAt::Part) {
                    entered.notify_one();
                    std::future::pending::<()>().await;
                }
                Ok(())
            })
        }

        async fn complete(&mut self) -> object_store::Result<object_store::PutResult> {
            if matches!(self.pause_at, PauseAt::Complete) {
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            Err(object_store::Error::Generic {
                store: "test",
                source: std::io::Error::other("complete failed").into(),
            })
        }

        async fn abort(&mut self) -> object_store::Result<()> {
            assert_eq!(self.active_parts.load(Ordering::SeqCst), 0);
            self.abort_count.fetch_add(1, Ordering::SeqCst);
            if matches!(self.pause_at, PauseAt::Abort) {
                self.entered.notify_one();
                self.resume_abort.notified().await;
            }
            self.aborted.notify_one();
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancelled_multipart_uploads_abort_after_stopping_parts() {
        for pause_at in [PauseAt::Part, PauseAt::Complete, PauseAt::Abort] {
            let entered = Arc::new(Notify::new());
            let aborted = Arc::new(Notify::new());
            let resume_abort = Arc::new(Notify::new());
            let abort_count = Arc::new(AtomicUsize::new(0));
            let upload = PausedUpload {
                pause_at,
                entered: entered.clone(),
                aborted: aborted.clone(),
                resume_abort: resume_abort.clone(),
                active_parts: Arc::new(AtomicUsize::new(0)),
                abort_count: abort_count.clone(),
            };
            let chunks = vec![Ok(Bytes::from(vec![0; MULTIPART_THRESHOLD + 1]))];
            let task = tokio::spawn(upload_stream(Box::new(upload), stream::iter(chunks)));
            tokio::time::timeout(Duration::from_secs(5), entered.notified())
                .await
                .unwrap();
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            resume_abort.notify_one();
            tokio::time::timeout(Duration::from_secs(5), aborted.notified())
                .await
                .unwrap();
            assert_eq!(abort_count.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn upload_roundtrip_across_multipart_threshold() {
        let base = std::env::temp_dir().join(format!("proxide-upload-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let storage = Storage::new(&local_config(&base)).unwrap();
        for size in [
            0,
            MULTIPART_THRESHOLD,
            MULTIPART_THRESHOLD + 1,
            21 * 1024 * 1024,
        ] {
            let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let encoded = storage.encode_raw(data.clone()).unwrap();
            storage.put_encoded(&encoded).await.unwrap();
            assert_eq!(storage.get(&encoded.path).await.unwrap(), data);
            storage.delete_many(&[encoded.path.clone()]).await[0]
                .1
                .as_ref()
                .unwrap();

            let file_path = base.join("upload-input");
            tokio::fs::write(&file_path, &data).await.unwrap();
            let file = storage.encode_raw_file(&file_path, encoded.storage_sha256, size as i64);
            storage.put_encoded_file(&file).await.unwrap();
            assert_eq!(storage.get(&file.path).await.unwrap(), data);
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn cas_roundtrip_and_corruption_detection() {
        let base =
            std::env::temp_dir().join(format!("proxide-storage-zstd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let data = br#"{"name":"pkg-999","dependencies":{"lodash":"4.99.0"},"integrity":"sha512-abcdef999"}"#.to_vec();

        let storage = Storage::new(&local_config(&base)).unwrap();
        let encoded = storage.encode_json(data.clone()).unwrap();
        assert!(encoded.path.starts_with("objects/zstd/dict-"));
        storage.put_encoded(&encoded).await.unwrap();
        let compressed = storage
            .get_result(&encoded.path)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(
            zstd::zstd_safe::get_dict_id_from_frame(&compressed),
            zstd::zstd_safe::get_dict_id_from_dict(ZSTD_DICTIONARY)
        );
        assert_eq!(storage.get(&encoded.path).await.unwrap(), data);

        let plain = zstd::encode_all(&data[..], 3).unwrap();
        let plain = encoded_object("zstd/plain", plain).unwrap();
        storage.put_encoded(&plain).await.unwrap();
        assert_eq!(storage.get(&plain.path).await.unwrap(), data);

        let raw = storage.encode_raw(data.clone()).unwrap();
        storage.put(&raw.path, b"corrupt".to_vec()).await.unwrap();
        let chunks = storage
            .get_result(&raw.path)
            .await
            .unwrap()
            .into_stream()
            .collect::<Vec<_>>()
            .await;
        assert_eq!(chunks.len(), 1);
        assert!(
            format!("{:#}", chunks[0].as_ref().unwrap_err()).contains("sha256"),
            "got: {:#}",
            chunks[0].as_ref().unwrap_err()
        );
        let error = storage.get(&raw.path).await.unwrap_err();
        assert!(format!("{error:#}").contains("sha256"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
