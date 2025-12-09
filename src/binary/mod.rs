pub mod bucket;
pub mod github;
pub mod imagemin;
pub mod npm_mirror;

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;

#[derive(Debug, Clone)]
pub struct BinaryEntry {
    pub name: String,
    pub is_dir: bool,
    /// 完整 URL（可选，如果是远端资源）
    pub url: Option<String>,
    /// 字节大小（未知则 None）
    pub size: Option<u64>,
    pub date: Option<DateTime<Utc>>,
}

pub trait BinarySource {
    /// dir 开头末尾都有/
    async fn list<'a>(&'a self, dir: &'a str) -> Result<BoxStream<'a, Result<BinaryEntry>>>;
}

pub enum BinaryProvider {
    Github(github::GithubProvider),
    NpmMirror(npm_mirror::NpmMirrorProvider),
    Bucket(bucket::BucketProvider),
    Imagemin(imagemin::ImageminProvider),
}
