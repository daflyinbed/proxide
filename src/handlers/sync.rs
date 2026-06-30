use crate::error::{WebError, WebResult};
use crate::npm::types::SyncResponse;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};

#[utoipa::path(
    put,
    tag = "registry",
    path = "/-/package/{fullname}/syncs",
    params(
        ("fullname" = String, Path, description = "Full package name"),
    ),
    responses(
        (status = OK, description = "Sync task enqueued", body = SyncResponse),
        (status = INTERNAL_SERVER_ERROR, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn trigger_sync(
    State(state): State<AppState>,
    Path(fullname): Path<String>,
) -> WebResult<Json<SyncResponse>> {
    match state.repo.enqueue_sync_task(&fullname, "api").await {
        Ok(Some(task_id)) => {
            log::info!(action = "enqueue"; "enqueued sync task source=api name={fullname} task_id={task_id}");
            Ok(Json(SyncResponse {
                ok: true,
                log: "queued".to_string(),
            }))
        }
        Ok(None) => {
            log::debug!(action = "enqueue_dedup"; "skipped, pending task exists source=api name={fullname}");
            Ok(Json(SyncResponse {
                ok: true,
                log: "already queued".to_string(),
            }))
        }
        Err(e) => Err(WebError::CustomApiError(e)),
    }
}
