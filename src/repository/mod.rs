pub mod mysql;
use anyhow::Result;

pub struct ListDirOptions {
    pub limit: Option<u32>,
    pub since: Option<String>,
}

pub trait Repository {
    async fn health_check(&self) -> bool;

    async fn migrate(&self) -> Result<()>;
}
