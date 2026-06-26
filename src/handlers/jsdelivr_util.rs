use crate::error::{WebError, WebResult};
use crate::extract;
use crate::handlers::fast_meta::{
    extract_dist_tags, extract_version_list, load_packument, resolve_specifier,
};
use crate::repository::PackageVersionRow;
use crate::state::AppState;

pub(crate) struct ResolvedVersion {
    pub version_row: PackageVersionRow,
    pub resolved: String,
    pub tarball_filename: String,
}

pub(crate) fn parse_pkg_spec_path(rest: &str) -> (String, String, String) {
    let s = percent_encoding::percent_decode_str(rest.trim_start_matches('/'))
        .decode_utf8_lossy()
        .to_string();

    if let Some(at) = s
        .char_indices()
        .skip(1)
        .find(|(_, c)| *c == '@')
        .map(|(i, _)| i)
    {
        let fullname = s[..at].to_string();
        let after = &s[at + 1..];
        if let Some(slash) = after.find('/') {
            (
                fullname,
                after[..slash].to_string(),
                after[slash + 1..].to_string(),
            )
        } else {
            (fullname, after.to_string(), String::new())
        }
    } else {
        let (fullname, tail) = split_fullname_and_tail(&s);
        (fullname, "latest".to_string(), tail)
    }
}

fn split_fullname_and_tail(s: &str) -> (String, String) {
    if s.starts_with('@') {
        match s.find('/') {
            Some(first) => match s[first + 1..].find('/') {
                Some(second) => {
                    let end = first + 1 + second;
                    (s[..end].to_string(), s[end + 1..].to_string())
                }
                None => (s.to_string(), String::new()),
            },
            None => (s.to_string(), String::new()),
        }
    } else {
        match s.find('/') {
            Some(idx) => (s[..idx].to_string(), s[idx + 1..].to_string()),
            None => (s.to_string(), String::new()),
        }
    }
}

pub(crate) async fn resolve_version(
    state: &AppState,
    fullname: &str,
    spec: &str,
) -> WebResult<ResolvedVersion> {
    let (pkg, packument) = load_packument(state, fullname, false).await?;

    let dist_tags = extract_dist_tags(&packument);
    let versions = extract_version_list(&packument);

    let resolved = resolve_specifier(spec, &dist_tags, &versions)
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{spec} not resolved")))?;

    let version_row = state
        .repo
        .get_version(pkg.id, &resolved)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{resolved} not found")))?;

    let tarball_filename = packument
        .get("versions")
        .and_then(|v| v.get(&resolved))
        .and_then(|v| v.get("dist"))
        .and_then(|d| d.get("tarball"))
        .and_then(|t| t.as_str())
        .and_then(|url| url.rsplit('/').next())
        .map(|s| s.to_string())
        .ok_or_else(|| WebError::NotFound(format!("no tarball for {fullname}@{resolved}")))?;

    Ok(ResolvedVersion {
        version_row,
        resolved,
        tarball_filename,
    })
}

pub(crate) async fn ensure_version_files_single_flight(
    state: &AppState,
    fullname: &str,
    resolved: &ResolvedVersion,
) -> WebResult<()> {
    let version_id = resolved.version_row.id;
    loop {
        if state
            .repo
            .has_version_files(version_id)
            .await
            .map_err(WebError::CustomApiError)?
        {
            return Ok(());
        }

        let (notify, is_leader) = state.extraction_inflight.get_or_insert(version_id);
        if is_leader {
            let result = extract::ensure_version_files(
                state,
                fullname,
                &resolved.version_row,
                &resolved.tarball_filename,
            )
            .await;
            state.extraction_inflight.remove(version_id);
            notify.notify_waiters();
            return result;
        } else {
            notify.notified().await;
        }
    }
}
