use crate::{
    config::Config, repository::Repository, repository::mysql::MysqlRepository,
    search::SearchIndex, unpacked::UnpackedStore,
};
use anyhow::Result;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::watch;
use tokio::task::JoinHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockOwner {
    Sync,
    Publish,
    Access,
}

impl std::fmt::Display for LockOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockOwner::Sync => write!(f, "sync"),
            LockOwner::Publish => write!(f, "publish"),
            LockOwner::Access => write!(f, "modified"),
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

#[derive(Clone, Debug, Default)]
pub struct TarballInflightSnapshot {
    pub ready: bool,
    pub available_bytes: u64,
    pub content_length: Option<u64>,
    pub completed: bool,
    pub error: Option<TarballInflightError>,
    pub reader_count: usize,
    pub cleanup_started: bool,
}

#[derive(Debug)]
pub struct TarballInflight {
    pub file_path: PathBuf,
    tx: watch::Sender<TarballInflightSnapshot>,
}

impl TarballInflight {
    pub fn new(file_path: PathBuf) -> Self {
        let (tx, _) = watch::channel(TarballInflightSnapshot::default());
        Self { file_path, tx }
    }

    pub fn subscribe(&self) -> watch::Receiver<TarballInflightSnapshot> {
        self.tx.subscribe()
    }

    pub fn mark_ready(&self, content_length: Option<u64>) {
        self.tx.send_modify(|s| {
            s.ready = true;
            s.content_length = content_length;
        });
    }

    pub fn advance(&self, available_bytes: u64) {
        self.tx.send_modify(|s| {
            s.available_bytes = available_bytes;
        });
    }

    pub fn finish(&self, available_bytes: u64) {
        self.tx.send_modify(|s| {
            s.available_bytes = available_bytes;
            s.completed = true;
            s.ready = true;
        });
    }

    pub fn fail(&self, error: TarballInflightError) {
        self.tx.send_modify(|s| {
            s.error = Some(error);
            s.completed = true;
        });
    }

    pub fn add_reader(&self) {
        self.tx.send_modify(|s| {
            s.reader_count += 1;
        });
    }

    pub fn remove_reader(&self) {
        self.tx.send_modify(|s| {
            if s.reader_count > 0 {
                s.reader_count -= 1;
            }
        });
    }

    pub fn try_start_cleanup(&self) -> bool {
        let mut result = false;
        self.tx.send_modify(|s| {
            if !s.cleanup_started && s.completed && s.reader_count == 0 {
                s.cleanup_started = true;
                result = true;
            }
        });
        result
    }

    pub fn snapshot(&self) -> TarballInflightSnapshot {
        self.tx.borrow().clone()
    }

    pub async fn wait_for_completion(&self) -> Result<(), TarballInflightError> {
        let mut rx = self.subscribe();
        loop {
            let snapshot = self.snapshot();
            if let Some(error) = snapshot.error {
                return Err(error);
            }
            if snapshot.completed {
                return Ok(());
            }
            if rx.changed().await.is_err() {
                return Err(TarballInflightError::Internal(
                    "tarball inflight sender dropped".to_string(),
                ));
            }
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

    pub fn get_inflight(&self, storage_key: &str) -> Option<Arc<TarballInflight>> {
        self.inner.get(storage_key).map(|v| v.clone())
    }

    pub fn remove(&self, storage_key: &str) {
        self.inner.remove(storage_key);
    }
}

#[derive(Clone, Default)]
pub struct ExtractionInflightMap {
    inner: Arc<DashMap<i64, watch::Sender<()>>>,
}

impl ExtractionInflightMap {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    pub fn get_or_insert(&self, version_id: i64) -> (watch::Receiver<()>, bool) {
        match self.inner.entry(version_id) {
            Entry::Occupied(entry) => (entry.get().subscribe(), false),
            Entry::Vacant(entry) => {
                let (tx, rx) = watch::channel(());
                entry.insert(tx);
                (rx, true)
            }
        }
    }

    pub fn remove(&self, version_id: i64) {
        self.inner.remove(&version_id);
    }

    pub fn guard(&self, version_id: i64) -> ExtractionGuard<'_> {
        ExtractionGuard {
            map: self,
            version_id,
        }
    }
}

pub struct ExtractionGuard<'a> {
    map: &'a ExtractionInflightMap,
    version_id: i64,
}

impl Drop for ExtractionGuard<'_> {
    fn drop(&mut self) {
        self.map.remove(self.version_id);
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
    pub extraction_inflight: ExtractionInflightMap,
    pub unpacked: Arc<UnpackedStore>,
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

        let unpacked = Arc::new(UnpackedStore::new(
            &config.cdn.unpacked_dir,
            config.cdn.unpacked_max_bytes,
            config.cdn.enabled,
        )?);

        Ok(Self {
            repo: Arc::new(repo),
            config,
            http,
            package_lock: PackageLock::new(),
            login_sessions: Arc::new(LoginSessionMap::new()),
            tarball_downloads: TarballInflightMap::new(),
            download_counters: Arc::new(DashMap::new()),
            search,
            extraction_inflight: ExtractionInflightMap::new(),
            unpacked,
        })
    }
}
