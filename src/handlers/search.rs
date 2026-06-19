use crate::error::{WebError, WebResult};
use crate::search::SearchDocument;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    #[serde(default)]
    pub from: usize,
    #[serde(default = "default_size")]
    pub size: usize,
}

fn default_size() -> usize {
    20
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub objects: Vec<SearchDocument>,
    pub total: usize,
}

pub async fn search_packages(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> WebResult<Json<SearchResponse>> {
    let text = q.text.trim();
    if text.is_empty() {
        return Err(WebError::BadRequest("text is required".to_string()));
    }
    let Some(idx) = &state.search else {
        return Err(WebError::NotImplemented(
            "search is not enabled".to_string(),
        ));
    };

    let results = idx
        .search(text, q.from, q.size)
        .await
        .map_err(WebError::CustomApiError)?;

    Ok(Json(SearchResponse {
        objects: results.hits.into_iter().map(|h| h.result).collect(),
        total: results.estimated_total_hits.unwrap_or(0),
    }))
}
