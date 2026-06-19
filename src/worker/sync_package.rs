use crate::config::Config;
use crate::npm::types::*;
use crate::npm::{build_abbreviated_version_entry, is_prerelease, pad_version, split_scope_name};
use crate::repository::{
    PackageVersionRow, PendingDist, Repository, SyncManifestParams, VersionCommitParams,
};
use crate::search::{build_search_document, sum_downloads, sum_local_downloads, SearchIndex};
use crate::state::{LockOwner, PackageLock, UnlockGuard};
use anyhow::{Context, Result};
use chrono::Datelike;
use chrono::NaiveDateTime;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

fn build_abbreviated_manifest(packument: &Packument) -> AbbreviatedPackument {
    let mut versions = HashMap::new();
    for (ver, data) in &packument.versions {
        versions.insert(
            ver.clone(),
            build_abbreviated_version_entry(data, packument.time.get(ver)),
        );
    }
    let time = if packument.time.is_empty() {
        None
    } else {
        Some(packument.time.clone())
    };
    AbbreviatedPackument {
        name: packument.name.clone(),
        modified: packument.time.get("modified").cloned(),
        dist_tags: packument.dist_tags.clone(),
        versions,
        time,
    }
}

fn build_abbreviated_version(name: &str, ver: &PackageVersion) -> Vec<u8> {
    let entry = build_abbreviated_version_entry(ver, None);
    let mut map = serde_json::to_value(&entry).unwrap_or_default();
    map["name"] = serde_json::Value::String(name.to_string());
    serde_json::to_vec(&map).unwrap_or_default()
}

pub enum SyncPackageError {
    Conflict(String),
    Other(anyhow::Error),
}

impl std::fmt::Display for SyncPackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncPackageError::Conflict(msg) => write!(f, "{msg}"),
            SyncPackageError::Other(err) => write!(f, "{err:#}"),
        }
    }
}

impl From<anyhow::Error> for SyncPackageError {
    fn from(err: anyhow::Error) -> Self {
        SyncPackageError::Other(err)
    }
}

