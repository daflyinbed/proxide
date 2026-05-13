use crate::handlers;
use crate::middleware::auth::require_auth;
use crate::state::AppState;
use axum::routing::{get, post, put};
use axum::Router;

pub fn build_router(state: AppState) -> Router {
    let npm = Router::new()
        .route("/", get(handlers::registry::registry_root))
        .route(
            "/-/user/org.couchdb.user:{name}",
            put(handlers::auth::login),
        )
        .route("/-/v1/login", post(handlers::cas::init_login))
        .route(
            "/-/v1/login/request/session/{sessionId}",
            get(handlers::cas::cas_callback),
        )
        .route(
            "/-/v1/login/done/session/{sessionId}",
            get(handlers::cas::poll_done),
        )
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
        .route("/{fullname}", get(handlers::registry::get_package))
        .route(
            "/{fullname}",
            put(handlers::publish::publish_package).layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth,
            )),
        );

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
