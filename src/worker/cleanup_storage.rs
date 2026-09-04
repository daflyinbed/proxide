use crate::config::StorageGcConfig;
use crate::repository::Repository;
use crate::storage::backend::is_valid_cas_path;
use anyhow::Result;
use futures::StreamExt;
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
        let mut result_count = 0usize;
        for (path, result) in results {
            result_count += 1;
            let id = ids_by_path.get(path.as_str()).ok_or_else(|| {
                anyhow::anyhow!("storage GC received an unexpected delete result for {path}")
            })?;
            match result {
                Ok(()) => {
                    deleted_ids.push(*id);
                }
                Err(error) => {
                    log::error!(action = "storage_gc_delete_failed"; "path={path} error={error:#}");
                }
            }
        }
        if result_count != orphans.len() {
            anyhow::bail!(
                "storage GC received {} delete results for {} objects",
                result_count,
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
    let batch_size = config.batch_size.clamp(1, 1000) as usize;
    let started = Instant::now();
    let budget =
        (config.max_duration_secs > 0).then(|| Duration::from_secs(config.max_duration_secs));
    let mut objects = repo.list_storage_objects("objects");
    let mut candidates = Vec::with_capacity(batch_size);
    let mut deleted = 0u64;
    while let Some(object) = objects.next().await {
        if budget.is_some_and(|budget| started.elapsed() >= budget) {
            break;
        }
        let object = object?;
        if object.last_modified >= cutoff {
            continue;
        }
        if !is_valid_cas_path(&object.path) {
            log::error!(action = "storage_full_scan_invalid_path"; "path={}", object.path);
            continue;
        }
        candidates.push(object.path);
        if candidates.len() == batch_size {
            deleted += delete_untracked_batch(repo, &candidates).await?;
            candidates.clear();
        }
    }
    if !candidates.is_empty() {
        deleted += delete_untracked_batch(repo, &candidates).await?;
    }
    Ok(deleted)
}

async fn delete_untracked_batch(repo: &Arc<dyn Repository>, paths: &[String]) -> Result<u64> {
    let existing = repo.existing_dist_paths(paths).await?;
    let untracked: Vec<String> = paths
        .iter()
        .filter(|path| !existing.contains(path.as_str()))
        .cloned()
        .collect();
    let mut deleted = 0u64;
    for (path, result) in repo.delete_storage_objects(&untracked).await {
        match result {
            Ok(()) => deleted += 1,
            Err(error) => {
                log::error!(action = "storage_full_scan_delete_failed"; "path={path} error={error:#}");
            }
        }
    }
    Ok(deleted)
}
