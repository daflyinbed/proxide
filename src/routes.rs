use crate::handlers;
use crate::state::AppState;
use axum::routing::{get, put};
use axum::Router;

pub fn build_router(state: AppState) -> Router {
    let npm = Router::new()
        .route("/", get(handlers::registry::registry_root))
        .route(
            "/-/package/{fullname}/syncs",
            put(handlers::sync::trigger_sync),
        )
        .route(
            "/{fullname}/-/{filename}",
            get(handlers::tarball::download_tarball),
        )
        .route(
            "/{fullname}/{version}",
            get(handlers::registry::get_package_version),
        )
        .route("/{fullname}", get(handlers::registry::get_package));

    let fast = Router::new()
        .route("/resolve/{pkg}", get(handlers::fast_meta::resolve_version))
        .route("/versions/{pkg}", get(handlers::fast_meta::get_versions))
        .route("/full/{pkg}", get(handlers::fast_meta::get_full));

    Router::new()
        .route("/-/ping", get(handlers::home::ping))
        .nest("/npm", npm)
        .nest("/fast", fast)
        .with_state(state)
}
