pub mod document;

pub use document::{
    build_search_document, sum_downloads, sum_local_downloads, SearchDocument,
};

use anyhow::{Context, Result};
use meilisearch_sdk::client::Client;
use meilisearch_sdk::errors::{Error as MeiliError, ErrorCode};
use meilisearch_sdk::search::SearchResults;
use meilisearch_sdk::settings::Settings;
use meilisearch_sdk::tasks::Task;

use crate::config::SearchConfig;
use crate::npm::types::Packument;
use crate::repository::Repository;
use chrono::Datelike;

const REINDEX_BATCH_SIZE: i64 = 500;

pub struct SearchIndex {
    client: Client,
    index_uid: String,
}

impl SearchIndex {
    pub async fn new(cfg: &SearchConfig) -> Result<Option<Self>> {
        if !cfg.is_enabled() {
            return Ok(None);
        }
        let key = if cfg.meili_key.is_empty() {
            None
        } else {
            Some(cfg.meili_key.as_str())
        };
        let client = Client::new(&cfg.meili_url, key)
            .with_context(|| format!("failed to connect to meilisearch at {}", cfg.meili_url))?;
        Ok(Some(Self {
            client,
            index_uid: cfg.index_name.clone(),
        }))
    }

    pub async fn ensure_index(&self) -> Result<()> {
        match self
            .client
            .create_index(&self.index_uid, Some("id"))
            .await
        {
            Ok(task) => {
                let outcome = task
                    .wait_for_completion(&self.client, None, None)
                    .await
                    .context("index creation task wait failed")?;
                match outcome {
                    Task::Succeeded { .. } => {}
                    Task::Failed { ref content }
                        if content.error.error_code == ErrorCode::IndexAlreadyExists =>
                    {
                        log::info!(
                            action = "search_init";
                            "meilisearch index `{}` already exists; proceeding to apply settings",
                            self.index_uid
                        );
                    }
                    Task::Failed { content } => {
                        return Err(content.error)
                            .context("index creation task failed");
                    }
                    other => {
                        return Err(anyhow::anyhow!(
                            "index creation task ended in unexpected state: {other:?}"
                        ));
                    }
                }
            }
            Err(MeiliError::Meilisearch(e))
                if e.error_code == ErrorCode::IndexAlreadyExists =>
            {
                log::info!(
                    action = "search_init";
                    "meilisearch index `{}` already exists; proceeding to apply settings",
                    self.index_uid
                );
            }
            Err(e) => {
                return Err(e).context("failed to submit index creation task");
            }
        }

        let index = self.client.index(&self.index_uid);
        let settings = Settings::new()
            .with_searchable_attributes([
                "package.name",
                "package.description",
                "package.keywords",
                "package.author.name",
                "package.maintainers.name",
            ])
            .with_ranking_rules([
                "words",
                "typo",
                "proximity",
                "attribute",
                "downloads.upstream:desc",
                "downloads.local:desc",
                "exactness",
            ])
            .with_filterable_attributes([
                "package.scope",
                "package.deprecated",
                "package.created",
            ])
            .with_sortable_attributes([
                "downloads.upstream",
                "downloads.local",
                "package.date",
                "package.created",
            ]);
        let task = index
            .set_settings(&settings)
            .await
            .context("failed to submit settings task")?;
        task.wait_for_completion(&self.client, None, None)
            .await
            .context("settings task failed")?;

        Ok(())
    }

    pub async fn upsert_package(&self, doc: &SearchDocument) -> Result<()> {
        let index = self.client.index(&self.index_uid);
        index
            .add_or_replace(&[doc], None)
            .await
            .context("failed to submit upsert task")?;
        Ok(())
    }

    pub async fn upsert_many(&self, docs: &[SearchDocument]) -> Result<()> {
        if docs.is_empty() {
            return Ok(());
        }
        let index = self.client.index(&self.index_uid);
        index
            .add_or_replace(docs, None)
            .await
            .context("failed to submit batch upsert task")?;
        Ok(())
    }

    pub async fn remove_package(&self, package_id: i64) -> Result<()> {
        let index = self.client.index(&self.index_uid);
        index
            .delete_document(&package_id.to_string())
            .await
            .context("failed to submit delete task")?;
        Ok(())
    }

    pub async fn search(
        &self,
        text: &str,
        offset: usize,
        limit: usize,
    ) -> Result<SearchResults<SearchDocument>> {
        let index = self.client.index(&self.index_uid);
        let results = index
            .search()
            .with_query(text)
            .with_offset(offset)
            .with_limit(limit)
            .execute::<SearchDocument>()
            .await
            .context("meilisearch query failed")?;
        Ok(results)
    }
}

pub fn unwrap_or_log<T: Default, E: std::fmt::Display>(
    res: Result<T, E>,
    ctx: impl FnOnce() -> String,
) -> T {
    match res {
        Ok(v) => v,
        Err(e) => {
            log::warn!(
                action = "search_downloads";
                "download aggregation query failed ({}): {e:#}",
                ctx()
            );
            T::default()
        }
    }
}

pub async fn reindex_all(repo: &dyn Repository, index: &SearchIndex) -> Result<()> {
    let now = chrono::Utc::now();
    let start = now - chrono::Duration::days(365);
    let mut offset = 0i64;

    loop {
        let packages = repo.list_packages(offset, REINDEX_BATCH_SIZE).await?;
        if packages.is_empty() {
            break;
        }
        let batch_len = packages.len();

        let mut docs = Vec::with_capacity(packages.len());
        for pkg in &packages {
            let Some(full_dist_id) = pkg.full_dist_id else {
                continue;
            };
            let (bytes, _) = match repo.get_content(full_dist_id).await {
                Ok(data) => data,
                Err(e) => {
                    log::warn!(action = "reindex"; "skip package {} (failed to read manifest): {e:#}", pkg.name);
                    continue;
                }
            };
            let packument: Packument = match serde_json::from_slice(&bytes) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!(action = "reindex"; "skip package {} (invalid manifest json): {e}", pkg.name);
                    continue;
                }
            };
            let upstream = unwrap_or_log(
                repo.query_upstream_downloads(
                    pkg.id,
                    start.year() as u16,
                    start.month() as u8,
                    now.year() as u16,
                    now.month() as u8,
                )
                .await,
                || format!("upstream downloads, package_id={}", pkg.id),
            );
            let local = unwrap_or_log(
                repo.query_package_downloads_by_package(
                    pkg.id,
                    start.year() as u16,
                    start.month() as u8,
                    now.year() as u16,
                    now.month() as u8,
                )
                .await,
                || format!("local downloads, package_id={}", pkg.id),
            );
            let doc = build_search_document(
                pkg.id,
                &packument,
                sum_downloads(&upstream),
                sum_local_downloads(&local),
            );
            docs.push(doc);
        }

        if !docs.is_empty() {
            if let Err(e) = index.upsert_many(&docs).await {
                log::warn!(action = "reindex"; "batch upsert failed at offset {offset}: {e:#}");
            }
        }
        log::info!(action = "reindex"; "processed {} packages (offset {}), {} indexed", batch_len, offset, docs.len());

        offset += REINDEX_BATCH_SIZE;
    }

    Ok(())
}
