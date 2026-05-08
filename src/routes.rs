use crate::handlers;
use crate::state::AppState;
use axum::Router;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_scalar::{Scalar, Servable};

#[derive(OpenApi)]
#[openapi(tags())]
pub struct ApiDoc;

pub fn build_router(state: AppState) -> Router {
    let (api_routes, openapi) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(handlers::home::ping))
        .split_for_parts();
    Router::new()
        .merge(Scalar::with_url("/scalar", openapi.clone()))
        .merge(api_routes)
        .with_state(state)
}