pub async fn sync_package(
    repo: &Arc<dyn Repository>,
    config: &Config,
    fullname: &str,
    client: &reqwest::Client,
    package_lock: &PackageLock,
    search: Option<&SearchIndex>,
) -> Result<(), SyncPackageError> {
    if !package_lock.try_lock(fullname, LockOwner::Sync) {
        return Err(SyncPackageError::Conflict(format!(
            "package {fullname} is locked by {}",
            package_lock
                .get_owner(fullname)
                .map(|o| o.to_string())
                .unwrap_or_default()
        )));
    }
    let _guard = UnlockGuard::new(package_lock, fullname.to_string());

    let mut request = client
        .get(format!("{}/{fullname}", config.worker.upstream_registry))
        .header("accept", "application/json");

    if !config.worker.upstream_auth_token.is_empty() {
        request = request.bearer_auth(&config.worker.upstream_auth_token);
    }

    let resp = request.send().await.context("failed to fetch upstream")?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(SyncPackageError::Other(anyhow::anyhow!(
            "package {fullname} not found upstream"
        )));
    }
    if !resp.status().is_success() {
        return Err(SyncPackageError::Other(anyhow::anyhow!(
            "upstream returned status {}",
            resp.status()
        )));
    }

    let raw_bytes = resp
        .bytes()
        .await
        .context("failed to read upstream response body")?;

    let mut packument: Packument =
        serde_json::from_slice(&raw_bytes).context("failed to parse upstream packument")?;

    for ver_data in packument.versions.values_mut() {
        if let Some(filename) = extract_tarball_filename(&ver_data.dist.tarball) {
            ver_data.dist.tarball =
                format!("{}/npm/{fullname}/-/{filename}", config.server.root_url);
        }
    }

    let (scope, _name) = split_scope_name(fullname);

    let (package_id, existing_source) = repo
        .upsert_package(
            fullname,
            scope,
            packument.description.as_deref(),
            Some(&config.worker.upstream_name),
        )
        .await?;

    if existing_source.as_deref() != Some(&config.worker.upstream_name) && existing_source.is_some()
    {
        return Err(SyncPackageError::Conflict(format!(
            "package {fullname} is a locally published package, sync is not allowed"
        )));
    }

    if let Some(upstream_maintainers) = &packument.maintainers {
        let mut user_ids: Vec<i64> = Vec::with_capacity(upstream_maintainers.len());
        for m in upstream_maintainers {
            let uid = repo
                .upsert_user(&m.name, m.email.as_deref(), &config.worker.upstream_name)
                .await?;
            user_ids.push(uid);
        }
        repo.sync_maintainers(package_id, &user_ids).await?;
    }

    let existing_versions = repo.list_versions(package_id).await?;
    let existing_map: HashMap<String, &PackageVersionRow> = existing_versions
        .iter()
        .map(|v| (v.version.clone(), v))
        .collect();

    // ── Phase 1: Upload each version independently (S3 then DB transaction) ──

    log::info!(action = "sync_progress"; "name={fullname} phase=upload_versions uploading version dists");

    let mut new_count = 0u32;

    for (ver_str, ver_data) in &packument.versions {
        if existing_map.contains_key(ver_str) {
            continue;
        }

        let publish_time = packument
            .time
            .get(ver_str)
            .or(packument.time.get("modified"))
            .and_then(|t| NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f").ok())
            .unwrap_or_else(|| chrono::Utc::now().naive_utc());

        let is_pre_release = is_prerelease(ver_str);
        let padding_version = Some(pad_version(ver_str));

        let abbrev_storage_key = format!("packages/{fullname}/{ver_str}/abbreviated.json");
        let manifest_storage_key = format!("packages/{fullname}/{ver_str}/package.json");

        let abbrev_data = build_abbreviated_version(&ver_data.name, ver_data);
        let manifest_data = serde_json::to_vec(&ver_data).unwrap_or_default();

        if let Err(e) = repo
            .put_storage(&abbrev_storage_key, abbrev_data.clone())
            .await
        {
            error!("Storage upload failed for {fullname}@{ver_str} abbreviated: {e:#}");
            continue;
        }

        if let Err(e) = repo
            .put_storage(&manifest_storage_key, manifest_data.clone())
            .await
        {
            error!("Storage upload failed for {fullname}@{ver_str} manifest: {e:#}");
            continue;
        }

        let version_params = VersionCommitParams {
            package_id,
            version: ver_str.clone(),
            publish_time,
            is_pre_release,
            padding_version,
            abbrev_dist: PendingDist {
                name: format!("{fullname}@{ver_str}-abbrev"),
                path: abbrev_storage_key.clone(),
                size: abbrev_data.len() as i64,
                shasum: None,
                integrity: None,
            },
            manifest_dist: PendingDist {
                name: format!("{fullname}@{ver_str}-manifest"),
                path: manifest_storage_key.clone(),
                size: manifest_data.len() as i64,
                shasum: None,
                integrity: None,
            },
        };

        if let Err(e) = repo.commit_version(version_params).await {
            error!("DB commit failed for {fullname}@{ver_str}: {e:#}");
            continue;
        }

        new_count += 1;
    }

    // ── Phase 2: Upload manifests + sync tags + swap pointers (transactional) ──

    log::info!(action = "sync_progress"; "name={fullname} phase=sync_manifests uploading manifests and swapping pointers");

    let old_pkg = repo.get_package_by_name(fullname).await?;
    let old_abbrev_dist_id = old_pkg.as_ref().and_then(|p| p.abbreviated_dist_id);
    let old_full_dist_id = old_pkg.as_ref().and_then(|p| p.full_dist_id);

    let abbreviated_manifest = build_abbreviated_manifest(&packument);
    let abbrev_storage_key = format!("packages/{fullname}/abbreviated_manifests.json");
    let full_storage_key = format!("packages/{fullname}/full_manifests.json");

    let abbrev_bytes = serde_json::to_vec(&abbreviated_manifest).unwrap_or_default();
    let full_bytes = serde_json::to_vec(&packument).unwrap_or_default();

    repo.put_storage(&abbrev_storage_key, abbrev_bytes.clone())
        .await
        .context("failed to upload abbreviated manifest to storage")?;
    repo.put_storage(&full_storage_key, full_bytes.clone())
        .await
        .context("failed to upload full manifest to storage")?;

    let params = SyncManifestParams {
        package_id,
        tags: packument.dist_tags.clone(),
        abbrev_manifest: PendingDist {
            name: format!("{fullname}-abbrev-manifests"),
            path: abbrev_storage_key.clone(),
            size: abbrev_bytes.len() as i64,
            shasum: None,
            integrity: None,
        },
        full_manifest: PendingDist {
            name: format!("{fullname}-full-manifests"),
            path: full_storage_key.clone(),
            size: full_bytes.len() as i64,
            shasum: None,
            integrity: None,
        },
    };

    if let Err(db_err) = repo.sync_manifest_commit(params).await {
        return Err(SyncPackageError::Other(
            db_err.context("DB transaction failed for manifest commit"),
        ));
    }

    if let Some(idx) = search {
        let now = chrono::Utc::now();
        let start = now - chrono::Duration::days(365);
        let upstream = crate::search::unwrap_or_log(
            repo.query_upstream_downloads(
                package_id,
                start.year() as u16,
                start.month() as u8,
                now.year() as u16,
                now.month() as u8,
            )
            .await,
            || format!("upstream downloads, package_id={package_id}"),
        );
        let local = crate::search::unwrap_or_log(
            repo.query_package_downloads_by_package(
                package_id,
                start.year() as u16,
                start.month() as u8,
                now.year() as u16,
                now.month() as u8,
            )
            .await,
            || format!("local downloads, package_id={package_id}"),
        );
        let doc = build_search_document(
            package_id,
            &packument,
            sum_downloads(&upstream),
            sum_local_downloads(&local),
        );
        if let Err(e) = idx.upsert_package(&doc).await {
            log::warn!(action = "search_index_upsert"; "name={fullname} failed: {e:#}");
        }
    }

    // ── Phase 3: Clean up old data (best-effort) ──

    log::info!(action = "sync_progress"; "name={fullname} phase=cleanup cleaning up old versions");

    let upstream_version_keys: Vec<String> = packument.versions.keys().cloned().collect();
    let versions_to_delete = repo
        .get_versions_not_in(package_id, &upstream_version_keys)
        .await?;

    if !versions_to_delete.is_empty() {
        let orphan_dist_ids: Vec<i64> = versions_to_delete
            .iter()
            .flat_map(|v| {
                [
                    &v.abbrev_dist_id,
                    &v.manifest_dist_id,
                    &v.tar_dist_id,
                    &v.readme_dist_id,
                ]
                .into_iter()
                .filter_map(|id| *id)
            })
            .collect();

        let version_ids: Vec<i64> = versions_to_delete.iter().map(|v| v.id).collect();
        repo.delete_versions_by_ids(&version_ids).await?;

        for &dist_id in &orphan_dist_ids {
            if let Err(e) = repo.delete_content(dist_id).await {
                error!("failed to delete orphan dist {dist_id}: {e:#}");
            }
        }
    }

    let old_manifest_dist_ids: Vec<i64> = [old_abbrev_dist_id, old_full_dist_id]
        .into_iter()
        .flatten()
        .collect();
    for dist_id in &old_manifest_dist_ids {
        if let Err(e) = repo.delete_content(*dist_id).await {
            error!("failed to delete old manifest dist {dist_id}: {e:#}");
        }
    }

    log::info!(
        action = "sync_done";
        "synced {fullname}: {new_count} new versions"
    );

    Ok(())
}

fn extract_tarball_filename(url: &str) -> Option<String> {
    let last_segment = url.rsplit('/').next()?;
    if last_segment.ends_with(".tgz") {
        Some(last_segment.to_string())
    } else {
        None
    }
}
