use crate::{
    binary::{BinaryEntry, BinarySource},
    config::ImageminConfig,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use futures::{StreamExt, stream};
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

    async fn entries_stream<'a>(
        &'a self,
        dir: &'a str,
        versions: Vec<VersionInfo>,
    ) -> Result<BoxStream<'a, Result<BinaryEntry>>> {
        let trimmed = dir.trim_matches('/');
        let segments: Vec<String> = if trimmed.is_empty() {
            Vec::new()
        } else {
            trimmed.split('/').map(|s| s.to_string()).collect()
        };
        let mut iter = segments.into_iter();
        let first = iter.next();
        let second = iter.next();
        let third = iter.next();
        let fourth = iter.next();
        let stream: BoxStream<'a, Result<BinaryEntry>> = match (first, second, third, fourth) {
            (None, None, None, None) => self.root_entries(versions),
            (Some(version_segment), None, None, None) => {
                self.version_root_entries(&versions, version_segment.as_str())
            }
            (Some(version_segment), Some(vendor), None, None) if vendor == "vendor" => {
                self.platform_entries(&versions, version_segment.as_str())
            }
            (Some(version_segment), Some(vendor), Some(platform), None) if vendor == "vendor" => {
                self.vendor_platform_entries(
                    dir,
                    &versions,
                    version_segment.as_str(),
                    platform.as_str(),
                )
                .await?
            }
            (Some(version_segment), Some(vendor), Some(platform), Some(arch))
                if vendor == "vendor" =>
            {
                let info = Self::find_version(&versions, version_segment.as_str()).cloned();
                if let Some(info) = info {
                    self.vendor_platform_arch_entries_stream(dir, info, platform, arch)
                } else {
                    stream::empty().boxed()
                }
            }

            _ => stream::empty().boxed(),
        };

        Ok(stream)
    }

    fn root_entries(&self, versions: Vec<VersionInfo>) -> BoxStream<'static, Result<BinaryEntry>> {
        stream::iter(versions.into_iter().map(|info| {
            Ok(BinaryEntry {
                name: format!("v{}/", info.version),
                is_dir: true,
                url: None,
                size: None,
                date: info.date,
            })
        }))
        .boxed()
    }

    fn version_root_entries(
        &self,
        versions: &[VersionInfo],
        version_segment: &str,
    ) -> BoxStream<'static, Result<BinaryEntry>> {
        Self::find_version(versions, version_segment)
            .map(|info| {
                stream::iter([Ok(BinaryEntry {
                    name: "vendor/".to_string(),
                    is_dir: true,
                    url: None,
                    size: None,
                    date: info.date.clone(),
                })])
                .boxed()
            })
            .unwrap_or_else(|| stream::empty().boxed())
    }

    fn platform_entries(
        &self,
        versions: &[VersionInfo],
        version_segment: &str,
    ) -> BoxStream<'static, Result<BinaryEntry>> {
        let platforms = self.config.node_platforms.clone();
        Self::find_version(versions, version_segment)
            .map(|info| {
                let date = info.date.clone();
                stream::iter(platforms.into_iter().map(move |platform| {
                    Ok(BinaryEntry {
                        name: format!("{platform}/"),
                        is_dir: true,
                        url: None,
                        size: None,
                        date: date.clone(),
                    })
                }))
                .boxed()
            })
            .unwrap_or_else(|| stream::empty().boxed())
    }

    async fn vendor_platform_entries<'a>(
        &'a self,
        dir: &'a str,
        versions: &[VersionInfo],
        version_segment: &str,
        platform: &str,
    ) -> Result<BoxStream<'a, Result<BinaryEntry>>> {
        if !self.is_supported_platform(platform) {
            return Ok(stream::empty().boxed());
        }

        let Some(info) = Self::find_version(versions, version_segment) else {
            return Ok(stream::empty().boxed());
        };

        if let Some(archs) = self
            .config
            .node_archs
            .get(platform)
            .cloned()
            .filter(|list| !list.is_empty())
        {
            let date = info.date.clone();
            let stream = stream::iter(archs.into_iter().map(move |arch| {
                Ok(BinaryEntry {
                    name: format!("{arch}/"),
                    is_dir: true,
                    url: None,
                    size: None,
                    date: date.clone(),
                })
            }))
            .boxed();
            Ok(stream)
        } else {
            let Some(files) = self.config.bin_files.get(platform).cloned() else {
                return Ok(stream::empty().boxed());
            };

            let dir = dir.to_string();
            let date = info.date.clone();
            Ok(stream::iter(files.into_iter())
                .then(move |file| {
                    let url = self.artifact_url(&dir, &file);
                    let date = date.clone();
                    let this = self;
                    async move {
                        let size = this.fetch_content_length(&url).await?;
                        Ok(BinaryEntry {
                            name: file,
                            is_dir: false,
                            url: Some(url),
                            size,
                            date,
                        })
                    }
                })
                .boxed())
        }
    }

    fn vendor_platform_arch_entries_stream<'a>(
        &'a self,
        dir: &'a str,
        info: VersionInfo,
        platform: String,
        arch: String,
    ) -> BoxStream<'a, Result<BinaryEntry>> {
        use futures::stream;
        if !self.is_supported_platform(&platform) {
            return stream::empty().boxed();
        }

        let arch_exists = self
            .config
            .node_archs
            .get(&platform)
            .map(|list| list.iter().any(|value| value == &arch))
            .unwrap_or(false);

        if !arch_exists {
            return stream::empty().boxed();
        }

        let Some(files) = self.config.bin_files.get(&platform) else {
            return stream::empty().boxed();
        };

        let dir = dir.to_string();
        let info_date = info.date.clone();

        let entries_stream = stream::iter(files.iter().cloned()).then(move |file| {
            let url = self.artifact_url(&dir, &file);
            let date = info_date.clone();
            let this = self;
            async move {
                let size = this.fetch_content_length(&url).await?;
                Ok(BinaryEntry {
                    name: file,
                    is_dir: false,
                    url: Some(url),
                    size,
                    date,
                })
            }
        });
        entries_stream.boxed()
    }
}

#[derive(Debug, Deserialize)]
struct NpmPackageResponse {
    #[serde(default)]
    versions: HashMap<String, Value>,
    #[serde(default)]
    time: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct VersionInfo {
    /// semver
    version: String,
    date: Option<DateTime<Utc>>,
}

impl BinarySource for ImageminProvider {
    async fn list<'a>(&'a self, dir: &'a str) -> Result<BoxStream<'a, Result<BinaryEntry>>> {
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
