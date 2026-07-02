use crate::error::{WebError, WebResult};
use crate::middleware::auth::{is_admin, validate_auth_any};
use crate::search::SearchDocument;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

const DEFAULT_SIZE: usize = 20;
const MAX_SIZE: usize = 250;
const MAX_FROM: usize = 10000;

#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchQuery {
    pub text: String,
    #[serde(default)]
    pub from: usize,
    #[serde(default = "default_size")]
    pub size: usize,
}

fn default_size() -> usize {
    DEFAULT_SIZE
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    pub objects: Vec<SearchDocument>,
    pub total: usize,
}

#[utoipa::path(
    get,
    tag = "search",
    path = "/-/v1/search",
    params(SearchQuery),
    responses(
        (status = OK, description = "Search results", body = SearchResponse),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = NOT_IMPLEMENTED, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn search_packages(
    State(state): State<AppState>,
    headers: HeaderMap,
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

    let from = q.from.min(MAX_FROM);
    let size = if q.size == 0 {
        DEFAULT_SIZE
    } else {
        q.size.min(MAX_SIZE)
    };

    let filter = match validate_auth_any(&state, &headers).await {
        Ok(auth) => {
            if is_admin(&auth.user, &state.config.auth.admins) {
                None
            } else {
                let escaped = escape_meili_filter_string(&auth.user.name);
                Some(format!(
                    r#"package.access = "public" OR package.access NOT EXISTS OR package.maintainers.name = "{escaped}""#
                ))
            }
        }
        Err(WebError::Unauthorized(_)) => {
            Some(r#"package.access = "public" OR package.access NOT EXISTS"#.to_string())
        }
        Err(e) => return Err(e),
    };

    let results = idx
        .search(text, from, size, filter.as_deref())
        .await
        .map_err(WebError::CustomApiError)?;

    Ok(Json(SearchResponse {
        objects: results.hits.into_iter().map(|h| h.result).collect(),
        total: results.estimated_total_hits.unwrap_or(0),
    }))
}

fn escape_meili_filter_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
