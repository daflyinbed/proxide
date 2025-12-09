use crate::binary::{BinaryEntry, BinarySource};
use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use futures::{StreamExt, stream};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct NpmMirrorProvider {
    client: Client,
    binary_name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ApiResponseItem {
    name: String,
    #[serde(rename = "type")]
    item_type: String,
    url: String,
    size: Option<u64>,
    date: String,
}

impl BinarySource for NpmMirrorProvider {
    async fn list<'a>(&'a self, dir: &'a str) -> Result<BoxStream<'a, Result<BinaryEntry>>> {
        let url = format!(
            "https://registry.npmmirror.com/-/binary/node/{}{}",
            self.binary_name, dir
        );

        let response = self.client.get(&url).send().await?;
        let items: Vec<ApiResponseItem> = response.json().await?;

        let iter = items.into_iter().map(|item| {
            let date = if item.date == "-" {
                None
            } else {
                Some(DateTime::parse_from_str(&item.date, "%d-%b-%Y %H:%M")?.with_timezone(&Utc))
            };
            Ok(BinaryEntry {
                name: item.name,
                is_dir: item.item_type == "dir",
                url: Some(item.url),
                size: item.size,
                date: date,
            })
        });
        Ok(stream::iter(iter).boxed())
    }
}
