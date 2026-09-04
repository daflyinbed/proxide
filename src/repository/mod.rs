use sqlx::Row;

pub mod mysql;

use crate::npm::types::Maintainer;
use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;
use sqlx::{Connection, MySqlConnection};
use std::collections::{HashMap, HashSet};

pub const MAINTAINER_SOURCE_TEAM: &str = "team";
pub const MAINTAINER_SOURCE_MANUAL: &str = "manual";
pub const MAINTAINER_SOURCE_UPSTREAM: &str = "upstream";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PackageRow {
    pub id: i64,
    pub name: String,
    pub scope: Option<String>,
    pub description: Option<String>,
    pub source: Option<String>,
    pub access: String,
    pub abbreviated_dist_id: Option<i64>,
    pub full_dist_id: Option<i64>,
}

impl PackageRow {
    pub fn is_public(&self) -> bool {
        self.scope.is_none() || self.access == "public"
    }
}

#[derive(Debug, Clone)]
pub struct PackageVersionRow {
    pub id: i64,
    pub package_id: i64,
    pub version: String,
    pub tar_dist_id: Option<i64>,
    pub readme_dist_id: Option<i64>,
    pub tar_size: Option<i64>,
    pub tar_shasum: Option<String>,
    pub tar_integrity: Option<String>,
    pub publish_time: chrono::NaiveDateTime,
    pub is_pre_release: bool,
    pub padding_version: Option<String>,
}

