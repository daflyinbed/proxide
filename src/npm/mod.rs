pub mod types;

use crate::npm::types::{AbbreviatedVersion, PackageVersion};
use percent_encoding::percent_decode_str;

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
    if let Some(scripts) = &ver.scripts
        && (scripts.contains_key("install")
            || scripts.contains_key("preinstall")
            || scripts.contains_key("postinstall"))
    {
        return Some(true);
    }
    None
}

pub fn build_abbreviated_version_entry(ver: &PackageVersion, publish_time_str: Option<&String>) -> AbbreviatedVersion {
    let has_install_script = if ver.has_install_script.unwrap_or(false) {
        Some(true)
    } else {
        detect_install_script(ver)
    };

    let libc = ver
        .other
        .get("libc")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let workspaces = ver.other.get("workspaces").cloned();
    let accept_dependencies = ver
        .other
        .get("acceptDependencies")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

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
