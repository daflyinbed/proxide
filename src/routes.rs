use crate::handlers;
use crate::state::AppState;
use axum::Router;
use axum::routing::{get, post, put};

pub fn build_router(state: AppState) -> Router {
    let npm = Router::new()
        .route("/", get(handlers::registry::registry_root))
        .route(
            "/-/user/org.couchdb.user:{name}",
            put(handlers::auth::login),
        )
        .route("/-/v1/login", post(handlers::web_login::init_login))
        .route(
            "/-/v1/login/done/session/{sessionId}",
            get(handlers::web_login::poll_done),
        )
        .route(
            "/-/package/{fullname}/syncs",
            put(handlers::sync::trigger_sync),
        )
        .route("/-/v1/search", get(handlers::search::search_packages))
        .fallback(
            get(handlers::package_dispatch::dispatch_get)
                .put(handlers::package_dispatch::dispatch_put),
        );

    let fast = Router::new()
        .route("/resolve/{pkg}", get(handlers::fast_meta::resolve_version))
        .route("/versions/{pkg}", get(handlers::fast_meta::get_versions))
        .route("/full/{pkg}", get(handlers::fast_meta::get_full));

    let api = Router::new()
        .route(
            "/auth/cas/callback/session/{sessionId}",
            get(handlers::sso::cas::cas_callback),
        )
        .route(
            "/downloads/point/{*rest}",
            get(handlers::downloads::downloads_point),
        )
        .route(
            "/downloads/range/{*rest}",
            get(handlers::downloads::downloads_range),
        );

    Router::new()
        .route("/-/ping", get(handlers::home::ping))
        .nest("/npm", npm)
        .nest("/fast", fast)
        .nest("/api", api)
        .with_state(state)
}
