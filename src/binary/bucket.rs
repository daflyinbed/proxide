use crate::{
    binary::{BinaryEntry, BinarySource},
    config::BucketConfig,
};
use anyhow::Result;
use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;
use toml::value::Datetime;

pub struct BucketProvider {
    config: BucketConfig,
    client: Client,
}
impl BucketProvider {
    pub fn new(config: BucketConfig, client: Client) -> Self {
        Self { config, client }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct BucketResponse {
    contents: Vec<Content>,
    common_prefixes: Vec<CommonPrefix>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Content {
    key: String,
    last_modified: Option<Datetime<Utc>>,
    size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CommonPrefix {
    prefix: String,
}

impl BinarySource for BucketProvider {
    async fn list(
        &self,
        dir: &str,
    ) -> Result<impl futures::Stream<Item = anyhow::Result<Vec<BinaryEntry>>>> {
        let resp = self
            .client
            .get(self.config.dist_url)
            .query(&[("delimiter", "/"), ("prefix", &dir)])
            .send()
            .await?
            .text()
            .await?;
        let BucketResponse{contents, common_prefixes}  = serde_xml_rs::from_str(&resp)?;
        let result = contents.into_iter().map(|content| {
            Ok(vec![BinaryEntry {
                name: content.key,
                is_dir: false,
                url: None,
                size: content.size,
                date: content
                    .last_modified
                    .and_then(|dt| chrono::DateTime::parse_from_rfc3339(&dt).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc)),
            }])
        });
        todo!()
    }
}
