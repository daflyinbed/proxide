use crate::config::StorageGcConfig;
use crate::repository::Repository;
use crate::storage::backend::is_valid_cas_path;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub async fn cleanup_orphan_storage(
    repo: &Arc<dyn Repository>,
    config: &StorageGcConfig,
) -> Result<u64> {
    let batch_size = config.batch_size.clamp(1, 1000);
    let started = Instant::now();
    let budget =
        (config.max_duration_secs > 0).then(|| Duration::from_secs(config.max_duration_secs));
    let mut deleted = 0u64;

    loop {
        if budget.is_some_and(|budget| started.elapsed() >= budget) {
            break;
        }
        let orphans = repo
            .list_orphan_dists(config.min_age_secs, batch_size)
            .await?;
        if orphans.is_empty() {
            break;
        }
        let ids_by_path: HashMap<&str, i64> = orphans
            .iter()
            .map(|dist| (dist.path.as_str(), dist.id))
            .collect();
        let paths: Vec<String> = orphans.iter().map(|dist| dist.path.clone()).collect();
        let results = repo.delete_storage_objects(&paths).await;
        let mut deleted_ids = Vec::with_capacity(orphans.len());
        let mut failed = false;
        for (path, result) in results {
            match result {
                Ok(()) => {
                    if let Some(id) = ids_by_path.get(path.as_str()) {
                        deleted_ids.push(*id);
                    }
                }
                Err(error) => {
                    failed = true;
                    log::error!(action = "storage_gc_delete_failed"; "path={path} error={error:#}");
                }
            }
        }
        if failed {
            break;
        }
        if deleted_ids.len() != orphans.len() {
            anyhow::bail!(
                "storage GC received {} delete results for {} objects",
                deleted_ids.len(),
                orphans.len()
            );
        }
        if !deleted_ids.is_empty() {
            let rows = repo.delete_dists_by_ids(&deleted_ids).await?;
            if rows != deleted_ids.len() as u64 {
                anyhow::bail!(
                    "storage GC deleted {} objects but only {rows} dist rows",
                    deleted_ids.len()
                );
            }
            deleted += rows;
        }
        if deleted_ids.is_empty() {
            break;
        }
    }

    Ok(deleted)
}

pub async fn cleanup_untracked_storage(
    repo: &Arc<dyn Repository>,
    config: &StorageGcConfig,
) -> Result<u64> {
    let cutoff =
        chrono::Utc::now() - chrono::Duration::seconds(config.full_scan_min_age_secs as i64);
    let objects = repo.list_storage_objects("objects").await?;
    let mut candidates = Vec::new();
    for object in objects {
        if object.last_modified >= cutoff {
            continue;
        }
        if !is_valid_cas_path(&object.path) {
            log::error!(action = "storage_full_scan_invalid_path"; "path={}", object.path);
            continue;
        }
        if !repo.dist_exists_by_path(&object.path).await? {
            candidates.push(object.path);
        }
    }

    let mut deleted = 0u64;
    for batch in candidates.chunks(config.batch_size.clamp(1, 1000) as usize) {
        for (path, result) in repo.delete_storage_objects(batch).await {
            match result {
                Ok(()) => deleted += 1,
                Err(error) => {
                    log::error!(action = "storage_full_scan_delete_failed"; "path={path} error={error:#}");
                }
            }
        }
    }
    Ok(deleted)
}
