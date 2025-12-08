use crate::{
    binary::{BinaryEntry, BinarySource},
    config::ImageminConfig,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::{Stream, stream};
use reqwest::{Client, header::CONTENT_LENGTH};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

const MIN_SUPPORTED_MAJOR: u64 = 4;

pub struct ImageminProvider {
    category: String,
    config: ImageminConfig,
    client: Client,
}

impl ImageminProvider {
    pub fn new(binary_name: impl Into<String>, config: ImageminConfig, client: Client) -> Self {
        Self {
            category: binary_name.into(),
            config,
            client,
        }
    }

    fn npm_package_name(&self) -> &str {
        self.config
            .npm_package_name
            .as_deref()
            .unwrap_or(&self.category)
    }

    fn artifact_url(&self, dir: &str, name: &str) -> String {
        let base = self.config.dist_url.trim_end_matches('/');
        let repo = self.config.repo.trim_start_matches('/');
        format!("{base}/{repo}{dir}{name}")
    }

    fn collect_versions(pkg: &NpmPackageResponse) -> Vec<VersionInfo> {
        let versions = pkg.versions.keys().cloned().collect::<Vec<_>>();

        let mut infos = Vec::with_capacity(versions.len());
        for version in versions {
            let major = version
                .split('.')
                .next()
                .and_then(|segment| segment.parse::<u64>().ok())
                .unwrap_or_default();
            if major < MIN_SUPPORTED_MAJOR {
                continue;
            }

            let date = pkg.time.get(&version).and_then(|ts| {
                DateTime::parse_from_rfc3339(ts)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            });

            infos.push(VersionInfo { version, date });
        }
        infos
    }

    fn find_version<'a>(versions: &'a [VersionInfo], segment: &str) -> Option<&'a VersionInfo> {
        versions.iter().find(|info| info.version == segment)
    }

    fn is_supported_platform(&self, platform: &str) -> bool {
        self.config
            .node_platforms
            .iter()
            .any(|value| value == platform)
    }

    async fn fetch_content_length(&self, url: &str) -> Result<Option<u64>> {
        let response = self.client.head(url).send().await?;
        if !response.status().is_success() {
            return Ok(None);
        }

        let size = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        Ok(size)
    }

    async fn entries_stream(
        &self,
        dir: &str,
        versions: Vec<VersionInfo>,
    ) -> Result<impl Stream<Item = Result<Vec<BinaryEntry>>>> {
        let trimmed = dir.trim_matches('/');
        let segments: Vec<&str> = if trimmed.is_empty() {
            Vec::new()
        } else {
            trimmed.split('/').collect()
        };

        let mut entries = match segments.as_slice() {
            [] => versions
                .iter()
                .map(|info| BinaryEntry {
                    name: format!("v{}/", info.version),
                    is_dir: true,
                    url: None,
                    size: None,
                    date: info.date.clone(),
                })
                .collect(),
            [version_segment] => {
                let version_segment = *version_segment;
                if let Some(info) = Self::find_version(&versions, version_segment) {
                    vec![BinaryEntry {
                        name: "vendor/".to_string(),
                        is_dir: true,
                        url: None,
                        size: None,
                        date: info.date.clone(),
                    }]
                } else {
                    Vec::new()
                }
            }
            [version_segment, "vendor"] => {
                let version_segment = *version_segment;
                if let Some(info) = Self::find_version(&versions, version_segment) {
                    self.config
                        .node_platforms
                        .iter()
                        .map(|platform| BinaryEntry {
                            name: format!("{platform}/"),
                            is_dir: true,
                            url: None,
                            size: None,
                            date: info.date.clone(),
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            }
            [version_segment, "vendor", platform] => {
                let version_segment = *version_segment;
                let platform = *platform;

                if !self.is_supported_platform(platform) {
                    Vec::new()
                } else if let Some(info) = Self::find_version(&versions, version_segment) {
                    if let Some(archs) = self
                        .config
                        .node_archs
                        .get(platform)
                        .filter(|list| !list.is_empty())
                    {
                        archs
                            .iter()
                            .map(|arch| BinaryEntry {
                                name: format!("{arch}/"),
                                is_dir: true,
                                url: None,
                                size: None,
                                date: info.date.clone(),
                            })
                            .collect()
                    } else {
                        self.config
                            .bin_files
                            .get(platform)
                            .map(|files| {
                                files
                                    .iter()
                                    .map(|file| BinaryEntry {
                                        name: file.clone(),
                                        is_dir: false,
                                        url: Some(self.artifact_url(dir, file)),
                                        size: None,
                                        date: info.date.clone(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    }
                } else {
                    Vec::new()
                }
            }
            [version_segment, "vendor", platform, arch] => {
                let version_segment = *version_segment;
                let platform = *platform;
                let arch = *arch;

                if !self.is_supported_platform(platform) {
                    Vec::new()
                } else if let Some(info) = Self::find_version(&versions, version_segment) {
                    let arch_exists = self
                        .config
                        .node_archs
                        .get(platform)
                        .map(|list| list.iter().any(|value| value == arch))
                        .unwrap_or(false);
                    if !arch_exists {
                        Vec::new()
                    } else {
                        self.config
                            .bin_files
                            .get(platform)
                            .map(|files| {
                                files
                                    .iter()
                                    .map(|file| BinaryEntry {
                                        name: file.clone(),
                                        is_dir: false,
                                        url: Some(self.artifact_url(dir, file)),
                                        size: None,
                                        date: info.date.clone(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    }
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };

        for entry in &mut entries {
            if !entry.is_dir {
                if let Some(url) = &entry.url {
                    entry.size = self.fetch_content_length(url).await?;
                }
            }
        }

        Ok(stream::iter(vec![Ok(entries)]))
    }
}

#[derive(Debug, Deserialize)]
struct NpmPackageResponse {
    #[serde(default)]
    versions: HashMap<String, Value>,
    #[serde(default)]
    time: HashMap<String, String>,
}

#[derive(Debug)]
struct VersionInfo {
    /// semver
    version: String,
    date: Option<DateTime<Utc>>,
}

impl BinarySource for ImageminProvider {
    async fn list(&self, dir: &str) -> Result<impl Stream<Item = Result<Vec<BinaryEntry>>>> {
        let pkg_url = format!("https://registry.npmjs.com/{}", self.npm_package_name());
        let pkg = self
            .client
            .get(&pkg_url)
            .send()
            .await?
            .json::<NpmPackageResponse>()
            .await?;

        let versions = Self::collect_versions(&pkg);
        self.entries_stream(dir, versions).await
    }
}
