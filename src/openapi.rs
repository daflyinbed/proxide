use utoipa::OpenApi;

use crate::error::ApiErrorDetail;
use crate::npm::types::{AbbreviatedPackument, Packument, PublishAttachment, PublishDist, PublishResponse};

#[utoipa::path(
    get,
    tag = "registry",
    path = "/npm/{fullname}",
    params(
        ("fullname" = String, Path, description = "Package full name, e.g. lodash or @babel/core"),
        ("Accept" = String, Header, description = "Send application/vnd.npm.install-v1+json for the abbreviated packument"),
    ),
    responses(
        (status = OK, description = "Full packument", body = Packument, content_type = "application/json"),
        (status = OK, description = "Abbreviated packument", body = AbbreviatedPackument, content_type = "application/vnd.npm.install-v1+json"),
        (status = NOT_FOUND, body = ApiErrorDetail),
        (status = INTERNAL_SERVER_ERROR, body = ApiErrorDetail),
    ),
)]
#[allow(dead_code)]
fn _get_package_packument() {}

#[utoipa::path(
    get,
    tag = "registry",
    path = "/npm/{fullname}/{version}",
    params(
        ("fullname" = String, Path, description = "Package full name"),
        ("version" = String, Path, description = "Semantic version"),
    ),
    responses(
        (status = OK, description = "Version manifest (package.json)", body = serde_json::Value, content_type = "application/json"),
        (status = NOT_FOUND, body = ApiErrorDetail),
        (status = INTERNAL_SERVER_ERROR, body = ApiErrorDetail),
    ),
)]
#[allow(dead_code)]
fn _get_package_version() {}

#[utoipa::path(
    get,
    tag = "registry",
    path = "/npm/{fullname}/-/{filename}",
    params(
        ("fullname" = String, Path, description = "Package full name"),
        ("filename" = String, Path, description = "Tarball filename, e.g. lodash-4.17.21.tgz"),
    ),
    responses(
        (status = OK, description = "Tarball binary stream", content_type = "application/octet-stream"),
        (status = BAD_REQUEST, body = ApiErrorDetail),
        (status = NOT_FOUND, body = ApiErrorDetail),
        (status = INTERNAL_SERVER_ERROR, body = ApiErrorDetail),
    ),
)]
#[allow(dead_code)]
fn _download_tarball() {}

#[utoipa::path(
    put,
    tag = "registry",
    path = "/npm/{fullname}",
    params(
        ("fullname" = String, Path, description = "Package full name"),
    ),
    request_body = serde_json::Value,
    responses(
        (status = OK, description = "Package published", body = PublishResponse),
        (status = BAD_REQUEST, body = ApiErrorDetail),
        (status = UNAUTHORIZED, body = ApiErrorDetail),
        (status = FORBIDDEN, body = ApiErrorDetail),
        (status = CONFLICT, body = ApiErrorDetail),
        (status = INTERNAL_SERVER_ERROR, body = ApiErrorDetail),
    ),
)]
#[allow(dead_code)]
fn _publish_package() {}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Proxide",
        description = "NPM registry mirror API",
        license(
            name = "MIT",
        ),
    ),
    tags(
        (name = "misc", description = "Health & misc endpoints"),
        (name = "registry", description = "NPM registry protocol endpoints"),
        (name = "auth", description = "Authentication (legacy login, web/CAS login)"),
        (name = "search", description = "Package search"),
        (name = "fast-meta", description = "fast-npm-meta protocol"),
        (name = "downloads", description = "npm download-counts API"),
        (name = "cdn", description = "jsDelivr-compatible CDN & data API"),
    ),
    paths(
        _get_package_packument,
        _get_package_version,
        _download_tarball,
        _publish_package,
    ),
    components(schemas(PublishDist, PublishAttachment, PublishResponse, ApiErrorDetail)),
)]
pub struct ApiDoc;
