pub mod mysql;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PackageRow {
    pub id: i64,
    pub name: String,
    pub scope: Option<String>,
    pub description: Option<String>,
    pub source: Option<String>,
    pub abbreviated_dist_id: Option<i64>,
    pub full_dist_id: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PackageVersionRow {
    pub id: i64,
    pub package_id: i64,
    pub version: String,
    pub abbrev_dist_id: Option<i64>,
    pub manifest_dist_id: Option<i64>,
    pub tar_dist_id: Option<i64>,
    pub readme_dist_id: Option<i64>,
    pub publish_time: chrono::NaiveDateTime,
    pub is_pre_release: bool,
    pub padding_version: Option<String>,
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
    pub name: String,
    pub path: String,
    pub size: i64,
    pub shasum: Option<String>,
    pub integrity: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
    pub upstream_name: String,
    pub password_salt: Option<String>,
    pub password_integrity: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TokenRow {
    pub id: i64,
    pub token_key: String,
    pub name: String,
    pub user_id: i64,
    pub is_readonly: bool,
    pub allowed_scopes: Option<String>,
    pub expired_at: Option<chrono::NaiveDateTime>,
}

#[derive(Debug, Clone)]
pub struct PendingDist {
    pub name: String,
    pub path: String,
    pub size: i64,
    pub shasum: Option<String>,
    pub integrity: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VersionCommitParams {
    pub package_id: i64,
    pub version: String,
    pub publish_time: chrono::NaiveDateTime,
    pub is_pre_release: bool,
    pub padding_version: Option<String>,
    pub abbrev_dist: PendingDist,
    pub manifest_dist: PendingDist,
}

#[derive(Debug, Clone)]
pub struct PublishVersionParams {
    pub package_id: i64,
    pub version: String,
    pub publish_time: chrono::NaiveDateTime,
    pub is_pre_release: bool,
    pub padding_version: Option<String>,
    pub abbrev_dist: PendingDist,
    pub manifest_dist: PendingDist,
    pub tar_dist: PendingDist,
    pub readme_dist: PendingDist,
}

#[derive(Debug, Clone)]
pub struct SyncManifestParams {
    pub package_id: i64,
    pub tags: HashMap<String, String>,
    pub abbrev_manifest: PendingDist,
    pub full_manifest: PendingDist,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChangeStreamCursorRow {
    pub id: i64,
    pub since: String,
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

#[async_trait]
pub trait Repository: Send + Sync + 'static {
    async fn migrate(&self) -> Result<()>;

    // ── content ──

    async fn get_content(&self, dist_id: i64) -> Result<(Vec<u8>, DistRow)>;
    async fn put_content(
        &self,
        name: &str,
        storage_key: &str,
        data: Vec<u8>,
        shasum: Option<&str>,
        integrity: Option<&str>,
    ) -> Result<i64>;
    async fn delete_content(&self, dist_id: i64) -> Result<()>;
    async fn put_storage(&self, storage_key: &str, data: Vec<u8>) -> Result<()>;

    // ── packages ──

    async fn get_package_by_name(&self, name: &str) -> Result<Option<PackageRow>>;
    async fn upsert_package(
        &self,
        name: &str,
        scope: Option<&str>,
        description: Option<&str>,
        source: Option<&str>,
    ) -> Result<(i64, Option<String>)>;
    async fn update_package_dists(
        &self,
        package_id: i64,
        abbreviated_dist_id: Option<i64>,
        full_dist_id: Option<i64>,
    ) -> Result<()>;
    async fn count_packages(&self) -> Result<i64>;

    // ── package_versions ──

    async fn get_version(&self, package_id: i64, version: &str) -> Result<Option<PackageVersionRow>>;
    async fn list_versions(&self, package_id: i64) -> Result<Vec<PackageVersionRow>>;
    async fn insert_version(
        &self,
        package_id: i64,
        version: &str,
        publish_time: chrono::NaiveDateTime,
        is_pre_release: bool,
        padding_version: Option<&str>,
    ) -> Result<i64>;
    async fn update_version_dists(
        &self,
        version_id: i64,
        abbrev_dist_id: Option<i64>,
        manifest_dist_id: Option<i64>,
        tar_dist_id: Option<i64>,
        readme_dist_id: Option<i64>,
    ) -> Result<()>;
    async fn get_version_by_tarball_filename(
        &self,
        package_id: i64,
        filename: &str,
    ) -> Result<Option<PackageVersionRow>>;
    async fn get_versions_not_in(
        &self,
        package_id: i64,
        keep_versions: &[String],
    ) -> Result<Vec<PackageVersionRow>>;
    async fn delete_versions_by_ids(&self, version_ids: &[i64]) -> Result<()>;

    // ── package_tags ──

    async fn list_tags(&self, package_id: i64) -> Result<Vec<PackageTagRow>>;
    async fn sync_tags(&self, package_id: i64, tags: &HashMap<String, String>) -> Result<()>;

    // ── dists ──

    async fn get_dist(&self, id: i64) -> Result<Option<DistRow>>;
    async fn list_orphan_dists(&self) -> Result<Vec<DistRow>>;
    async fn delete_dists_by_ids(&self, ids: &[i64]) -> Result<u64>;

    // ── change_stream_cursors ──

    async fn get_cursor(&self) -> Result<Option<ChangeStreamCursorRow>>;
    async fn upsert_cursor(&self, since: &str) -> Result<()>;

    // ── sync ──

    async fn commit_version(&self, params: VersionCommitParams) -> Result<()>;
    async fn sync_manifest_commit(&self, params: SyncManifestParams) -> Result<()>;

    // ── sync_tasks ──

    async fn enqueue_sync_task(&self, name: &str, source: &str) -> Result<Option<i64>>;
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
    async fn create_token(
        &self,
        token_key: &str,
        name: &str,
        user_id: i64,
        is_readonly: bool,
        allowed_scopes: Option<&str>,
        expired_at: Option<chrono::NaiveDateTime>,
    ) -> Result<i64>;
    async fn touch_token(&self, id: i64) -> Result<()>;

    // ── maintainers ──

    async fn save_maintainer(&self, package_id: i64, user_id: i64) -> Result<()>;
    async fn is_maintainer(&self, package_id: i64, user_id: i64) -> Result<bool>;
    async fn sync_maintainers(&self, package_id: i64, user_ids: &[i64]) -> Result<()>;

    // ── publish ──

    async fn commit_published_version(&self, params: PublishVersionParams) -> Result<()>;

    // ── sync_tasks ──

    async fn fail_task_no_retry(&self, id: i64, error: &str) -> Result<()>;
}
