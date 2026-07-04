use crate::handlers;
use crate::openapi::ApiDoc;
use crate::state::AppState;
use utoipa::OpenApi;
use utoipa_axum::{routes, router::OpenApiRouter};
use utoipa_scalar::{Scalar, Servable};

pub fn build_router(state: AppState) -> axum::Router {
    let npm = OpenApiRouter::new()
        .routes(routes!(handlers::registry::registry_root))
        .routes(routes!(handlers::auth::login, handlers::auth::show_user))
        .routes(routes!(handlers::web_login::init_login))
        .routes(routes!(handlers::web_login::poll_done))
        .routes(routes!(handlers::sync::trigger_sync))
        .routes(routes!(handlers::dist_tags::list_dist_tags))
        .routes(routes!(handlers::dist_tags::set_dist_tag, handlers::dist_tags::remove_dist_tag))
        .routes(routes!(handlers::search::search_packages))
        .routes(routes!(handlers::tokens::whoami))
        .routes(routes!(handlers::tokens::logout))
        .routes(routes!(handlers::tokens::list_tokens, handlers::tokens::create_token))
        .routes(routes!(handlers::tokens::revoke_token))
        .routes(routes!(handlers::profile::get_profile, handlers::profile::update_profile))
        .routes(routes!(handlers::access::list_collaborators))
        .routes(routes!(handlers::access::get_visibility))
        .routes(routes!(handlers::access::set_access))
        .routes(routes!(handlers::access::list_packages_by_user))
        .fallback(handlers::package_dispatch::dispatch);

    let fast = OpenApiRouter::new()
        .routes(routes!(handlers::fast_meta::resolve_version))
        .routes(routes!(handlers::fast_meta::get_versions))
        .routes(routes!(handlers::fast_meta::get_full));

    let api = OpenApiRouter::new()
        .routes(routes!(handlers::sso::cas::cas_callback))
        .routes(routes!(handlers::downloads::downloads_point))
        .routes(routes!(handlers::downloads::downloads_range));

    let jsdelivr_npm = OpenApiRouter::new().routes(routes!(handlers::cdn::serve_file));
    let jsdelivr_api = OpenApiRouter::new().routes(routes!(handlers::data_api::version_files));

    let router = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(handlers::home::ping))
        .nest("/npm", npm)
        .nest("/fast", fast)
        .nest("/api", api)
        .nest("/jsdelivr/npm", jsdelivr_npm)
        .nest("/jsdelivr/api/npm", jsdelivr_api);

    let (router, openapi) = router.split_for_parts();
    router
        .merge(Scalar::with_url("/docs", openapi))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use utoipa::OpenApi;

    fn collect_paths() -> Vec<String> {
        let mut router = OpenApiRouter::with_openapi(ApiDoc::openapi())
            .routes(routes!(handlers::home::ping));
        let npm = OpenApiRouter::new()
            .routes(routes!(handlers::registry::registry_root))
        .routes(routes!(handlers::auth::login, handlers::auth::show_user))
            .routes(routes!(handlers::web_login::init_login))
            .routes(routes!(handlers::web_login::poll_done))
            .routes(routes!(handlers::sync::trigger_sync))
            .routes(routes!(handlers::dist_tags::list_dist_tags))
            .routes(routes!(handlers::dist_tags::set_dist_tag, handlers::dist_tags::remove_dist_tag))
            .routes(routes!(handlers::search::search_packages))
            .routes(routes!(handlers::tokens::whoami))
            .routes(routes!(handlers::tokens::logout))
            .routes(routes!(handlers::tokens::list_tokens, handlers::tokens::create_token))
            .routes(routes!(handlers::tokens::revoke_token))
            .routes(routes!(handlers::profile::get_profile, handlers::profile::update_profile))
            .routes(routes!(handlers::access::list_collaborators))
            .routes(routes!(handlers::access::get_visibility))
            .routes(routes!(handlers::access::set_access))
            .routes(routes!(handlers::access::list_packages_by_user));
        let fast = OpenApiRouter::new()
            .routes(routes!(handlers::fast_meta::resolve_version))
            .routes(routes!(handlers::fast_meta::get_versions))
            .routes(routes!(handlers::fast_meta::get_full));
        let api = OpenApiRouter::new()
            .routes(routes!(handlers::sso::cas::cas_callback))
            .routes(routes!(handlers::downloads::downloads_point))
            .routes(routes!(handlers::downloads::downloads_range));
        let jsdelivr_npm = OpenApiRouter::new().routes(routes!(handlers::cdn::serve_file));
        let jsdelivr_api = OpenApiRouter::new().routes(routes!(handlers::data_api::version_files));
        router = router
            .nest("/npm", npm)
            .nest("/fast", fast)
            .nest("/api", api)
            .nest("/jsdelivr/npm", jsdelivr_npm)
            .nest("/jsdelivr/api/npm", jsdelivr_api);
        router
            .to_openapi()
            .paths
            .paths
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn openapi_paths_resolved_with_nest_prefix() {
        let paths = collect_paths();
        let expected = [
            "/-/ping",
            "/npm",
            "/npm/-/user/org.couchdb.user:{name}",
            "/npm/-/user/token/{token}",
            "/npm/-/whoami",
            "/npm/-/v1/login",
            "/npm/-/v1/login/done/session/{sessionId}",
            "/npm/-/package/{fullname}/syncs",
            "/npm/-/package/{fullname}/dist-tags",
            "/npm/-/package/{fullname}/dist-tags/{tag}",
            "/npm/-/v1/search",
            "/npm/-/npm/v1/tokens",
            "/npm/-/npm/v1/tokens/token/{key}",
            "/npm/-/npm/v1/user",
            "/npm/-/package/{fullname}/collaborators",
            "/npm/-/package/{fullname}/visibility",
            "/npm/-/package/{fullname}/access",
            "/npm/-/org/{username}/package",
            "/npm/{fullname}",
            "/npm/{fullname}/{version}",
            "/npm/{fullname}/-/{filename}",
            "/fast/resolve/{pkg}",
            "/fast/versions/{pkg}",
            "/fast/full/{pkg}",
            "/api/auth/cas/callback/session/{sessionId}",
            "/api/downloads/point/{*rest}",
            "/api/downloads/range/{*rest}",
            "/jsdelivr/npm/{*rest}",
            "/jsdelivr/api/npm/{*rest}",
        ];
        for e in expected {
            assert!(paths.contains(&e.to_string()), "missing OpenAPI path: {e}\ngot: {paths:?}");
        }
    }
}