impl<'r> sqlx::FromRow<'r, sqlx::mysql::MySqlRow> for PackageVersionRow {
    fn from_row(row: &'r sqlx::mysql::MySqlRow) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            package_id: row.try_get("package_id")?,
            version: row.try_get("version")?,
            tar_dist_id: row.try_get("tar_dist_id")?,
            readme_dist_id: row.try_get("readme_dist_id")?,
            tar_size: row.try_get("tar_size")?,
            tar_shasum: row.try_get("tar_shasum")?,
            tar_integrity: row.try_get("tar_integrity")?,
            publish_time: row.try_get("publish_time")?,
            is_pre_release: {
                let v: i8 = row.try_get("is_pre_release")?;
                v != 0
            },
            padding_version: row.try_get("padding_version")?,
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PackageTagRow {
    pub id: i64,
    pub package_id: i64,
    pub tag: String,
    pub version: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DistRow {
    pub id: i64,
    pub storage_sha256: Vec<u8>,
    pub path: String,
    pub stored_size: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
    pub upstream_name: String,
    pub password_salt: Option<String>,
    pub password_integrity: Option<String>,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TokenRow {
    pub id: i64,
    pub token_key: String,
    pub name: String,
    pub user_id: i64,
    pub is_readonly: bool,
    pub allowed_scopes: Option<String>,
    pub cidr_whitelist: Option<String>,
    pub expired_at: Option<chrono::NaiveDateTime>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
    pub last_used_at: Option<chrono::NaiveDateTime>,
}

#[derive(Debug, Clone)]
pub struct PreparedDist {
    dist_id: i64,
    path: String,
    storage_sha256: [u8; 32],
    stored_size: i64,
}

impl PreparedDist {
    pub fn id(&self) -> i64 {
        self.dist_id
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn storage_sha256(&self) -> &[u8; 32] {
        &self.storage_sha256
    }

    pub fn stored_size(&self) -> i64 {
        self.stored_size
    }

    pub(crate) fn new(
        dist_id: i64,
        path: String,
        storage_sha256: [u8; 32],
        stored_size: i64,
    ) -> Self {
        Self {
            dist_id,
            path,
            storage_sha256,
            stored_size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PublishCommitParams {
    pub name: String,
    pub scope: Option<String>,
    pub description: Option<String>,
    pub publisher_id: i64,
    pub access: Option<String>,
    pub expected_full_dist_id: Option<i64>,
    pub version: String,
    pub publish_time: chrono::NaiveDateTime,
    pub is_pre_release: bool,
    pub padding_version: Option<String>,
    pub tar_dist: PreparedDist,
    pub readme_dist: PreparedDist,
    pub tar_size: i64,
    pub tar_shasum: String,
    pub tar_integrity: String,
    pub tags: HashMap<String, String>,
    pub abbrev_manifest: PreparedDist,
    pub full_manifest: PreparedDist,
    pub developers_team: Option<(i64, Vec<i64>)>,
}

#[derive(Debug, Clone)]
pub struct LocalManifestCommitParams {
    pub package_id: i64,
    pub expected_full_dist_id: Option<i64>,
    pub tags: HashMap<String, String>,
    pub maintainers: Option<(Vec<i64>, String)>,
    pub delete_version_id: Option<i64>,
    pub abbrev_manifest: PreparedDist,
    pub full_manifest: PreparedDist,
}

#[derive(Debug, Clone)]
pub struct SyncVersionInput {
    pub version: String,
    pub publish_time: Option<chrono::NaiveDateTime>,
    pub is_pre_release: bool,
    pub padding_version: Option<String>,
    pub tar_shasum: Option<String>,
    pub tar_integrity: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SyncPackageCommitParams {
    pub name: String,
    pub scope: Option<String>,
    pub description: Option<String>,
    pub source: String,
    pub maintainer_user_ids: Vec<i64>,
    pub versions: Vec<SyncVersionInput>,
    pub tags: HashMap<String, String>,
    pub abbrev_manifest: PreparedDist,
    pub full_manifest: PreparedDist,
}

#[derive(Debug, Clone)]
pub struct SyncPackageCommitResult {
    pub package_id: i64,
    pub deleted_version_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachDistOutcome {
    Attached,
    AlreadyAttached,
    VersionDeleted,
}

#[derive(Debug, Clone)]
pub struct StorageObjectMeta {
    pub path: String,
    pub last_modified: chrono::DateTime<chrono::Utc>,
}

pub struct ProcessLock {
    name: String,
    connection: Option<MySqlConnection>,
}

impl ProcessLock {
    pub(crate) fn new(name: String, connection: MySqlConnection) -> Self {
        Self {
            name,
            connection: Some(connection),
        }
    }

    pub async fn check(&mut self) -> Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("process lock {} was released", self.name))?;
        let held = sqlx::query_scalar!(
            r#"SELECT IF(IS_USED_LOCK(?) = CONNECTION_ID(), 1, 0) AS `held!`"#,
            self.name
        )
        .fetch_one(&mut *connection)
        .await?;
        if held != 1 {
            anyhow::bail!("lost process lock {}", self.name);
        }
        Ok(())
    }

    pub async fn release(mut self) -> Result<()> {
        if let Some(mut connection) = self.connection.take() {
            let released =
                sqlx::query_scalar!(r#"SELECT RELEASE_LOCK(?) AS `released`"#, self.name)
                    .fetch_one(&mut connection)
                    .await?;
            connection.close().await?;
            if released != Some(1) {
                anyhow::bail!("failed to release process lock {}", self.name);
            }
        }
        Ok(())
    }
}

impl Drop for ProcessLock {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            tokio::spawn(async move {
                let _ = connection.close().await;
            });
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChangeStreamCursorRow {
    pub id: i64,
    pub since: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PackageDownloadRow {
    pub id: i64,
    pub package_version_id: i64,
    pub year: u16,
    pub month: u8,
    pub d01: u32,
    pub d02: u32,
    pub d03: u32,
    pub d04: u32,
    pub d05: u32,
    pub d06: u32,
    pub d07: u32,
    pub d08: u32,
    pub d09: u32,
    pub d10: u32,
    pub d11: u32,
    pub d12: u32,
    pub d13: u32,
    pub d14: u32,
    pub d15: u32,
    pub d16: u32,
    pub d17: u32,
    pub d18: u32,
    pub d19: u32,
    pub d20: u32,
    pub d21: u32,
    pub d22: u32,
    pub d23: u32,
    pub d24: u32,
    pub d25: u32,
    pub d26: u32,
    pub d27: u32,
    pub d28: u32,
    pub d29: u32,
    pub d30: u32,
    pub d31: u32,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UpstreamPackageDownloadRow {
    pub id: i64,
    pub package_id: i64,
    pub year: u16,
    pub month: u8,
    pub d01: u32,
    pub d02: u32,
    pub d03: u32,
    pub d04: u32,
    pub d05: u32,
    pub d06: u32,
    pub d07: u32,
    pub d08: u32,
    pub d09: u32,
    pub d10: u32,
    pub d11: u32,
    pub d12: u32,
    pub d13: u32,
    pub d14: u32,
    pub d15: u32,
    pub d16: u32,
    pub d17: u32,
    pub d18: u32,
    pub d19: u32,
    pub d20: u32,
    pub d21: u32,
    pub d22: u32,
    pub d23: u32,
    pub d24: u32,
    pub d25: u32,
    pub d26: u32,
    pub d27: u32,
    pub d28: u32,
    pub d29: u32,
    pub d30: u32,
    pub d31: u32,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SyncTaskRow {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub error: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub started_at: Option<chrono::NaiveDateTime>,
    pub finished_at: Option<chrono::NaiveDateTime>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OrganizationRow {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OrgMemberRow {
    pub id: i64,
    pub org_id: i64,
    pub user_id: i64,
    pub role: String,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TeamRow {
    pub id: i64,
    pub org_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TeamMemberRow {
    pub id: i64,
    pub team_id: i64,
    pub user_id: i64,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PackageTeamPermissionRow {
    pub id: i64,
    pub package_id: i64,
    pub team_id: i64,
    pub permission: String,
    pub created_at: chrono::NaiveDateTime,
}

#[async_trait]
pub trait Repository: Send + Sync + 'static {
    async fn migrate(&self) -> Result<()>;
    async fn try_acquire_process_lock(&self, name: &str) -> Result<Option<ProcessLock>>;

    async fn storage_get_result(&self, key: &str) -> Result<object_store::GetResult>;
    async fn delete_storage_objects(&self, keys: &[String]) -> Vec<(String, Result<()>)>;
    fn list_storage_objects(&self, prefix: &str) -> BoxStream<'static, Result<StorageObjectMeta>>;

    // ── content ──

    async fn get_content(&self, dist_id: i64) -> Result<(Vec<u8>, DistRow)>;
    async fn prepare_raw_dist(&self, data: Vec<u8>) -> Result<PreparedDist>;
    async fn prepare_raw_dist_file(&self, path: &std::path::Path) -> Result<PreparedDist>;
    async fn prepare_json_dist(&self, data: Vec<u8>) -> Result<PreparedDist>;

    // ── packages ──

    async fn get_package_by_name(&self, name: &str) -> Result<Option<PackageRow>>;
    async fn list_packages(&self, offset: i64, limit: i64) -> Result<Vec<PackageRow>>;
    async fn set_package_access(&self, package_id: i64, access: &str) -> Result<()>;
    async fn count_packages(&self) -> Result<i64>;

    // ── package_versions ──

    async fn get_version(
        &self,
        package_id: i64,
        version: &str,
    ) -> Result<Option<PackageVersionRow>>;
    async fn list_versions(&self, package_id: i64) -> Result<Vec<PackageVersionRow>>;
    async fn attach_tar_dist(
        &self,
        version_id: i64,
        dist: &PreparedDist,
        tar_size: i64,
        sha1_digest: &[u8],
        sha512_digest: &[u8],
    ) -> Result<AttachDistOutcome>;

    // ── package_tags ──

    async fn list_tags(&self, package_id: i64) -> Result<Vec<PackageTagRow>>;

    // ── dists ──

    async fn get_dist(&self, id: i64) -> Result<Option<DistRow>>;
    async fn existing_dist_paths(&self, paths: &[String]) -> Result<HashSet<String>>;
    async fn list_orphan_dists(&self, min_age_secs: u64, limit: u32) -> Result<Vec<DistRow>>;
    async fn delete_dists_by_ids(&self, ids: &[i64]) -> Result<u64>;

    // ── change_stream_cursors ──

    async fn get_cursor(&self) -> Result<Option<ChangeStreamCursorRow>>;
    async fn upsert_cursor(&self, since: &str) -> Result<()>;

    // ── sync ──

    async fn commit_publish(&self, params: PublishCommitParams) -> Result<(i64, String)>;
    async fn commit_local_manifest(&self, params: LocalManifestCommitParams) -> Result<()>;
    async fn commit_sync_package(
        &self,
        params: SyncPackageCommitParams,
    ) -> Result<SyncPackageCommitResult>;
    async fn delete_local_package(
        &self,
        package_id: i64,
        expected_full_dist_id: Option<i64>,
    ) -> Result<()>;

    // ── sync_tasks ──

    async fn enqueue_sync_task(&self, name: &str, source: &str) -> Result<Option<i64>>;
    async fn bulk_enqueue_sync_tasks(&self, names: &[String], source: &str) -> Result<u64>;
    async fn claim_sync_task(&self) -> Result<Option<SyncTaskRow>>;
    async fn complete_sync_task(&self, id: i64, error: Option<&str>) -> Result<()>;
    async fn requeue_stale_tasks(&self, timeout_secs: u64) -> Result<u64>;
    async fn count_tasks_by_status(&self) -> Result<HashMap<String, i64>>;
    async fn cleanup_old_tasks(&self, retention_days: u32) -> Result<u64>;

    // ── users ──

    async fn get_user_by_name(&self, name: &str) -> Result<Option<UserRow>>;
    async fn get_user_by_id(&self, id: i64) -> Result<Option<UserRow>>;
    async fn create_user(
        &self,
        name: &str,
        email: Option<&str>,
        password_salt: Option<&str>,
        password_integrity: Option<&str>,
    ) -> Result<i64>;
    async fn upsert_user(
        &self,
        name: &str,
        email: Option<&str>,
        upstream_name: &str,
    ) -> Result<i64>;

    // ── tokens ──

    async fn find_token_by_key(&self, token_key: &str) -> Result<Option<TokenRow>>;
    async fn list_tokens_by_user(&self, user_id: i64) -> Result<Vec<TokenRow>>;
    async fn delete_token_by_id(&self, id: i64) -> Result<()>;
    #[allow(clippy::too_many_arguments)]
    async fn create_token(
        &self,
        token_key: &str,
        name: &str,
        user_id: i64,
        is_readonly: bool,
        allowed_scopes: Option<&str>,
        cidr_whitelist: Option<&str>,
        expired_at: Option<chrono::NaiveDateTime>,
    ) -> Result<i64>;
    async fn touch_token(&self, id: i64) -> Result<()>;

    // ── maintainers ──

    async fn save_maintainer(&self, package_id: i64, user_id: i64, source: &str) -> Result<()>;
    async fn is_maintainer(&self, package_id: i64, user_id: i64) -> Result<bool>;
    async fn list_maintainers(&self, package_id: i64) -> Result<Vec<Maintainer>>;
    async fn list_packages_by_user_id(&self, user_id: i64) -> Result<Vec<PackageRow>>;
    async fn list_packages_by_user_id_readable(
        &self,
        target_user_id: i64,
        viewer_user_id: i64,
    ) -> Result<Vec<PackageRow>>;

    // ── organizations ──

    async fn create_org(&self, name: &str, description: Option<&str>) -> Result<i64>;
    async fn create_org_with_owner(
        &self,
        name: &str,
        owner_user_id: i64,
        developers_team_name: &str,
    ) -> Result<i64>;
    async fn get_org_by_name(&self, name: &str) -> Result<Option<OrganizationRow>>;
    async fn delete_org(&self, id: i64) -> Result<()>;

    // ── org_members ──

    async fn remove_org_member_cascade(&self, org_id: i64, user_id: i64) -> Result<bool>;
    async fn set_org_member_role_and_join_developers(
        &self,
        org_id: i64,
        user_id: i64,
        role: &str,
        developers_team_name: &str,
    ) -> Result<bool>;
    async fn list_org_members(&self, org_id: i64) -> Result<Vec<OrgMemberRow>>;
    async fn list_org_member_roster(&self, org_id: i64) -> Result<Vec<(String, String)>>;
    async fn get_org_member(&self, org_id: i64, user_id: i64) -> Result<Option<OrgMemberRow>>;
    async fn count_org_owners(&self, org_id: i64) -> Result<i64>;
    async fn count_org_members(&self, org_id: i64) -> Result<i64>;

    // ── teams ──

    async fn create_team(&self, org_id: i64, name: &str, description: Option<&str>) -> Result<i64>;
    async fn get_team_by_org_name(&self, org_id: i64, team_name: &str) -> Result<Option<TeamRow>>;
    async fn delete_team(&self, team_id: i64) -> Result<()>;
    async fn list_teams_in_org(&self, org_id: i64) -> Result<Vec<TeamRow>>;

    // ── team_members ──

    async fn add_team_member(&self, team_id: i64, user_id: i64) -> Result<()>;
    async fn remove_team_member(&self, team_id: i64, user_id: i64) -> Result<()>;
    async fn list_team_members(&self, team_id: i64) -> Result<Vec<TeamMemberRow>>;
    async fn list_team_member_names(&self, team_id: i64) -> Result<Vec<String>>;

    // ── package_team_permissions ──

    async fn grant_team_permission(
        &self,
        package_id: i64,
        team_id: i64,
        permission: &str,
    ) -> Result<()>;
    async fn revoke_team_permission(&self, package_id: i64, team_id: i64) -> Result<()>;
    async fn list_packages_for_team(&self, team_id: i64) -> Result<Vec<(PackageRow, String)>>;
    async fn list_org_package_viewer_permissions(
        &self,
        org_id: i64,
        viewer_user_id: i64,
    ) -> Result<HashMap<i64, bool>>;

    // ── org/team auth helpers ──

    async fn user_has_team_access(
        &self,
        package_id: i64,
        user_id: i64,
        min_permission: &str,
    ) -> Result<bool>;
    async fn user_is_org_manager_for_scope(&self, scope: &str, user_id: i64) -> Result<bool>;
    async fn list_all_packages_in_org(&self, org_id: i64) -> Result<Vec<PackageRow>>;
    async fn list_packages_in_org_viewable(
        &self,
        org_id: i64,
        viewer_user_id: i64,
    ) -> Result<Vec<PackageRow>>;

    // ── sync_tasks ──

    async fn fail_task_no_retry(&self, id: i64, error: &str) -> Result<()>;

    // ── package_downloads ──

    async fn existing_version_ids(&self, version_ids: &[i64]) -> Result<Vec<i64>>;
    async fn increment_package_download(
        &self,
        package_version_id: i64,
        year: u16,
        month: u8,
        day: u8,
        count: u64,
    ) -> Result<()>;
    async fn query_package_downloads_by_version(
        &self,
        package_version_id: i64,
        year: u16,
    ) -> Result<Vec<PackageDownloadRow>>;
    async fn query_package_downloads_by_package(
        &self,
        package_id: i64,
        start_year: u16,
        start_month: u8,
        end_year: u16,
        end_month: u8,
    ) -> Result<Vec<(i64, String, PackageDownloadRow)>>;

    // ── upstream_package_downloads ──

    async fn upsert_upstream_download(
        &self,
        package_id: i64,
        year: u16,
        month: u8,
        day: u8,
        count: u64,
    ) -> Result<()>;
    async fn query_upstream_downloads(
        &self,
        package_id: i64,
        start_year: u16,
        start_month: u8,
        end_year: u16,
        end_month: u8,
    ) -> Result<Vec<UpstreamPackageDownloadRow>>;
}
