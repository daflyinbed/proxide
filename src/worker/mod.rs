pub mod bootstrap;
pub mod changes_poller;
pub mod cleanup;
pub mod cleanup_storage;
pub mod sync_package;
pub mod task_consumer;

pub use changes_poller::run_changes_poller;
pub use cleanup::run_cleanup_scheduler;
pub use sync_package::sync_package;
pub use task_consumer::run_task_consumer;

use crate::config::Config;
use crate::repository::Repository;
use crate::search::SearchIndex;
use crate::state::PackageLock;
use anyhow::Result;
use std::sync::Arc;

pub async fn run_worker(
    repo: Arc<dyn Repository>,
    config: Config,
    client: reqwest::Client,
    package_lock: PackageLock,
    search: Option<Arc<SearchIndex>>,
) -> Result<()> {
    log::info!(
        consumer_count = config.worker.consumer_count,
        task_timeout_secs = config.worker.task_timeout_secs,
        task_retention_days = config.worker.task_retention_days;
        "worker starting"
    );

    if let Err(e) = cleanup::cleanup_once(&repo, &config).await {
        log::warn!(action = "initial_cleanup"; "initial cleanup failed: {e:#}");
    }

    let mut consumer_handles = Vec::new();
    for i in 0..config.worker.consumer_count {
        let repo = repo.clone();
        let config = config.clone();
        let client = client.clone();
        let package_lock = package_lock.clone();
        let search = search.clone();
        consumer_handles.push(tokio::spawn(async move {
            log::info!(consumer = i; "consumer starting");
            task_consumer::run_task_consumer(repo, config, client, package_lock, search).await
        }));
    }

    let poller_handle = tokio::spawn({
        let repo = repo.clone();
        let config = config.clone();
        let client = client.clone();
        async move { changes_poller::run_changes_poller(repo, config, client).await }
    });

    let cleanup_handle = tokio::spawn({
        let repo = repo.clone();
        let config = config.clone();
        async move { cleanup::run_cleanup_scheduler(repo, config).await }
    });

    let result = tokio::select! {
        r = poller_handle => r?,
        r = cleanup_handle => r?,
        r = async {
            for h in consumer_handles {
                let _ = h.await;
            }
            Ok(())
        } => r,
    };

    result
}
