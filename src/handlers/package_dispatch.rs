use crate::error::{WebError, WebResult};
use crate::handlers::{publish, registry, tarball};
use crate::middleware::auth::validate_auth;
use crate::npm::types::PublishPayload;
use crate::state::AppState;
use axum::body;
use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};

fn decode_path(path: &str) -> String {
    percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .to_string()
}

enum PackageRoute {
    Package { fullname: String },
    Version { fullname: String, version: String },
    Tarball { fullname: String, filename: String },
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
        (
            format!("{}/{}", segments[0], segments[1]),
            &segments[2..],
        )
    } else {
        (segments[0].to_string(), &segments[1..])
    };

    match rest {
        [] => Ok(PackageRoute::Package { fullname }),
        ["-", filename] => Ok(PackageRoute::Tarball {
            fullname,
            filename: filename.to_string(),
        }),
        [version] => Ok(PackageRoute::Version {
            fullname,
            version: version.to_string(),
        }),
        _ => Err(WebError::NotFound(format!("not found: {path}"))),
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
            let json =
                registry::get_package_version_inner(&state, &fullname, &version).await?;
            Ok(json.into_response())
        }
        PackageRoute::Tarball { fullname, filename } => {
            tarball::download_tarball_inner(&state, &fullname, &filename).await
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

    let PackageRoute::Package { fullname } = route else {
        return Err(WebError::NotFound(format!(
            "PUT not supported for: {path}"
        )));
    };

    let auth = validate_auth(&state, &headers).await?;

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
