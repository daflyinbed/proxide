use crate::{config::Config, repository::mysql::MysqlRepository, repository::Repository};
use anyhow::Result;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<dyn Repository>,
    pub config: Config,
    pub http: reqwest::Client,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let repo = MysqlRepository::new(&config.database, &config.storage).await?;
        let http = reqwest::Client::new();
        Ok(Self {
            repo: Arc::new(repo),
            config,
            http,
        })
    }
}
