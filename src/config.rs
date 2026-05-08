use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub log: LogConfig,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseConfig {
    pub uri: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    #[serde(default = "default_binding")]
    pub binding: String,
    pub port: u16,
    pub root_url: String,
}

impl ServerConfig {
    pub fn full_url(&self) -> String {
        format!("{}:{}", self.binding, self.port)
    }
}

fn default_binding() -> String {
    "localhost".to_string()
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum StorageConfig {
    Local(LocalConfig),
    S3(S3Config),
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalConfig {
    pub max_size: u64,
    pub directory: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Config {
    pub max_size: u64,
    pub endpoint: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket_name: String,
    pub with_virtual_hosted_style_request: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogConfig {
    pub level: LogLevel,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}
impl Into<logforth::record::Level> for LogLevel {
    fn into(self) -> logforth::record::Level {
        match self {
            LogLevel::Error => logforth::record::Level::Error,
            LogLevel::Warn => logforth::record::Level::Warn,
            LogLevel::Info => logforth::record::Level::Info,
            LogLevel::Debug => logforth::record::Level::Debug,
            LogLevel::Trace => logforth::record::Level::Trace,
        }
    }
}

pub fn load_config(path: &str) -> Result<Config> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}
