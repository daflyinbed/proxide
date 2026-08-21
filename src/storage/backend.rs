use crate::config::StorageConfig;
use anyhow::{Context, Result};
use bytes::Bytes;
use futures::StreamExt;
use object_store::ObjectStore;
use object_store::PutPayload;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{GetResult, WriteMultipart};
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::sync::mpsc;

const ZSTD_SUFFIX: &str = ".zst";
const ZSTD_DICTIONARY: &[u8] = include_bytes!("../../assets/zstd-dictionary.bin");
const MULTIPART_THRESHOLD: usize = 10 * 1024 * 1024;
const STREAM_CHUNK_SIZE: usize = 1024 * 1024;
const MULTIPART_UPLOAD_CONCURRENCY: usize = 4;

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
        });
        Ok(Self {
            inner: store,
            compress_json,
            zstd_level,
            zstd_dict,
        })
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let path = Path::from(key);
        let result = self
            .inner
            .get(&path)
            .await
            .with_context(|| format!("failed to get object: {key}"))?;
        let bytes = result
            .bytes()
            .await
            .with_context(|| format!("failed to read object body: {key}"))?;
        let bytes = bytes.to_vec();
        if key.ends_with(ZSTD_SUFFIX) {
            self.decompress(&bytes, key)
        } else {
            Ok(bytes)
        }
    }

    fn decompress(&self, bytes: &[u8], key: &str) -> Result<Vec<u8>> {
        let ctx = || format!("failed to zstd-decompress object: {key}");
        let frame_dict_id = zstd::zstd_safe::get_dict_id_from_frame(bytes)
            .map(|id| id.get())
            .unwrap_or(0);
        match frame_dict_id {
            0 => zstd::decode_all(bytes).with_context(ctx),
            id if id == self.zstd_dict.dict_id => {
                let mut decoder =
                    zstd::Decoder::with_prepared_dictionary(bytes, &self.zstd_dict.decoder)
                        .with_context(ctx)?;
                let mut out = Vec::new();
                decoder.read_to_end(&mut out).with_context(ctx)?;
                Ok(out)
            }
            _ => anyhow::bail!(
                "object {key} requires zstd dictionary id {frame_dict_id}, but this binary embeds dictionary id {}",
                self.zstd_dict.dict_id
            ),
        }
    }

    pub async fn get_result(&self, key: &str) -> Result<GetResult> {
        let path = Path::from(key);
        self.inner
            .get(&path)
            .await
            .with_context(|| format!("failed to get object: {key}"))
    }

    pub async fn put(&self, key: &str, data: impl Into<PutPayload>) -> Result<()> {
        let path = Path::from(key);
        self.inner
            .put(&path, data.into())
            .await
            .with_context(|| format!("failed to put object: {key}"))?;
        Ok(())
    }

    pub async fn put_compressed(&self, key: &str, data: Vec<u8>) -> Result<String> {
        if !self.compress_json {
            self.put(key, data).await?;
            return Ok(key.to_string());
        }
        if data.len() > MULTIPART_THRESHOLD {
            self.put_compressed_streaming(key, data).await
        } else {
            let ctx = || format!("failed to zstd-compress object with dictionary: {key}");
            let mut compressor =
                zstd::bulk::Compressor::with_prepared_dictionary(&self.zstd_dict.encoder)
                    .with_context(ctx)?;
            let compressed = compressor.compress(&data).with_context(ctx)?;
            let actual_key = format!("{key}{ZSTD_SUFFIX}");
            self.put(&actual_key, compressed).await?;
            Ok(actual_key)
        }
    }

    async fn put_compressed_streaming(&self, key: &str, data: Vec<u8>) -> Result<String> {
        let actual_key = format!("{key}{ZSTD_SUFFIX}");
        let mut upload = self.put_multipart(&actual_key).await?;

        let level = self.zstd_level;
        let (tx, mut rx) = mpsc::channel::<Bytes>(MULTIPART_UPLOAD_CONCURRENCY);

        let compress_handle = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let writer = ChannelWriter::new(tx);
            let mut encoder = zstd::stream::write::Encoder::new(writer, level)?;
            for chunk in data.chunks(STREAM_CHUNK_SIZE) {
                encoder.write_all(chunk)?;
            }
            let mut writer = encoder.finish()?;
            writer.flush()?;
            Ok(())
        });

        while let Some(chunk) = rx.recv().await {
            upload
                .wait_for_capacity(MULTIPART_UPLOAD_CONCURRENCY)
                .await
                .with_context(|| format!("multipart upload backpressure: {key}"))?;
            upload.put(chunk);
        }

        upload
            .finish()
            .await
            .with_context(|| format!("multipart upload finish: {key}"))?;

        compress_handle
            .await
            .map_err(|e| anyhow::anyhow!("compression task join failed: {e}"))?
            .map_err(|e| anyhow::anyhow!("compression failed: {e}"))?;

        Ok(actual_key)
    }

    pub async fn put_multipart(&self, key: &str) -> Result<WriteMultipart> {
        let path = Path::from(key);
        let upload = self
            .inner
            .put_multipart(&path)
            .await
            .with_context(|| format!("failed to start multipart upload: {key}"))?;
        Ok(WriteMultipart::new(upload))
    }

    pub async fn exists(&self, key: &str) -> Result<bool> {
        let path = Path::from(key);
        match self.inner.head(&path).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(e).with_context(|| format!("failed to check object existence: {key}")),
        }
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

    pub async fn delete(&self, key: &str) -> Result<()> {
        let path = Path::from(key);
        self.inner
            .delete(&path)
            .await
            .with_context(|| format!("failed to delete object: {key}"))?;
        Ok(())
    }

    pub async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        let path = Path::from(prefix);
        let mut objects = Vec::new();
        let mut stream = self.inner.list(Some(&path));
        while let Some(meta) = stream.next().await {
            let meta =
                meta.with_context(|| format!("failed to list objects with prefix: {prefix}"))?;
            objects.push(meta.location.to_string());
        }
        Ok(objects)
    }
}

