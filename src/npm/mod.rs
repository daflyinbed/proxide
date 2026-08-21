pub mod types;

use crate::npm::types::{AbbreviatedPackument, AbbreviatedVersion, PackageVersion, Packument};
use percent_encoding::percent_decode_str;
use std::collections::HashMap;

pub fn build_abbreviated_manifest(packument: &Packument) -> AbbreviatedPackument {
    let mut versions = HashMap::new();
    for (ver, data) in &packument.versions {
        versions.insert(
            ver.clone(),
            build_abbreviated_version_entry(data, packument.time.get(ver)),
        );
    }
    let time = if packument.time.is_empty() {
        None
    } else {
        Some(packument.time.clone())
    };
    AbbreviatedPackument {
        name: packument.name.clone(),
        modified: packument.time.get("modified").cloned(),
        dist_tags: packument.dist_tags.clone(),
        versions,
        time,
    }
}

pub fn split_scope_name(fullname: &str) -> (Option<&str>, &str) {
    fullname
        .strip_prefix('@')
        .and_then(|rest| rest.split_once('/').map(|(s, n)| (Some(s), n)))
        .unwrap_or((None, fullname))
}

pub fn decode_fullname(encoded: &str) -> String {
    percent_decode_str(encoded).decode_utf8_lossy().to_string()
}

pub fn is_prerelease(version: &str) -> bool {
    let v = version.trim_start_matches('v');
    match semver::Version::parse(v) {
        Ok(sv) => !sv.pre.is_empty(),
        Err(_) => true,
    }
}

pub fn pad_version(version: &str) -> String {
    let v = version.trim_start_matches('v');
    match semver::Version::parse(v) {
        Ok(sv) => format!("{:016}{:016}{:016}", sv.major, sv.minor, sv.patch),
        Err(_) => "0".repeat(48),
    }
}

pub fn detect_install_script(ver: &PackageVersion) -> Option<bool> {
    let scripts = ver.scripts.as_ref()?;
    let has_install = scripts.contains_key("install")
        || scripts.contains_key("preinstall")
        || scripts.contains_key("postinstall");
    has_install.then_some(true)
}

pub fn build_abbreviated_version(name: &str, ver: &PackageVersion) -> Vec<u8> {
    let mut entry = build_abbreviated_version_entry(ver, None);
    entry.name = name.to_string();
    serde_json::to_vec(&entry).unwrap_or_default()
}

pub fn build_abbreviated_version_entry(
    ver: &PackageVersion,
    publish_time_str: Option<&String>,
) -> AbbreviatedVersion {
    let has_install_script = ver
        .has_install_script
        .unwrap_or(false)
        .then_some(true)
        .or_else(|| detect_install_script(ver));

    let libc = ver.libc.clone();
    let workspaces = ver.workspaces.clone();
    let accept_dependencies = ver.accept_dependencies.clone();

    let publish_time = publish_time_str.and_then(|t| {
        chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f")
            .ok()
            .map(|dt| dt.and_utc().timestamp_millis() / 1000)
    });

    AbbreviatedVersion {
        name: ver.name.clone(),
        version: ver.version.clone(),
        deprecated: ver.deprecated.clone(),
        dependencies: ver.dependencies.clone(),
        optional_dependencies: ver.optional_dependencies.clone(),
        dev_dependencies: ver.dev_dependencies.clone(),
        bundle_dependencies: ver.bundle_dependencies.clone(),
        peer_dependencies: ver.peer_dependencies.clone(),
        peer_dependencies_meta: ver.peer_dependencies_meta.clone(),
        bin: ver.bin.clone(),
        directories: ver.directories.clone(),
        dist: ver.dist.clone(),
        engines: ver.engines.clone(),
        _has_shrinkwrap: ver._has_shrinkwrap,
        has_install_script,
        funding: ver.funding.clone(),
        cpu: ver.cpu.clone(),
        os: ver.os.clone(),
        libc,
        workspaces,
        accept_dependencies,
        publish_time,
    }
}
