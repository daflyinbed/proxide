use crate::{config::Config, repository::mysql::MysqlRepository, repository::Repository};
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::task::JoinHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockOwner {
    Sync,
    Publish,
}

impl std::fmt::Display for LockOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockOwner::Sync => write!(f, "sync"),
            LockOwner::Publish => write!(f, "publish"),
        }
    }
}

#[derive(Clone)]
pub struct PackageLock {
    inner: Arc<DashMap<String, LockOwner>>,
}

impl PackageLock {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    pub fn try_lock(&self, name: &str, owner: LockOwner) -> bool {
        use dashmap::mapref::entry::Entry;
        match self.inner.entry(name.to_string()) {
            Entry::Vacant(e) => {
                e.insert(owner);
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    pub fn unlock(&self, name: &str) {
        self.inner.remove(name);
    }

    pub fn get_owner(&self, name: &str) -> Option<LockOwner> {
        self.inner.get(name).map(|v| *v.value())
    }
}

pub struct UnlockGuard<'a> {
    lock: &'a PackageLock,
    name: String,
}

impl Drop for UnlockGuard<'_> {
    fn drop(&mut self) {
        self.lock.unlock(&self.name);
    }
}

impl UnlockGuard<'_> {
    pub fn new(lock: &PackageLock, name: String) -> UnlockGuard<'_> {
        UnlockGuard { lock, name }
    }
}

#[derive(Clone, Debug)]
pub struct LoginSession {
    pub token: Option<String>,
    pub user_id: Option<i64>,
    pub expired_at: chrono::NaiveDateTime,
}

pub struct LoginSessionMap {
    inner: DashMap<String, LoginSession>,
    cleanup_handle: JoinHandle<()>,
}

impl LoginSessionMap {
    pub fn new() -> Self {
        let inner: DashMap<String, LoginSession> = DashMap::new();
        let cleanup_inner = inner.clone();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let now = chrono::Utc::now().naive_utc();
                cleanup_inner.retain(|_, v| v.expired_at > now);
            }
        });
        Self {
            inner,
            cleanup_handle: handle,
        }
    }

    pub fn insert(&self, session_id: String, session: LoginSession) {
        self.inner.insert(session_id, session);
    }

    pub fn get(&self, session_id: &str) -> Option<LoginSession> {
        self.inner.get(session_id).map(|v| v.value().clone())
    }

    pub fn remove(&self, session_id: &str) -> Option<LoginSession> {
        self.inner.remove(session_id).map(|(_, v)| v)
    }
}

impl Drop for LoginSessionMap {
    fn drop(&mut self) {
        self.cleanup_handle.abort();
    }
}

#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<dyn Repository>,
    pub config: Config,
    pub http: reqwest::Client,
    pub package_lock: PackageLock,
    pub login_sessions: Arc<LoginSessionMap>,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let repo = MysqlRepository::new(&config.database, &config.storage).await?;
        let http = reqwest::Client::new();
        Ok(Self {
            repo: Arc::new(repo),
            config,
            http,
            package_lock: PackageLock::new(),
            login_sessions: Arc::new(LoginSessionMap::new()),
        })
    }
}
