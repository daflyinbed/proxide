use crate::config::{DatabaseConfig, StorageConfig};
use crate::repository::{
    ChangeStreamCursorRow, DistRow, PackageRow, PackageTagRow, PackageVersionRow,
    PublishVersionParams, Repository, SyncManifestParams, SyncTaskRow, TokenRow, UserRow,
    VersionCommitParams,
};
use crate::storage::Storage;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use tracing::error;
use sqlx::{MySql, Pool, Row};

#[derive(Debug, Clone)]
pub struct MysqlRepository {
    pool: Pool<MySql>,
    storage: Storage,
}

impl MysqlRepository {
    pub async fn new(db_config: &DatabaseConfig, storage_config: &StorageConfig) -> Result<Self> {
        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(20)
            .connect(&db_config.uri)
            .await?;
        let storage = Storage::new(storage_config)?;
        Ok(Self { pool, storage })
    }

    pub async fn health_check(&self) -> bool {
        sqlx::query_scalar!("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .is_ok()
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    async fn insert_dist(
        &self,
        name: &str,
        path: &str,
        size: i64,
        shasum: Option<&str>,
        integrity: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query!(
            r#"INSERT INTO dists (name, path, size, shasum, integrity) VALUES (?, ?, ?, ?, ?)"#,
            name,
            path,
            size,
            shasum,
            integrity
        )
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_id() as i64)
    }

    async fn delete_dist(&self, id: i64) -> Result<()> {
        sqlx::query!("DELETE FROM dists WHERE id = ?", id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Repository for MysqlRepository {
    // ── content ──

    async fn get_content(&self, dist_id: i64) -> Result<(Vec<u8>, DistRow)> {
        let dist = self
            .get_dist(dist_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("dist not found: {dist_id}"))?;
        let data = self.storage.get(&dist.path).await?;
        Ok((data, dist))
    }

    async fn put_content(
        &self,
        name: &str,
        storage_key: &str,
        data: Vec<u8>,
        shasum: Option<&str>,
        integrity: Option<&str>,
    ) -> Result<i64> {
        let len = data.len() as i64;
        self.storage.put(storage_key, data).await?;
        let dist_id = self
            .insert_dist(name, storage_key, len, shasum, integrity)
            .await?;
        Ok(dist_id)
    }

    async fn delete_content(&self, dist_id: i64) -> Result<()> {
        let dist_path = self.get_dist(dist_id).await?.map(|d| d.path);
        self.delete_dist(dist_id).await?;
        if let Some(path) = dist_path {
            if let Err(e) = self.storage.delete(&path).await {
                error!("failed to delete storage object for dist {dist_id} path {path}: {e:#}");
            }
        }
        Ok(())
    }

    async fn put_storage(&self, storage_key: &str, data: Vec<u8>) -> Result<()> {
        self.storage.put(storage_key, data).await
    }

    // ── packages ──

    async fn get_package_by_name(&self, name: &str) -> Result<Option<PackageRow>> {
        let row = sqlx::query_as!(
            PackageRow,
            r#"SELECT id, name, scope, description, source, abbreviated_dist_id, full_dist_id FROM packages WHERE name = ?"#,
            name
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn upsert_package(
        &self,
        name: &str,
        scope: Option<&str>,
        description: Option<&str>,
        source: Option<&str>,
    ) -> Result<(i64, Option<String>)> {
        sqlx::query!(
            r#"INSERT INTO packages (name, scope, description, source) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE description = IF(VALUES(description) IS NULL, description, VALUES(description)), source = IF(source IS NULL, VALUES(source), source)"#,
            name,
            scope,
            description,
            source
        )
        .execute(&self.pool)
        .await?;
        let row = sqlx::query!(
            r#"SELECT id, source FROM packages WHERE name = ?"#,
            name
        )
        .fetch_one(&self.pool)
        .await?;
        Ok((row.id as i64, row.source))
    }

    async fn update_package_dists(
        &self,
        package_id: i64,
        abbreviated_dist_id: Option<i64>,
        full_dist_id: Option<i64>,
    ) -> Result<()> {
        sqlx::query!(
            r#"UPDATE packages SET abbreviated_dist_id = ?, full_dist_id = ? WHERE id = ?"#,
            abbreviated_dist_id,
            full_dist_id,
            package_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn count_packages(&self) -> Result<i64> {
        let row = sqlx::query!(r#"SELECT COUNT(*) AS `count` FROM packages"#)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.count)
    }

    // ── package_versions ──

    async fn get_version(
        &self,
        package_id: i64,
        version: &str,
    ) -> Result<Option<PackageVersionRow>> {
        let row = sqlx::query_as!(
            PackageVersionRow,
            r#"SELECT id, package_id, version, abbrev_dist_id, manifest_dist_id, tar_dist_id, readme_dist_id, publish_time, is_pre_release as "is_pre_release: bool", padding_version FROM package_versions WHERE package_id = ? AND version = ?"#,
            package_id,
            version
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn list_versions(&self, package_id: i64) -> Result<Vec<PackageVersionRow>> {
        let rows = sqlx::query_as!(
            PackageVersionRow,
            r#"SELECT id, package_id, version, abbrev_dist_id, manifest_dist_id, tar_dist_id, readme_dist_id, publish_time, is_pre_release as "is_pre_release: bool", padding_version FROM package_versions WHERE package_id = ? ORDER BY publish_time DESC"#,
            package_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn insert_version(
        &self,
        package_id: i64,
        version: &str,
        publish_time: chrono::NaiveDateTime,
        is_pre_release: bool,
        padding_version: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query!(
            r#"INSERT INTO package_versions (package_id, version, publish_time, is_pre_release, padding_version) VALUES (?, ?, ?, ?, ?)"#,
            package_id,
            version,
            publish_time,
            is_pre_release,
            padding_version
        )
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_id() as i64)
    }

    async fn update_version_dists(
        &self,
        version_id: i64,
        abbrev_dist_id: Option<i64>,
        manifest_dist_id: Option<i64>,
        tar_dist_id: Option<i64>,
        readme_dist_id: Option<i64>,
    ) -> Result<()> {
        sqlx::query!(
            r#"UPDATE package_versions SET abbrev_dist_id = ?, manifest_dist_id = ?, tar_dist_id = ?, readme_dist_id = ? WHERE id = ?"#,
            abbrev_dist_id,
            manifest_dist_id,
            tar_dist_id,
            readme_dist_id,
            version_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_version_by_tarball_filename(
        &self,
        package_id: i64,
        filename: &str,
    ) -> Result<Option<PackageVersionRow>> {
        let name_part = filename.strip_suffix(".tgz").unwrap_or(filename);
        let row = sqlx::query_as!(
            PackageVersionRow,
            r#"SELECT pv.id, pv.package_id, pv.version, pv.abbrev_dist_id, pv.manifest_dist_id, pv.tar_dist_id, pv.readme_dist_id, pv.publish_time, pv.is_pre_release as "is_pre_release: bool", pv.padding_version FROM package_versions pv JOIN dists d ON pv.tar_dist_id = d.id WHERE pv.package_id = ? AND d.name = ?"#,
            package_id,
            name_part
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn get_versions_not_in(
        &self,
        package_id: i64,
        keep_versions: &[String],
    ) -> Result<Vec<PackageVersionRow>> {
        if keep_versions.is_empty() {
            let rows = sqlx::query_as!(
                PackageVersionRow,
                r#"SELECT id, package_id, version, abbrev_dist_id, manifest_dist_id, tar_dist_id, readme_dist_id, publish_time, is_pre_release as "is_pre_release: bool", padding_version FROM package_versions WHERE package_id = ?"#,
                package_id
            )
            .fetch_all(&self.pool)
            .await?;
            return Ok(rows);
        }
        let placeholders: Vec<String> = keep_versions.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT id, package_id, version, abbrev_dist_id, manifest_dist_id, tar_dist_id, readme_dist_id, publish_time, is_pre_release as \"is_pre_release: bool\", padding_version FROM package_versions WHERE package_id = ? AND version NOT IN ({})",
            placeholders.join(",")
        );
        let mut query = sqlx::query_as::<_, PackageVersionRow>(&sql).bind(package_id);
        for v in keep_versions {
            query = query.bind(v);
        }
        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    async fn delete_versions_by_ids(&self, version_ids: &[i64]) -> Result<()> {
        if version_ids.is_empty() {
            return Ok(());
        }
        let placeholders: Vec<String> = version_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "DELETE FROM package_versions WHERE id IN ({})",
            placeholders.join(",")
        );
        let mut query = sqlx::query(&sql);
        for id in version_ids {
            query = query.bind(id);
        }
        query.execute(&self.pool).await?;
        Ok(())
    }

    // ── package_tags ──

    async fn list_tags(&self, package_id: i64) -> Result<Vec<PackageTagRow>> {
        let rows = sqlx::query_as!(
            PackageTagRow,
            r#"SELECT id, package_id, tag, version FROM package_tags WHERE package_id = ?"#,
            package_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn sync_tags(
        &self,
        package_id: i64,
        tags: &HashMap<String, String>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sync_tags_tx(&mut tx, package_id, tags).await?;
        tx.commit().await?;
        Ok(())
    }

    // ── dists ──

    async fn get_dist(&self, id: i64) -> Result<Option<DistRow>> {
        let row = sqlx::query_as!(
            DistRow,
            r#"SELECT id, name, path, size, shasum, integrity FROM dists WHERE id = ?"#,
            id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn list_orphan_dists(&self) -> Result<Vec<DistRow>> {
        let rows = sqlx::query_as!(
            DistRow,
            r#"SELECT d.id, d.name, d.path, d.size, d.shasum, d.integrity
               FROM dists d
               LEFT JOIN packages p ON p.abbreviated_dist_id = d.id OR p.full_dist_id = d.id
               LEFT JOIN package_versions pv ON pv.abbrev_dist_id = d.id OR pv.manifest_dist_id = d.id OR pv.tar_dist_id = d.id OR pv.readme_dist_id = d.id
               WHERE p.id IS NULL AND pv.id IS NULL"#
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete_dists_by_ids(&self, ids: &[i64]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!("DELETE FROM dists WHERE id IN ({})", placeholders.join(","));
        let mut query = sqlx::query(&sql);
        for id in ids {
            query = query.bind(id);
        }
        let result = query.execute(&self.pool).await?;
        Ok(result.rows_affected())
    }

    // ── change_stream_cursors ──

    async fn get_cursor(&self) -> Result<Option<ChangeStreamCursorRow>> {
        let row = sqlx::query_as!(
            ChangeStreamCursorRow,
            r#"SELECT id, since FROM change_stream_cursors WHERE id = 1"#
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn upsert_cursor(&self, since: &str) -> Result<()> {
        sqlx::query!(
            r#"INSERT INTO change_stream_cursors (id, since) VALUES (1, ?) ON DUPLICATE KEY UPDATE since = VALUES(since)"#,
            since
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── sync ──

    async fn commit_version(&self, params: VersionCommitParams) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let abbrev_dist_id = insert_dist_tx(
            &mut tx,
            &params.abbrev_dist.name,
            &params.abbrev_dist.path,
            params.abbrev_dist.size,
            params.abbrev_dist.shasum.as_deref(),
            params.abbrev_dist.integrity.as_deref(),
        )
        .await?;

        let manifest_dist_id = insert_dist_tx(
            &mut tx,
            &params.manifest_dist.name,
            &params.manifest_dist.path,
            params.manifest_dist.size,
            params.manifest_dist.shasum.as_deref(),
            params.manifest_dist.integrity.as_deref(),
        )
        .await?;

        sqlx::query!(
            r#"INSERT INTO package_versions (package_id, version, publish_time, is_pre_release, padding_version, abbrev_dist_id, manifest_dist_id) VALUES (?, ?, ?, ?, ?, ?, ?)"#,
            params.package_id,
            params.version,
            params.publish_time,
            params.is_pre_release,
            params.padding_version,
            abbrev_dist_id,
            manifest_dist_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn sync_manifest_commit(&self, params: SyncManifestParams) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sync_tags_tx(&mut tx, params.package_id, &params.tags).await?;

        let abbrev_manifest_dist_id = insert_dist_tx(
            &mut tx,
            &params.abbrev_manifest.name,
            &params.abbrev_manifest.path,
            params.abbrev_manifest.size,
            params.abbrev_manifest.shasum.as_deref(),
            params.abbrev_manifest.integrity.as_deref(),
        )
        .await?;

        let full_manifest_dist_id = insert_dist_tx(
            &mut tx,
            &params.full_manifest.name,
            &params.full_manifest.path,
            params.full_manifest.size,
            params.full_manifest.shasum.as_deref(),
            params.full_manifest.integrity.as_deref(),
        )
        .await?;

        sqlx::query!(
            r#"UPDATE packages SET abbreviated_dist_id = ?, full_dist_id = ? WHERE id = ?"#,
            abbrev_manifest_dist_id,
            full_manifest_dist_id,
            params.package_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    // ── sync_tasks ──

    async fn enqueue_sync_task(&self, name: &str, source: &str) -> Result<Option<i64>> {
        let result = sqlx::query(
            "INSERT INTO sync_tasks (name, source, status) \
             SELECT ?, ?, 'pending' \
             FROM DUAL \
             WHERE NOT EXISTS (\
                 SELECT 1 FROM sync_tasks WHERE name = ? AND status = 'pending'\
             )",
        )
        .bind(name)
        .bind(source)
        .bind(name)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        Ok(Some(result.last_insert_id() as i64))
    }

    async fn claim_sync_task(&self) -> Result<Option<SyncTaskRow>> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query_as::<_, SyncTaskRow>(
            "SELECT id, name, source, status, attempts, max_attempts, error, created_at, started_at, finished_at \
             FROM sync_tasks \
             WHERE status = 'pending' \
             ORDER BY created_at \
             LIMIT 1 \
             FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;

        let Some(task) = row else {
            tx.rollback().await?;
            return Ok(None);
        };

        sqlx::query("UPDATE sync_tasks SET status = 'running', attempts = attempts + 1, started_at = NOW() WHERE id = ?")
            .bind(task.id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(Some(SyncTaskRow {
            status: "running".to_string(),
            attempts: task.attempts + 1,
            started_at: Some(chrono::Utc::now().naive_utc()),
            ..task
        }))
    }

    async fn complete_sync_task(&self, id: i64, error: Option<&str>) -> Result<()> {
        match error {
            None => {
                sqlx::query("UPDATE sync_tasks SET status = 'done', finished_at = NOW(), error = NULL WHERE id = ?")
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
            }
            Some(err) => {
                let task = sqlx::query_as::<_, SyncTaskRow>(
                    "SELECT id, name, source, status, attempts, max_attempts, error, created_at, started_at, finished_at FROM sync_tasks WHERE id = ?",
                )
                .bind(id)
                .fetch_one(&self.pool)
                .await?;

                if task.attempts < task.max_attempts {
                    sqlx::query("UPDATE sync_tasks SET status = 'pending', started_at = NULL, error = ? WHERE id = ?")
                        .bind(err)
                        .bind(id)
                        .execute(&self.pool)
                        .await?;
                } else {
                    sqlx::query("UPDATE sync_tasks SET status = 'failed', finished_at = NOW(), error = ? WHERE id = ?")
                        .bind(err)
                        .bind(id)
                        .execute(&self.pool)
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn requeue_stale_tasks(&self, timeout_secs: u64) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE sync_tasks SET status = 'pending', started_at = NULL \
             WHERE status = 'running' AND started_at < DATE_SUB(NOW(), INTERVAL ? SECOND)",
        )
        .bind(timeout_secs as i64)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    async fn count_tasks_by_status(&self) -> Result<HashMap<String, i64>> {
        let rows = sqlx::query("SELECT status, COUNT(*) AS `count` FROM sync_tasks GROUP BY status")
            .fetch_all(&self.pool)
            .await?;

        let mut map = HashMap::new();
        for row in rows {
            let status: String = row.try_get("status")?;
            let count: i64 = row.try_get("count")?;
            map.insert(status, count);
        }
        Ok(map)
    }

    async fn cleanup_old_tasks(&self, retention_days: u32) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM sync_tasks \
             WHERE status IN ('done', 'failed') \
             AND finished_at < DATE_SUB(NOW(), INTERVAL ? DAY)",
        )
        .bind(retention_days as i64)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    // ── users ──

    async fn get_user_by_name(&self, name: &str) -> Result<Option<UserRow>> {
        let row = sqlx::query_as!(
            UserRow,
            r#"SELECT id, name, email, upstream_name, password_salt, password_integrity FROM users WHERE name = ? AND upstream_name = ''"#,
            name
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn get_user_by_id(&self, id: i64) -> Result<Option<UserRow>> {
        let row = sqlx::query_as!(
            UserRow,
            r#"SELECT id, name, email, upstream_name, password_salt, password_integrity FROM users WHERE id = ?"#,
            id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn create_user(
        &self,
        name: &str,
        email: Option<&str>,
        password_salt: Option<&str>,
        password_integrity: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query!(
            r#"INSERT INTO users (name, email, upstream_name, password_salt, password_integrity) VALUES (?, ?, '', ?, ?)"#,
            name,
            email,
            password_salt,
            password_integrity
        )
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_id() as i64)
    }

    async fn upsert_user(
        &self,
        name: &str,
        email: Option<&str>,
        upstream_name: &str,
    ) -> Result<i64> {
        sqlx::query!(
            r#"INSERT INTO users (name, email, upstream_name) VALUES (?, ?, ?) ON DUPLICATE KEY UPDATE email = COALESCE(VALUES(email), email)"#,
            name,
            email,
            upstream_name
        )
        .execute(&self.pool)
        .await?;
        let row = sqlx::query_as!(
            UserRow,
            r#"SELECT id, name, email, upstream_name, password_salt, password_integrity FROM users WHERE name = ? AND upstream_name = ?"#,
            name,
            upstream_name
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.id)
    }

    // ── tokens ──

    async fn find_token_by_key(&self, token_key: &str) -> Result<Option<TokenRow>> {
        let row = sqlx::query_as!(
            TokenRow,
            r#"SELECT id, token_key, name, user_id, is_readonly as "is_readonly: bool", allowed_scopes, expired_at FROM tokens WHERE token_key = ?"#,
            token_key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn create_token(
        &self,
        token_key: &str,
        name: &str,
        user_id: i64,
        is_readonly: bool,
        allowed_scopes: Option<&str>,
        expired_at: Option<chrono::NaiveDateTime>,
    ) -> Result<i64> {
        let result = sqlx::query!(
            r#"INSERT INTO tokens (token_key, name, user_id, is_readonly, allowed_scopes, expired_at) VALUES (?, ?, ?, ?, ?, ?)"#,
            token_key,
            name,
            user_id,
            is_readonly,
            allowed_scopes,
            expired_at
        )
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_id() as i64)
    }

    async fn touch_token(&self, id: i64) -> Result<()> {
        sqlx::query!(r#"UPDATE tokens SET last_used_at = NOW() WHERE id = ?"#, id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── maintainers ──

    async fn save_maintainer(&self, package_id: i64, user_id: i64) -> Result<()> {
        sqlx::query!(
            r#"INSERT IGNORE INTO maintainers (package_id, user_id) VALUES (?, ?)"#,
            package_id,
            user_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn is_maintainer(&self, package_id: i64, user_id: i64) -> Result<bool> {
        let row = sqlx::query!(
            r#"SELECT COUNT(*) AS `count` FROM maintainers WHERE package_id = ? AND user_id = ?"#,
            package_id,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.count > 0)
    }

    async fn sync_maintainers(&self, package_id: i64, user_ids: &[i64]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        if user_ids.is_empty() {
            sqlx::query!(
                r#"DELETE FROM maintainers WHERE package_id = ?"#,
                package_id
            )
            .execute(&mut *tx)
            .await?;
        } else {
            let placeholders: Vec<String> = user_ids.iter().map(|_| "?".to_string()).collect();
            let sql = format!(
                "DELETE FROM maintainers WHERE package_id = ? AND user_id NOT IN ({})",
                placeholders.join(",")
            );
            let mut query = sqlx::query(&sql).bind(package_id);
            for id in user_ids {
                query = query.bind(id);
            }
            query.execute(&mut *tx).await?;

            for &user_id in user_ids {
                sqlx::query!(
                    r#"INSERT IGNORE INTO maintainers (package_id, user_id) VALUES (?, ?)"#,
                    package_id,
                    user_id
                )
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        Ok(())
    }

    // ── publish ──

    async fn commit_published_version(&self, params: PublishVersionParams) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let abbrev_dist_id = insert_dist_tx(
            &mut tx,
            &params.abbrev_dist.name,
            &params.abbrev_dist.path,
            params.abbrev_dist.size,
            params.abbrev_dist.shasum.as_deref(),
            params.abbrev_dist.integrity.as_deref(),
        )
        .await?;

        let manifest_dist_id = insert_dist_tx(
            &mut tx,
            &params.manifest_dist.name,
            &params.manifest_dist.path,
            params.manifest_dist.size,
            params.manifest_dist.shasum.as_deref(),
            params.manifest_dist.integrity.as_deref(),
        )
        .await?;

        let tar_dist_id = insert_dist_tx(
            &mut tx,
            &params.tar_dist.name,
            &params.tar_dist.path,
            params.tar_dist.size,
            params.tar_dist.shasum.as_deref(),
            params.tar_dist.integrity.as_deref(),
        )
        .await?;

        let readme_dist_id = insert_dist_tx(
            &mut tx,
            &params.readme_dist.name,
            &params.readme_dist.path,
            params.readme_dist.size,
            params.readme_dist.shasum.as_deref(),
            params.readme_dist.integrity.as_deref(),
        )
        .await?;

        sqlx::query!(
            r#"INSERT INTO package_versions (package_id, version, publish_time, is_pre_release, padding_version, abbrev_dist_id, manifest_dist_id, tar_dist_id, readme_dist_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            params.package_id,
            params.version,
            params.publish_time,
            params.is_pre_release,
            params.padding_version,
            abbrev_dist_id,
            manifest_dist_id,
            tar_dist_id,
            readme_dist_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn fail_task_no_retry(&self, id: i64, error: &str) -> Result<()> {
        sqlx::query(
            "UPDATE sync_tasks SET status = 'failed', attempts = max_attempts, finished_at = NOW(), error = ? WHERE id = ?",
        )
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

async fn insert_dist_tx(
    tx: &mut sqlx::Transaction<'_, MySql>,
    name: &str,
    path: &str,
    size: i64,
    shasum: Option<&str>,
    integrity: Option<&str>,
) -> Result<i64> {
    let result = sqlx::query!(
        r#"INSERT INTO dists (name, path, size, shasum, integrity) VALUES (?, ?, ?, ?, ?)"#,
        name,
        path,
        size,
        shasum,
        integrity
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.last_insert_id() as i64)
}

async fn sync_tags_tx(
    tx: &mut sqlx::Transaction<'_, MySql>,
    package_id: i64,
    tags: &HashMap<String, String>,
) -> Result<()> {
    let existing = sqlx::query_as!(
        PackageTagRow,
        r#"SELECT id, package_id, tag, version FROM package_tags WHERE package_id = ?"#,
        package_id
    )
    .fetch_all(&mut **tx)
    .await?;

    let existing_map: HashMap<String, String> = existing
        .into_iter()
        .map(|t| (t.tag, t.version))
        .collect();

    for (tag, version) in tags {
        if let Some(existing_ver) = existing_map.get(tag) {
            if existing_ver != version {
                sqlx::query!(
                    r#"UPDATE package_tags SET version = ? WHERE package_id = ? AND tag = ?"#,
                    version,
                    package_id,
                    tag
                )
                .execute(&mut **tx)
                .await?;
            }
        } else {
            sqlx::query!(
                r#"INSERT INTO package_tags (package_id, tag, version) VALUES (?, ?, ?)"#,
                package_id,
                tag,
                version
            )
            .execute(&mut **tx)
            .await?;
        }
    }

    for existing_tag in existing_map.keys() {
        if !tags.contains_key(existing_tag) {
            sqlx::query!(
                r#"DELETE FROM package_tags WHERE package_id = ? AND tag = ?"#,
                package_id,
                existing_tag
            )
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(())
}
