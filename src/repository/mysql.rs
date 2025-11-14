use crate::{
    config::DatabaseConfig,
    repository::{Binary, Repository},
};
use anyhow::Result;
use sqlx::{MySql, Pool};

#[derive(Debug, Clone)]
pub struct MysqlRepository {
    pool: Pool<MySql>,
}

impl MysqlRepository {
    pub async fn new(config: &DatabaseConfig) -> Result<Self> {
        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .connect(&config.uri)
            .await
            .expect("Failed to connect to the database");
        Ok(Self { pool })
    }
}

impl Repository for MysqlRepository {
    async fn health_check(&self) -> bool {
        sqlx::query("SELECT 1").execute(&self.pool).await.is_ok()
    }

    async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    async fn list_binaries(&self, category: &str, parent: &str) -> Result<Vec<Binary>> {
        let rows = sqlx::query_as!(
            Binary,
            r#"
            SELECT id, category, parent, name, is_dir as "is_dir: _", size, date, gmt_modified as updated_at
            FROM binaries
            WHERE category = ? AND parent = ?
            ORDER BY name
            "#,
            category, parent
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn find_binary(
        &self,
        binary_name: &str,
        parent: &str,
        name: &str,
    ) -> Result<Option<Binary>> {
        let row = sqlx::query_as!(
            Binary,
            r#"
            SELECT id, category, parent, name, is_dir as "is_dir: _", size, date, gmt_modified as updated_at
            FROM binaries
            WHERE category = ? AND parent = ? AND name = ?
            "#,
            binary_name, parent, name
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}
