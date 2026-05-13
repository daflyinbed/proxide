use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub log: LogConfig,
    pub worker: WorkerConfig,
    #[serde(default)]
    pub auth: AuthConfig,
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
    pub directory: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Config {
    pub endpoint: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket_name: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
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

impl From<LogLevel> for logforth::record::Level {
    fn from(val: LogLevel) -> Self {
        match val {
            LogLevel::Error => logforth::record::Level::Error,
            LogLevel::Warn => logforth::record::Level::Warn,
            LogLevel::Info => logforth::record::Level::Info,
            LogLevel::Debug => logforth::record::Level::Debug,
            LogLevel::Trace => logforth::record::Level::Trace,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerConfig {
    #[serde(default = "default_upstream_registry")]
    pub upstream_registry: String,
    #[serde(default)]
    pub upstream_auth_token: String,
    #[serde(default = "default_changes_stream_url")]
    pub changes_stream_url: String,
    #[serde(default = "default_cron_interval")]
    pub cron_interval_secs: u64,
    #[serde(default = "default_max_concurrent_syncs")]
    pub max_concurrent_syncs: usize,
    #[serde(default = "default_consumer_count")]
    pub consumer_count: usize,
    #[serde(default = "default_consumer_poll_interval_ms")]
    pub consumer_poll_interval_ms: u64,
    #[serde(default = "default_task_timeout_secs")]
    pub task_timeout_secs: u64,
    #[serde(default = "default_task_retention_days")]
    pub task_retention_days: u32,
    #[serde(default = "default_upstream_name")]
    pub upstream_name: String,
}

fn default_upstream_registry() -> String {
    "https://registry.npmjs.org".to_string()
}

fn default_changes_stream_url() -> String {
    "https://replicate.npmjs.com/_changes".to_string()
}

fn default_cron_interval() -> u64 {
    60
}

fn default_max_concurrent_syncs() -> usize {
    10
}

fn default_consumer_count() -> usize {
    5
}

fn default_consumer_poll_interval_ms() -> u64 {
    500
}

fn default_task_timeout_secs() -> u64 {
    600
}

fn default_task_retention_days() -> u32 {
    7
}

fn default_upstream_name() -> String {
    "npmjs".to_string()
}

pub fn load_config(path: &str) -> Result<Config> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthConfig {
    #[serde(default)]
    pub cas_url: String,
    #[serde(default)]
    pub allow_scopes: Vec<String>,
    #[serde(default)]
    pub allow_publish_non_scope_package: bool,
    #[serde(default)]
    pub admins: Vec<String>,
}

impl AuthConfig {
    pub fn is_cas_enabled(&self) -> bool {
        !self.cas_url.is_empty()
    }
}
