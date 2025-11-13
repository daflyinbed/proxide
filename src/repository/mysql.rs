use crate::{config::DatabaseConfig, repository::Repository};
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

    async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }
}
