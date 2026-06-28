use axum::{Json, debug_handler};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct Ping {}

#[debug_handler]
#[utoipa::path(
    get,
    tag = "misc",
    path = "/-/ping",
    responses((status = OK, body = Ping)),
)]
pub async fn ping() -> Json<Ping> {
    Json(Ping {})
}
