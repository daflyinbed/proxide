use crate::{
    binary::{BinaryEntry, BinarySource},
    config::BucketConfig,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use futures::{StreamExt, stream};
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct BucketProvider {
    config: BucketConfig,
    client: Client,
}
impl BucketProvider {
    pub fn new(config: BucketConfig, client: Client) -> Self {
        Self { config, client }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct BucketResponse {
    contents: Vec<Content>,
    #[serde(default)]
    common_prefixes: Option<Vec<CommonPrefix>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct Content {
    key: String,
    last_modified: Option<DateTime<Utc>>,
    size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct CommonPrefix {
    prefix: String,
}

impl BinarySource for BucketProvider {
    async fn list<'a>(&'a self, dir: &'a str) -> Result<BoxStream<'a, Result<BinaryEntry>>> {
        let prefix = dir.trim_start_matches('/');
        let resp = self
            .client
            .get(&self.config.dist_url)
            .query(&[("delimiter", "/"), ("prefix", prefix)])
            .send()
            .await?
            .text()
            .await?;
        let BucketResponse {
            contents,
            common_prefixes,
        } = serde_xml_rs::from_str(&resp)?;
        let content_entries = contents.into_iter().filter_map(|content| {
            if content.key.ends_with('/') {
                return None;
            }
            let name = content.key.rsplit("/").next().unwrap_or(&content.key);
            Some(Ok(BinaryEntry {
                name: name.to_string(),
                is_dir: false,
                url: Some(format!("{}{}", self.config.dist_url, content.key)),
                size: content.size,
                date: content.last_modified,
            }))
        });
        if let Some(common) = common_prefixes {
            let prefix_entries = common.into_iter().filter_map(move |prefix| {
                let trimmed = prefix.prefix.trim_end_matches('/');
                let leaf = trimmed.rsplit('/').next().unwrap_or(trimmed);
                let name = format!("{}/", leaf);
                let full_path = format!("{}{}", dir, name);
                if self
                    .config
                    .ignore_dirs
                    .iter()
                    .any(|ignored| ignored == &full_path)
                {
                    return None;
                }
                Some(Ok(BinaryEntry {
                    name,
                    is_dir: true,
                    url: None,
                    size: None,
                    date: None,
                }))
            });
            return Ok(stream::iter(content_entries.chain(prefix_entries)).boxed());
        }

        Ok(stream::iter(content_entries).boxed())
    }
}
