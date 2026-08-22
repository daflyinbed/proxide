use crate::config::Config;
use crate::repository::Repository;
use crate::search::SearchIndex;
use crate::state::PackageLock;
use crate::worker::sync_package::{self, SyncPackageError};
use std::sync::Arc;
use std::time::Duration;

pub async fn run_task_consumer(
    repo: Arc<dyn Repository>,
    config: Config,
    client: reqwest::Client,
    package_lock: PackageLock,
    search: Option<Arc<SearchIndex>>,
) -> anyhow::Result<()> {
    let poll_interval = Duration::from_millis(config.worker.consumer_poll_interval_ms);

    loop {
        match repo.claim_sync_task().await {
            Ok(Some(task)) => {
                log::info!(
                    action = "sync_start";
                    "syncing package task_id={} name={} source={} attempt={}",
                    task.id, task.name, task.source, task.attempts,
                );

                let start = std::time::Instant::now();
                let result = sync_package::sync_package(
                    &repo,
                    &config,
                    &task.name,
                    &client,
                    &package_lock,
                    search.as_deref(),
                )
                .await;
                let elapsed = start.elapsed().as_millis() as u64;

                match result {
                    Ok(()) => {
                        log::info!(
                            action = "sync_done";
                            "task_id={} name={} elapsed_ms={elapsed}",
                            task.id, task.name,
                        );
                        repo.complete_sync_task(task.id, None).await?;
                    }
                    Err(SyncPackageError::Conflict(msg)) => {
                        log::warn!(
                            action = "sync_conflict";
                            "task_id={} name={} conflict: {msg}",
                            task.id, task.name,
                        );
                        repo.fail_task_no_retry(task.id, &msg).await?;
                    }
                    Err(SyncPackageError::Other(e)) => {
                        let err_str = format!("{e:#}");
                        log::warn!(
                            action = "sync_failed";
                            "task_id={} name={} attempt={}/{} error={err_str}",
                            task.id, task.name, task.attempts, task.max_attempts,
                        );
                        if task.attempts >= task.max_attempts {
                            log::error!(
                                action = "sync_dead";
                                "task exhausted retries task_id={} name={} attempts={}",
                                task.id, task.name, task.attempts,
                            );
                        }
                        repo.complete_sync_task(task.id, Some(&err_str)).await?;
                    }
                }

                if let Ok(statuses) = repo.count_tasks_by_status().await {
                    let pending = statuses.get("pending").copied().unwrap_or(0);
                    let running = statuses.get("running").copied().unwrap_or(0);
                    let done = statuses.get("done").copied().unwrap_or(0);
                    let failed = statuses.get("failed").copied().unwrap_or(0);
                    log::info!(
                        action = "queue_summary";
                        "queue status pending={pending} running={running} done={done} failed={failed}",
                    );
                }
            }
            Ok(None) => {
                tokio::time::sleep(poll_interval).await;
            }
            Err(e) => {
                log::error!(action = "claim_error"; "failed to claim task: {e:#}");
                tokio::time::sleep(poll_interval).await;
            }
        }
    }
}
