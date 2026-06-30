use crate::error::{WebError, WebResult};
use crate::middleware::auth::{AuthContext, is_admin};
use crate::npm::types::*;
use crate::npm::{
    build_abbreviated_version, is_prerelease, pad_version, split_scope_name,
};
use crate::repository::{upload_and_commit_manifests, CommitVersionParams, PendingDist};
use crate::state::{AppState, LockOwner, UnlockGuard};
use axum::Json;
use axum::http::HeaderMap;
use base64::Engine;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::collections::HashMap;
use std::sync::LazyLock;

static BASE64_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new("^[A-Za-z0-9+/]{4}").unwrap());

fn validate_npm_command(headers: &HeaderMap) -> WebResult<()> {
    let command = headers
        .get("npm-command")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get("referer")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split_whitespace().next())
        });

    if matches!(command, Some("star" | "unstar")) {
        return Err(WebError::Forbidden(format!(
            "npm {} is not allowed",
            command.unwrap()
        )));
    }
    Ok(())
}

fn compute_shasum(data: &[u8]) -> String {
    format!("{:x}", Sha1::digest(data))
}

fn compute_integrity_sha512(data: &[u8]) -> String {
    format!(
        "sha512-{}",
        base64::engine::general_purpose::STANDARD.encode(Sha512::digest(data))
    )
}

fn verify_integrity(data: &[u8], integrity: &str) -> bool {
    let Some((algo, hash_b64)) = integrity.split_once('-') else {
        return false;
    };
    let computed = match algo {
        "sha512" => Sha512::digest(data).to_vec(),
        "sha1" => Sha1::digest(data).to_vec(),
        _ => return false,
    };
    let expected = base64::engine::general_purpose::STANDARD.decode(hash_b64);
    expected.is_ok_and(|bytes| computed.as_slice() == bytes.as_slice())
}

