use utoipa::OpenApi;

use crate::error::ApiErrorDetail;
use crate::handlers;
use crate::handlers::downloads::{DayDownloads, DownloadsPoint, DownloadsRange};
use crate::handlers::home::Ping;
use crate::handlers::search::{SearchResponse};
use crate::handlers::web_login::{LoginRequestBody, WebLoginResponse};
use crate::npm::types::{
    Dist, FastMetaFull, FastMetaResolved, FastMetaVersions, LoginPayload, LoginResponse,
    Maintainer, PeerDepMeta, Person, PublishAttachment, PublishDist, PublishResponse, RegistryInfo,
    SyncResponse, VersionMeta,
};
use crate::search::document::{
    AuthorDoc, DownloadsDoc, MaintainerDoc, NpmUserDoc, PackageDoc, SearchDocument,
};

#[utoipa::path(
    get,
    tag = "registry",
    path = "/npm/{fullname}",
    params(
        ("fullname" = String, Path, description = "Package full name, e.g. lodash or @babel/core"),
    ),
    responses(
        (status = OK, description = "Package packument (full or abbreviated manifest)", body = serde_json::Value, content_type = "application/json"),
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
    security(("bearerAuth" = [])),
)]
#[allow(dead_code)]
fn _publish_package() {}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Proxide",
        version = "0.1.0",
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
        handlers::home::ping,
        handlers::registry::registry_root,
        handlers::auth::login,
        handlers::web_login::init_login,
        handlers::web_login::poll_done,
        handlers::sync::trigger_sync,
        handlers::search::search_packages,
        handlers::fast_meta::resolve_version,
        handlers::fast_meta::get_versions,
        handlers::fast_meta::get_full,
        handlers::downloads::downloads_point,
        handlers::downloads::downloads_range,
        handlers::sso::cas::cas_callback,
        handlers::cdn::serve_file,
        handlers::data_api::version_files,
        _get_package_packument,
        _get_package_version,
        _download_tarball,
        _publish_package,
    ),
    components(
        schemas(
            Ping,
            RegistryInfo,
            LoginPayload,
            LoginResponse,
            LoginRequestBody,
            WebLoginResponse,
            SyncResponse,
            SearchResponse,
            SearchDocument,
            PackageDoc,
            MaintainerDoc,
            AuthorDoc,
            NpmUserDoc,
            DownloadsDoc,
            FastMetaResolved,
            FastMetaVersions,
            FastMetaFull,
            VersionMeta,
            DownloadsPoint,
            DownloadsRange,
            DayDownloads,
            PublishDist,
            PublishAttachment,
            PublishResponse,
            Dist,
            Maintainer,
            Person,
            PeerDepMeta,
            ApiErrorDetail,
        )
    ),
)]
pub struct ApiDoc;
