use crate::error::{WebError, WebResult};
use crate::extract;
use crate::handlers::fast_meta::{load_abbreviated_packument, resolve_specifier};
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
    let (pkg, packument) = load_abbreviated_packument(state, fullname).await?;

    let versions: Vec<String> = packument.versions.keys().cloned().collect();
    let resolved = resolve_specifier(spec, &packument.dist_tags, &versions)
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{spec} not resolved")))?;

    let version_row = state
        .repo
        .get_version(pkg.id, &resolved)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{resolved} not found")))?;

    let tarball_url = &packument
        .versions
        .get(&resolved)
        .ok_or_else(|| WebError::NotFound(format!("no tarball for {fullname}@{resolved}")))?
        .dist
        .tarball;
    let tarball_filename = tarball_url.rsplit('/').next().unwrap_or(tarball_url).to_string();

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

        let (mut rx, is_leader) = state.extraction_inflight.get_or_insert(version_id);
        if is_leader {
            let result = extract::ensure_version_files(
                state,
                fullname,
                &resolved.version_row,
                &resolved.tarball_filename,
            )
            .await;
            state.extraction_inflight.remove(version_id);
            return result;
        } else {
            let _ = rx.changed().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_pkg_spec_path;

    #[test]
    fn simple_pkg_with_version_and_file() {
        assert_eq!(
            parse_pkg_spec_path("lodash@4.17.21/index.js"),
            ("lodash".to_string(), "4.17.21".to_string(), "index.js".to_string())
        );
    }

    #[test]
    fn simple_pkg_with_version_no_file() {
        assert_eq!(
            parse_pkg_spec_path("lodash@4.17.21"),
            ("lodash".to_string(), "4.17.21".to_string(), String::new())
        );
    }

    #[test]
    fn simple_pkg_with_nested_file() {
        assert_eq!(
            parse_pkg_spec_path("lodash@4.17.21/dist/lodash.js"),
            ("lodash".to_string(), "4.17.21".to_string(), "dist/lodash.js".to_string())
        );
    }

    #[test]
    fn simple_pkg_latest() {
        assert_eq!(
            parse_pkg_spec_path("lodash"),
            ("lodash".to_string(), "latest".to_string(), String::new())
        );
    }

    #[test]
    fn simple_pkg_latest_with_file() {
        assert_eq!(
            parse_pkg_spec_path("lodash/index.js"),
            ("lodash".to_string(), "latest".to_string(), "index.js".to_string())
        );
    }

    #[test]
    fn scoped_pkg_with_version_and_file() {
        assert_eq!(
            parse_pkg_spec_path("@babel/core@7.24.0/lib/index.js"),
            ("@babel/core".to_string(), "7.24.0".to_string(), "lib/index.js".to_string())
        );
    }

    #[test]
    fn scoped_pkg_with_version_no_file() {
        assert_eq!(
            parse_pkg_spec_path("@babel/core@7.24.0"),
            ("@babel/core".to_string(), "7.24.0".to_string(), String::new())
        );
    }

    #[test]
    fn scoped_pkg_dist_tag() {
        assert_eq!(
            parse_pkg_spec_path("@babel/core@next/dist.js"),
            ("@babel/core".to_string(), "next".to_string(), "dist.js".to_string())
        );
    }

    #[test]
    fn scoped_pkg_latest() {
        assert_eq!(
            parse_pkg_spec_path("@babel/core"),
            ("@babel/core".to_string(), "latest".to_string(), String::new())
        );
    }

    #[test]
    fn scoped_pkg_latest_with_nested_file() {
        assert_eq!(
            parse_pkg_spec_path("@babel/core/lib/index.js"),
            ("@babel/core".to_string(), "latest".to_string(), "lib/index.js".to_string())
        );
    }

    #[test]
    fn leading_slash_stripped() {
        assert_eq!(
            parse_pkg_spec_path("/lodash@1.0.0/file.js"),
            ("lodash".to_string(), "1.0.0".to_string(), "file.js".to_string())
        );
    }

    #[test]
    fn leading_slash_scoped() {
        assert_eq!(
            parse_pkg_spec_path("/@babel/core@7.24.0/file.js"),
            ("@babel/core".to_string(), "7.24.0".to_string(), "file.js".to_string())
        );
    }

    #[test]
    fn percent_encoded_scope() {
        assert_eq!(
            parse_pkg_spec_path("@babel%2Fcore@7.24.0"),
            ("@babel/core".to_string(), "7.24.0".to_string(), String::new())
        );
    }

    #[test]
    fn percent_encoded_at_and_slash() {
        assert_eq!(
            parse_pkg_spec_path("lodash%404.17.21%2Findex.js"),
            ("lodash".to_string(), "4.17.21".to_string(), "index.js".to_string())
        );
    }

    #[test]
    fn caret_range_spec() {
        assert_eq!(
            parse_pkg_spec_path("lodash@^4.17.0/index.js"),
            ("lodash".to_string(), "^4.17.0".to_string(), "index.js".to_string())
        );
    }

    #[test]
    fn file_containing_at_sign() {
        assert_eq!(
            parse_pkg_spec_path("lodash@1.0.0/foo@bar.js"),
            ("lodash".to_string(), "1.0.0".to_string(), "foo@bar.js".to_string())
        );
    }

    #[test]
    fn empty_input() {
        assert_eq!(
            parse_pkg_spec_path(""),
            (String::new(), "latest".to_string(), String::new())
        );
    }
}