fn validate_package_name(name: &str) -> WebResult<()> {
    if name.is_empty() {
        return Err(WebError::BadRequest("package name is empty".to_string()));
    }
    if name.len() > 214 {
        return Err(WebError::BadRequest(
            "package name cannot exceed 214 characters".to_string(),
        ));
    }
    if name.starts_with('.') || name.starts_with('_') {
        return Err(WebError::BadRequest(
            "package name cannot start with . or _".to_string(),
        ));
    }
    let lower = name.to_lowercase();
    if lower != name {
        return Err(WebError::BadRequest(
            "package name cannot contain uppercase characters".to_string(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn publish_package_inner(
    state: &AppState,
    headers: &HeaderMap,
    auth: &AuthContext,
    fullname: &str,
    payload: PublishPayload,
) -> WebResult<Json<PublishResponse>> {
    validate_npm_command(headers)?;

    let fullname = fullname.trim().to_string();
    if fullname != payload.name {
        return Err(WebError::BadRequest(format!(
            "fullname({fullname}) not match package.name({})",
            payload.name
        )));
    }

    validate_package_name(&fullname)?;

    let (scope, _name) = split_scope_name(&fullname);

    if !is_admin(&auth.user, &state.config.auth.admins) {
        crate::middleware::auth::check_scope_access(
            scope,
            &state.config.auth.allow_scopes,
            state.config.auth.allow_publish_non_scope_package,
        )?;
    }

    let versions: Vec<&PublishVersion> = payload.versions.values().collect();
    if versions.is_empty() {
        return Err(WebError::BadRequest("versions is empty".to_string()));
    }

    let attachment_keys: Vec<String> = payload.attachments.keys().cloned().collect();
    if attachment_keys.is_empty() {
        return Err(WebError::BadRequest("_attachments is empty".to_string()));
    }

    let package_version = versions[0];
    let attachment_filename = &attachment_keys[0];
    let attachment = &payload.attachments[attachment_filename];

    if semver::Version::parse(&package_version.version).is_err() {
        return Err(WebError::BadRequest(format!(
            "invalid version: {}",
            package_version.version
        )));
    }

    let mut dist_tags = payload.dist_tags.clone();
    let tag_names: Vec<String> = dist_tags.keys().cloned().collect();
    if tag_names.is_empty() {
        return Err(WebError::BadRequest("dist-tags is empty".to_string()));
    }

    let tag_version = dist_tags[&tag_names[0]].clone();
    if tag_version != package_version.version {
        return Err(WebError::BadRequest(format!(
            "dist-tags version \"{tag_version}\" not match package version \"{}\"",
            package_version.version
        )));
    }

    if attachment.data.is_empty() {
        return Err(WebError::BadRequest(
            "attachment.data format invalid".to_string(),
        ));
    }

    if !BASE64_RE.is_match(&attachment.data) {
        return Err(WebError::BadRequest(
            "attachment.data string format invalid".to_string(),
        ));
    }

    let tarball_bytes = base64::engine::general_purpose::STANDARD
        .decode(&attachment.data)
        .map_err(|_| WebError::BadRequest("attachment.data base64 decode failed".to_string()))?;

    if tarball_bytes.len() != attachment.length {
        return Err(WebError::BadRequest(format!(
            "attachment size {} not match download size {}",
            attachment.length,
            tarball_bytes.len()
        )));
    }

    if let Some(ref integrity) = package_version.dist.integrity {
        if !verify_integrity(&tarball_bytes, integrity) {
            return Err(WebError::BadRequest("dist.integrity invalid".to_string()));
        }
    } else if let Some(ref shasum) = package_version.dist.shasum {
        let computed = compute_shasum(&tarball_bytes);
        if computed != *shasum {
            return Err(WebError::BadRequest("dist.shasum invalid".to_string()));
        }
    }

    let shasum = compute_shasum(&tarball_bytes);
    let integrity = compute_integrity_sha512(&tarball_bytes);

    if !state.package_lock.try_lock(&fullname, LockOwner::Publish) {
        let owner = state.package_lock.get_owner(&fullname);
        return Err(WebError::Conflict(format!(
            "package {fullname} is currently being {}",
            owner.map(|o| o.to_string()).unwrap_or_default()
        )));
    }

    let _unlock = UnlockGuard::new(&state.package_lock, fullname.clone());

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?;

    if let Some(ref pkg) = pkg {
        let existing = state
            .repo
            .get_version(pkg.id, &package_version.version)
            .await
            .map_err(WebError::CustomApiError)?;
        if existing.is_some() {
            return Err(WebError::Conflict(format!(
                "Can't modify pre-existing version: {fullname}@{}",
                package_version.version
            )));
        }

        if !is_admin(&auth.user, &state.config.auth.admins) {
            let is_maintainer = state
                .repo
                .is_maintainer(pkg.id, auth.user.id)
                .await
                .map_err(WebError::CustomApiError)?;
            if !is_maintainer {
                return Err(WebError::Forbidden(format!(
                    "\"{}\" not authorized to modify {fullname}, please contact maintainers",
                    auth.user.name
                )));
            }
        }
    }

    let description = payload
        .description
        .as_deref()
        .or(package_version.description.as_deref())
        .map(|s| if s.len() > 10240 { &s[..10240] } else { s });

    let (package_id, existing_source) = state
        .repo
        .upsert_package(&fullname, scope, description, None)
        .await
        .map_err(WebError::CustomApiError)?;

    if let Some(source) = existing_source {
        return Err(WebError::Forbidden(format!(
            "package {fullname} was synced from upstream ({source}), local publish is not allowed"
        )));
    }

    state
        .repo
        .save_maintainer(package_id, auth.user.id)
        .await
        .map_err(WebError::CustomApiError)?;

    let pkg_exists = pkg.is_some();
    if !dist_tags.contains_key("latest") {
        let needs_latest = if pkg_exists {
            let existing_tags = state
                .repo
                .list_tags(package_id)
                .await
                .map_err(WebError::CustomApiError)?;
            !existing_tags.iter().any(|t| t.tag == "latest")
        } else {
            true
        };
        if needs_latest {
            dist_tags.insert("latest".to_string(), package_version.version.clone());
        }
    }

    let version_str = &package_version.version;
    let is_pre_release = is_prerelease(version_str);
    let padding_version = Some(pad_version(version_str));
    let publish_time = chrono::Utc::now().naive_utc();

    let tar_storage_key = format!("packages/{fullname}/{version_str}/{attachment_filename}");
    let max_tarball_size = state.config.cdn.max_tarball_size;
    if tarball_bytes.len() as u64 > max_tarball_size {
        return Err(WebError::BadRequest(format!(
            "tarball for {fullname}@{version_str} exceeds cdn.maxTarballSize ({max_tarball_size})"
        )));
    }
    state
        .repo
        .put_storage(&tar_storage_key, tarball_bytes.clone())
        .await
        .map_err(WebError::CustomApiError)?;

    let mut version_json: serde_json::Value =
        serde_json::to_value(package_version).unwrap_or_default();
    if let Some(obj) = version_json.as_object_mut() {
        obj.remove("_attachments");
        obj.remove("readme");
        let dist = obj
            .entry("dist")
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        if let Some(dist_obj) = dist.as_object_mut() {
            dist_obj.insert(
                "tarball".to_string(),
                serde_json::Value::String(format!(
                    "{}/npm/{fullname}/-/{attachment_filename}",
                    state.config.server.root_url
                )),
            );
            dist_obj.insert(
                "shasum".to_string(),
                serde_json::Value::String(shasum.clone()),
            );
            dist_obj.insert(
                "integrity".to_string(),
                serde_json::Value::String(integrity.clone()),
            );
        }
    }

    let manifest_data = serde_json::to_vec(&version_json).unwrap_or_default();
    let manifest_base_key = format!("packages/{fullname}/{version_str}/package.json");

    let abbrev_ver = PackageVersion {
        id: None,
        name: fullname.clone(),
        version: version_str.clone(),
        description: package_version.description.clone(),
        keywords: package_version.keywords.clone(),
        homepage: package_version.homepage.clone(),
        license: package_version.license.clone(),
        repository: package_version.repository.clone(),
        author: package_version.author.clone(),
        bugs: package_version.bugs.clone(),
        contributors: package_version.contributors.clone(),
        readme_filename: package_version.readme_filename.clone(),
        deprecated: package_version.deprecated.clone(),
        dependencies: package_version.dependencies.clone(),
        dev_dependencies: package_version.dev_dependencies.clone(),
        optional_dependencies: package_version.optional_dependencies.clone(),
        peer_dependencies: package_version.peer_dependencies.clone(),
        peer_dependencies_meta: package_version.peer_dependencies_meta.clone(),
        bundle_dependencies: package_version.bundle_dependencies.clone(),
        bin: package_version.bin.clone(),
        directories: package_version.directories.clone(),
        man: package_version.man.clone(),
        dist: Dist {
            shasum: Some(shasum.clone()),
            tarball: format!(
                "{}/npm/{fullname}/-/{attachment_filename}",
                state.config.server.root_url
            ),
            integrity: Some(integrity.clone()),
            file_count: None,
            unpacked_size: None,
            npm_signature: None,
        },
        engines: package_version.engines.clone(),
        has_install_script: None,
        _has_shrinkwrap: package_version._has_shrinkwrap,
        funding: package_version.funding.clone(),
        cpu: package_version.cpu.clone(),
        os: package_version.os.clone(),
        libc: package_version.libc.clone(),
        workspaces: package_version.workspaces.clone(),
        accept_dependencies: package_version.accept_dependencies.clone(),
        _npm_user: Some(Person {
            name: Some(auth.user.name.clone()),
            email: auth.user.email.clone(),
            url: None,
        }),
        _npm_version: None,
        _node_version: None,
        main: package_version.main.clone(),
        module: package_version.module.clone(),
        types: package_version.types.clone(),
        typings: package_version.typings.clone(),
        module_type: package_version.module_type.clone(),
        browser: package_version.browser.clone(),
        exports: package_version.exports.clone(),
        imports: package_version.imports.clone(),
        scripts: package_version.scripts.clone(),
        config: package_version.config.clone(),
        files: package_version.files.clone(),
        publish_config: package_version.publish_config.clone(),
        is_private: package_version.is_private,
        prefer_global: package_version.prefer_global,
        git_head: package_version.git_head.clone(),
        types_versions: package_version.types_versions.clone(),
        side_effects: package_version.side_effects.clone(),
        unpkg: package_version.unpkg.clone(),
        jsdelivr: package_version.jsdelivr.clone(),
        jsnext_main: package_version.jsnext_main.clone(),
        package_manager: package_version.package_manager.clone(),
        overrides: package_version.overrides.clone(),
        resolutions: package_version.resolutions.clone(),
    };

    let abbrev_data = build_abbreviated_version(&fullname, &abbrev_ver);
    let abbrev_base_key = format!("packages/{fullname}/{version_str}/abbreviated.json");

    let readme_content = payload.readme.as_deref().unwrap_or("");
    let readme_data = readme_content.as_bytes().to_vec();
    let readme_base_key = format!("packages/{fullname}/{version_str}/readme.md");

    let manifest_storage_key = state
        .repo
        .put_storage_compressed(&manifest_base_key, manifest_data.clone())
        .await
        .map_err(WebError::CustomApiError)?;
    let abbrev_storage_key = state
        .repo
        .put_storage_compressed(&abbrev_base_key, abbrev_data.clone())
        .await
        .map_err(WebError::CustomApiError)?;
    let readme_storage_key = state
        .repo
        .put_storage_compressed(&readme_base_key, readme_data.clone())
        .await
        .map_err(WebError::CustomApiError)?;

    let version_params = CommitVersionParams {
        package_id,
        version: version_str.clone(),
        publish_time,
        is_pre_release,
        padding_version,
        abbrev_dist: PendingDist {
            name: format!("{fullname}@{version_str}-abbrev"),
            path: abbrev_storage_key,
            size: abbrev_data.len() as i64,
            shasum: None,
            integrity: None,
        },
        manifest_dist: PendingDist {
            name: format!("{fullname}@{version_str}-manifest"),
            path: manifest_storage_key,
            size: manifest_data.len() as i64,
            shasum: None,
            integrity: None,
        },
        tar_dist: Some(PendingDist {
            name: format!("{fullname}@{version_str}-tar"),
            path: tar_storage_key,
            size: tarball_bytes.len() as i64,
            shasum: Some(shasum),
            integrity: Some(integrity),
        }),
        readme_dist: Some(PendingDist {
            name: format!("{fullname}@{version_str}-readme"),
            path: readme_storage_key,
            size: readme_data.len() as i64,
            shasum: None,
            integrity: None,
        }),
    };

    state
        .repo
        .commit_version(version_params)
        .await
        .map_err(|e| {
            if is_duplicate_key_error(&e) {
                WebError::Conflict(format!(
                    "Can't modify pre-existing version: {fullname}@{version_str}"
                ))
            } else {
                WebError::CustomApiError(e)
            }
        })?;

    let full_manifest =
        refresh_manifests(
            state,
            package_id,
            &fullname,
            description,
            &dist_tags,
        )
        .await?;

    if let Some(idx) = &state.search {
        crate::search::upsert_search_document(&*state.repo, idx, package_id, &full_manifest).await;
    }

    log::info!(
        action = "publish";
        "name={fullname} version={version_str} user={}",
        auth.user.name
    );

    Ok(Json(PublishResponse {
        ok: true,
        rev: format!("{package_id}-{version_str}"),
    }))
}

async fn refresh_manifests(
    state: &AppState,
    package_id: i64,
    fullname: &str,
    description: Option<&str>,
    dist_tags: &HashMap<String, String>,
) -> WebResult<Packument> {
    let all_versions = state
        .repo
        .list_versions(package_id)
        .await
        .map_err(WebError::CustomApiError)?;

    let mut full_versions: HashMap<String, PackageVersion> = HashMap::new();
    let mut abbrev_versions: HashMap<String, AbbreviatedVersion> = HashMap::new();
    let mut time_map: HashMap<String, String> = HashMap::new();

    for v in &all_versions {
        let v_str = &v.version;
        time_map.insert(
            v_str.clone(),
            v.publish_time.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
        );

        if let Some(abbrev_id) = v.abbrev_dist_id
            && let Ok((data, _)) = state.repo.get_content(abbrev_id).await
            && let Ok(val) = serde_json::from_slice::<serde_json::Value>(&data)
            && let Ok(abbrev) = serde_json::from_value::<AbbreviatedVersion>(val.clone())
        {
            abbrev_versions.insert(v_str.clone(), abbrev);
        }

        if let Some(manifest_id) = v.manifest_dist_id
            && let Ok((data, _)) = state.repo.get_content(manifest_id).await
            && let Ok(ver) = serde_json::from_slice::<PackageVersion>(&data)
        {
            full_versions.insert(v_str.clone(), ver);
        }
    }

    time_map.insert(
        "modified".to_string(),
        chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.f")
            .to_string(),
    );
    if let Some(created) = all_versions.last() {
        time_map.entry("created".to_string()).or_insert_with(|| {
            created
                .publish_time
                .format("%Y-%m-%dT%H:%M:%S%.f")
                .to_string()
        });
    }

    let latest_version = dist_tags
        .get("latest")
        .and_then(|v| full_versions.get(v));

    let (author, keywords, homepage, license, repository, bugs, contributors, readme_filename) =
        if let Some(latest) = latest_version {
            (
                latest.author.clone(),
                latest.keywords.clone(),
                latest.homepage.clone(),
                latest.license.clone(),
                latest.repository.clone(),
                latest.bugs.clone(),
                latest.contributors.clone(),
                latest.readme_filename.clone(),
            )
        } else {
            (None, None, None, None, None, None, None, None)
        };

    let maintainers_list = state
        .repo
        .list_maintainers(package_id)
        .await
        .map_err(WebError::CustomApiError)?;

    let abbrev_manifest = AbbreviatedPackument {
        name: fullname.to_string(),
        modified: time_map.get("modified").cloned(),
        dist_tags: dist_tags.clone(),
        versions: abbrev_versions,
        time: if time_map.is_empty() {
            None
        } else {
            Some(time_map.clone())
        },
    };
    let abbrev_manifest_bytes = serde_json::to_vec(&abbrev_manifest).unwrap_or_default();

    let full_manifest = Packument {
        id: Some(fullname.to_string()),
        rev: Some(package_id.to_string()),
        name: fullname.to_string(),
        description: description.map(String::from),
        dist_tags: dist_tags.clone(),
        versions: full_versions,
        time: time_map,
        maintainers: if maintainers_list.is_empty() {
            None
        } else {
            Some(maintainers_list)
        },
        readme: Some(String::new()),
        readme_filename,
        keywords,
        homepage,
        license,
        repository,
        author,
        bugs,
        contributors,
        users: None,
    };
    let full_manifest_bytes = serde_json::to_vec(&full_manifest).unwrap_or_default();

    upload_and_commit_manifests(
        &*state.repo,
        package_id,
        fullname,
        dist_tags,
        &abbrev_manifest_bytes,
        &full_manifest_bytes,
    )
    .await
    .map_err(WebError::CustomApiError)?;

    Ok(full_manifest)
}

fn is_duplicate_key_error(err: &anyhow::Error) -> bool {
    err.chain()
        .filter_map(|e| e.downcast_ref::<sqlx::mysql::MySqlDatabaseError>())
        .any(|db_err| db_err.code() == Some("23000") && db_err.number() == 1062)
}
