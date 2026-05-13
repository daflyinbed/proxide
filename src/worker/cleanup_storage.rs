use crate::repository::Repository;
use std::sync::Arc;
use tracing::error;

pub async fn cleanup_orphan_storage(repo: &Arc<dyn Repository>) -> anyhow::Result<()> {
    let orphans = repo.list_orphan_dists().await?;

    if orphans.is_empty() {
        println!("No orphan storage objects found.");
        return Ok(());
    }

    println!("Found {} orphan dist(s) in DB:", orphans.len());
    for d in &orphans {
        println!("  id={} path={}", d.id, d.path);
    }

    let mut deleted = 0u32;
    let mut failed = 0u32;
    let mut ids_to_delete: Vec<i64> = Vec::new();

    for d in &orphans {
        match repo.delete_content(d.id).await {
            Ok(()) => {
                deleted += 1;
            }
            Err(e) => {
                error!("failed to delete orphan dist id={} path={}: {e:#}", d.id, d.path);
                failed += 1;
                ids_to_delete.push(d.id);
            }
        }
    }

    if !ids_to_delete.is_empty() {
        let db_deleted = repo.delete_dists_by_ids(&ids_to_delete).await?;
        println!("Force-deleted {db_deleted} dist row(s) from DB (storage objects may still exist).");
    }

    println!("Done: {deleted} object(s) deleted, {failed} deletion(s) failed.");
    Ok(())
}
