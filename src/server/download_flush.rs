use crate::state::AppState;
use chrono::Datelike;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub async fn run_download_flush(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        if let Err(e) = flush_download_counters(&state).await {
            log::error!("download counter flush failed: {e:#}");
        }
    }
}

pub async fn flush_download_counters(state: &AppState) -> anyhow::Result<()> {
    let entries: Vec<(i64, u64)> = state
        .download_counters
        .iter()
        .filter_map(|entry| {
            let count = entry.value().swap(0, Ordering::Relaxed);
            if count > 0 {
                Some((*entry.key(), count))
            } else {
                None
            }
        })
        .collect();

    if entries.is_empty() {
        return Ok(());
    }

    let pending_ids: Vec<i64> = entries.iter().map(|(id, _)| *id).collect();
    let existing: Option<HashSet<i64>> = match state.repo.existing_version_ids(&pending_ids).await {
        Ok(ids) => Some(ids.into_iter().collect()),
        Err(e) => {
            log::warn!("existing_version_ids query failed, skipping staleness check: {e:#}");
            None
        }
    };

    let now = chrono::Utc::now();
    let year = now.year() as u16;
    let month = now.month() as u8;
    let day = now.day() as u8;

    for (package_version_id, count) in entries {
        let stale = existing
            .as_ref()
            .is_some_and(|set| !set.contains(&package_version_id));
        if stale {
            log::warn!(
                "discarded download counter for deleted package_version_id={package_version_id} (count={count})"
            );
            continue;
        }
        if let Err(e) = state
            .repo
            .increment_package_download(package_version_id, year, month, day, count)
            .await
        {
            log::error!("failed to flush download counter for pv_id={package_version_id}: {e:#}");
            state
                .download_counters
                .entry(package_version_id)
                .or_insert(AtomicU64::new(0))
                .fetch_add(count, Ordering::Relaxed);
        }
    }

    state
        .download_counters
        .retain(|_, v| v.load(Ordering::Relaxed) > 0);

    Ok(())
}
