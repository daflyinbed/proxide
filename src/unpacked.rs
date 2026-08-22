use crate::state::AppState;
use anyhow::{Context, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const SHARD_COUNT: u64 = 256;
const MANIFEST_SUFFIX: &str = ".meta.json";
const STAGING_PREFIX: &str = ".staging-";
const MIN_ACCESS_AGE: Duration = Duration::from_secs(5);
const ZOMBIE_QUERY_CHUNK: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    pub hash: String,
    pub content_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionManifest {
    pub total_size: u64,
    pub files: Vec<ManifestFile>,
}

impl VersionManifest {
    pub fn find(&self, path: &str) -> Option<&ManifestFile> {
        self.files
            .binary_search_by(|f| f.path.as_str().cmp(path))
            .ok()
            .map(|i| &self.files[i])
    }
}

pub fn validate_filepath(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches('/');
    if trimmed.is_empty()
        || trimmed.len() > 4096
        || trimmed.contains('\0')
        || trimmed.contains('\\')
    {
        return None;
    }
    if Path::new(trimmed)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    let mut segments = Vec::new();
    for seg in trimmed.split('/') {
        let bytes = seg.as_bytes();
        if seg.is_empty()
            || seg == "."
            || seg == ".."
            || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        {
            return None;
        }
        segments.push(seg);
    }
    Some(segments.join("/"))
}

pub(crate) struct VersionEntry {
    pub manifest: Arc<VersionManifest>,
    pub disk_size: u64,
    pub last_access: SystemTime,
}

#[derive(Clone)]
pub struct UnpackedStore {
    dir: PathBuf,
    max_bytes: u64,
    index: Arc<DashMap<i64, VersionEntry>>,
}

impl UnpackedStore {
    pub fn new(dir: &str, max_bytes: u64, create: bool) -> Result<Self> {
        if create {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder
                .recursive(true)
                .create(dir)
                .with_context(|| format!("failed to create unpacked dir: {dir}"))?;
        }
        Ok(Self {
            dir: dir.into(),
            max_bytes,
            index: Arc::new(DashMap::new()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.dir
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    pub fn shard_dir(&self, version_id: i64) -> PathBuf {
        let shard = (version_id.unsigned_abs() % SHARD_COUNT) as u16;
        self.dir.join(format!("{shard:02x}"))
    }

    pub fn version_dir(&self, version_id: i64) -> PathBuf {
        self.shard_dir(version_id).join(format!("v-{version_id}"))
    }

    pub fn manifest_path(&self, version_id: i64) -> PathBuf {
        self.shard_dir(version_id)
            .join(format!("v-{version_id}{MANIFEST_SUFFIX}"))
    }

    pub fn staging_dir(&self, version_id: i64, token: &str) -> PathBuf {
        self.shard_dir(version_id)
            .join(format!("{STAGING_PREFIX}v-{version_id}-{token}"))
    }

    pub fn file_path(&self, version_id: i64, filepath: &str) -> PathBuf {
        self.version_dir(version_id).join(filepath)
    }

    pub fn contains(&self, version_id: i64) -> bool {
        self.index.contains_key(&version_id)
    }

    pub fn get(&self, version_id: i64) -> Option<Arc<VersionManifest>> {
        self.index.get_mut(&version_id).map(|mut e| {
            e.last_access = SystemTime::now();
            e.manifest.clone()
        })
    }

    pub fn insert(&self, version_id: i64, manifest: Arc<VersionManifest>, disk_size: u64) {
        self.index.insert(
            version_id,
            VersionEntry {
                manifest,
                disk_size,
                last_access: SystemTime::now(),
            },
        );
    }

    fn insert_with_access(
        &self,
        version_id: i64,
        manifest: Arc<VersionManifest>,
        disk_size: u64,
        last_access: SystemTime,
    ) {
        self.index.insert(
            version_id,
            VersionEntry {
                manifest,
                disk_size,
                last_access,
            },
        );
    }

    pub fn remove(&self, version_id: i64) {
        self.index.remove(&version_id);
    }

    fn snapshot_entries(&self) -> Vec<(i64, u64, SystemTime)> {
        self.index
            .iter()
            .map(|e| (*e.key(), e.value().disk_size, e.value().last_access))
            .collect()
    }

    pub async fn remove_version(&self, version_id: i64) {
        self.index.remove(&version_id);
        let dir = self.version_dir(version_id);
        let manifest = self.manifest_path(version_id);
        if let Err(e) = tokio::task::spawn_blocking(move || {
            if let Err(e) = std::fs::remove_dir_all(&dir)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                log::error!(
                    action = "unpacked_remove";
                    "failed to remove unpacked dir {}; disk usage may be undercounted until restart: {e}",
                    dir.display()
                );
            }
            if let Err(e) = std::fs::remove_file(&manifest)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                log::error!(
                    action = "unpacked_remove";
                    "failed to remove unpacked manifest {}; disk usage may be undercounted until restart: {e}",
                    manifest.display()
                );
            }
        })
        .await
        {
            log::error!(
                action = "unpacked_remove";
                "unpacked removal task failed; disk usage may be undercounted until restart: {e}"
            );
        }
    }
}

fn remove_path(path: &Path) {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    if let Err(e) = result
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!(
            action = "unpacked_scan";
            "failed to remove garbage {}: {e}",
            path.display()
        );
    }
}

pub(crate) fn version_disk_usage(version_dir: &Path, manifest_path: &Path) -> u64 {
    path_disk_usage(version_dir).saturating_add(path_disk_usage(manifest_path))
}

fn path_disk_usage(path: &Path) -> u64 {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    let mut total = allocated_bytes(&meta);
    if meta.is_dir()
        && let Ok(entries) = std::fs::read_dir(path)
    {
        for entry in entries.flatten() {
            total = total.saturating_add(path_disk_usage(&entry.path()));
        }
    }
    total
}

#[cfg(unix)]
fn allocated_bytes(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn allocated_bytes(meta: &std::fs::Metadata) -> u64 {
    meta.len()
}

fn is_shard_name(name: &str) -> bool {
    name.len() == 2
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn parse_version_dir_name(name: &str) -> Option<i64> {
    parse_version_id(name.strip_prefix("v-")?)
}

fn parse_manifest_name(name: &str) -> Option<i64> {
    parse_version_id(name.strip_suffix(MANIFEST_SUFFIX)?.strip_prefix("v-")?)
}

fn parse_tmp_manifest_name(name: &str) -> Option<i64> {
    parse_manifest_name(name.strip_suffix(".tmp")?)
}

fn parse_staging_name(name: &str) -> Option<i64> {
    let rest = name.strip_prefix(STAGING_PREFIX)?.strip_prefix("v-")?;
    let (id, token) = rest.split_once('-')?;
    if token.len() != 32
        || !token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    parse_version_id(id)
}

fn parse_version_id(raw: &str) -> Option<i64> {
    raw.parse::<i64>().ok().filter(|id| *id > 0)
}

fn belongs_to_shard(version_id: i64, shard: u64) -> bool {
    version_id.unsigned_abs() % SHARD_COUNT == shard
}

struct ScanFound {
    version_id: i64,
    manifest: Arc<VersionManifest>,
    disk_size: u64,
    mtime: SystemTime,
}

fn scan_shard(shard: &Path, shard_id: u64) -> Result<Vec<ScanFound>> {
    let mut version_dirs: HashMap<i64, PathBuf> = HashMap::new();
    let mut manifests: HashMap<i64, PathBuf> = HashMap::new();
    let mut garbage: Vec<PathBuf> = Vec::new();

    for entry in
        std::fs::read_dir(shard).with_context(|| format!("failed to read {}", shard.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        let file_type = entry.file_type().ok();
        if let Some(id) = parse_staging_name(&name)
            && belongs_to_shard(id, shard_id)
            && file_type.is_some_and(|t| t.is_dir())
        {
            garbage.push(path);
        } else if let Some(id) = parse_tmp_manifest_name(&name)
            && belongs_to_shard(id, shard_id)
            && file_type.is_some_and(|t| t.is_file())
        {
            garbage.push(path);
        } else if let Some(id) = parse_version_dir_name(&name)
            && belongs_to_shard(id, shard_id)
            && file_type.is_some_and(|t| t.is_dir())
        {
            version_dirs.insert(id, path);
        } else if let Some(id) = parse_manifest_name(&name)
            && belongs_to_shard(id, shard_id)
            && file_type.is_some_and(|t| t.is_file())
        {
            manifests.insert(id, path);
        }
    }

    for path in garbage {
        remove_path(&path);
    }

    let mut found = Vec::new();
    let mut ids: HashSet<i64> = version_dirs.keys().copied().collect();
    ids.extend(manifests.keys().copied());

    for id in ids {
        let dir = version_dirs.get(&id);
        let manifest = manifests.get(&id);
        let mut keep = None;
        if let (Some(dir), Some(manifest_path)) = (dir, manifest)
            && let Ok(bytes) = std::fs::read(manifest_path)
            && let Ok(parsed) = serde_json::from_slice::<VersionManifest>(&bytes)
            && !parsed.files.is_empty()
            && let Ok(meta) = std::fs::metadata(manifest_path)
            && let Ok(mtime) = meta.modified()
        {
            keep = Some(ScanFound {
                version_id: id,
                manifest: Arc::new(parsed),
                disk_size: version_disk_usage(dir, manifest_path),
                mtime,
            });
        }
        match keep {
            Some(f) => found.push(f),
            None => {
                if let Some(dir) = dir {
                    remove_path(dir);
                }
                if let Some(manifest) = manifest {
                    remove_path(manifest);
                }
            }
        }
    }

    Ok(found)
}

fn scan_disk(root: &Path) -> Result<Vec<ScanFound>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in
        std::fs::read_dir(root).with_context(|| format!("failed to read {}", root.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy().into_owned();
        let path = entry.path();
        if !is_shard_name(&name) || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let shard_id = u64::from_str_radix(&name, 16).expect("validated hexadecimal shard name");
        found.extend(scan_shard(&path, shard_id)?);
    }
    Ok(found)
}

pub async fn startup_scan(state: &AppState) -> Result<usize> {
    let root = state.unpacked.root().to_path_buf();
    let found = tokio::task::spawn_blocking(move || scan_disk(&root))
        .await
        .context("unpacked scan task join failed")??;

    let ids: Vec<i64> = found.iter().map(|f| f.version_id).collect();
    let mut existing: HashSet<i64> = HashSet::new();
    for chunk in ids.chunks(ZOMBIE_QUERY_CHUNK) {
        existing.extend(state.repo.existing_version_ids(chunk).await?);
    }

    let mut kept = 0;
    for f in found {
        if existing.contains(&f.version_id) {
            state
                .unpacked
                .insert_with_access(f.version_id, f.manifest, f.disk_size, f.mtime);
            kept += 1;
        } else {
            state.unpacked.remove_version(f.version_id).await;
        }
    }
    log::info!(action = "unpacked_scan"; "rebuilt unpacked index: {kept} version(s) kept");
    Ok(kept)
}

fn select_eviction_candidates(
    entries: Vec<(i64, u64, SystemTime)>,
    total: u64,
    max_bytes: u64,
    low_bytes: u64,
    now: SystemTime,
) -> Vec<i64> {
    if total <= max_bytes {
        return Vec::new();
    }
    let mut sorted = entries;
    sorted.sort_by_key(|(_, _, last_access)| *last_access);
    let mut freed: u64 = 0;
    let mut candidates = Vec::new();
    for (id, size, last_access) in sorted {
        if total.saturating_sub(freed) <= low_bytes {
            break;
        }
        let age = now.duration_since(last_access).unwrap_or(Duration::ZERO);
        if age < MIN_ACCESS_AGE {
            continue;
        }
        freed = freed.saturating_add(size);
        candidates.push(id);
    }
    candidates
}

async fn evict_once(state: &AppState) -> Result<(usize, u64)> {
    let store = &state.unpacked;
    let max_bytes = store.max_bytes();
    if max_bytes == 0 {
        return Ok((0, 0));
    }

    let entries = store.snapshot_entries();
    let total: u64 = entries.iter().map(|(_, size, _)| size).sum();
    let low_bytes = state.config.cdn.unpacked_low_watermark_bytes();
    let candidates =
        select_eviction_candidates(entries, total, max_bytes, low_bytes, SystemTime::now());
    if candidates.is_empty() {
        return Ok((0, 0));
    }

    let mut evicted = 0;
    for id in candidates {
        let (_rx, is_leader) = state.extraction_inflight.get_or_insert(id);
        if !is_leader {
            continue;
        }
        let _guard = state.extraction_inflight.guard(id);
        if !store.contains(id) {
            continue;
        }
        store.remove_version(id).await;
        evicted += 1;
    }

    if evicted > 0 {
        log::info!(action = "unpacked_evict"; "evicted {evicted} version(s), total was {total} bytes (max {max_bytes})");
    }
    Ok((evicted, total))
}

pub async fn run_eviction(state: AppState) {
    let interval_secs = state.config.cdn.unpacked_eviction_interval_secs.max(1);
    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    loop {
        interval.tick().await;
        if let Err(e) = evict_once(&state).await {
            log::warn!(action = "unpacked_evict"; "unpacked eviction failed: {e:#}");
        }
    }
}

pub(crate) fn manifest_from_entries(files: BTreeMap<String, ManifestFile>) -> VersionManifest {
    let total_size = files.values().map(|f| f.size).sum();
    VersionManifest {
        total_size,
        files: files.into_values().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_manifest(dir: &Path, id: i64, paths: &[&str]) {
        let shard = dir.join(format!("{:02x}", id % SHARD_COUNT as i64));
        fs::create_dir_all(shard.join(format!("v-{id}"))).unwrap();
        for p in paths {
            let f = shard.join(format!("v-{id}")).join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, b"x").unwrap();
        }
        let manifest = VersionManifest {
            total_size: paths.len() as u64,
            files: paths
                .iter()
                .map(|p| ManifestFile {
                    path: p.to_string(),
                    size: 1,
                    hash: String::new(),
                    content_type: "text/plain".to_string(),
                })
                .collect(),
        };
        fs::write(
            shard.join(format!("v-{id}{MANIFEST_SUFFIX}")),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn validate_filepath_accepts_normal_paths() {
        assert_eq!(validate_filepath("index.js").as_deref(), Some("index.js"));
        assert_eq!(
            validate_filepath("dist/lodash.js").as_deref(),
            Some("dist/lodash.js")
        );
        assert_eq!(
            validate_filepath("/leading/slash").as_deref(),
            Some("leading/slash")
        );
        assert_eq!(
            validate_filepath("foo@bar.js").as_deref(),
            Some("foo@bar.js")
        );
    }

    #[test]
    fn validate_filepath_rejects_traversal_and_junk() {
        assert_eq!(validate_filepath(""), None);
        assert_eq!(validate_filepath("/"), None);
        assert_eq!(validate_filepath(".."), None);
        assert_eq!(validate_filepath("../etc/passwd"), None);
        assert_eq!(validate_filepath("a/../b"), None);
        assert_eq!(validate_filepath("a//b"), None);
        assert_eq!(validate_filepath("a/./b"), None);
        assert_eq!(validate_filepath("a/\0b"), None);
        assert_eq!(validate_filepath(r"C:\Windows\system.ini"), None);
        assert_eq!(validate_filepath("C:/Windows/system.ini"), None);
        assert_eq!(validate_filepath("C:Windows/system.ini"), None);
        assert_eq!(validate_filepath(r"\\server\share\file.txt"), None);
        assert_eq!(validate_filepath(&"a".repeat(4097)), None);
    }

    #[test]
    fn manifest_find_uses_sorted_files() {
        let manifest = VersionManifest {
            total_size: 2,
            files: vec![
                ManifestFile {
                    path: "a.js".to_string(),
                    size: 1,
                    hash: "h1".to_string(),
                    content_type: "text/javascript".to_string(),
                },
                ManifestFile {
                    path: "b/c.js".to_string(),
                    size: 1,
                    hash: "h2".to_string(),
                    content_type: "text/javascript".to_string(),
                },
            ],
        };
        assert_eq!(manifest.find("b/c.js").map(|f| f.hash.as_str()), Some("h2"));
        assert!(manifest.find("missing").is_none());
    }

    #[test]
    fn manifest_serde_roundtrip_camel_case() {
        let manifest = manifest_from_entries(BTreeMap::from([(
            "lib/x.js".to_string(),
            ManifestFile {
                path: "lib/x.js".to_string(),
                size: 7,
                hash: "abc".to_string(),
                content_type: "text/javascript".to_string(),
            },
        )]));
        let json = serde_json::to_value(&manifest).unwrap();
        assert_eq!(json["totalSize"], 7);
        assert_eq!(json["files"][0]["contentType"], "text/javascript");
        let parsed: VersionManifest = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.total_size, 7);
    }

    #[test]
    fn scan_disk_cleans_owned_garbage_and_preserves_unknown_paths() {
        let base = std::env::temp_dir().join(format!("proxide-unpacked-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        write_manifest(&base, 7, &["index.js"]);
        write_manifest(&base, 9, &["a.js"]);

        let shard9 = base.join(format!("{:02x}", 9 % SHARD_COUNT as i64));
        fs::remove_file(shard9.join(format!("v-9{MANIFEST_SUFFIX}"))).unwrap();
        fs::create_dir_all(shard9.join(".staging-v-11-abc")).unwrap();
        fs::create_dir_all(shard9.join(".staging-v-9-0123456789abcdef0123456789abcdef")).unwrap();
        fs::write(shard9.join("v-9.meta.json.tmp"), b"partial").unwrap();
        fs::write(shard9.join("random-junk"), b"x").unwrap();
        fs::write(base.join("not-a-shard"), b"x").unwrap();
        fs::write(base.join("00"), b"not a shard directory").unwrap();

        let manifest20 = base
            .join(format!("{:02x}", 20 % SHARD_COUNT as i64))
            .join(format!("v-20{MANIFEST_SUFFIX}"));
        fs::create_dir_all(manifest20.parent().unwrap()).unwrap();
        fs::write(&manifest20, b"corrupt").unwrap();

        let found = scan_disk(&base).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].version_id, 7);

        assert!(!shard9.join("v-9").exists());
        assert!(shard9.join(".staging-v-11-abc").exists());
        assert!(
            !shard9
                .join(".staging-v-9-0123456789abcdef0123456789abcdef")
                .exists()
        );
        assert!(!shard9.join("v-9.meta.json.tmp").exists());
        assert!(shard9.join("random-junk").exists());
        assert!(base.join("not-a-shard").exists());
        assert!(base.join("00").exists());
        assert!(!manifest20.exists());
        assert!(!manifest20.parent().unwrap().join("v-20").exists());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn eviction_selects_oldest_until_low_watermark() {
        let now = SystemTime::now();
        let old = now - Duration::from_secs(600);
        let mid = now - Duration::from_secs(300);
        let fresh = now - Duration::from_secs(1);

        let entries = vec![(1, 100, old), (2, 100, mid), (3, 100, old), (4, 50, fresh)];
        let candidates = select_eviction_candidates(entries, 350, 200, 100, now);
        assert_eq!(candidates, vec![1, 3, 2]);

        let entries = vec![(1, 100, old)];
        assert!(select_eviction_candidates(entries, 100, 200, 180, now).is_empty());

        let entries = vec![(1, 100, fresh)];
        assert!(select_eviction_candidates(entries, 100, 50, 40, now).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn disk_usage_counts_block_allocation_not_payload() {
        let base = std::env::temp_dir().join(format!("proxide-unpacked-du-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let dir = base.join("v-1");
        for i in 0..64 {
            let p = dir.join(format!("f{i}.txt"));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"x").unwrap();
        }
        let usage = path_disk_usage(&dir);
        assert!(
            usage > 64,
            "expected allocated blocks ({} bytes) to exceed payload (64 bytes)",
            usage
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn scan_reports_disk_usage_above_payload_sum() {
        let base =
            std::env::temp_dir().join(format!("proxide-unpacked-scan-du-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let names: Vec<String> = (0..32).map(|i| format!("f{i}.txt")).collect();
        let paths: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        write_manifest(&base, 7, &paths);

        let found = scan_disk(&base).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].version_id, 7);
        assert_eq!(found[0].manifest.total_size, 32);
        assert!(
            found[0].disk_size > found[0].manifest.total_size,
            "disk usage ({}) should exceed payload sum ({})",
            found[0].disk_size,
            found[0].manifest.total_size
        );

        let _ = fs::remove_dir_all(&base);
    }
}
