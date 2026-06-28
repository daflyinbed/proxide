use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;

// ═══════════════════════════════════════════════════════════════════════════
// Polymorphic npm field types
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct Person {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum License {
    Spdx(String),
    Object {
        #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
        typ: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum Author {
    Person(Person),
    Name(String),
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum Repository {
    Object {
        #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
        typ: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        directory: Option<String>,
    },
    Shorthand(String),
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum Bugs {
    Object {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        email: Option<String>,
    },
    Url(String),
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct FundingEntry {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub typ: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum Funding {
    Url(String),
    Single(FundingEntry),
    Multiple(Vec<FundingEntry>),
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum Bin {
    Map(BTreeMap<String, String>),
    Path(String),
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum Man {
    List(Vec<String>),
    Single(String),
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum StringOrList {
    List(Vec<String>),
    Single(String),
}

impl StringOrList {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            StringOrList::List(v) => v.clone(),
            StringOrList::Single(s) => vec![s.clone()],
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum Workspaces {
    Globs(Vec<String>),
    Config {
        #[serde(default)]
        packages: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nohoist: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum SideEffects {
    Flag(bool),
    Globs(Vec<String>),
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum BrowserField {
    Map(BTreeMap<String, BrowserOverride>),
    Path(String),
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum BrowserOverride {
    Path(String),
    Disabled(bool),
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum ExportsTarget {
    Path(String),
    Conditions(BTreeMap<String, ExportsTarget>),
    Alternatives(Vec<ExportsTarget>),
    Null,
}

pub type Exports = BTreeMap<String, ExportsTarget>;

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct Directories {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lib: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub man: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct PublishConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Packument (full packument document)
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Packument {
    #[serde(rename = "_id", default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "_rev", default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, rename = "dist-tags")]
    pub dist_tags: HashMap<String, String>,
    #[serde(default)]
    pub versions: HashMap<String, PackageVersion>,
    #[serde(default)]
    pub time: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintainers: Option<Vec<Maintainer>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<StringOrList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<Repository>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bugs: Option<Bugs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributors: Option<Vec<Author>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub users: Option<HashMap<String, bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readme: Option<String>,
    #[serde(
        default,
        rename = "readmeFilename",
        skip_serializing_if = "Option::is_none"
    )]
    pub readme_filename: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════════
// PackageVersion (per-version manifest inside a packument)
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackageVersion {
    #[serde(rename = "_id", default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<StringOrList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<Repository>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bugs: Option<Bugs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributors: Option<Vec<Author>>,
    #[serde(
        default,
        rename = "readmeFilename",
        skip_serializing_if = "Option::is_none"
    )]
    pub readme_filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, alias = "bundledDependencies", skip_serializing_if = "Option::is_none")]
    pub bundle_dependencies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<Bin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directories: Option<Directories>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub man: Option<Man>,
    pub dist: Dist,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engines: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_install_script: Option<bool>,
    #[serde(rename = "_hasShrinkwrap", default, skip_serializing_if = "Option::is_none")]
    pub _has_shrinkwrap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding: Option<Funding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libc: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<Workspaces>,
    #[serde(default, rename = "acceptDependencies", skip_serializing_if = "HashMap::is_empty")]
    pub accept_dependencies: HashMap<String, String>,
    #[serde(rename = "_npmUser", default, skip_serializing_if = "Option::is_none")]
    pub _npm_user: Option<Person>,
    #[serde(rename = "_npmVersion", default, skip_serializing_if = "Option::is_none")]
    pub _npm_version: Option<String>,
    #[serde(rename = "_nodeVersion", default, skip_serializing_if = "Option::is_none")]
    pub _node_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typings: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub module_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exports: Option<Exports>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imports: Option<BTreeMap<String, BTreeMap<String, ExportsTarget>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scripts: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<StringOrList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_config: Option<PublishConfig>,
    #[serde(default, rename = "private", skip_serializing_if = "Option::is_none")]
    pub is_private: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer_global: Option<bool>,
    #[serde(rename = "gitHead", default, skip_serializing_if = "Option::is_none")]
    pub git_head: Option<String>,
    #[serde(
        rename = "typesVersions",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub types_versions: Option<BTreeMap<String, BTreeMap<String, Vec<String>>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effects: Option<SideEffects>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpkg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jsdelivr: Option<String>,
    #[serde(rename = "jsnext:main", default, skip_serializing_if = "Option::is_none")]
    pub jsnext_main: Option<String>,
    #[serde(rename = "packageManager", default, skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolutions: Option<BTreeMap<String, String>>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Supporting types
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct Dist {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shasum: Option<String>,
    pub tarball: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
    #[serde(default, rename = "fileCount", skip_serializing_if = "Option::is_none")]
    pub file_count: Option<i64>,
    #[serde(default, rename = "unpackedSize", skip_serializing_if = "Option::is_none")]
    pub unpacked_size: Option<i64>,
    #[serde(default, rename = "npm-signature", skip_serializing_if = "Option::is_none")]
    pub npm_signature: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct Maintainer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct PeerDepMeta {
    #[serde(default)]
    pub optional: bool,
}

// ═══════════════════════════════════════════════════════════════════════════
// Abbreviated packument (application/vnd.npm.install-v1+json)
// ═══════════════════════════════════════════════════════════════════════════

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
    pub bin: Option<Bin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directories: Option<Directories>,
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
    pub funding: Option<Funding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libc: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<Workspaces>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub accept_dependencies: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_time: Option<i64>,
}

// ═══════════════════════════════════════════════════════════════════════════
// CouchDB _changes feed types
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChangesResult {
    pub results: Vec<ChangeEntry>,
    pub last_seq: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSeqResponse {
    pub update_seq: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChangeEntry {
    pub seq: ChangeSeq,
    pub id: String,
    #[serde(default)]
    pub changes: Vec<ChangeRev>,
    #[serde(default)]
    pub deleted: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ChangeSeq {
    Number(u64),
    String(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChangeRev {
    pub rev: String,
}

// ═══════════════════════════════════════════════════════════════════════════
// Registry info & sync response
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RegistryInfo {
    pub db_name: String,
    pub doc_count: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SyncResponse {
    pub ok: bool,
    pub log: String,
}

// ═══════════════════════════════════════════════════════════════════════════
// Fast-npm-meta types
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FastMetaResolved {
    pub name: String,
    pub specifier: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FastMetaVersions {
    pub name: String,
    pub specifier: String,
    pub dist_tags: HashMap<String, String>,
    pub versions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, ToSchema)]
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
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct Provenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Publish protocol types
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PublishPayload {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub versions: HashMap<String, PublishVersion>,
    #[serde(rename = "dist-tags", default)]
    pub dist_tags: HashMap<String, String>,
    #[serde(rename = "_attachments", default)]
    pub attachments: HashMap<String, PublishAttachment>,
    #[serde(default)]
    pub readme: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PublishVersion {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<StringOrList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<Repository>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bugs: Option<Bugs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributors: Option<Vec<Author>>,
    #[serde(
        default,
        rename = "readmeFilename",
        skip_serializing_if = "Option::is_none"
    )]
    pub readme_filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, alias = "bundledDependencies", skip_serializing_if = "Option::is_none")]
    pub bundle_dependencies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<Bin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directories: Option<Directories>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub man: Option<Man>,
    #[serde(default)]
    pub dist: PublishDist,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engines: Option<HashMap<String, String>>,
    #[serde(rename = "_hasShrinkwrap", default, skip_serializing_if = "Option::is_none")]
    pub _has_shrinkwrap: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding: Option<Funding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libc: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<Workspaces>,
    #[serde(default, rename = "acceptDependencies", skip_serializing_if = "HashMap::is_empty")]
    pub accept_dependencies: HashMap<String, String>,
    #[serde(rename = "_id", default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "_npmUser", default, skip_serializing_if = "Option::is_none")]
    pub _npm_user: Option<Person>,
    #[serde(rename = "_npmVersion", default, skip_serializing_if = "Option::is_none")]
    pub _npm_version: Option<String>,
    #[serde(rename = "_nodeVersion", default, skip_serializing_if = "Option::is_none")]
    pub _node_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typings: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub module_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exports: Option<Exports>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imports: Option<BTreeMap<String, BTreeMap<String, ExportsTarget>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scripts: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<StringOrList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_config: Option<PublishConfig>,
    #[serde(default, rename = "private", skip_serializing_if = "Option::is_none")]
    pub is_private: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer_global: Option<bool>,
    #[serde(rename = "gitHead", default, skip_serializing_if = "Option::is_none")]
    pub git_head: Option<String>,
    #[serde(
        rename = "typesVersions",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub types_versions: Option<BTreeMap<String, BTreeMap<String, Vec<String>>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effects: Option<SideEffects>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpkg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jsdelivr: Option<String>,
    #[serde(rename = "jsnext:main", default, skip_serializing_if = "Option::is_none")]
    pub jsnext_main: Option<String>,
    #[serde(rename = "packageManager", default, skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolutions: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct PublishDist {
    #[serde(default)]
    pub shasum: Option<String>,
    #[serde(default)]
    pub integrity: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct PublishAttachment {
    #[serde(rename = "content_type", default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub length: usize,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PublishResponse {
    pub ok: bool,
    pub rev: String,
}

// ═══════════════════════════════════════════════════════════════════════════
// Auth types
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct LoginPayload {
    pub name: String,
    pub password: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, rename = "_id")]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub typ: Option<String>,
    #[serde(default)]
    pub roles: Option<Vec<String>>,
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LoginResponse {
    pub ok: bool,
    pub id: String,
    pub rev: String,
    pub token: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_polymorphic() {
        assert!(matches!(
            serde_json::from_str::<License>(r#""MIT""#).unwrap(),
            License::Spdx(_)
        ));
        assert!(matches!(
            serde_json::from_str::<License>(r#"{"type":"MIT","url":"https://x"}"#).unwrap(),
            License::Object { .. }
        ));
    }

    #[test]
    fn author_polymorphic() {
        assert!(matches!(
            serde_json::from_str::<Author>(r#""Microsoft Corp.""#).unwrap(),
            Author::Name(_)
        ));
        assert!(matches!(
            serde_json::from_str::<Author>(r#"{"name":"TJ","email":"t@x.com"}"#).unwrap(),
            Author::Person(_)
        ));
    }

    #[test]
    fn repository_polymorphic() {
        assert!(matches!(
            serde_json::from_str::<Repository>(r#""github:a/b""#).unwrap(),
            Repository::Shorthand(_)
        ));
        assert!(matches!(
            serde_json::from_str::<Repository>(
                r#"{"type":"git","url":"git+https://x","directory":"pkg"}"#
            )
            .unwrap(),
            Repository::Object { .. }
        ));
    }

    #[test]
    fn funding_three_forms() {
        assert!(matches!(
            serde_json::from_str::<Funding>(r#""https://x.com""#).unwrap(),
            Funding::Url(_)
        ));
        assert!(matches!(
            serde_json::from_str::<Funding>(r#"{"type":"github","url":"https://x"}"#).unwrap(),
            Funding::Single(_)
        ));
        assert!(matches!(
            serde_json::from_str::<Funding>(r#"[{"url":"https://x"}]"#).unwrap(),
            Funding::Multiple(_)
        ));
    }

    #[test]
    fn bin_two_forms() {
        assert!(matches!(
            serde_json::from_str::<Bin>(r#"{"tsc":"bin/tsc"}"#).unwrap(),
            Bin::Map(_)
        ));
        assert!(matches!(
            serde_json::from_str::<Bin>(r#""./cmd.js""#).unwrap(),
            Bin::Path(_)
        ));
    }

    #[test]
    fn workspaces_two_forms() {
        assert!(matches!(
            serde_json::from_str::<Workspaces>(r#"["packages/*"]"#).unwrap(),
            Workspaces::Globs(_)
        ));
        assert!(matches!(
            serde_json::from_str::<Workspaces>(r#"{"packages":["p/*"]}"#).unwrap(),
            Workspaces::Config { .. }
        ));
    }

    #[test]
    fn side_effects_two_forms() {
        assert!(matches!(
            serde_json::from_str::<SideEffects>("false").unwrap(),
            SideEffects::Flag(false)
        ));
        assert!(matches!(
            serde_json::from_str::<SideEffects>(r#"["*.css"]"#).unwrap(),
            SideEffects::Globs(_)
        ));
    }

    #[test]
    fn exports_recursive_with_null() {
        let ex: Exports = serde_json::from_str(
            r#"{
                ".": {"import":"./index.mjs","require":"./index.cjs","default":["./fallback.js"]},
                "./package.json": "./package.json",
                "./internal": null
            }"#,
        )
        .unwrap();
        assert!(matches!(
            ex.get(".").unwrap(),
            ExportsTarget::Conditions(_)
        ));
        assert!(matches!(
            ex.get("./package.json").unwrap(),
            ExportsTarget::Path(_)
        ));
        assert!(matches!(
            ex.get("./internal").unwrap(),
            ExportsTarget::Null
        ));
        if let ExportsTarget::Conditions(m) = ex.get(".").unwrap() {
            assert!(matches!(
                m.get("default").unwrap(),
                ExportsTarget::Alternatives(_)
            ));
        }
    }

    #[test]
    fn keywords_two_forms() {
        assert_eq!(
            serde_json::from_str::<StringOrList>(r#"["a","b"]"#)
                .unwrap()
                .to_vec(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            serde_json::from_str::<StringOrList>(r#""solo""#)
                .unwrap()
                .to_vec(),
            vec!["solo".to_string()]
        );
    }

    #[test]
    fn browser_field_two_forms() {
        assert!(matches!(
            serde_json::from_str::<BrowserField>(r#""./browser.js""#).unwrap(),
            BrowserField::Path(_)
        ));
        assert!(matches!(
            serde_json::from_str::<BrowserField>(r#"{"fs":false,"mod":"./b.js"}"#).unwrap(),
            BrowserField::Map(_)
        ));
    }

    #[test]
    fn change_seq_two_forms() {
        assert!(matches!(
            serde_json::from_str::<ChangeSeq>("42").unwrap(),
            ChangeSeq::Number(42)
        ));
        assert!(matches!(
            serde_json::from_str::<ChangeSeq>(r#""42-g1""#).unwrap(),
            ChangeSeq::String(_)
        ));
    }

    #[test]
    fn bundled_dependencies_alias() {
        let pv: PackageVersion = serde_json::from_str(
            r#"{"name":"x","version":"1.0.0","dist":{"tarball":"t"},"bundledDependencies":["foo"]}"#,
        )
        .unwrap();
        assert_eq!(
            pv.bundle_dependencies,
            Some(vec!["foo".to_string()])
        );
    }

    #[test]
    fn full_version_manifest_roundtrip() {
        let json = r#"{
            "name":"express","version":"4.18.2",
            "description":"Fast",
            "license":"MIT",
            "author":{"name":"TJ","email":"t@v.com"},
            "repository":{"type":"git","url":"git+https://github.com/expressjs/express.git"},
            "bugs":{"url":"https://github.com/expressjs/express/issues"},
            "homepage":"http://expressjs.com/","keywords":["express","web"],
            "main":"index.js","scripts":{"test":"mocha"},
            "dependencies":{"accepts":"~1.3.8"},
            "engines":{"node":">= 0.10.0"},
            "dist":{"shasum":"abc","tarball":"https://r/express/-/express-4.18.2.tgz","integrity":"sha512-"},
            "_npmUser":{"name":"dougwilson"},
            "funding":{"type":"opencollective","url":"https://opencollective.com/express"},
            "bin":{"express":"./bin/express.js"},
            "os":["linux"],"cpu":["x64"]
        }"#;
        let pv: PackageVersion = serde_json::from_str(json).unwrap();
        assert_eq!(pv.name, "express");
        assert_eq!(pv.version, "4.18.2");
        assert!(matches!(pv.license.as_ref().unwrap(), License::Spdx(_)));
        assert!(matches!(pv.author.as_ref().unwrap(), Author::Person(_)));
        assert!(matches!(pv.funding.as_ref().unwrap(), Funding::Single(_)));
        assert!(matches!(pv.bin.as_ref().unwrap(), Bin::Map(_)));
        let out = serde_json::to_string(&pv).unwrap();
        let pv2: PackageVersion = serde_json::from_str(&out).unwrap();
        assert_eq!(pv2.name, "express");
    }
}