struct ChannelWriter {
    tx: mpsc::Sender<Bytes>,
    buf: Vec<u8>,
}

impl ChannelWriter {
    fn new(tx: mpsc::Sender<Bytes>) -> Self {
        Self {
            tx,
            buf: Vec::with_capacity(STREAM_CHUNK_SIZE),
        }
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        while self.buf.len() >= STREAM_CHUNK_SIZE {
            let remaining = self.buf.split_off(STREAM_CHUNK_SIZE);
            let chunk = std::mem::replace(&mut self.buf, remaining);
            self.tx
                .blocking_send(Bytes::from(chunk))
                .map_err(|_| std::io::Error::other("multipart upload cancelled"))?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            let chunk = std::mem::take(&mut self.buf);
            self.tx
                .blocking_send(Bytes::from(chunk))
                .map_err(|_| std::io::Error::other("multipart upload cancelled"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LocalConfig, StorageConfig};
    use std::path::Path;

    fn alt_dict_bytes() -> Vec<u8> {
        let samples: Vec<Vec<u8>> = (0..200)
            .map(|i| {
                format!(
                    "{{\"pkg\":\"other-{i}\",\"deps\":{{\"chalk\":\"5.{i}.2\"}},\"sig\":\"sha512-zzzzzz{i}\"}}"
                )
                .into_bytes()
            })
            .collect();
        zstd::dict::from_samples(&samples, 4096).unwrap()
    }

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

    #[tokio::test]
    async fn storage_embedded_dict_roundtrip_and_compatibility() {
        let base =
            std::env::temp_dir().join(format!("proxide-storage-zstd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let data = br#"{"name":"pkg-999","dependencies":{"lodash":"4.99.0"},"integrity":"sha512-abcdef999"}"#.to_vec();

        let storage = Storage::new(&local_config(&base)).unwrap();
        let actual = storage
            .put_compressed("pkg/x.json", data.clone())
            .await
            .unwrap();
        assert!(actual.ends_with(".zst"));
        let compressed = storage
            .get_result(&actual)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(
            zstd::zstd_safe::get_dict_id_from_frame(&compressed),
            zstd::zstd_safe::get_dict_id_from_dict(ZSTD_DICTIONARY)
        );
        assert_eq!(storage.get(&actual).await.unwrap(), data);

        let plain_key = "pkg/plain.json.zst";
        let plain = zstd::encode_all(&data[..], 3).unwrap();
        storage.put(plain_key, plain).await.unwrap();
        assert_eq!(storage.get(plain_key).await.unwrap(), data);

        let alt_dict = alt_dict_bytes();
        let mut compressor = zstd::bulk::Compressor::with_dictionary(3, &alt_dict).unwrap();
        let mismatched = compressor.compress(&data).unwrap();
        assert_ne!(
            zstd::zstd_safe::get_dict_id_from_frame(&mismatched),
            zstd::zstd_safe::get_dict_id_from_dict(ZSTD_DICTIONARY)
        );
        let mismatched_key = "pkg/mismatched.json.zst";
        storage.put(mismatched_key, mismatched).await.unwrap();

        let err = storage.get(mismatched_key).await.unwrap_err();
        assert!(format!("{err:#}").contains("dictionary id"), "got: {err:#}");

        let _ = std::fs::remove_dir_all(&base);
    }
}
