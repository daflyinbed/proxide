use crate::config::Config;
use crate::npm::types::{ChangesResult, UpdateSeqResponse};
use crate::repository::Repository;
use anyhow::{Context, Result};
use std::sync::Arc;

pub async fn run_changes_poller(
    repo: Arc<dyn Repository>,
    config: Config,
    client: reqwest::Client,
) -> Result<()> {
    let interval = std::time::Duration::from_secs(config.worker.cron_interval_secs);

    log::info!(
        action = "poller_start";
        "changes poller starting, interval={}s, upstream={}",
        config.worker.cron_interval_secs,
        config.worker.changes_stream_url,
    );

    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;
        if let Err(e) = poll_once(&repo, &config, &client).await {
            log::error!(action = "poll_error"; "{e:#}");
        }
    }
}

async fn poll_once(
    repo: &Arc<dyn Repository>,
    config: &Config,
    client: &reqwest::Client,
) -> Result<()> {
    let since = match repo.get_cursor().await? {
        Some(c) => c.since,
        None => {
            let initial = fetch_initial_since(config, client).await?;
            log::info!(
                action = "init_since";
                "no cursor found, fetching initial since={initial} from update_seq_url={}",
                config.worker.update_seq_url,
            );
            initial
        }
    };

    let url = format!("{}?since={since}", config.worker.changes_stream_url);

    log::debug!(action = "poll_changes"; "polling changes from {url}");

    let resp = client
        .get(&url)
        .send()
        .await
        .context("failed to fetch changes")?;

    if !resp.status().is_success() {
        anyhow::bail!("changes endpoint returned {}", resp.status());
    }

    let changes: ChangesResult = resp.json().await.context("failed to parse changes")?;

    let last_seq = changes.last_seq.to_string();
    let package_names: Vec<String> = changes
        .results
        .into_iter()
        .filter(|c| c.deleted != Some(true) && !c.id.starts_with('_'))
        .map(|c| c.id)
        .collect();

    let count = package_names.len();
    log::info!(action = "poll_changes"; "got {count} changed packages since {since}");

    for name in &package_names {
        match repo.enqueue_sync_task(name, "poller").await {
            Ok(Some(task_id)) => {
                log::info!(action = "enqueue"; "enqueued sync task source=poller name={name} task_id={task_id}");
            }
            Ok(None) => {
                log::debug!(action = "enqueue_dedup"; "skipped, pending task exists source=poller name={name}");
            }
            Err(e) => {
                log::warn!(action = "enqueue_error"; "failed to enqueue name={name}: {e:#}");
            }
        }
    }

    repo.upsert_cursor(&last_seq).await?;

    Ok(())
}

async fn fetch_initial_since(config: &Config, client: &reqwest::Client) -> Result<String> {
    let url = &config.worker.update_seq_url;
    let resp = client.get(url).send().await
        .with_context(|| format!("failed to fetch update_seq from {url}"))?;

    if !resp.status().is_success() {
        anyhow::bail!("update_seq endpoint {url} returned {}", resp.status());
    }

    let body: UpdateSeqResponse = resp
        .json()
        .await
        .with_context(|| format!("failed to parse update_seq response from {url}"))?;

    let since = body.update_seq.saturating_sub(10);
    Ok(since.to_string())
}
