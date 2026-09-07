use crate::error::{WebError, WebResult};
use crate::handlers::orgs::require_org_member;
use crate::handlers::{ensure_local_package, load_tag_map, lock_package};
use crate::middleware::auth::{AuthContext, is_admin};
use crate::npm::types::*;
use crate::npm::{
    build_abbreviated_manifest, is_prerelease, pad_version, split_scope_name,
    verify_integrity_digests,
};
use crate::repository::{
    LocalManifestCommitParams, MAINTAINER_SOURCE_MANUAL, PackageRow, PreparedDist,
    PublishCommitParams,
};
use crate::state::{AppState, LockOwner};
use axum::Json;
use axum::http::HeaderMap;
use base64::Engine;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::collections::HashMap;
use std::sync::LazyLock;

static BASE64_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new("^[A-Za-z0-9+/]{4}").unwrap());

pub fn get_npm_command(headers: &HeaderMap) -> Option<String> {
    headers
        .get("npm-command")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get("referer")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split_whitespace().next())
        })
        .map(|s| s.to_string())
}

fn validate_npm_command(headers: &HeaderMap) -> WebResult<()> {
    let command = get_npm_command(headers);

    if matches!(command.as_deref(), Some("star" | "unstar")) {
        return Err(WebError::Forbidden(format!(
            "npm {} is not allowed",
            command.unwrap()
        )));
    }
    Ok(())
}

struct TarballDigests {
    sha1: [u8; 20],
    sha512: [u8; 64],
}

impl TarballDigests {
    fn compute(data: &[u8]) -> Self {
        Self {
            sha1: Sha1::digest(data).into(),
            sha512: Sha512::digest(data).into(),
        }
    }

    fn shasum(&self) -> String {
        hex::encode(self.sha1)
    }

    fn integrity(&self) -> String {
        format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(self.sha512)
        )
    }

    fn verify_integrity(&self, integrity: &str) -> bool {
        verify_integrity_digests(&self.sha1, &self.sha512, integrity)
    }
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

const DEVELOPERS_TEAM: &str = "developers";

async fn load_developers_team_default(
    state: &AppState,
    scope: &str,
    publisher_id: i64,
) -> WebResult<Option<(i64, Vec<i64>)>> {
    let org = state
        .repo
        .get_org_by_name(scope)
        .await
        .map_err(WebError::CustomApiError)?;
    let Some(org) = org else {
        return Ok(None);
    };
    let dev_team = state
        .repo
        .get_team_by_org_name(org.id, DEVELOPERS_TEAM)
        .await
        .map_err(WebError::CustomApiError)?;
    let Some(dev_team) = dev_team else {
        return Ok(None);
    };
    let members = state
        .repo
        .list_team_members(dev_team.id)
        .await
        .map_err(WebError::CustomApiError)?;
    let mut user_ids: Vec<i64> = members.iter().map(|m| m.user_id).collect();
    if !user_ids.contains(&publisher_id) {
        user_ids.push(publisher_id);
    }
    Ok(Some((dev_team.id, user_ids)))
}

pub(crate) struct PreparedManifests {
    pub full_manifest: Packument,
    pub abbrev_dist: PreparedDist,
    pub full_dist: PreparedDist,
}

pub(crate) struct ManifestCandidateParams<'a> {
    pub package: Option<&'a PackageRow>,
    pub fullname: &'a str,
    pub description: Option<&'a str>,
    pub dist_tags: &'a HashMap<String, String>,
    pub added_version: Option<(PackageVersion, chrono::NaiveDateTime)>,
    pub removed_version: Option<&'a str>,
    pub maintainers: Option<Vec<Maintainer>>,
}

pub(crate) struct ManifestCommitChanges {
    pub tags: HashMap<String, String>,
    pub maintainers: Option<(Vec<i64>, String)>,
    pub delete_version_id: Option<i64>,
}

