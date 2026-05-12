use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Packument {
    #[serde(rename = "_id")]
    pub id: Option<String>,
    #[serde(rename = "_rev")]
    pub rev: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "dist-tags")]
    pub dist_tags: HashMap<String, String>,
    #[serde(default)]
    pub versions: HashMap<String, PackageVersion>,
    #[serde(default)]
    pub time: HashMap<String, String>,
    #[serde(default)]
    pub maintainers: Option<Vec<Maintainer>>,
    #[serde(default)]
    pub readme: Option<String>,
    #[serde(default)]
    pub readme_filename: Option<String>,
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<RepositoryInfo>,
    #[serde(default)]
    pub author: Option<Person>,
    #[serde(default)]
    pub bugs: Option<BugsInfo>,
    #[serde(default)]
    pub users: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackageVersion {
    #[serde(rename = "_id")]
    pub id: Option<String>,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub deprecated: Option<String>,
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
    #[serde(default)]
    pub dev_dependencies: HashMap<String, String>,
    #[serde(default)]
    pub optional_dependencies: HashMap<String, String>,
    #[serde(default)]
    pub peer_dependencies: HashMap<String, String>,
    #[serde(default)]
    pub peer_dependencies_meta: HashMap<String, PeerDepMeta>,
    #[serde(default)]
    pub bundle_dependencies: Option<Vec<String>>,
    #[serde(default)]
    pub bin: Option<serde_json::Value>,
    #[serde(default)]
    pub directories: Option<serde_json::Value>,
    pub dist: Dist,
    #[serde(default)]
    pub engines: Option<HashMap<String, String>>,
    #[serde(default)]
    pub _has_shrinkwrap: Option<bool>,
    #[serde(default)]
    pub has_install_script: Option<bool>,
    #[serde(default)]
    pub funding: Option<serde_json::Value>,
    #[serde(default)]
    pub cpu: Option<Vec<String>>,
    #[serde(default)]
    pub os: Option<Vec<String>>,
    #[serde(default)]
    pub _npm_user: Option<Person>,
    #[serde(default)]
    pub _npm_version: Option<String>,
    #[serde(default)]
    pub _node_version: Option<String>,
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub types: Option<String>,
    #[serde(default)]
    pub typings: Option<String>,
    #[serde(default)]
    pub exports: Option<serde_json::Value>,
    #[serde(default)]
    pub scripts: Option<HashMap<String, String>>,
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Dist {
    pub shasum: Option<String>,
    pub tarball: String,
    pub integrity: Option<String>,
    #[serde(default)]
    pub file_count: Option<i64>,
    #[serde(default)]
    pub unpacked_size: Option<i64>,
    #[serde(default)]
    pub npm_signature: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Maintainer {
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Person {
    pub name: Option<String>,
    pub email: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepositoryInfo {
    #[serde(rename = "type")]
    pub repo_type: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BugsInfo {
    pub url: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PeerDepMeta {
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbbreviatedPackument {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(default, rename = "dist-tags")]
    pub dist_tags: HashMap<String, String>,
    #[serde(default)]
    pub versions: HashMap<String, AbbreviatedVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbbreviatedVersion {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub dependencies: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub optional_dependencies: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub dev_dependencies: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_dependencies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub peer_dependencies: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub peer_dependencies_meta: HashMap<String, PeerDepMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directories: Option<serde_json::Value>,
    pub dist: Dist,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engines: Option<HashMap<String, String>>,
    #[serde(
        default,
        rename = "_hasShrinkwrap",
        skip_serializing_if = "Option::is_none"
    )]
    pub _has_shrinkwrap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_install_script: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libc: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept_dependencies: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_time: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChangesResult {
    pub results: Vec<ChangeEntry>,
    pub last_seq: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChangeEntry {
    pub seq: serde_json::Value,
    pub id: String,
    #[serde(default)]
    pub changes: Vec<ChangeRev>,
    #[serde(default)]
    pub deleted: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChangeRev {
    pub rev: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryInfo {
    pub db_name: String,
    pub doc_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncResponse {
    pub ok: bool,
    pub log: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FastMetaResolved {
    pub name: String,
    pub specifier: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FastMetaVersions {
    pub name: String,
    pub specifier: String,
    pub dist_tags: HashMap<String, String>,
    pub versions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FastMetaFull {
    pub name: String,
    pub dist_tags: HashMap<String, String>,
    pub versions_meta: HashMap<String, VersionMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engines: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<serde_json::Value>,
}
