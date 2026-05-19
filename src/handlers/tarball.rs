use crate::error::{WebError, WebResult};
use crate::state::AppState;
use axum::body::Body;
use axum::response::Response;
use reqwest::StatusCode;

fn tarball_response(data: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/octet-stream")
        .header("content-length", data.len())
        .body(Body::from(data))
        .unwrap()
}

fn extract_version(fullname: &str, filename: &str) -> Option<String> {
    let name = fullname.rsplit_once('/').map(|(_, n)| n).unwrap_or(fullname);
    let target = filename.strip_suffix(".tgz").unwrap_or(filename);
    target
        .strip_prefix(name)
        .and_then(|s| s.strip_prefix('-'))
        .map(|s| s.to_string())
}

pub async fn download_tarball_inner(
    state: &AppState,
    fullname: &str,
    filename: &str,
) -> WebResult<Response> {
    if !filename.ends_with(".tgz") {
        return Err(WebError::BadRequest(format!("{filename} not a tarball file")));
    }

    let pkg = state
        .repo
        .get_package_by_name(fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let versions = state
        .repo
        .list_versions(pkg.id)
        .await
        .map_err(WebError::CustomApiError)?;

    let version_name = extract_version(fullname, filename)
        .ok_or_else(|| WebError::NotFound(format!("{fullname}: invalid tarball filename {filename}")))?;

    let ver = versions
        .iter()
        .find(|v| v.version == version_name)
        .ok_or_else(|| WebError::NotFound(format!("{fullname}@{version_name} not found")))?;

    if let Some(tar_dist_id) = ver.tar_dist_id {
        let (data, _) = state
            .repo
            .get_content(tar_dist_id)
            .await
            .map_err(WebError::CustomApiError)?;

        return Ok(tarball_response(data));
    }

    let upstream_url = format!(
        "{}/{}/-/{filename}",
        state.config.worker.upstream_registry, fullname
    );

    let upstream_resp = state
        .http
        .get(&upstream_url)
        .send()
        .await
        .map_err(|e| WebError::CustomApiError(e.into()))?;

    if !upstream_resp.status().is_success() {
        return Err(WebError::NotFound(format!("\"{filename}\" not found")));
    }

    let bytes = upstream_resp
        .bytes()
        .await
        .map_err(|e| WebError::CustomApiError(e.into()))?;

    let data = bytes.to_vec();

    let storage_key = format!("packages/{fullname}/{version_name}/{filename}");
    let dist_id = state
        .repo
        .put_content(filename, &storage_key, data.clone(), None, None)
        .await
        .map_err(WebError::CustomApiError)?;

    state
        .repo
        .update_version_dists(
            ver.id,
            ver.abbrev_dist_id,
            ver.manifest_dist_id,
            Some(dist_id),
            ver.readme_dist_id,
        )
        .await
        .map_err(WebError::CustomApiError)?;

    Ok(tarball_response(data))
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
