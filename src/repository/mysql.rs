use crate::config::{DatabaseConfig, StorageConfig};
use crate::npm::types::Maintainer;
use crate::repository::{
    ChangeStreamCursorRow, CommitVersionParams, DistRow, MAINTAINER_SOURCE_MANUAL, NewVersionFile,
    OrganizationRow, OrgMemberRow, PackageDownloadRow, PackageRow, PackageTagRow, PackageVersionRow,
    Repository, SyncManifestParams, SyncTaskRow, TeamMemberRow, TeamRow, TokenRow,
    UpstreamPackageDownloadRow, UserRow, VersionFileRow,
};
use crate::storage::Storage;
use anyhow::Result;
use async_trait::async_trait;
use sqlx::{MySql, Pool, Row};
use std::collections::HashMap;
use tracing::error;

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
    async fn storage_exists(&self, key: &str) -> Result<bool> {
        self.storage.exists(key).await
    }

    async fn storage_get_result(&self, key: &str) -> Result<object_store::GetResult> {
        self.storage.get_result(key).await
    }

    async fn storage_put_multipart(&self, key: &str) -> Result<object_store::WriteMultipart> {
        self.storage.put_multipart(key).await
    }

    async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

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

    async fn create_dist(
        &self,
        name: &str,
        storage_key: &str,
        size: i64,
        shasum: Option<&str>,
        integrity: Option<&str>,
    ) -> Result<i64> {
        self.insert_dist(name, storage_key, size, shasum, integrity)
            .await
    }

    async fn delete_content(&self, dist_id: i64) -> Result<()> {
        let dist_path = self.get_dist(dist_id).await?.map(|d| d.path);
        self.delete_dist(dist_id).await?;
        if let Some(path) = dist_path
            && let Err(e) = self.storage.delete(&path).await
        {
            error!("failed to delete storage object for dist {dist_id} path {path}: {e:#}");
        }
        Ok(())
    }

    async fn put_storage(&self, storage_key: &str, data: Vec<u8>) -> Result<()> {
        self.storage.put(storage_key, data).await
    }

    async fn put_storage_compressed(&self, storage_key: &str, data: Vec<u8>) -> Result<String> {
        self.storage.put_compressed(storage_key, data).await
    }

    // ── packages ──

    async fn get_package_by_name(&self, name: &str) -> Result<Option<PackageRow>> {
        let row = sqlx::query_as!(
            PackageRow,
            r#"SELECT id, name, scope, description, source, access, abbreviated_dist_id, full_dist_id FROM packages WHERE name = ?"#,
            name
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn list_packages(&self, offset: i64, limit: i64) -> Result<Vec<PackageRow>> {
        let rows = sqlx::query_as!(
            PackageRow,
            r#"SELECT id, name, scope, description, source, access, abbreviated_dist_id, full_dist_id
               FROM packages ORDER BY id LIMIT ? OFFSET ?"#,
            limit,
            offset
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
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
        let row = sqlx::query!(r#"SELECT id, source FROM packages WHERE name = ?"#, name)
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

    async fn set_package_access(&self, package_id: i64, access: &str) -> Result<()> {
        sqlx::query!(
            r#"UPDATE packages SET access = ? WHERE id = ?"#,
            access,
            package_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_package_by_id(&self, package_id: i64) -> Result<()> {
        sqlx::query!(r#"DELETE FROM packages WHERE id = ?"#, package_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn upsert_package_for_publish(
        &self,
        name: &str,
        scope: Option<&str>,
        description: Option<&str>,
        user_id: i64,
        access: Option<&str>,
    ) -> Result<(i64, Option<String>)> {
        let mut tx = self.pool.begin().await?;

        sqlx::query!(
            r#"INSERT INTO packages (name, scope, description, source) VALUES (?, ?, ?, NULL) ON DUPLICATE KEY UPDATE description = IF(source IS NULL AND VALUES(description) IS NOT NULL, VALUES(description), description), source = IF(source IS NULL, VALUES(source), source)"#,
            name,
            scope,
            description,
        )
        .execute(&mut *tx)
        .await?;

        let row = sqlx::query!(r#"SELECT id, source FROM packages WHERE name = ?"#, name)
            .fetch_one(&mut *tx)
            .await?;
        let package_id = row.id as i64;
        let existing_source = row.source;

        if existing_source.is_none() {
            sqlx::query!(
                r#"INSERT IGNORE INTO maintainers (package_id, user_id, source) VALUES (?, ?, ?)"#,
                package_id,
                user_id,
                MAINTAINER_SOURCE_MANUAL,
            )
            .execute(&mut *tx)
            .await?;

            if let Some(access) = access {
                sqlx::query!(
                    r#"UPDATE packages SET access = ? WHERE id = ?"#,
                    access,
                    package_id,
                )
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        Ok((package_id, existing_source))
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
            "SELECT id, package_id, version, abbrev_dist_id, manifest_dist_id, tar_dist_id, readme_dist_id, publish_time, is_pre_release, padding_version FROM package_versions WHERE package_id = ? AND version NOT IN ({})",
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

    async fn sync_tags(&self, package_id: i64, tags: &HashMap<String, String>) -> Result<()> {
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

    async fn get_dist_by_path(&self, path: &str) -> Result<Option<DistRow>> {
        let row = sqlx::query_as!(
            DistRow,
            r#"SELECT id, name, path, size, shasum, integrity FROM dists WHERE path = ? LIMIT 1"#,
            path
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
               LEFT JOIN package_version_files pvf ON pvf.dist_id = d.id
               WHERE p.id IS NULL AND pv.id IS NULL AND pvf.id IS NULL"#
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

    // ── package_version_files ──

    async fn has_version_files(&self, version_id: i64) -> Result<bool> {
        let row = sqlx::query_scalar!(
            "SELECT 1 FROM package_version_files WHERE package_version_id = ? LIMIT 1",
            version_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    async fn get_version_file(
        &self,
        version_id: i64,
        filepath: &str,
    ) -> Result<Option<VersionFileRow>> {
        let row = sqlx::query_as!(
            VersionFileRow,
            r#"SELECT pvf.filepath AS `filepath`,
                      pvf.content_type AS `content_type`,
                      d.size AS `size`,
                      d.shasum AS `shasum`,
                      d.path AS `storage_path`
               FROM package_version_files pvf
               JOIN dists d ON d.id = pvf.dist_id
               WHERE pvf.package_version_id = ? AND pvf.filepath = ?
               LIMIT 1"#,
            version_id,
            filepath
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn list_version_files(&self, version_id: i64) -> Result<Vec<VersionFileRow>> {
        let rows = sqlx::query_as!(
            VersionFileRow,
            r#"SELECT pvf.filepath AS `filepath`,
                      pvf.content_type AS `content_type`,
                      d.size AS `size`,
                      d.shasum AS `shasum`,
                      d.path AS `storage_path`
               FROM package_version_files pvf
               JOIN dists d ON d.id = pvf.dist_id
               WHERE pvf.package_version_id = ?
               ORDER BY pvf.filepath"#,
            version_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn insert_version_files(&self, version_id: i64, files: &[NewVersionFile]) -> Result<()> {
        const DISTS_NAME_MAX: usize = 512;
        let mut tx = self.pool.begin().await?;
        for f in files {
            if f.filepath.chars().count() > DISTS_NAME_MAX {
                anyhow::bail!(
                    "filepath for version {version_id} exceeds {DISTS_NAME_MAX} chars: {}",
                    f.filepath
                );
            }
            let dist_result = sqlx::query!(
                r#"INSERT INTO dists (name, path, size, shasum, integrity) VALUES (?, ?, ?, ?, NULL)"#,
                f.filepath,
                f.storage_key,
                f.size,
                f.shasum,
            )
            .execute(&mut *tx)
            .await?;
            let dist_id = dist_result.last_insert_id() as i64;
            sqlx::query!(
                r#"INSERT INTO package_version_files
                   (package_version_id, dist_id, filepath, content_type)
                   VALUES (?, ?, ?, ?)"#,
                version_id,
                dist_id,
                f.filepath,
                f.content_type,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn get_version_file_dist_ids(&self, version_ids: &[i64]) -> Result<Vec<(i64, String)>> {
        if version_ids.is_empty() {
            return Ok(Vec::new());
        }
        // sqlx::query! / query_as! require a fixed SQL string at compile time.
        // The IN clause here contains one placeholder per runtime version id,
        // so the query must be assembled dynamically.
        let placeholders: Vec<String> = version_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT d.id, d.path FROM package_version_files pvf JOIN dists d ON d.id = pvf.dist_id WHERE pvf.package_version_id IN ({})",
            placeholders.join(",")
        );
        let mut query = sqlx::query_as::<_, (i64, String)>(&sql);
        for id in version_ids {
            query = query.bind(id);
        }
        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
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

    async fn commit_version(&self, params: CommitVersionParams) -> Result<()> {
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

        let tar_dist_id = if let Some(d) = &params.tar_dist {
            Some(
                insert_dist_tx(
                    &mut tx,
                    &d.name,
                    &d.path,
                    d.size,
                    d.shasum.as_deref(),
                    d.integrity.as_deref(),
                )
                .await?,
            )
        } else {
            None
        };

        let readme_dist_id = if let Some(d) = &params.readme_dist {
            Some(
                insert_dist_tx(
                    &mut tx,
                    &d.name,
                    &d.path,
                    d.size,
                    d.shasum.as_deref(),
                    d.integrity.as_deref(),
                )
                .await?,
            )
        } else {
            None
        };

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

    async fn bulk_enqueue_sync_tasks(&self, names: &[String], source: &str) -> Result<u64> {
        if names.is_empty() {
            return Ok(0);
        }

        let mut total = 0u64;
        const BATCH: usize = 500;

        for chunk in names.chunks(BATCH) {
            let placeholders: Vec<String> = (0..chunk.len())
                .map(|i| {
                    if i == 0 {
                        "SELECT ? AS name, ? AS source".to_string()
                    } else {
                        "UNION ALL SELECT ?, ?".to_string()
                    }
                })
                .collect();
            let sql = format!(
                "INSERT INTO sync_tasks (name, source, status) \
                 SELECT t.name, t.source, 'pending' \
                 FROM ({}) AS t \
                 WHERE NOT EXISTS (\
                     SELECT 1 FROM sync_tasks st WHERE st.name = t.name AND st.status = 'pending'\
                 )",
                placeholders.join(" ")
            );

            let mut query = sqlx::query(&sql);
            for name in chunk {
                query = query.bind(name).bind(source);
            }
            let result = query.execute(&self.pool).await?;
            total += result.rows_affected();
        }

        Ok(total)
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
        let rows =
            sqlx::query("SELECT status, COUNT(*) AS `count` FROM sync_tasks GROUP BY status")
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
            r#"SELECT id, name, email, upstream_name, password_salt, password_integrity, created_at FROM users WHERE name = ? AND upstream_name = ''"#,
            name
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn get_user_by_id(&self, id: i64) -> Result<Option<UserRow>> {
        let row = sqlx::query_as!(
            UserRow,
            r#"SELECT id, name, email, upstream_name, password_salt, password_integrity, created_at FROM users WHERE id = ?"#,
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
            r#"SELECT id, name, email, upstream_name, password_salt, password_integrity, created_at FROM users WHERE name = ? AND upstream_name = ?"#,
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
            r#"SELECT id, token_key, name, user_id, is_readonly as "is_readonly: bool", allowed_scopes, cidr_whitelist, expired_at, created_at, updated_at, last_used_at FROM tokens WHERE token_key = ?"#,
            token_key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn list_tokens_by_user(&self, user_id: i64) -> Result<Vec<TokenRow>> {
        let rows = sqlx::query_as!(
            TokenRow,
            r#"SELECT id, token_key, name, user_id, is_readonly as "is_readonly: bool", allowed_scopes, cidr_whitelist, expired_at, created_at, updated_at, last_used_at FROM tokens WHERE user_id = ? ORDER BY id DESC"#,
            user_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete_token_by_id(&self, id: i64) -> Result<()> {
        sqlx::query!(r#"DELETE FROM tokens WHERE id = ?"#, id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn create_token(
        &self,
        token_key: &str,
        name: &str,
        user_id: i64,
        is_readonly: bool,
        allowed_scopes: Option<&str>,
        cidr_whitelist: Option<&str>,
        expired_at: Option<chrono::NaiveDateTime>,
    ) -> Result<i64> {
        let result = sqlx::query!(
            r#"INSERT INTO tokens (token_key, name, user_id, is_readonly, allowed_scopes, cidr_whitelist, expired_at) VALUES (?, ?, ?, ?, ?, ?, ?)"#,
            token_key,
            name,
            user_id,
            is_readonly,
            allowed_scopes,
            cidr_whitelist,
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

    async fn save_maintainer(
        &self,
        package_id: i64,
        user_id: i64,
        source: &str,
    ) -> Result<()> {
        sqlx::query!(
            r#"INSERT IGNORE INTO maintainers (package_id, user_id, source) VALUES (?, ?, ?)"#,
            package_id,
            user_id,
            source
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

    async fn sync_maintainers(
        &self,
        package_id: i64,
        user_ids: &[i64],
        source: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sync_maintainers_tx(&mut tx, package_id, user_ids, source).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn sync_maintainers_and_grant_team_permission(
        &self,
        package_id: i64,
        user_ids: &[i64],
        source: &str,
        team_id: i64,
        permission: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sync_maintainers_tx(&mut tx, package_id, user_ids, source).await?;
        sqlx::query!(
            r#"INSERT INTO package_team_permissions (package_id, team_id, permission)
               VALUES (?, ?, ?)
               ON DUPLICATE KEY UPDATE permission = VALUES(permission)"#,
            package_id,
            team_id,
            permission
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn list_maintainers(&self, package_id: i64) -> Result<Vec<Maintainer>> {
        let rows = sqlx::query!(
            r#"SELECT u.name, u.email FROM maintainers m JOIN users u ON u.id = m.user_id WHERE m.package_id = ?"#,
            package_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Maintainer {
                name: r.name,
                email: r.email,
            })
            .collect())
    }

    async fn list_packages_by_user_id(&self, user_id: i64) -> Result<Vec<PackageRow>> {
        let rows = sqlx::query_as!(
            PackageRow,
            r#"SELECT p.id, p.name, p.scope, p.description, p.source, p.access, p.abbreviated_dist_id, p.full_dist_id
               FROM packages p JOIN maintainers m ON m.package_id = p.id
               WHERE m.user_id = ? ORDER BY p.id"#,
            user_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_packages_by_user_id_readable(
        &self,
        target_user_id: i64,
        viewer_user_id: i64,
    ) -> Result<Vec<PackageRow>> {
        let rows = sqlx::query_as!(
            PackageRow,
            r#"SELECT DISTINCT p.id, p.name, p.scope, p.description, p.source, p.access, p.abbreviated_dist_id, p.full_dist_id
               FROM packages p JOIN maintainers m ON m.package_id = p.id
               WHERE m.user_id = ?
                 AND (p.access = 'public' OR EXISTS (
                   SELECT 1 FROM maintainers m2 WHERE m2.package_id = p.id AND m2.user_id = ?
                 ))
               ORDER BY p.id"#,
            target_user_id,
            viewer_user_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── organizations ──

    async fn create_org(&self, name: &str, description: Option<&str>) -> Result<i64> {
        let result = sqlx::query!(
            r#"INSERT INTO organizations (name, description) VALUES (?, ?)"#,
            name,
            description
        )
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_id() as i64)
    }

    async fn get_org_by_name(&self, name: &str) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as!(
            OrganizationRow,
            r#"SELECT id, name, description, created_at, updated_at FROM organizations WHERE name = ?"#,
            name
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn delete_org(&self, id: i64) -> Result<()> {
        sqlx::query!(r#"DELETE FROM organizations WHERE id = ?"#, id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── org_members ──

    async fn add_org_member(&self, org_id: i64, user_id: i64, role: &str) -> Result<()> {
        sqlx::query!(
            r#"INSERT INTO org_members (org_id, user_id, role) VALUES (?, ?, ?)
               ON DUPLICATE KEY UPDATE role = VALUES(role)"#,
            org_id,
            user_id,
            role
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_org_member_cascade(&self, org_id: i64, user_id: i64) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let target = sqlx::query!(
            r#"SELECT role FROM org_members WHERE org_id = ? AND user_id = ? FOR UPDATE"#,
            org_id,
            user_id
        )
        .fetch_optional(&mut *tx)
        .await?;
        if target.as_ref().is_some_and(|t| t.role == "owner") {
            let row = sqlx::query!(
                r#"SELECT COUNT(*) AS `count` FROM org_members
                   WHERE org_id = ? AND role = 'owner' FOR UPDATE"#,
                org_id
            )
            .fetch_one(&mut *tx)
            .await?;
            if row.count <= 1 {
                return Ok(false);
            }
        }
        sqlx::query!(
            r#"DELETE tm FROM team_members tm
               JOIN teams t ON t.id = tm.team_id
               WHERE t.org_id = ? AND tm.user_id = ?"#,
            org_id,
            user_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            r#"DELETE m FROM maintainers m
               JOIN packages p ON p.id = m.package_id
               JOIN organizations o ON o.name = p.scope
               WHERE o.id = ? AND m.user_id = ? AND m.source = 'team'"#,
            org_id,
            user_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            r#"DELETE FROM org_members WHERE org_id = ? AND user_id = ?"#,
            org_id,
            user_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn set_org_member_role_guarded(
        &self,
        org_id: i64,
        user_id: i64,
        role: &str,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let current = sqlx::query!(
            r#"SELECT role FROM org_members WHERE org_id = ? AND user_id = ? FOR UPDATE"#,
            org_id,
            user_id
        )
        .fetch_optional(&mut *tx)
        .await?;
        if current.as_ref().is_some_and(|c| c.role == "owner") && role != "owner" {
            let row = sqlx::query!(
                r#"SELECT COUNT(*) AS `count` FROM org_members
                   WHERE org_id = ? AND role = 'owner' FOR UPDATE"#,
                org_id
            )
            .fetch_one(&mut *tx)
            .await?;
            if row.count <= 1 {
                return Ok(false);
            }
        }
        sqlx::query!(
            r#"INSERT INTO org_members (org_id, user_id, role) VALUES (?, ?, ?)
               ON DUPLICATE KEY UPDATE role = VALUES(role)"#,
            org_id,
            user_id,
            role
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn list_org_members(&self, org_id: i64) -> Result<Vec<OrgMemberRow>> {
        let rows = sqlx::query_as!(
            OrgMemberRow,
            r#"SELECT id, org_id, user_id, role, created_at FROM org_members WHERE org_id = ? ORDER BY user_id"#,
            org_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_org_member_roster(&self, org_id: i64) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query!(
            r#"SELECT u.name, om.role FROM org_members om JOIN users u ON u.id = om.user_id WHERE om.org_id = ?"#,
            org_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| (r.name, r.role)).collect())
    }

    async fn get_org_member(
        &self,
        org_id: i64,
        user_id: i64,
    ) -> Result<Option<OrgMemberRow>> {
        let row = sqlx::query_as!(
            OrgMemberRow,
            r#"SELECT id, org_id, user_id, role, created_at FROM org_members WHERE org_id = ? AND user_id = ?"#,
            org_id,
            user_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn count_org_owners(&self, org_id: i64) -> Result<i64> {
        let row = sqlx::query!(
            r#"SELECT COUNT(*) AS `count` FROM org_members WHERE org_id = ? AND role = 'owner'"#,
            org_id
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.count)
    }

    async fn count_org_members(&self, org_id: i64) -> Result<i64> {
        let row = sqlx::query!(
            r#"SELECT COUNT(*) AS `count` FROM org_members WHERE org_id = ?"#,
            org_id
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.count)
    }

    // ── teams ──

    async fn create_team(
        &self,
        org_id: i64,
        name: &str,
        description: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query!(
            r#"INSERT INTO teams (org_id, name, description) VALUES (?, ?, ?)"#,
            org_id,
            name,
            description
        )
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_id() as i64)
    }

    async fn get_team_by_org_name(
        &self,
        org_id: i64,
        team_name: &str,
    ) -> Result<Option<TeamRow>> {
        let row = sqlx::query_as!(
            TeamRow,
            r#"SELECT id, org_id, name, description, created_at FROM teams WHERE org_id = ? AND name = ?"#,
            org_id,
            team_name
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn delete_team(&self, team_id: i64) -> Result<()> {
        sqlx::query!(r#"DELETE FROM teams WHERE id = ?"#, team_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_teams_in_org(&self, org_id: i64) -> Result<Vec<TeamRow>> {
        let rows = sqlx::query_as!(
            TeamRow,
            r#"SELECT id, org_id, name, description, created_at FROM teams WHERE org_id = ? ORDER BY name"#,
            org_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── team_members ──

    async fn add_team_member(&self, team_id: i64, user_id: i64) -> Result<()> {
        sqlx::query!(
            r#"INSERT IGNORE INTO team_members (team_id, user_id) VALUES (?, ?)"#,
            team_id,
            user_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_team_member(&self, team_id: i64, user_id: i64) -> Result<()> {
        sqlx::query!(
            r#"DELETE FROM team_members WHERE team_id = ? AND user_id = ?"#,
            team_id,
            user_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_team_members(&self, team_id: i64) -> Result<Vec<TeamMemberRow>> {
        let rows = sqlx::query_as!(
            TeamMemberRow,
            r#"SELECT id, team_id, user_id, created_at FROM team_members WHERE team_id = ? ORDER BY user_id"#,
            team_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_team_member_names(&self, team_id: i64) -> Result<Vec<String>> {
        let rows = sqlx::query!(
            r#"SELECT u.name FROM team_members tm JOIN users u ON u.id = tm.user_id WHERE tm.team_id = ?"#,
            team_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.name).collect())
    }

    // ── package_team_permissions ──

    async fn grant_team_permission(
        &self,
        package_id: i64,
        team_id: i64,
        permission: &str,
    ) -> Result<()> {
        sqlx::query!(
            r#"INSERT INTO package_team_permissions (package_id, team_id, permission)
               VALUES (?, ?, ?)
               ON DUPLICATE KEY UPDATE permission = VALUES(permission)"#,
            package_id,
            team_id,
            permission
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn revoke_team_permission(&self, package_id: i64, team_id: i64) -> Result<()> {
        sqlx::query!(
            r#"DELETE FROM package_team_permissions WHERE package_id = ? AND team_id = ?"#,
            package_id,
            team_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_packages_for_team(
        &self,
        team_id: i64,
    ) -> Result<Vec<(PackageRow, String)>> {
        let rows = sqlx::query!(
            r#"SELECT p.id, p.name, p.scope, p.description, p.source, p.access,
                      p.abbreviated_dist_id, p.full_dist_id,
                      ptp.permission
               FROM package_team_permissions ptp
               JOIN packages p ON p.id = ptp.package_id
               WHERE ptp.team_id = ?
               ORDER BY p.id"#,
            team_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    PackageRow {
                        id: r.id,
                        name: r.name,
                        scope: r.scope,
                        description: r.description,
                        source: r.source,
                        access: r.access,
                        abbreviated_dist_id: r.abbreviated_dist_id,
                        full_dist_id: r.full_dist_id,
                    },
                    r.permission,
                )
            })
            .collect())
    }

    async fn list_org_package_viewer_permissions(
        &self,
        org_id: i64,
        viewer_user_id: i64,
    ) -> Result<HashMap<i64, bool>> {
        let rows = sqlx::query!(
            r#"SELECT p.id,
                      CASE WHEN (
                          EXISTS (SELECT 1 FROM maintainers m WHERE m.package_id = p.id AND m.user_id = ?)
                          OR EXISTS (
                              SELECT 1 FROM org_members om
                              WHERE om.org_id = ? AND om.user_id = ? AND om.role IN ('owner','admin')
                          )
                          OR EXISTS (
                              SELECT 1 FROM package_team_permissions ptp
                              JOIN team_members tm ON tm.team_id = ptp.team_id
                              WHERE ptp.package_id = p.id AND tm.user_id = ? AND ptp.permission = 'write'
                          )
                      ) THEN 1 ELSE 0 END AS `has_write`
               FROM packages p
               LEFT JOIN organizations o ON o.name = p.scope
               WHERE o.id = ?"#,
            viewer_user_id,
            org_id,
            viewer_user_id,
            viewer_user_id,
            org_id
        )
        .fetch_all(&self.pool)
        .await?;
        let mut map = HashMap::with_capacity(rows.len());
        for row in rows {
            map.insert(row.id, row.has_write != 0);
        }
        Ok(map)
    }

    // ── org/team auth helpers ──

    async fn user_has_team_access(
        &self,
        package_id: i64,
        user_id: i64,
        min_permission: &str,
    ) -> Result<bool> {
        let row = sqlx::query!(
            r#"SELECT EXISTS(
                SELECT 1 FROM package_team_permissions ptp
                JOIN team_members tm ON tm.team_id = ptp.team_id
                WHERE ptp.package_id = ?
                  AND tm.user_id = ?
                  AND (ptp.permission = 'write' OR ? = 'read')
            ) AS `exists`"#,
            package_id,
            user_id,
            min_permission
        )
        .fetch_one(&self.pool)
        .await?;
        let exists: i64 = row.exists;
        Ok(exists != 0)
    }

    async fn user_is_org_manager_for_scope(
        &self,
        scope: &str,
        user_id: i64,
    ) -> Result<bool> {
        let row = sqlx::query!(
            r#"SELECT EXISTS(
                SELECT 1 FROM org_members om
                JOIN organizations o ON o.id = om.org_id
                WHERE o.name = ? AND om.user_id = ? AND om.role IN ('owner', 'admin')
            ) AS `exists`"#,
            scope,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;
        let exists: i64 = row.exists;
        Ok(exists != 0)
    }

    async fn list_all_packages_in_org(&self, org_id: i64) -> Result<Vec<PackageRow>> {
        let rows = sqlx::query_as!(
            PackageRow,
            r#"SELECT DISTINCT p.id, p.name, p.scope, p.description, p.source, p.access,
                      p.abbreviated_dist_id, p.full_dist_id
               FROM packages p
               JOIN organizations o ON o.name = p.scope
               WHERE o.id = ?
               ORDER BY p.id"#,
            org_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn list_packages_in_org_viewable(
        &self,
        org_id: i64,
        viewer_user_id: i64,
    ) -> Result<Vec<PackageRow>> {
        let rows = sqlx::query_as!(
            PackageRow,
            r#"SELECT DISTINCT p.id, p.name, p.scope, p.description, p.source, p.access,
                      p.abbreviated_dist_id, p.full_dist_id
               FROM packages p
               LEFT JOIN organizations o ON o.name = p.scope
               WHERE o.id = ?
                 AND (p.access = 'public'
                      OR EXISTS (SELECT 1 FROM maintainers m WHERE m.package_id = p.id AND m.user_id = ?)
                      OR EXISTS (
                          SELECT 1 FROM package_team_permissions ptp
                          JOIN team_members tm ON tm.team_id = ptp.team_id
                          WHERE ptp.package_id = p.id AND tm.user_id = ?
                      )
                      OR EXISTS (
                          SELECT 1 FROM org_members om
                          JOIN organizations o2 ON o2.id = om.org_id
                          WHERE o2.id = ? AND om.user_id = ? AND om.role IN ('owner','admin')
                      ))
               ORDER BY p.id"#,
            org_id,
            viewer_user_id,
            viewer_user_id,
            org_id,
            viewer_user_id
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── sync_tasks ──

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

    // ── package_downloads ──

    async fn existing_version_ids(&self, version_ids: &[i64]) -> Result<Vec<i64>> {
        if version_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: Vec<String> = version_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT id FROM package_versions WHERE id IN ({})",
            placeholders.join(",")
        );
        let mut query = sqlx::query_scalar::<_, i64>(&sql);
        for id in version_ids {
            query = query.bind(id);
        }
        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    async fn increment_package_download(
        &self,
        package_version_id: i64,
        year: u16,
        month: u8,
        day: u8,
        count: u64,
    ) -> Result<()> {
        match day {
            1 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d01) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d01 = d01 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            2 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d02) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d02 = d02 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            3 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d03) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d03 = d03 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            4 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d04) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d04 = d04 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            5 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d05) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d05 = d05 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            6 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d06) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d06 = d06 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            7 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d07) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d07 = d07 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            8 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d08) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d08 = d08 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            9 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d09) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d09 = d09 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            10 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d10) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d10 = d10 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            11 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d11) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d11 = d11 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            12 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d12) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d12 = d12 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            13 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d13) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d13 = d13 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            14 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d14) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d14 = d14 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            15 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d15) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d15 = d15 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            16 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d16) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d16 = d16 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            17 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d17) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d17 = d17 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            18 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d18) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d18 = d18 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            19 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d19) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d19 = d19 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            20 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d20) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d20 = d20 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            21 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d21) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d21 = d21 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            22 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d22) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d22 = d22 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            23 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d23) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d23 = d23 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            24 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d24) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d24 = d24 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            25 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d25) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d25 = d25 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            26 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d26) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d26 = d26 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            27 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d27) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d27 = d27 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            28 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d28) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d28 = d28 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            29 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d29) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d29 = d29 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            30 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d30) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d30 = d30 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            31 => sqlx::query!(
                r#"INSERT INTO package_downloads (package_version_id, year, month, d31) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d31 = d31 + ?"#,
                package_version_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            _ => anyhow::bail!("invalid day: {day}"),
        };
        Ok(())
    }

    async fn query_package_downloads_by_version(
        &self,
        package_version_id: i64,
        year: u16,
    ) -> Result<Vec<PackageDownloadRow>> {
        let rows = sqlx::query_as!(
            PackageDownloadRow,
            r#"SELECT id, package_version_id, year, month,
            d01, d02, d03, d04, d05, d06, d07, d08, d09, d10,
            d11, d12, d13, d14, d15, d16, d17, d18, d19, d20,
            d21, d22, d23, d24, d25, d26, d27, d28, d29, d30, d31
            FROM package_downloads
            WHERE package_version_id = ? AND year = ?"#,
            package_version_id,
            year
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn query_package_downloads_by_package(
        &self,
        package_id: i64,
        start_year: u16,
        start_month: u8,
        end_year: u16,
        end_month: u8,
    ) -> Result<Vec<(i64, String, PackageDownloadRow)>> {
        let rows = sqlx::query!(
            r#"SELECT pd.id, pd.package_version_id, pv.version, pd.year, pd.month,
            pd.d01, pd.d02, pd.d03, pd.d04, pd.d05, pd.d06, pd.d07, pd.d08, pd.d09, pd.d10,
            pd.d11, pd.d12, pd.d13, pd.d14, pd.d15, pd.d16, pd.d17, pd.d18, pd.d19, pd.d20,
            pd.d21, pd.d22, pd.d23, pd.d24, pd.d25, pd.d26, pd.d27, pd.d28, pd.d29, pd.d30, pd.d31
            FROM package_downloads pd
            JOIN package_versions pv ON pv.id = pd.package_version_id
            WHERE pv.package_id = ?
            AND (pd.year > ? OR (pd.year = ? AND pd.month >= ?))
            AND (pd.year < ? OR (pd.year = ? AND pd.month <= ?))"#,
            package_id,
            start_year,
            start_year,
            start_month,
            end_year,
            end_year,
            end_month
        )
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::new();
        for row in rows {
            let dl_row = PackageDownloadRow {
                id: row.id,
                package_version_id: row.package_version_id,
                year: row.year,
                month: row.month,
                d01: row.d01,
                d02: row.d02,
                d03: row.d03,
                d04: row.d04,
                d05: row.d05,
                d06: row.d06,
                d07: row.d07,
                d08: row.d08,
                d09: row.d09,
                d10: row.d10,
                d11: row.d11,
                d12: row.d12,
                d13: row.d13,
                d14: row.d14,
                d15: row.d15,
                d16: row.d16,
                d17: row.d17,
                d18: row.d18,
                d19: row.d19,
                d20: row.d20,
                d21: row.d21,
                d22: row.d22,
                d23: row.d23,
                d24: row.d24,
                d25: row.d25,
                d26: row.d26,
                d27: row.d27,
                d28: row.d28,
                d29: row.d29,
                d30: row.d30,
                d31: row.d31,
            };
            result.push((dl_row.package_version_id, row.version, dl_row));
        }
        Ok(result)
    }

    // ── upstream_package_downloads ──

    async fn upsert_upstream_download(
        &self,
        package_id: i64,
        year: u16,
        month: u8,
        day: u8,
        count: u64,
    ) -> Result<()> {
        match day {
            1 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d01) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d01 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            2 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d02) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d02 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            3 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d03) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d03 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            4 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d04) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d04 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            5 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d05) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d05 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            6 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d06) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d06 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            7 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d07) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d07 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            8 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d08) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d08 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            9 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d09) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d09 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            10 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d10) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d10 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            11 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d11) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d11 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            12 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d12) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d12 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            13 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d13) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d13 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            14 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d14) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d14 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            15 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d15) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d15 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            16 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d16) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d16 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            17 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d17) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d17 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            18 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d18) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d18 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            19 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d19) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d19 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            20 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d20) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d20 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            21 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d21) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d21 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            22 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d22) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d22 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            23 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d23) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d23 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            24 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d24) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d24 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            25 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d25) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d25 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            26 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d26) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d26 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            27 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d27) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d27 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            28 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d28) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d28 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            29 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d29) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d29 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            30 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d30) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d30 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            31 => sqlx::query!(
                r#"INSERT INTO upstream_package_downloads (package_id, year, month, d31) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE d31 = ?"#,
                package_id,
                year,
                month,
                count,
                count
            )
            .execute(&self.pool)
            .await?,
            _ => anyhow::bail!("invalid day: {day}"),
        };
        Ok(())
    }

    async fn query_upstream_downloads(
        &self,
        package_id: i64,
        start_year: u16,
        start_month: u8,
        end_year: u16,
        end_month: u8,
    ) -> Result<Vec<UpstreamPackageDownloadRow>> {
        let rows = sqlx::query_as!(
            UpstreamPackageDownloadRow,
            r#"SELECT id, package_id, year, month,
            d01, d02, d03, d04, d05, d06, d07, d08, d09, d10,
            d11, d12, d13, d14, d15, d16, d17, d18, d19, d20,
            d21, d22, d23, d24, d25, d26, d27, d28, d29, d30, d31
            FROM upstream_package_downloads
            WHERE package_id = ?
            AND (year > ? OR (year = ? AND month >= ?))
            AND (year < ? OR (year = ? AND month <= ?))"#,
            package_id,
            start_year,
            start_year,
            start_month,
            end_year,
            end_year,
            end_month
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
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

async fn sync_maintainers_tx(
    tx: &mut sqlx::Transaction<'_, MySql>,
    package_id: i64,
    user_ids: &[i64],
    source: &str,
) -> Result<()> {
    if user_ids.is_empty() {
        sqlx::query!(
            r#"DELETE FROM maintainers WHERE package_id = ? AND source = ?"#,
            package_id,
            source
        )
        .execute(&mut **tx)
        .await?;
    } else {
        let placeholders: Vec<String> = user_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "DELETE FROM maintainers WHERE package_id = ? AND source = ? AND user_id NOT IN ({})",
            placeholders.join(",")
        );
        let mut query = sqlx::query(&sql).bind(package_id).bind(source);
        for id in user_ids {
            query = query.bind(id);
        }
        query.execute(&mut **tx).await?;

        for &user_id in user_ids {
            sqlx::query!(
                r#"INSERT IGNORE INTO maintainers (package_id, user_id, source) VALUES (?, ?, ?)"#,
                package_id,
                user_id,
                source
            )
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
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

    let existing_map: HashMap<String, String> =
        existing.into_iter().map(|t| (t.tag, t.version)).collect();

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
