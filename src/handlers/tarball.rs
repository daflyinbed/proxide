use crate::error::{WebError, WebResult};
use crate::middleware::auth::ensure_package_readable;
use crate::state::AppState;
use crate::tarball;
use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::Response;
use reqwest::StatusCode;
use std::sync::atomic::{AtomicU64, Ordering};

const TARBALL_CONTENT_TYPE: &str = "application/octet-stream";

fn tarball_response(body: Body, content_length: Option<u64>) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", TARBALL_CONTENT_TYPE);

    if let Some(content_length) = content_length {
        builder = builder.header("content-length", content_length);
    }

    builder.body(body).unwrap()
}

pub(crate) fn extract_version(fullname: &str, filename: &str) -> Option<String> {
    let name = fullname
        .rsplit_once('/')
        .map(|(_, n)| n)
        .unwrap_or(fullname);
    let target = filename.strip_suffix(".tgz").unwrap_or(filename);
    target
        .strip_prefix(name)
        .and_then(|s| s.strip_prefix('-'))
        .map(|s| s.to_string())
}

pub async fn download_tarball_inner(
    state: &AppState,
    headers: &HeaderMap,
    fullname: &str,
    filename: &str,
) -> WebResult<Response> {
    if !filename.ends_with(".tgz") {
        return Err(WebError::BadRequest(format!(
            "{filename} not a tarball file"
        )));
    }

    let version_name = extract_version(fullname, filename).ok_or_else(|| {
        WebError::NotFound(format!("{fullname}: invalid tarball filename {filename}"))
    })?;

    let pkg = state
        .repo
        .get_package_by_name(fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    ensure_package_readable(state, headers, &pkg).await?;

    let version = state
        .repo
        .get_version(pkg.id, &version_name)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{version_name} not found")))?;

    state
        .download_counters
        .entry(version.id)
        .or_insert(AtomicU64::new(0))
        .fetch_add(1, Ordering::Relaxed);

    let tarball = tarball::acquire(state, fullname, &version, filename).await?;
    Ok(tarball_response(
        Body::from_stream(tarball.stream),
        tarball.content_length,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_version_simple_package() {
        assert_eq!(
            extract_version("lodash", "lodash-4.17.21.tgz"),
            Some("4.17.21".into())
        );
    }

    #[test]
    fn extract_version_hyphenated_package() {
        assert_eq!(
            extract_version("core-js", "core-js-3.36.0.tgz"),
            Some("3.36.0".into())
        );
    }

    #[test]
    fn extract_version_scoped_package() {
        assert_eq!(
            extract_version("@babel/core", "core-7.24.0.tgz"),
            Some("7.24.0".into())
        );
    }

    #[test]
    fn extract_version_scoped_hyphenated_package() {
        assert_eq!(
            extract_version("@core-js/pure", "pure-3.36.0.tgz"),
            Some("3.36.0".into())
        );
    }

    #[test]
    fn extract_version_prerelease() {
        assert_eq!(
            extract_version("foo", "foo-1.0.0-beta.1.tgz"),
            Some("1.0.0-beta.1".into())
        );
    }

    #[test]
    fn extract_version_name_prefix_mismatch() {
        assert_eq!(extract_version("bar", "baz-1.0.0.tgz"), None);
    }

    #[test]
    fn extract_version_missing_separator() {
        assert_eq!(extract_version("foo", "foo1.0.0.tgz"), None);
    }

    #[test]
    fn extract_version_long_hyphenated_name() {
        assert_eq!(
            extract_version(
                "@cnpmcore/test-sync-package-has-two-versions",
                "test-sync-package-has-two-versions-2.0.0.tgz"
            ),
            Some("2.0.0".into())
        );
    }

    #[test]
    fn extract_version_cnpmcore_deprecated() {
        assert_eq!(
            extract_version(
                "cnpmcore-test-sync-deprecated",
                "cnpmcore-test-sync-deprecated-0.0.0.tgz"
            ),
            Some("0.0.0".into())
        );
    }
}
