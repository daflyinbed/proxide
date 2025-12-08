pub mod mysql;
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct Binary {
    pub id: u64,
    pub category: String,
    pub parent: String,
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub date: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

pub struct ListDirOptions {
    pub limit: Option<u32>,
    pub since: Option<String>,
}

pub trait Repository {
    async fn health_check(&self) -> bool;

    async fn migrate(&self) -> Result<()>;

    async fn list_binaries(&self, category: &str, parent: &str) -> Result<Vec<Binary>>;

    async fn find_binary(&self, category: &str, parent: &str, name: &str)
    -> Result<Option<Binary>>;
}
