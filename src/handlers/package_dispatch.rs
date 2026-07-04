use crate::error::{WebError, WebResult};
use crate::handlers::{publish, registry, tarball};
use crate::middleware::auth::validate_auth;
use crate::npm::types::{MaintainerUpdatePayload, PublishPayload, PublishResponse};
use crate::state::AppState;
use axum::Json;
use axum::body;
use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};

fn decode_path(path: &str) -> String {
    percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .to_string()
}

#[derive(Debug)]
#[allow(dead_code)]
enum PackageRoute {
    Package { fullname: String },
    Version { fullname: String, version: String },
    Tarball { fullname: String, filename: String },
    Rev { fullname: String, rev: String },
    TarballRev { fullname: String, filename: String, rev: String },
}

fn parse_package_route(path: &str) -> WebResult<PackageRoute> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return Err(WebError::NotFound("path is empty".to_string()));
    }

    let segments: Vec<&str> = path.split('/').collect();

    let (fullname, rest) = if segments[0].starts_with('@') {
        if segments.len() < 2 {
            return Err(WebError::NotFound(
                "incomplete scoped package name".to_string(),
            ));
        }
        (format!("{}/{}", segments[0], segments[1]), &segments[2..])
    } else {
        (segments[0].to_string(), &segments[1..])
    };

    match rest {
        [] => Ok(PackageRoute::Package { fullname }),
        ["-", filename] => Ok(PackageRoute::Tarball {
            fullname,
            filename: filename.to_string(),
        }),
        ["-rev", rev] => Ok(PackageRoute::Rev {
            fullname,
            rev: rev.to_string(),
        }),
        ["-", filename, "-rev", rev] => Ok(PackageRoute::TarballRev {
            fullname,
            filename: filename.to_string(),
            rev: rev.to_string(),
        }),
        [version] => Ok(PackageRoute::Version {
            fullname,
            version: version.to_string(),
        }),
        _ => Err(WebError::NotFound(format!("not found: {path}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_package() {
        let route = parse_package_route("lodash").unwrap();
        let PackageRoute::Package { fullname } = route else {
            panic!("expected Package");
        };
        assert_eq!(fullname, "lodash");
    }

    #[test]
    fn simple_package_with_leading_slash() {
        let route = parse_package_route("/lodash").unwrap();
        let PackageRoute::Package { fullname } = route else {
            panic!("expected Package");
        };
        assert_eq!(fullname, "lodash");
    }

    #[test]
    fn scoped_package() {
        let route = parse_package_route("@babel/core").unwrap();
        let PackageRoute::Package { fullname } = route else {
            panic!("expected Package");
        };
        assert_eq!(fullname, "@babel/core");
    }

    #[test]
    fn scoped_package_with_leading_slash() {
        let route = parse_package_route("/@babel/core").unwrap();
        let PackageRoute::Package { fullname } = route else {
            panic!("expected Package");
        };
        assert_eq!(fullname, "@babel/core");
    }

    #[test]
    fn simple_package_version() {
        let route = parse_package_route("lodash/4.17.21").unwrap();
        let PackageRoute::Version { fullname, version } = route else {
            panic!("expected Version");
        };
        assert_eq!(fullname, "lodash");
        assert_eq!(version, "4.17.21");
    }

    #[test]
    fn scoped_package_version() {
        let route = parse_package_route("@babel/core/7.24.0").unwrap();
        let PackageRoute::Version { fullname, version } = route else {
            panic!("expected Version");
        };
        assert_eq!(fullname, "@babel/core");
        assert_eq!(version, "7.24.0");
    }

    #[test]
    fn simple_package_tarball() {
        let route = parse_package_route("lodash/-/lodash-4.17.21.tgz").unwrap();
        let PackageRoute::Tarball { fullname, filename } = route else {
            panic!("expected Tarball");
        };
        assert_eq!(fullname, "lodash");
        assert_eq!(filename, "lodash-4.17.21.tgz");
    }

    #[test]
    fn scoped_package_tarball() {
        let route = parse_package_route("@babel/core/-/core-7.24.0.tgz").unwrap();
        let PackageRoute::Tarball { fullname, filename } = route else {
            panic!("expected Tarball");
        };
        assert_eq!(fullname, "@babel/core");
        assert_eq!(filename, "core-7.24.0.tgz");
    }

    #[test]
    fn empty_path() {
        let err = parse_package_route("").unwrap_err();
        assert!(matches!(err, WebError::NotFound(_)));
    }

    #[test]
    fn only_slashes() {
        let err = parse_package_route("///").unwrap_err();
        assert!(matches!(err, WebError::NotFound(_)));
    }

    #[test]
    fn incomplete_scope() {
        let err = parse_package_route("@babel").unwrap_err();
        assert!(matches!(err, WebError::NotFound(_)));
    }

    #[test]
    fn incomplete_scope_with_slash() {
        let err = parse_package_route("/@babel").unwrap_err();
        assert!(matches!(err, WebError::NotFound(_)));
    }

    #[test]
    fn too_many_segments() {
        let err = parse_package_route("lodash/foo/bar").unwrap_err();
        assert!(matches!(err, WebError::NotFound(_)));
    }

    #[test]
    fn scoped_package_too_many_segments() {
        let err = parse_package_route("@babel/core/foo/bar").unwrap_err();
        assert!(matches!(err, WebError::NotFound(_)));
    }

    #[test]
    fn simple_package_rev() {
        let route = parse_package_route("lodash/-rev/12-abc").unwrap();
        let PackageRoute::Rev { fullname, rev } = route else {
            panic!("expected Rev");
        };
        assert_eq!(fullname, "lodash");
        assert_eq!(rev, "12-abc");
    }

    #[test]
    fn scoped_package_rev() {
        let route = parse_package_route("@babel/core/-rev/1-xyz").unwrap();
        let PackageRoute::Rev { fullname, rev } = route else {
            panic!("expected Rev");
        };
        assert_eq!(fullname, "@babel/core");
        assert_eq!(rev, "1-xyz");
    }

    #[test]
    fn simple_package_tarball_rev() {
        let route = parse_package_route("lodash/-/lodash-4.17.21.tgz/-rev/12-abc").unwrap();
        let PackageRoute::TarballRev { fullname, filename, rev } = route else {
            panic!("expected TarballRev");
        };
        assert_eq!(fullname, "lodash");
        assert_eq!(filename, "lodash-4.17.21.tgz");
        assert_eq!(rev, "12-abc");
    }

    #[test]
    fn scoped_package_tarball_rev() {
        let route = parse_package_route("@babel/core/-/core-7.24.0.tgz/-rev/5-def").unwrap();
        let PackageRoute::TarballRev { fullname, filename, rev } = route else {
            panic!("expected TarballRev");
        };
        assert_eq!(fullname, "@babel/core");
        assert_eq!(filename, "core-7.24.0.tgz");
        assert_eq!(rev, "5-def");
    }
}

pub async fn dispatch(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Request,
) -> WebResult<Response> {
    let method = req.method().clone();
    match method {
        axum::http::Method::GET => dispatch_get(State(state), headers, req).await,
        axum::http::Method::PUT => dispatch_put(State(state), headers, req).await,
        axum::http::Method::DELETE => dispatch_delete(State(state), headers, req).await,
        _ => Err(WebError::MethodNotAllowed(format!(
            "method {method} not supported"
        ))),
    }
}

pub async fn dispatch_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Request,
) -> WebResult<Response> {
    let path = decode_path(req.uri().path());
    let route = parse_package_route(&path)?;

    match route {
        PackageRoute::Package { fullname } => {
            registry::get_package_inner(&state, &headers, &fullname).await
        }
        PackageRoute::Version { fullname, version } => {
            let json = registry::get_package_version_inner(&state, &headers, &fullname, &version).await?;
            Ok(json.into_response())
        }
        PackageRoute::Tarball { fullname, filename } => {
            tarball::download_tarball_inner(&state, &headers, &fullname, &filename).await
        }
        PackageRoute::Rev { .. } | PackageRoute::TarballRev { .. } => {
            Err(WebError::NotFound(format!("GET not supported for: {path}")))
        }
    }
}

pub async fn dispatch_put(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Request,
) -> WebResult<Response> {
    let path = decode_path(req.uri().path());
    let route = parse_package_route(&path)?;

    let auth = validate_auth(&state, &headers).await?;

    match route {
        PackageRoute::Package { fullname } => {
            let body_bytes = body::to_bytes(req.into_body(), 10 * 1024 * 1024)
                .await
                .map_err(|e| {
                    WebError::CustomApiError(anyhow::anyhow!("failed to read body: {e}"))
                })?;

            let payload: PublishPayload = serde_json::from_slice(&body_bytes)
                .map_err(|e| WebError::BadRequest(format!("invalid JSON: {e}")))?;

            let result =
                publish::publish_package_inner(&state, &headers, &auth, &fullname, payload).await?;
            Ok(result.into_response())
        }
        // NOTE: The `rev` segment from `PUT /{fullname}/-rev/{rev}` is intentionally
        // ignored. npm's `_rev` is treated as a cosmetic protocol-compatibility value
        // (we emit `pkg.id`), not a true optimistic-concurrency token. As a result two
        // concurrent `npm owner` operations based on the same packument will both
        // succeed with last-write-wins, instead of one returning 409. This matches the
        // behavior of cnpmcore (the reference implementation), whose
        // `UpdatePackageController` also drops `rev` and whose `Package` model has no
        // revision column. We accept the lost-update risk for owner changes rather than
        // diverging from the ecosystem; revisit if stronger guarantees are ever required.
        PackageRoute::Rev { fullname, .. } => {
            if publish::get_npm_command(&headers).as_deref() == Some("unpublish") {
                return Ok(Json(PublishResponse {
                    ok: false,
                    rev: String::new(),
                })
                .into_response());
            }

            let body_bytes = body::to_bytes(req.into_body(), 10 * 1024 * 1024)
                .await
                .map_err(|e| {
                    WebError::CustomApiError(anyhow::anyhow!("failed to read body: {e}"))
                })?;

            let payload: MaintainerUpdatePayload = serde_json::from_slice(&body_bytes)
                .map_err(|e| WebError::BadRequest(format!("invalid JSON: {e}")))?;

            let result =
                publish::update_maintainers_inner(&state, &headers, &auth, &fullname, payload)
                    .await?;
            Ok(result.into_response())
        }
        _ => Err(WebError::NotFound(format!("PUT not supported for: {path}"))),
    }
}

pub async fn dispatch_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Request,
) -> WebResult<Response> {
    let path = decode_path(req.uri().path());
    let route = parse_package_route(&path)?;

    let auth = validate_auth(&state, &headers).await?;

    match route {
        PackageRoute::Rev { fullname, .. } => {
            let result =
                publish::unpublish_package_inner(&state, &headers, &auth, &fullname).await?;
            Ok(result.into_response())
        }
        PackageRoute::TarballRev {
            fullname,
            filename,
            ..
        } => {
            let result =
                publish::unpublish_version_inner(&state, &headers, &auth, &fullname, &filename)
                    .await?;
            Ok(result.into_response())
        }
        _ => Err(WebError::NotFound(format!(
            "DELETE not supported for: {path}"
        ))),
    }
}
