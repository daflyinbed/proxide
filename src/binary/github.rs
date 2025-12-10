use crate::{
    binary::{BinaryEntry, BinarySource},
    config::GithubConfig,
};
use anyhow::Result;
use futures::stream::BoxStream;
use futures::{StreamExt, future, stream};
use octocrab::Octocrab;

pub struct GithubProvider {
    config: GithubConfig,
    api: Octocrab,
}
impl GithubProvider {
    pub fn new(config: GithubConfig, api: Octocrab) -> Result<Self> {
        Ok(Self { config, api })
    }
}

impl BinarySource for GithubProvider {
    async fn list<'a>(&'a self, dir: &'a str) -> Result<BoxStream<'a, Result<BinaryEntry>>> {
        let releases = self
            .api
            .repos(&self.config.owner, &self.config.repo)
            .releases()
            .list()
            .per_page(100)
            .send()
            .await?
            .into_stream(&self.api);
        let stream = releases
            .filter(move |release| {
                if dir == "/" {
                    return future::ready(true);
                }
                match release {
                    Ok(rel) => future::ready(dir == format!("/{}/", rel.tag_name)),
                    _ => future::ready(true),
                }
            })
            .flat_map(move |release| match release {
                Ok(release) => {
                    let entries = if dir == "/" {
                        vec![BinaryEntry {
                            name: format!("{}/", release.tag_name),
                            is_dir: true,
                            url: Some(release.url.to_string()),
                            size: None,
                            date: release.published_at,
                        }]
                    } else {
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
                                name: format!("{}.zip", release.tag_name),
                                is_dir: false,
                                url: Some(format!(
                                    "https://github.com/{}/{}/archive/{}.zip",
                                    self.config.owner, self.config.repo, release.tag_name
                                )),
                                size: None,
                                date: release.published_at,
                            });
                        }
                        result
                    };

                    stream::iter(entries.into_iter().map(Ok)).boxed()
                }
                Err(err) => stream::once(async { Err(err.into()) }).boxed(),
            });

        Ok(stream.boxed())
    }
}
