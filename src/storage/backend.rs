use crate::config::StorageConfig;
use anyhow::{Context, Result};
use futures::StreamExt;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::ObjectStore;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Storage {
    inner: Arc<dyn ObjectStore>,
}

impl Storage {
    pub fn new(config: &StorageConfig) -> Result<Self> {
        let store: Arc<dyn ObjectStore> = match config {
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
                Arc::new(store)
            }
            StorageConfig::Local(local_cfg) => {
                let store = LocalFileSystem::new_with_prefix(&local_cfg.directory)
                    .context("failed to create local storage")?;
                Arc::new(store)
            }
        };
        Ok(Self { inner: store })
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
        Ok(bytes.to_vec())
    }

    pub async fn put(&self, key: &str, data: Vec<u8>) -> Result<()> {
        let path = Path::from(key);
        self.inner
            .put(&path, data.into())
            .await
            .with_context(|| format!("failed to put object: {key}"))?;
        Ok(())
    }

    pub async fn exists(&self, key: &str) -> Result<bool> {
        let path = Path::from(key);
        match self.inner.head(&path).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(e).with_context(|| format!("failed to check object existence: {key}")),
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
            let meta = meta.with_context(|| format!("failed to list objects with prefix: {prefix}"))?;
            objects.push(meta.location.to_string());
        }
        Ok(objects)
    }
}