pub(crate) async fn prepare_manifest_candidate(
    state: &AppState,
    params: ManifestCandidateParams<'_>,
) -> WebResult<PreparedManifests> {
    let ManifestCandidateParams {
        package,
        fullname,
        description,
        dist_tags,
        added_version,
        removed_version,
        maintainers,
    } = params;
    let all_versions = if let Some(package) = package {
        state
            .repo
            .list_versions(package.id)
            .await
            .map_err(WebError::CustomApiError)?
    } else {
        Vec::new()
    };
    let mut full_versions = if let Some(full_dist_id) = package.and_then(|p| p.full_dist_id) {
        let (data, _) = state
            .repo
            .get_content(full_dist_id)
            .await
            .map_err(WebError::ServiceUnavailable)?;
        serde_json::from_slice::<Packument>(&data)
            .map_err(|error| WebError::ServiceUnavailable(error.into()))?
            .versions
    } else {
        HashMap::new()
    };
    if let Some(version) = removed_version {
        full_versions.remove(version);
    }
    if let Some((version, _)) = &added_version {
        full_versions.insert(version.version.clone(), version.clone());
    }
    let mut time = HashMap::new();
    for version in &all_versions {
        if removed_version == Some(version.version.as_str()) {
            continue;
        }
        time.insert(
            version.version.clone(),
            version
                .publish_time
                .format("%Y-%m-%dT%H:%M:%S%.f")
                .to_string(),
        );
    }
    if let Some((version, publish_time)) = &added_version {
        time.insert(
            version.version.clone(),
            publish_time.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
        );
    }
    let now = chrono::Utc::now().naive_utc();
    time.insert(
        "modified".to_string(),
        now.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
    );
    let created = all_versions
        .iter()
        .filter(|version| removed_version != Some(version.version.as_str()))
        .map(|version| version.publish_time)
        .chain(added_version.iter().map(|(_, time)| *time))
        .min();
    if let Some(created) = created {
        time.insert(
            "created".to_string(),
            created.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
        );
    }
    let latest = dist_tags
        .get("latest")
        .and_then(|version| full_versions.get(version));
    let (author, keywords, homepage, license, repository, bugs, contributors, readme_filename) =
        if let Some(latest) = latest {
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
    let maintainers = if let Some(maintainers) = maintainers {
        maintainers
    } else if let Some(package) = package {
        state
            .repo
            .list_maintainers(package.id)
            .await
            .map_err(WebError::CustomApiError)?
    } else {
        Vec::new()
    };
    let full_manifest = Packument {
        id: Some(fullname.to_string()),
        rev: None,
        name: fullname.to_string(),
        description: description.map(str::to_string),
        dist_tags: dist_tags.clone(),
        versions: full_versions,
        time,
        maintainers: (!maintainers.is_empty()).then_some(maintainers),
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
        extra: HashMap::new(),
    };
    let abbreviated = build_abbreviated_manifest(&full_manifest);
    let abbrev_dist = state
        .repo
        .prepare_json_dist(
            serde_json::to_vec(&abbreviated)
                .map_err(|error| WebError::CustomApiError(error.into()))?,
        )
        .await
        .map_err(WebError::CustomApiError)?;
    let full_dist = state
        .repo
        .prepare_json_dist(
            serde_json::to_vec(&full_manifest)
                .map_err(|error| WebError::CustomApiError(error.into()))?,
        )
        .await
        .map_err(WebError::CustomApiError)?;
    Ok(PreparedManifests {
        full_manifest,
        abbrev_dist,
        full_dist,
    })
}

pub(crate) async fn commit_manifest_candidate(
    state: &AppState,
    package: &PackageRow,
    changes: ManifestCommitChanges,
    manifests: PreparedManifests,
) -> WebResult<()> {
    let ManifestCommitChanges {
        tags,
        maintainers,
        delete_version_id,
    } = changes;
    let PreparedManifests {
        full_manifest,
        abbrev_dist,
        full_dist,
    } = manifests;
    state
        .repo
        .commit_local_manifest(LocalManifestCommitParams {
            package_id: package.id,
            expected_full_dist_id: package.full_dist_id,
            tags,
            maintainers,
            delete_version_id,
            abbrev_manifest: abbrev_dist,
            full_manifest: full_dist,
        })
        .await
        .map_err(WebError::CustomApiError)?;

    if let Some(index) = &state.search {
        crate::search::upsert_search_document(
            &*state.repo,
            index,
            package.id,
            &package.access,
            &full_manifest,
        )
        .await;
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

    let is_org_scope = if let Some(scope_name) = scope {
        if state
            .repo
            .get_org_by_name(scope_name)
            .await
            .map_err(WebError::CustomApiError)?
            .is_some()
        {
            require_org_member(state, auth, scope_name).await?;
            true
        } else {
            false
        }
    } else {
        false
    };

    if !is_org_scope && !is_admin(&auth.user, &state.config.auth.admins) {
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

    for tag in &tag_names {
        crate::handlers::dist_tags::validate_dist_tag(tag)?;
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

    let digests = TarballDigests::compute(&tarball_bytes);
    let shasum = digests.shasum();
    if let Some(ref integrity) = package_version.dist.integrity
        && !digests.verify_integrity(integrity)
    {
        return Err(WebError::BadRequest("dist.integrity invalid".to_string()));
    }
    if let Some(ref expected_shasum) = package_version.dist.shasum
        && shasum != *expected_shasum
    {
        return Err(WebError::BadRequest("dist.shasum invalid".to_string()));
    }

    let integrity = digests.integrity();

    let max_tarball_size = state.config.cdn.max_tarball_size;
    if tarball_bytes.len() as u64 > max_tarball_size {
        return Err(WebError::BadRequest(format!(
            "tarball for {fullname}@{} exceeds cdn.maxTarballSize ({max_tarball_size})",
            package_version.version
        )));
    }

    let _unlock = lock_package(state, &fullname, LockOwner::Publish)?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?;

    if let Some(ref pkg) = pkg {
        ensure_local_package(pkg.source.as_deref(), &fullname)?;
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

        crate::middleware::auth::ensure_package_write_access(state, auth, pkg).await?;
    }

    let description = payload
        .description
        .as_deref()
        .or(package_version.description.as_deref())
        .map(|s| if s.len() > 10240 { &s[..10240] } else { s });

    let pkg_exists = pkg.is_some();
    let requested_access = payload.access.as_deref().or_else(|| {
        package_version
            .publish_config
            .as_ref()
            .and_then(|c| c.access.as_deref())
    });
    if let Some(access) = requested_access
        && !matches!(access, "public" | "restricted" | "private")
    {
        return Err(WebError::BadRequest(format!("invalid access: {access}")));
    }
    if scope.is_none() && matches!(requested_access, Some("restricted") | Some("private")) {
        return Err(WebError::BadRequest(
            "unscoped packages are always public; restricted access requires a scope".to_string(),
        ));
    }
    let (desired_access, _package_access): (Option<&str>, &str) = if !pkg_exists && scope.is_some()
    {
        let access = if requested_access == Some("public") {
            "public"
        } else {
            "restricted"
        };
        (
            if access == "restricted" {
                Some(access)
            } else {
                None
            },
            access,
        )
    } else if pkg_exists && scope.is_some() {
        let current = pkg.as_ref().map(|p| p.access.as_str()).unwrap_or("public");
        let access = match requested_access {
            Some("public") => "public",
            Some("restricted") | Some("private") => "restricted",
            _ => current,
        };
        if access != current {
            (Some(access), access)
        } else {
            (
                if access == "restricted" {
                    Some(access)
                } else {
                    None
                },
                access,
            )
        }
    } else {
        (None, "public")
    };

    if !dist_tags.contains_key("latest") {
        let needs_latest = if let Some(package) = &pkg {
            let existing_tags = state
                .repo
                .list_tags(package.id)
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

    let tar_dist = state
        .repo
        .prepare_raw_dist(tarball_bytes.clone())
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

    let stored_version: PackageVersion =
        serde_json::from_value(version_json).map_err(|e| WebError::CustomApiError(e.into()))?;

    let readme_content = payload.readme.as_deref().unwrap_or("");
    let readme_data = readme_content.as_bytes().to_vec();
    let readme_dist = state
        .repo
        .prepare_json_dist(readme_data.clone())
        .await
        .map_err(WebError::CustomApiError)?;

    let developers_team = if !pkg_exists {
        if let Some(scope) = scope {
            load_developers_team_default(state, scope, auth.user.id).await?
        } else {
            None
        }
    } else {
        None
    };
    let mut maintainers = if let Some(package) = &pkg {
        state
            .repo
            .list_maintainers(package.id)
            .await
            .map_err(WebError::CustomApiError)?
    } else {
        Vec::new()
    };
    if !maintainers.iter().any(|user| user.name == auth.user.name) {
        maintainers.push(Maintainer {
            name: auth.user.name.clone(),
            email: auth.user.email.clone(),
        });
    }
    if let Some((_, user_ids)) = &developers_team {
        for user_id in user_ids {
            if let Some(user) = state
                .repo
                .get_user_by_id(*user_id)
                .await
                .map_err(WebError::CustomApiError)?
                && !maintainers.iter().any(|current| current.name == user.name)
            {
                maintainers.push(Maintainer {
                    name: user.name,
                    email: user.email,
                });
            }
        }
    }
    let manifests = prepare_manifest_candidate(
        state,
        ManifestCandidateParams {
            package: pkg.as_ref(),
            fullname: &fullname,
            description,
            dist_tags: &dist_tags,
            added_version: Some((stored_version, publish_time)),
            removed_version: None,
            maintainers: Some(maintainers),
        },
    )
    .await?;
    let expected_full_dist_id = pkg.as_ref().and_then(|package| package.full_dist_id);
    let (package_id, committed_access) = state
        .repo
        .commit_publish(PublishCommitParams {
            name: fullname.clone(),
            scope: scope.map(str::to_string),
            description: description.map(str::to_string),
            publisher_id: auth.user.id,
            access: desired_access.map(str::to_string),
            expected_full_dist_id,
            version: version_str.clone(),
            publish_time,
            is_pre_release,
            padding_version,
            tar_dist,
            readme_dist,
            tar_size: tarball_bytes.len() as i64,
            tar_shasum: shasum,
            tar_integrity: integrity,
            tags: dist_tags,
            abbrev_manifest: manifests.abbrev_dist,
            full_manifest: manifests.full_dist,
            developers_team,
        })
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

    if let Some(idx) = &state.search {
        crate::search::upsert_search_document(
            &*state.repo,
            idx,
            package_id,
            &committed_access,
            &manifests.full_manifest,
        )
        .await;
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

fn is_duplicate_key_error(err: &anyhow::Error) -> bool {
    err.chain()
        .filter_map(|e| e.downcast_ref::<sqlx::mysql::MySqlDatabaseError>())
        .any(|db_err| db_err.code() == Some("23000") && db_err.number() == 1062)
}

// ═══════════════════════════════════════════════════════════════════════════
// PUT /{fullname}/-rev/{rev} — npm owner add/rm (update maintainers)
// ═══════════════════════════════════════════════════════════════════════════

pub async fn update_maintainers_inner(
    state: &AppState,
    headers: &HeaderMap,
    auth: &AuthContext,
    fullname: &str,
    payload: MaintainerUpdatePayload,
) -> WebResult<Json<PublishResponse>> {
    let command = get_npm_command(headers);
    if matches!(command.as_deref(), Some("unpublish")) {
        return Ok(Json(PublishResponse {
            ok: false,
            rev: String::new(),
        }));
    }
    if command.as_deref() != Some("owner") {
        return Err(WebError::BadRequest(format!(
            "npm-command expected \"owner\", but got \"{}\"",
            command.as_deref().unwrap_or("")
        )));
    }

    let fullname = fullname.trim().to_string();

    if payload.maintainers.is_empty() {
        return Err(WebError::BadRequest(
            "maintainers must not be empty".to_string(),
        ));
    }

    let _unlock = lock_package(state, &fullname, LockOwner::Publish)?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;
    crate::middleware::auth::ensure_package_readable_with_auth(state, auth, &pkg).await?;
    crate::middleware::auth::ensure_package_write_access(state, auth, &pkg).await?;
    ensure_local_package(pkg.source.as_deref(), &fullname)?;

    let mut user_ids = Vec::with_capacity(payload.maintainers.len());
    for m in &payload.maintainers {
        let user = state
            .repo
            .get_user_by_name(&m.name)
            .await
            .map_err(WebError::CustomApiError)?
            .ok_or_else(|| WebError::BadRequest(format!("Maintainer \"{}\" not exists", m.name)))?;
        user_ids.push(user.id);
    }

    let tag_map = load_tag_map(state, pkg.id).await?;

    let manifests = prepare_manifest_candidate(
        state,
        ManifestCandidateParams {
            package: Some(&pkg),
            fullname: &fullname,
            description: pkg.description.as_deref(),
            dist_tags: &tag_map,
            added_version: None,
            removed_version: None,
            maintainers: Some(payload.maintainers.clone()),
        },
    )
    .await?;
    commit_manifest_candidate(
        state,
        &pkg,
        ManifestCommitChanges {
            tags: tag_map,
            maintainers: Some((user_ids, MAINTAINER_SOURCE_MANUAL.to_string())),
            delete_version_id: None,
        },
        manifests,
    )
    .await?;

    log::info!(
        action = "owner_update";
        "name={fullname} user={} maintainers={}",
        auth.user.name,
        payload.maintainers.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(",")
    );

    Ok(Json(PublishResponse {
        ok: true,
        rev: pkg.id.to_string(),
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// DELETE /{fullname}/-rev/{rev} — npm unpublish (whole package or latest version)
// ═══════════════════════════════════════════════════════════════════════════

pub async fn unpublish_package_inner(
    state: &AppState,
    headers: &HeaderMap,
    auth: &AuthContext,
    fullname: &str,
) -> WebResult<Json<PublishResponse>> {
    if get_npm_command(headers).as_deref() != Some("unpublish") {
        return Err(WebError::BadRequest(
            "Only allow \"unpublish\" npm-command".to_string(),
        ));
    }

    let fullname = fullname.trim().to_string();

    let _unlock = lock_package(state, &fullname, LockOwner::Publish)?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;
    crate::middleware::auth::ensure_package_readable_with_auth(state, auth, &pkg).await?;
    crate::middleware::auth::ensure_package_write_access(state, auth, &pkg).await?;
    ensure_local_package(pkg.source.as_deref(), &fullname)?;

    let version_ids = state
        .repo
        .list_versions(pkg.id)
        .await
        .map_err(WebError::CustomApiError)?
        .into_iter()
        .map(|version| version.id)
        .collect::<Vec<_>>();

    delete_package_completely(state, &pkg, &version_ids)
        .await
        .map_err(WebError::CustomApiError)?;
    remove_package_from_search(state, pkg.id).await;

    log::info!(
        action = "unpublish";
        "name={fullname} user={}",
        auth.user.name
    );

    Ok(Json(PublishResponse {
        ok: true,
        rev: pkg.id.to_string(),
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// DELETE /{fullname}/-/{filename}/-rev/{rev} — npm unpublish single version
// ═══════════════════════════════════════════════════════════════════════════

pub async fn unpublish_version_inner(
    state: &AppState,
    headers: &HeaderMap,
    auth: &AuthContext,
    fullname: &str,
    filename: &str,
) -> WebResult<Json<PublishResponse>> {
    if get_npm_command(headers).as_deref() != Some("unpublish") {
        return Err(WebError::BadRequest(
            "Only allow \"unpublish\" npm-command".to_string(),
        ));
    }

    let fullname = fullname.trim().to_string();

    let _unlock = lock_package(state, &fullname, LockOwner::Publish)?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;
    crate::middleware::auth::ensure_package_readable_with_auth(state, auth, &pkg).await?;
    crate::middleware::auth::ensure_package_write_access(state, auth, &pkg).await?;
    ensure_local_package(pkg.source.as_deref(), &fullname)?;

    let version_name = crate::handlers::tarball::extract_version(&fullname, filename)
        .ok_or_else(|| WebError::NotFound(format!("{fullname} tarball {filename} not found")))?;

    let version = state
        .repo
        .get_version(pkg.id, &version_name)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{version_name} not found")))?;

    let version_str = version.version.clone();
    remove_version_and_refresh(state, &fullname, &pkg, version).await?;

    log::info!(
        action = "unpublish_version";
        "name={fullname} version={} tarball={} user={}",
        version_str,
        filename,
        auth.user.name
    );

    Ok(Json(PublishResponse {
        ok: true,
        rev: pkg.id.to_string(),
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

async fn remove_version_and_refresh(
    state: &AppState,
    fullname: &str,
    pkg: &crate::repository::PackageRow,
    version: crate::repository::PackageVersionRow,
) -> WebResult<()> {
    let mut remaining = state
        .repo
        .list_versions(pkg.id)
        .await
        .map_err(WebError::CustomApiError)?;
    let version_ids = remaining
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    remaining.retain(|candidate| candidate.id != version.id);

    if remaining.is_empty() {
        delete_package_completely(state, pkg, &version_ids)
            .await
            .map_err(WebError::CustomApiError)?;
        remove_package_from_search(state, pkg.id).await;
        return Ok(());
    }

    let remaining_set: std::collections::HashSet<&str> =
        remaining.iter().map(|v| v.version.as_str()).collect();

    let mut tag_map = load_tag_map(state, pkg.id).await?;

    let latest_dangling = tag_map
        .get("latest")
        .is_some_and(|v| !remaining_set.contains(v.as_str()));

    let mut tags_changed = false;
    tag_map.retain(|_, v| {
        let keep = remaining_set.contains(v.as_str());
        if !keep {
            tags_changed = true;
        }
        keep
    });

    if latest_dangling && let Some(new_latest) = pick_latest_version(&remaining) {
        tag_map.insert("latest".to_string(), new_latest);
        tags_changed = true;
    }

    if tags_changed {
        log::info!(action = "unpublish_tags_repaired"; "name={fullname}");
    }

    let manifests = prepare_manifest_candidate(
        state,
        ManifestCandidateParams {
            package: Some(pkg),
            fullname,
            description: pkg.description.as_deref(),
            dist_tags: &tag_map,
            added_version: None,
            removed_version: Some(&version.version),
            maintainers: None,
        },
    )
    .await?;
    commit_manifest_candidate(
        state,
        pkg,
        ManifestCommitChanges {
            tags: tag_map,
            maintainers: None,
            delete_version_id: Some(version.id),
        },
        manifests,
    )
    .await?;

    state.download_counters.remove(&version.id);
    state.unpacked.remove_version(version.id).await;

    Ok(())
}

fn pick_latest_version(versions: &[crate::repository::PackageVersionRow]) -> Option<String> {
    versions
        .iter()
        .filter_map(|v| semver::Version::parse(&v.version).ok().map(|sv| (v, sv)))
        .max_by(|(_, asv), (_, bsv)| asv.cmp(bsv))
        .map(|(v, _)| v.version.clone())
}

async fn delete_package_completely(
    state: &AppState,
    pkg: &crate::repository::PackageRow,
    version_ids: &[i64],
) -> anyhow::Result<()> {
    state
        .repo
        .delete_local_package(pkg.id, pkg.full_dist_id)
        .await?;
    for version_id in version_ids {
        state.download_counters.remove(version_id);
        state.unpacked.remove_version(*version_id).await;
    }

    Ok(())
}

async fn remove_package_from_search(state: &AppState, package_id: i64) {
    if let Some(index) = &state.search
        && let Err(error) = index.remove_package(package_id).await
    {
        log::warn!(
            action = "search_index_remove";
            "package_id={package_id} remove failed: {error:#}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn integrity_sha1(data: &[u8]) -> String {
        format!(
            "sha1-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha1::digest(data))
        )
    }

    #[test]
    fn verify_integrity_accepts_multiple_digests() {
        let data = b"proxide";
        let integrity = format!(
            "{} {}",
            integrity_sha1(data),
            TarballDigests::compute(data).integrity()
        );

        assert!(TarballDigests::compute(data).verify_integrity(&integrity));
    }

    #[test]
    fn verify_integrity_does_not_fall_back_from_mismatched_sha512() {
        let data = b"proxide";
        let integrity = format!(
            "{} {}",
            TarballDigests::compute(b"different").integrity(),
            integrity_sha1(data)
        );

        assert!(!TarballDigests::compute(data).verify_integrity(&integrity));
    }

    #[test]
    fn verify_integrity_accepts_any_matching_digest_of_strongest_algorithm() {
        let data = b"proxide";
        let integrity = format!(
            "{} {}?source=test",
            TarballDigests::compute(b"different").integrity(),
            TarballDigests::compute(data).integrity()
        );

        assert!(TarballDigests::compute(data).verify_integrity(&integrity));
    }
}
