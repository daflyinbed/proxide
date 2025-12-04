use crate::{
    binary::{BinaryEntry, BinarySource},
    config::GithubConfig,
};
use anyhow::Result;
use futures::{Stream, StreamExt, future};
use octocrab::Octocrab;

pub struct GithubProvider {
    config: GithubConfig,
    api: Octocrab,
}
impl GithubProvider {
    pub fn new(config: GithubConfig) -> Result<Self> {
        let api = octocrab::OctocrabBuilder::default().build()?;
        Ok(Self { config, api })
    }
}

impl BinarySource for GithubProvider {
    async fn list(&self, dir: &str) -> Result<impl Stream<Item = Result<Vec<BinaryEntry>>>> {
        let releases = self
            .api
            .repos(&self.config.owner, &self.config.repo)
            .releases()
            .list()
            .per_page(100)
            .send()
            .await?
            .into_stream(&self.api);
        let result = releases
            .filter(move |release| {
                if dir == "/" {
                    return future::ready(true);
                }
                match release {
                    Ok(rel) => future::ready(dir == format!("/{}/", rel.tag_name)),
                    _ => future::ready(true),
                }
            })
            .map(move |release| {
                release
                    .map(|release| {
                        if dir == "/" {
                            return vec![BinaryEntry {
                                name: format!("{}/", release.tag_name),
                                is_dir: true,
                                url: Some(release.url.to_string()),
                                size: None,
                                date: release.published_at,
                            }];
                        }
                        let size = release.assets.len()
                            + if release.tarball_url.is_some() { 1 } else { 0 }
                            + if release.zipball_url.is_some() { 1 } else { 0 };
                        let mut result = Vec::with_capacity(size);
                        for asset in release.assets {
                            result.push(BinaryEntry {
                                name: asset.name,
                                is_dir: false,
                                url: Some(asset.browser_download_url.to_string()),
                                size: Some(asset.size as u64),
                                date: Some(asset.updated_at),
                            })
                        }
                        if release.tarball_url.is_some() {
                            result.push(BinaryEntry {
                                name: format!("{}.tar.gz", release.tag_name),
                                is_dir: false,
                                url: Some(format!(
                                    "https://github.com/{}/{}/archive/{}.tar.gz",
                                    self.config.owner, self.config.repo, release.tag_name
                                )),
                                size: None,
                                date: release.published_at,
                            });
                        }
                        if release.zipball_url.is_some() {
                            result.push(BinaryEntry {
                                name: format!("{}.tar.gz", release.tag_name),
                                is_dir: false,
                                url: Some(format!(
                                    "https://github.com/{}/{}/archive/{}.tar.gz",
                                    self.config.owner, self.config.repo, release.tag_name
                                )),
                                size: None,
                                date: release.published_at,
                            });
                        }
                        result
                    })
                    .map_err(|e| e.into())
            });
        Ok(result)
    }
}
