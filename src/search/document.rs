use serde::{Deserialize, Serialize};

use crate::npm::split_scope_name;
use crate::npm::types::{Maintainer, Packument, Person};
use crate::repository::UpstreamPackageDownloadRow;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchDocument {
    pub id: String,
    pub package: PackageDoc,
    pub downloads: DownloadsDoc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDoc {
    pub name: String,
    pub version: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub versions: Vec<String>,
    #[serde(default, rename = "dist-tags")]
    pub dist_tags: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub maintainers: Vec<MaintainerDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<AuthorDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "created")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "modified")]
    pub modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_source_registry_name")]
    pub source_registry_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_npmUser")]
    pub npm_user: Option<NpmUserDoc>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "publish_time")]
    pub publish_time: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintainerDoc {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorDoc {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpmUserDoc {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadsDoc {
    pub all: u64,
}

pub fn build_search_document(packument: &Packument, downloads_all: u64) -> SearchDocument {
    let latest_version = packument.dist_tags.get("latest");
    let latest_manifest = latest_version
        .and_then(|v| packument.versions.get(v));

    let scope = split_scope_name(&packument.name)
        .0
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unscoped".to_string());

    let keywords = extract_keywords(&packument.keywords);
    let license = extract_license(&packument.license);
    let maintainers = packument
        .maintainers
        .as_ref()
        .map(|ms| ms.iter().map(maintainer_to_doc).collect())
        .unwrap_or_default();
    let author = packument.author.as_ref().and_then(value_to_author_doc);

    let date = latest_version.and_then(|v| packument.time.get(v)).cloned();
    let created = packument.time.get("created").cloned();
    let modified = packument.time.get("modified").cloned();

    let deprecated = latest_manifest.and_then(|m| m.deprecated.clone());
    let npm_user = latest_manifest.and_then(|m| m._npm_user.as_ref()).map(person_to_npm_user);
    let publish_time = latest_manifest
        .and_then(|m| packument.time.get(&m.version))
        .and_then(|t| {
            chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|dt| dt.and_utc().timestamp_millis())
        });

    let versions: Vec<String> = packument.versions.keys().cloned().collect();
    let dist_tags: std::collections::BTreeMap<String, String> =
        packument.dist_tags.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

    let package = PackageDoc {
        name: packument.name.clone(),
        version: latest_version.cloned().unwrap_or_default(),
        scope,
        description: packument.description.clone(),
        license,
        keywords,
        versions,
        dist_tags,
        maintainers,
        author,
        date,
        created,
        modified,
        deprecated,
        source_registry_name: None,
        npm_user,
        publish_time,
    };

    SearchDocument {
        id: packument.name.clone(),
        package,
        downloads: DownloadsDoc { all: downloads_all },
    }
}

pub fn sum_downloads(rows: &[UpstreamPackageDownloadRow]) -> u64 {
    rows.iter().map(|r| sum_row_downloads(r)).sum()
}

fn sum_row_downloads(row: &UpstreamPackageDownloadRow) -> u64 {
    [
        row.d01, row.d02, row.d03, row.d04, row.d05, row.d06, row.d07,
        row.d08, row.d09, row.d10, row.d11, row.d12, row.d13, row.d14,
        row.d15, row.d16, row.d17, row.d18, row.d19, row.d20, row.d21,
        row.d22, row.d23, row.d24, row.d25, row.d26, row.d27, row.d28,
        row.d29, row.d30, row.d31,
    ]
    .iter()
    .map(|d| *d as u64)
    .sum()
}

fn extract_keywords(value: &Option<serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn extract_license(value: &Option<serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Object(obj)) => {
            obj.get("type").and_then(|v| v.as_str()).map(|s| s.to_string())
        }
        _ => None,
    }
}

fn maintainer_to_doc(m: &Maintainer) -> MaintainerDoc {
    MaintainerDoc {
        name: m.name.clone(),
        email: m.email.clone(),
        username: Some(m.name.clone()),
    }
}

fn person_to_npm_user(p: &Person) -> NpmUserDoc {
    NpmUserDoc {
        name: p.name.clone(),
        email: p.email.clone(),
    }
}

fn value_to_author_doc(value: &serde_json::Value) -> Option<AuthorDoc> {
    match value {
        serde_json::Value::String(s) => Some(AuthorDoc {
            name: Some(s.clone()),
            email: None,
            url: None,
            username: Some(s.clone()),
        }),
        serde_json::Value::Object(obj) => {
            let name = obj.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
            Some(AuthorDoc {
                username: name.clone(),
                name,
                email: obj.get("email").and_then(|v| v.as_str()).map(|s| s.to_string()),
                url: obj.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()),
            })
        }
        _ => None,
    }
}
