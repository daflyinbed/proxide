use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::npm::split_scope_name;
use crate::npm::types::{Author, License, Maintainer, Packument, Person, StringOrList};
use crate::repository::{PackageDownloadRow, UpstreamPackageDownloadRow};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchDocument {
    pub id: String,
    pub package: PackageDoc,
    pub downloads: DownloadsDoc,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "_source_registry_name"
    )]
    pub source_registry_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_npmUser")]
    pub npm_user: Option<NpmUserDoc>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "publish_time")]
    pub publish_time: Option<i64>,
    #[serde(default = "default_access")]
    pub access: String,
}

fn default_access() -> String {
    "public".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MaintainerDoc {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NpmUserDoc {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DownloadsDoc {
    pub upstream: u64,
    pub local: u64,
}

pub fn build_search_document(
    package_id: i64,
    packument: &Packument,
    upstream: u64,
    local: u64,
    access: &str,
) -> SearchDocument {
    let latest_version = packument.dist_tags.get("latest");
    let latest_manifest = latest_version.and_then(|v| packument.versions.get(v));
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
    let author = packument.author.as_ref().and_then(author_to_doc);

    let date = latest_version.and_then(|v| packument.time.get(v)).cloned();
    let created = packument.time.get("created").cloned();
    let modified = packument.time.get("modified").cloned();

    let deprecated = latest_manifest.and_then(|m| m.deprecated.clone());
    let npm_user = latest_manifest
        .and_then(|m| m._npm_user.as_ref())
        .map(person_to_npm_user);
    let publish_time = latest_manifest
        .and_then(|m| packument.time.get(&m.version))
        .and_then(|t| {
            let trimmed = t.strip_suffix('Z').unwrap_or(t);
            chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|dt| dt.and_utc().timestamp_millis())
        });

    let mut versions: Vec<String> = packument.versions.keys().cloned().collect();
    versions.sort();
    let dist_tags: std::collections::BTreeMap<String, String> = packument
        .dist_tags
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

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
        access: access.to_string(),
    };

    SearchDocument {
        id: package_id.to_string(),
        package,
        downloads: DownloadsDoc { upstream, local },
    }
}

pub fn sum_downloads(rows: &[UpstreamPackageDownloadRow]) -> u64 {
    rows.iter().map(sum_row_downloads).sum()
}

pub fn sum_local_downloads(rows: &[(i64, String, PackageDownloadRow)]) -> u64 {
    rows.iter()
        .map(|(_, _, r)| sum_local_row_downloads(r))
        .sum()
}

fn row_days_u64(row: &[u32; 31]) -> u64 {
    row.iter().map(|d| *d as u64).sum()
}

fn sum_row_downloads(row: &UpstreamPackageDownloadRow) -> u64 {
    row_days_u64(&[
        row.d01, row.d02, row.d03, row.d04, row.d05, row.d06, row.d07, row.d08, row.d09, row.d10,
        row.d11, row.d12, row.d13, row.d14, row.d15, row.d16, row.d17, row.d18, row.d19, row.d20,
        row.d21, row.d22, row.d23, row.d24, row.d25, row.d26, row.d27, row.d28, row.d29, row.d30,
        row.d31,
    ])
}

fn sum_local_row_downloads(row: &PackageDownloadRow) -> u64 {
    row_days_u64(&[
        row.d01, row.d02, row.d03, row.d04, row.d05, row.d06, row.d07, row.d08, row.d09, row.d10,
        row.d11, row.d12, row.d13, row.d14, row.d15, row.d16, row.d17, row.d18, row.d19, row.d20,
        row.d21, row.d22, row.d23, row.d24, row.d25, row.d26, row.d27, row.d28, row.d29, row.d30,
        row.d31,
    ])
}

fn extract_keywords(value: &Option<StringOrList>) -> Vec<String> {
    value.as_ref().map(|v| v.to_vec()).unwrap_or_default()
}

fn extract_license(value: &Option<License>) -> Option<String> {
    match value {
        Some(License::Spdx(s)) => Some(s.clone()),
        Some(License::Object { typ, .. }) => typ.clone(),
        None => None,
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

fn author_to_doc(author: &Author) -> Option<AuthorDoc> {
    match author {
        Author::Person(p) => Some(AuthorDoc {
            name: p.name.clone(),
            email: p.email.clone(),
            url: p.url.clone(),
            username: p.name.clone(),
        }),
        Author::Name(s) => Some(AuthorDoc {
            name: Some(s.clone()),
            email: None,
            url: None,
            username: Some(s.clone()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npm::types::Packument;

    fn packument_with_time(time_value: &str) -> Packument {
        let json = serde_json::json!({
            "name": "@scope/pkg",
            "dist-tags": { "latest": "1.2.3" },
            "versions": {
                "1.2.3": {
                    "name": "@scope/pkg",
                    "version": "1.2.3",
                    "dist": { "tarball": "https://example.com/@scope/pkg/-/pkg-1.2.3.tgz" }
                }
            },
            "time": {
                "created": "2020-01-01T00:00:00.000Z",
                "modified": "2021-09-30T20:34:49.756Z",
                "1.2.3": time_value
            }
        });
        serde_json::from_value(json).expect("packument must deserialize")
    }

    #[test]
    fn publish_time_parsed_from_upstream_z_suffix() {
        let packument = packument_with_time("2021-09-30T20:34:49.756Z");
        let doc = build_search_document(1, &packument, 0, 0, "public");
        assert_eq!(doc.package.publish_time, Some(1_633_034_089_756));
        assert_eq!(
            doc.package.date.as_deref(),
            Some("2021-09-30T20:34:49.756Z")
        );
    }

    #[test]
    fn publish_time_parsed_from_local_no_suffix() {
        let packument = packument_with_time("2021-09-30T20:34:49.756");
        let doc = build_search_document(1, &packument, 42, 7, "public");
        assert_eq!(doc.package.publish_time, Some(1_633_034_089_756));
        assert_eq!(doc.downloads.upstream, 42);
        assert_eq!(doc.downloads.local, 7);
    }

    #[test]
    fn build_search_document_basic_structure() {
        let packument = packument_with_time("2021-09-30T20:34:49.756Z");
        let doc = build_search_document(42, &packument, 7, 3, "public");

        assert_eq!(doc.id, "42");
        assert_eq!(doc.package.name, "@scope/pkg");
        assert_eq!(doc.package.version, "1.2.3");
        assert_eq!(doc.package.scope, "scope");
        assert_eq!(doc.downloads.upstream, 7);
        assert_eq!(doc.downloads.local, 3);
        assert_eq!(doc.package.access, "public");
        assert_eq!(doc.package.versions, vec!["1.2.3".to_string()]);
        assert_eq!(
            doc.package.dist_tags.get("latest").map(String::as_str),
            Some("1.2.3")
        );
        assert_eq!(
            doc.package.created.as_deref(),
            Some("2020-01-01T00:00:00.000Z")
        );
        assert_eq!(
            doc.package.modified.as_deref(),
            Some("2021-09-30T20:34:49.756Z")
        );
    }

    #[test]
    fn build_search_document_uses_unique_db_id() {
        assert_eq!(
            build_search_document(
                1,
                &packument_with_time("2021-09-30T20:34:49.756Z"),
                0,
                0,
                "public"
            )
            .id,
            "1"
        );
        assert_eq!(
            build_search_document(
                999,
                &packument_with_time("2021-09-30T20:34:49.756Z"),
                0,
                0,
                "public"
            )
            .id,
            "999"
        );
    }

    fn local_row(
        version_id: i64,
        version: &str,
        d01: u32,
        d15: u32,
        d31: u32,
    ) -> (i64, String, PackageDownloadRow) {
        (
            version_id,
            version.to_string(),
            PackageDownloadRow {
                id: version_id,
                package_version_id: version_id,
                year: 2025,
                month: 6,
                d01,
                d02: 0,
                d03: 0,
                d04: 0,
                d05: 0,
                d06: 0,
                d07: 0,
                d08: 0,
                d09: 0,
                d10: 0,
                d11: 0,
                d12: 0,
                d13: 0,
                d14: 0,
                d15,
                d16: 0,
                d17: 0,
                d18: 0,
                d19: 0,
                d20: 0,
                d21: 0,
                d22: 0,
                d23: 0,
                d24: 0,
                d25: 0,
                d26: 0,
                d27: 0,
                d28: 0,
                d29: 0,
                d30: 0,
                d31,
            },
        )
    }

    #[test]
    fn sum_local_downloads_across_versions() {
        let rows = vec![
            local_row(1, "1.0.0", 10, 0, 5),
            local_row(2, "2.0.0", 0, 7, 3),
        ];
        assert_eq!(sum_local_downloads(&rows), 25);
        assert_eq!(sum_local_downloads(&[]), 0);
    }
}
