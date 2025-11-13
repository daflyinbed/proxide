pub mod mysql;
pub trait Repository {
    async fn health_check(&self) -> bool;

    async fn migrate(&self) -> anyhow::Result<()>;
}
