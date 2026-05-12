use crate::config::Config;
use crate::repository::Repository;
use std::sync::Arc;

pub async fn run_cleanup_scheduler(
    repo: Arc<dyn Repository>,
    config: Config,
) -> anyhow::Result<()> {
    let interval = std::time::Duration::from_secs(24 * 3600);

    loop {
        if let Err(e) = cleanup_once(&repo, &config).await {
            log::error!(action = "cleanup_error"; "cleanup failed: {e:#}");
        }
        tokio::time::sleep(interval).await;
    }
}

pub async fn cleanup_once(repo: &Arc<dyn Repository>, config: &Config) -> anyhow::Result<()> {
    let stale = repo.requeue_stale_tasks(config.worker.task_timeout_secs).await?;
    if stale > 0 {
        log::warn!(action = "requeue_stale", count = stale; "requeued stale tasks");
    }

    let deleted = repo.cleanup_old_tasks(config.worker.task_retention_days).await?;
    if deleted > 0 {
        log::info!(action = "cleanup_old", count = deleted, retention_days = config.worker.task_retention_days; "cleaned up old tasks");
    }

    Ok(())
}
