use crate::config::Config;
use crate::npm::types::*;
use crate::npm::{build_abbreviated_manifest, is_prerelease, pad_version, split_scope_name};
use crate::repository::{Repository, SyncPackageCommitParams, SyncVersionInput};
use crate::search::SearchIndex;
use crate::state::{LockOwner, PackageLock, UnlockGuard};
use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use std::sync::Arc;

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
    let mut versions = Vec::with_capacity(packument.versions.len());
    for (ver_str, upstream_version) in &packument.versions {
        let publish_time = packument
            .time
            .get(ver_str)
            .or(packument.time.get("modified"))
            .and_then(|t| NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f").ok())
            .unwrap_or_else(|| chrono::Utc::now().naive_utc());

        let is_pre_release = is_prerelease(ver_str);
        let padding_version = Some(pad_version(ver_str));

        versions.push(SyncVersionInput {
            version: ver_str.clone(),
            publish_time,
            is_pre_release,
            padding_version,
            tar_shasum: upstream_version.dist.shasum.clone(),
            tar_integrity: upstream_version.dist.integrity.clone(),
        });
    }
    let mut maintainer_user_ids = Vec::new();
    if let Some(upstream_maintainers) = &packument.maintainers {
        maintainer_user_ids.reserve(upstream_maintainers.len());
        for maintainer in upstream_maintainers {
            maintainer_user_ids.push(
                repo.upsert_user(
                    &maintainer.name,
                    maintainer.email.as_deref(),
                    &config.worker.upstream_name,
                )
                .await?,
            );
        }
    }

    let abbreviated_manifest = build_abbreviated_manifest(&packument);
    let abbrev_bytes = serde_json_canonicalizer::to_vec(&abbreviated_manifest)
        .context("failed to serialize abbreviated manifest")?;
    packument.readme = Some(String::new());
    let full_bytes = serde_json_canonicalizer::to_vec(&packument)
        .context("failed to serialize full manifest")?;
    let abbrev_manifest = repo.prepare_json_dist(abbrev_bytes).await?;
    let full_manifest = repo.prepare_json_dist(full_bytes).await?;
    let result = repo
        .commit_sync_package(SyncPackageCommitParams {
            name: fullname.to_string(),
            scope: scope.map(str::to_string),
            description: packument.description.clone(),
            source: config.worker.upstream_name.clone(),
            maintainer_user_ids,
            versions,
            tags: packument.dist_tags.clone(),
            abbrev_manifest,
            full_manifest,
        })
        .await
        .map_err(|error| {
            if error.to_string().contains("owned by a different source") {
                SyncPackageError::Conflict(format!(
                    "package {fullname} is locally owned, sync is not allowed"
                ))
            } else {
                SyncPackageError::Other(error)
            }
        })?;

    if let Some(idx) = search {
        crate::search::upsert_search_document(
            &**repo,
            idx,
            result.package_id,
            "public",
            &packument,
        )
        .await;
    }

    log::info!(
        action = "sync_done";
        "synced {fullname}: {} versions, {} removed",
        packument.versions.len(),
        result.deleted_version_ids.len()
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

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use std::collections::HashMap;

    #[derive(Serialize)]
    struct Manifest {
        dependencies: HashMap<String, String>,
        versions: HashMap<String, HashMap<String, String>>,
    }

    #[test]
    fn canonical_json_is_independent_of_hashmap_insertion_order() {
        let mut dependencies_a = HashMap::new();
        dependencies_a.insert("zod".to_string(), "4.0.0".to_string());
        dependencies_a.insert("axios".to_string(), "1.0.0".to_string());
        let mut versions_a = HashMap::new();
        versions_a.insert("2.0.0".to_string(), dependencies_a.clone());
        versions_a.insert("1.0.0".to_string(), dependencies_a.clone());

        let mut dependencies_b = HashMap::new();
        dependencies_b.insert("axios".to_string(), "1.0.0".to_string());
        dependencies_b.insert("zod".to_string(), "4.0.0".to_string());
        let mut versions_b = HashMap::new();
        versions_b.insert("1.0.0".to_string(), dependencies_b.clone());
        versions_b.insert("2.0.0".to_string(), dependencies_b.clone());

        let a = Manifest {
            dependencies: dependencies_a,
            versions: versions_a,
        };
        let b = Manifest {
            dependencies: dependencies_b,
            versions: versions_b,
        };

        assert_eq!(
            serde_json_canonicalizer::to_vec(&a).unwrap(),
            serde_json_canonicalizer::to_vec(&b).unwrap()
        );
    }
}
