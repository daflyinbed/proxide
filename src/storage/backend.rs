use crate::config::StorageConfig;
use anyhow::{Context, Result};
use bytes::Bytes;
use futures::StreamExt;
use object_store::ObjectStore;
use object_store::PutPayload;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{GetResult, ObjectMeta, WriteMultipart};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncReadExt;

const ZSTD_DICTIONARY: &[u8] = include_bytes!("../../assets/zstd-dictionary.bin");
const MULTIPART_THRESHOLD: usize = 10 * 1024 * 1024;

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
        let (codec, expected_hash) = parse_cas_path(key)?;
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
        let actual_hash = Sha256::digest(&bytes);
        if actual_hash.as_slice() != expected_hash.as_slice() {
            anyhow::bail!("object {key} sha256 does not match its CAS path");
        }
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
        if data.len() > MULTIPART_THRESHOLD {
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

    pub async fn encode_raw_file(&self, file_path: &FsPath) -> Result<EncodedFile> {
        let mut file = tokio::fs::File::open(file_path)
            .await
            .with_context(|| format!("failed to open object file: {}", file_path.display()))?;
        let mut hasher = Sha256::new();
        let mut stored_size = 0i64;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .await
                .with_context(|| format!("failed to read object file: {}", file_path.display()))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            stored_size = stored_size
                .checked_add(read as i64)
                .context("object is too large")?;
        }
        let storage_sha256: [u8; 32] = hasher.finalize().into();
        let hash = hex::encode(storage_sha256);
        Ok(EncodedFile {
            path: format!("objects/raw/sha256/{}/{}/{hash}", &hash[..2], &hash[2..4]),
            storage_sha256,
            stored_size,
            file_path: file_path.to_path_buf(),
        })
    }

    pub async fn put_encoded_file(&self, object: &EncodedFile) -> Result<()> {
        let path = Path::from(object.path.as_str());
        let multipart = self
            .inner
            .put_multipart(&path)
            .await
            .with_context(|| format!("failed to start multipart upload: {}", object.path))?;
        let mut upload = WriteMultipart::new(multipart);
        let mut file = match tokio::fs::File::open(&object.file_path).await {
            Ok(file) => file,
            Err(error) => {
                upload.abort().await.ok();
                return Err(error).with_context(|| {
                    format!("failed to open object file: {}", object.file_path.display())
                });
            }
        };
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let read = match file.read(&mut buffer).await {
                Ok(read) => read,
                Err(error) => {
                    upload.abort().await.ok();
                    return Err(error).with_context(|| {
                        format!("failed to read object file: {}", object.file_path.display())
                    });
                }
            };
            if read == 0 {
                break;
            }
            if let Err(error) = upload.wait_for_capacity(4).await {
                upload.abort().await.ok();
                return Err(error)
                    .with_context(|| format!("multipart upload failed: {}", object.path));
            }
            upload.put(Bytes::copy_from_slice(&buffer[..read]));
        }
        upload
            .finish()
            .await
            .with_context(|| format!("failed to finish multipart upload: {}", object.path))?;
        Ok(())
    }

    pub async fn get_result(&self, key: &str) -> Result<GetResult> {
        parse_cas_path(key)?;
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

    pub async fn list_meta(&self, prefix: &str) -> Result<Vec<ObjectMeta>> {
        let path = Path::from(prefix);
        let mut objects = Vec::new();
        let mut stream = self.inner.list(Some(&path));
        while let Some(meta) = stream.next().await {
            objects.push(
                meta.with_context(|| format!("failed to list objects with prefix: {prefix}"))?,
            );
        }
        Ok(objects)
    }
}

fn encoded_object(namespace: &str, bytes: Vec<u8>) -> Result<EncodedObject> {
    let digest = Sha256::digest(&bytes);
    let storage_sha256: [u8; 32] = digest.into();
    let hash = hex::encode(storage_sha256);
    let path = format!(
        "objects/{namespace}/sha256/{}/{}/{hash}",
        &hash[..2],
        &hash[2..4]
    );
    let stored_size = i64::try_from(bytes.len()).context("object is too large")?;
    Ok(EncodedObject {
        path,
        storage_sha256,
        stored_size,
        bytes,
    })
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
        let error = storage.get(&raw.path).await.unwrap_err();
        assert!(format!("{error:#}").contains("sha256"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
