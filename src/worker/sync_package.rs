use crate::config::Config;
use crate::npm::{split_scope_name, types::*};
use crate::repository::{
    PendingDist, PackageVersionRow, Repository, SyncManifestParams, VersionCommitParams,
};
use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;
fn build_abbreviated_manifest(packument: &Packument) -> AbbreviatedPackument {
    let mut versions = HashMap::new();
    for (ver, data) in &packument.versions {
        versions.insert(ver.clone(), build_abbreviated_version_entry(data, packument.time.get(ver)));
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

fn build_abbreviated_version_entry(ver: &PackageVersion, publish_time_str: Option<&String>) -> AbbreviatedVersion {
    let has_install_script = if ver.has_install_script.unwrap_or(false) {
        Some(true)
    } else {
        detect_install_script(ver)
    };

    let publish_time = publish_time_str.and_then(|t| {
        NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f")
            .ok()
            .map(|dt| dt.and_utc().timestamp_millis() / 1000)
    });

    let libc = ver.other.get("libc").and_then(|v| serde_json::from_value(v.clone()).ok());
    let workspaces = ver.other.get("workspaces").cloned();
    let accept_dependencies = ver.other.get("acceptDependencies").and_then(|v| serde_json::from_value(v.clone()).ok());

    AbbreviatedVersion {
        name: ver.name.clone(),
        version: ver.version.clone(),
        deprecated: ver.deprecated.clone(),
        dependencies: ver.dependencies.clone(),
        optional_dependencies: ver.optional_dependencies.clone(),
        dev_dependencies: ver.dev_dependencies.clone(),
        bundle_dependencies: ver.bundle_dependencies.clone(),
        peer_dependencies: ver.peer_dependencies.clone(),
        peer_dependencies_meta: ver.peer_dependencies_meta.clone(),
        bin: ver.bin.clone(),
        directories: ver.directories.clone(),
        dist: ver.dist.clone(),
        engines: ver.engines.clone(),
        _has_shrinkwrap: ver._has_shrinkwrap,
        has_install_script,
        funding: ver.funding.clone(),
        cpu: ver.cpu.clone(),
        os: ver.os.clone(),
        libc,
        workspaces,
        accept_dependencies,
        publish_time,
    }
}

fn detect_install_script(ver: &PackageVersion) -> Option<bool> {
    if let Some(scripts) = &ver.scripts
        && (scripts.contains_key("install")
            || scripts.contains_key("preinstall")
            || scripts.contains_key("postinstall"))
    {
        return Some(true);
    }
    None
}

fn is_prerelease(version: &str) -> bool {
    let v = version.trim_start_matches('v');
    match semver::Version::parse(v) {
        Ok(sv) => !sv.pre.is_empty(),
        Err(_) => true,
    }
}

fn pad_version(version: &str) -> String {
    let v = version.trim_start_matches('v');
    match semver::Version::parse(v) {
        Ok(sv) => format!(
            "{:016}{:016}{:016}",
            sv.major, sv.minor, sv.patch
        ),
        Err(_) => "0".repeat(48),
    }
}

pub async fn sync_package(
    repo: &Arc<dyn Repository>,
    config: &Config,
    fullname: &str,
    client: &reqwest::Client,
) -> Result<String> {
    let mut request = client
        .get(format!("{}/{fullname}", config.worker.upstream_registry))
        .header("accept", "application/json");

    if !config.worker.upstream_auth_token.is_empty() {
        request = request.bearer_auth(&config.worker.upstream_auth_token);
    }

    let resp = request.send().await.context("failed to fetch upstream")?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!("package {fullname} not found upstream");
    }
    if !resp.status().is_success() {
        anyhow::bail!("upstream returned status {}", resp.status());
    }

    let raw_bytes = resp
        .bytes()
        .await
        .context("failed to read upstream response body")?;

    let packument: Packument = serde_json::from_slice(&raw_bytes)
        .context("failed to parse upstream packument")?;

    let (scope, _name) = split_scope_name(fullname);

    let package_id = repo
        .upsert_package(fullname, scope, packument.description.as_deref())
        .await?;

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

        let is_pre_release = is_prerelease(ver_str) as i8;
        let padding_version = Some(pad_version(ver_str));

        let abbrev_s3 = format!("packages/{fullname}/{ver_str}/abbreviated.json");
        let manifest_s3 = format!("packages/{fullname}/{ver_str}/package.json");

        let abbrev_data = build_abbreviated_version(&ver_data.name, ver_data);
        let manifest_data = serde_json::to_vec(&ver_data).unwrap_or_default();

        if let Err(e) = repo.put_s3(&abbrev_s3, abbrev_data.clone()).await {
            error!("S3 upload failed for {fullname}@{ver_str} abbreviated: {e:#}");
            continue;
        }

        if let Err(e) = repo.put_s3(&manifest_s3, manifest_data.clone()).await {
            error!("S3 upload failed for {fullname}@{ver_str} manifest: {e:#}");
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
                path: abbrev_s3.clone(),
                size: abbrev_data.len() as i64,
                shasum: None,
                integrity: None,
            },
            manifest_dist: PendingDist {
                name: format!("{fullname}@{ver_str}-manifest"),
                path: manifest_s3.clone(),
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
    let abbrev_s3_key = format!("packages/{fullname}/abbreviated_manifests.json");
    let full_s3_key = format!("packages/{fullname}/full_manifests.json");

    let abbrev_bytes = serde_json::to_vec(&abbreviated_manifest).unwrap_or_default();
    let full_bytes: Vec<u8> = raw_bytes.to_vec();

    repo.put_s3(&abbrev_s3_key, abbrev_bytes.clone())
        .await
        .context("failed to upload abbreviated manifest to S3")?;
    repo.put_s3(&full_s3_key, full_bytes.clone())
        .await
        .context("failed to upload full manifest to S3")?;

    let params = SyncManifestParams {
        package_id,
        tags: packument.dist_tags.clone(),
        abbrev_manifest: PendingDist {
            name: format!("{fullname}-abbrev-manifests"),
            path: abbrev_s3_key.clone(),
            size: abbrev_bytes.len() as i64,
            shasum: None,
            integrity: None,
        },
        full_manifest: PendingDist {
            name: format!("{fullname}-full-manifests"),
            path: full_s3_key.clone(),
            size: full_bytes.len() as i64,
            shasum: None,
            integrity: None,
        },
    };

    if let Err(db_err) = repo.sync_manifest_commit(params).await {
        return Err(db_err.context("DB transaction failed for manifest commit"));
    }

    // ── Phase 3: Clean up old data (best-effort) ──

    log::info!(action = "sync_progress"; "name={fullname} phase=cleanup cleaning up old versions");

    let upstream_version_keys: Vec<String> = packument.versions.keys().cloned().collect();
    let versions_to_delete = repo
        .get_versions_not_in(package_id, &upstream_version_keys)
        .await?;

    if !versions_to_delete.is_empty() {
        let mut orphan_dist_ids: Vec<i64> = Vec::new();
        for v in &versions_to_delete {
            if let Some(id) = v.abbrev_dist_id {
                orphan_dist_ids.push(id);
            }
            if let Some(id) = v.manifest_dist_id {
                orphan_dist_ids.push(id);
            }
            if let Some(id) = v.tar_dist_id {
                orphan_dist_ids.push(id);
            }
            if let Some(id) = v.readme_dist_id {
                orphan_dist_ids.push(id);
            }
        }

        let version_ids: Vec<i64> = versions_to_delete.iter().map(|v| v.id).collect();
        repo.delete_versions_by_ids(&version_ids).await?;

        for &dist_id in &orphan_dist_ids {
            if let Err(e) = repo.delete_content(dist_id).await {
                error!("failed to delete orphan dist {dist_id}: {e:#}");
            }
        }
    }

    let mut old_manifest_dist_ids: Vec<i64> = Vec::new();
    if let Some(id) = old_abbrev_dist_id {
        old_manifest_dist_ids.push(id);
    }
    if let Some(id) = old_full_dist_id {
        old_manifest_dist_ids.push(id);
    }
    for &dist_id in &old_manifest_dist_ids {
        if let Err(e) = repo.delete_content(dist_id).await {
            error!("failed to delete old manifest dist {dist_id}: {e:#}");
        }
    }

    Ok(format!(
        "synced {fullname}: {new_count} new versions"
    ))
}
