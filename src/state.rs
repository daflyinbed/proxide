use crate::{config::Config, repository::Repository, repository::mysql::MysqlRepository, search::SearchIndex};
use anyhow::Result;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::Notify;
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

impl Default for PackageLock {
    fn default() -> Self {
        Self::new()
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
    inner: Arc<DashMap<String, LoginSession>>,
    cleanup_handle: JoinHandle<()>,
}

impl LoginSessionMap {
    pub fn new() -> Self {
        let inner = Arc::new(DashMap::new());
        let cleanup_inner = inner.clone();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let now = chrono::Utc::now().naive_utc();
                cleanup_inner.retain(|_, v: &mut LoginSession| v.expired_at > now);
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

impl Default for LoginSessionMap {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LoginSessionMap {
    fn drop(&mut self) {
        self.cleanup_handle.abort();
    }
}

#[derive(Clone, Debug)]
pub enum TarballInflightError {
    NotFound(String),
    Internal(String),
}

#[derive(Clone, Debug)]
pub struct TarballInflightSnapshot {
    pub ready: bool,
    pub bytes_written: u64,
    pub content_length: Option<u64>,
    pub completed: bool,
    pub error: Option<TarballInflightError>,
}

#[derive(Debug, Default)]
struct TarballInflightProgress {
    ready: bool,
    bytes_written: u64,
    content_length: Option<u64>,
    completed: bool,
    reader_count: usize,
    cleanup_started: bool,
    error: Option<TarballInflightError>,
}

#[derive(Debug)]
pub struct TarballInflight {
    pub file_path: PathBuf,
    progress: Mutex<TarballInflightProgress>,
    pub notify: Arc<Notify>,
}

impl TarballInflight {
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            file_path,
            progress: Mutex::new(TarballInflightProgress::default()),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn mark_ready(&self, content_length: Option<u64>) {
        let mut progress = self.progress.lock();
        progress.ready = true;
        progress.content_length = content_length;
        drop(progress);
        self.notify.notify_waiters();
    }

    pub fn advance(&self, bytes_written: u64) {
        let mut progress = self.progress.lock();
        progress.bytes_written = bytes_written;
        drop(progress);
        self.notify.notify_waiters();
    }

    pub fn finish(&self) {
        let mut progress = self.progress.lock();
        progress.completed = true;
        progress.ready = true;
        drop(progress);
        self.notify.notify_waiters();
    }

    pub fn fail(&self, error: TarballInflightError) {
        let mut progress = self.progress.lock();
        progress.error = Some(error);
        progress.completed = true;
        drop(progress);
        self.notify.notify_waiters();
    }

    pub fn add_reader(&self) {
        let mut progress = self.progress.lock();
        progress.reader_count += 1;
    }

    pub fn remove_reader(&self) {
        let mut progress = self.progress.lock();
        if progress.reader_count > 0 {
            progress.reader_count -= 1;
        }
        drop(progress);
        self.notify.notify_waiters();
    }

    pub fn try_start_cleanup(&self) -> bool {
        let mut progress = self.progress.lock();
        if progress.cleanup_started || !progress.completed || progress.reader_count > 0 {
            return false;
        }
        progress.cleanup_started = true;
        true
    }

    pub fn snapshot(&self) -> TarballInflightSnapshot {
        let progress = self.progress.lock();
        TarballInflightSnapshot {
            ready: progress.ready,
            bytes_written: progress.bytes_written,
            content_length: progress.content_length,
            completed: progress.completed,
            error: progress.error.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub struct TarballInflightMap {
    inner: Arc<DashMap<String, Arc<TarballInflight>>>,
}

impl TarballInflightMap {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    pub fn get_or_insert(
        &self,
        storage_key: &str,
        file_path: PathBuf,
    ) -> (Arc<TarballInflight>, bool) {
        match self.inner.entry(storage_key.to_string()) {
            Entry::Occupied(entry) => (entry.get().clone(), false),
            Entry::Vacant(entry) => {
                let inflight = Arc::new(TarballInflight::new(file_path));
                entry.insert(inflight.clone());
                (inflight, true)
            }
        }
    }

    pub fn remove(&self, storage_key: &str) {
        self.inner.remove(storage_key);
    }
}

#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<dyn Repository>,
    pub config: Config,
    pub http: reqwest::Client,
    pub package_lock: PackageLock,
    pub login_sessions: Arc<LoginSessionMap>,
    pub tarball_downloads: TarballInflightMap,
    pub download_counters: Arc<DashMap<i64, AtomicU64>>,
    pub search: Option<Arc<SearchIndex>>,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let repo = MysqlRepository::new(&config.database, &config.storage).await?;
        tokio::fs::create_dir_all(&config.server.tarball_cache_dir).await?;
        let http = reqwest::Client::new();

        let search = SearchIndex::new(&config.search).await?.map(Arc::new);
        if let Some(ref idx) = search
            && let Err(e) = idx.ensure_index().await
        {
            log::warn!(action = "search_init"; "failed to ensure meilisearch index: {e:#}");
        }

        Ok(Self {
            repo: Arc::new(repo),
            config,
            http,
            package_lock: PackageLock::new(),
            login_sessions: Arc::new(LoginSessionMap::new()),
            tarball_downloads: TarballInflightMap::new(),
            download_counters: Arc::new(DashMap::new()),
            search,
        })
    }
}
