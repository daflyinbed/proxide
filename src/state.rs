use crate::{config::Config, repository::mysql::MysqlRepository};
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct AppState {
    pub repo: MysqlRepository,
    pub config: Config,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let repo = MysqlRepository::new(&config.database).await?;
        Ok(Self { repo, config })
    }
}
