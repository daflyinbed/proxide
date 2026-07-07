use crate::error::{WebError, WebResult};
use crate::handlers::orgs::{require_org_manager, require_org_member};
use crate::middleware::auth::RequireAuth;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

const DEVELOPERS_TEAM: &str = "developers";

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTeamRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TeamCreatedResponse {
    pub name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TeamMemberRequest {
    pub user: String,
}

#[derive(Debug, Deserialize)]
pub struct FormatQuery {
    #[serde(default)]
    pub format: Option<String>,
}

fn validate_team_name(name: &str) -> WebResult<()> {
    if name == DEVELOPERS_TEAM {
        return Err(WebError::BadRequest(
            "team name \"developers\" is reserved".to_string(),
        ));
    }
    let valid = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !valid
        || name.is_empty()
        || !name.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false)
    {
        return Err(WebError::BadRequest(
            "team names must be lowercase, start with a letter, and contain only lowercase letters, digits, hyphens, or underscores".to_string(),
        ));
    }
    Ok(())
}

async fn resolve_team_by_org(
    state: &AppState,
    org_id: i64,
    scope: &str,
    team_name: &str,
) -> WebResult<crate::repository::TeamRow> {
    state
        .repo
        .get_team_by_org_name(org_id, team_name)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Team \"{scope}:{team_name}\" not found")))
}

#[utoipa::path(
    put,
    tag = "team",
    path = "/-/org/{scope}/team",
    params(
        ("scope" = String, Path, description = "Organization scope (without @)"),
    ),
    request_body = CreateTeamRequest,
    responses(
        (status = CREATED, description = "Team created", body = TeamCreatedResponse),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = CONFLICT, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn create_team(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path(scope): Path<String>,
    Json(body): Json<CreateTeamRequest>,
) -> WebResult<(StatusCode, Json<TeamCreatedResponse>)> {
    let org = require_org_manager(&state, &auth, &scope).await?;
    validate_team_name(&body.name)?;

    if state
        .repo
        .get_team_by_org_name(org.id, &body.name)
        .await
        .map_err(WebError::CustomApiError)?
        .is_some()
    {
        return Err(WebError::Conflict(format!(
            "Team \"{scope}:{}\" already exists",
            body.name
        )));
    }

    state
        .repo
        .create_team(org.id, &body.name, body.description.as_deref())
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(
        action = "team_create";
        "scope={scope} team={} actor={}",
        body.name, auth.user.name
    );

    Ok((
        StatusCode::CREATED,
        Json(TeamCreatedResponse { name: body.name }),
    ))
}

#[utoipa::path(
    delete,
    tag = "team",
    path = "/-/team/{scope}/{team}",
    params(
        ("scope" = String, Path, description = "Organization scope (without @)"),
        ("team" = String, Path, description = "Team name"),
    ),
    responses(
        (status = NO_CONTENT, description = "Team destroyed"),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn destroy_team(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path((scope, team)): Path<(String, String)>,
) -> WebResult<StatusCode> {
    let org = require_org_manager(&state, &auth, &scope).await?;

    if team == DEVELOPERS_TEAM {
        return Err(WebError::BadRequest(format!(
            "the \"{DEVELOPERS_TEAM}\" team cannot be removed"
        )));
    }

    let team_row = resolve_team_by_org(&state, org.id, &scope, &team).await?;

    state
        .repo
        .delete_team(team_row.id)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(
        action = "team_destroy";
        "scope={scope} team={team} actor={}",
        auth.user.name
    );

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    tag = "team",
    path = "/-/team/{scope}/{team}/user",
    params(
        ("scope" = String, Path, description = "Organization scope (without @)"),
        ("team" = String, Path, description = "Team name"),
    ),
    request_body = TeamMemberRequest,
    responses(
        (status = CREATED, description = "User added to team", body = serde_json::Value),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn add_user(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path((scope, team)): Path<(String, String)>,
    Json(body): Json<TeamMemberRequest>,
) -> WebResult<(StatusCode, Json<serde_json::Value>)> {
    let org = require_org_manager(&state, &auth, &scope).await?;
    let team_row = resolve_team_by_org(&state, org.id, &scope, &team).await?;

    let user = state
        .repo
        .get_user_by_name(&body.user)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("User \"{}\" not found", body.user)))?;

    let is_member = state
        .repo
        .get_org_member(org.id, user.id)
        .await
        .map_err(WebError::CustomApiError)?
        .is_some();
    if !is_member {
        return Err(WebError::Forbidden(format!(
            "User \"{}\" must be a member of organization \"{scope}\" before being added to a team",
            body.user
        )));
    }

    state
        .repo
        .add_team_member(team_row.id, user.id)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(
        action = "team_add_user";
        "scope={scope} team={team} user={} actor={}",
        body.user, auth.user.name
    );

    Ok((StatusCode::CREATED, Json(serde_json::json!({}))))
}

#[utoipa::path(
    delete,
    tag = "team",
    path = "/-/team/{scope}/{team}/user",
    params(
        ("scope" = String, Path, description = "Organization scope (without @)"),
        ("team" = String, Path, description = "Team name"),
    ),
    request_body = TeamMemberRequest,
    responses(
        (status = NO_CONTENT, description = "User removed from team"),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn rm_user(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path((scope, team)): Path<(String, String)>,
    Json(body): Json<TeamMemberRequest>,
) -> WebResult<StatusCode> {
    let org = require_org_manager(&state, &auth, &scope).await?;

    if team == DEVELOPERS_TEAM {
        return Err(WebError::BadRequest(format!(
            "the \"{DEVELOPERS_TEAM}\" team membership is managed automatically and cannot be removed per-user"
        )));
    }

    let team_row = resolve_team_by_org(&state, org.id, &scope, &team).await?;

    let user = state
        .repo
        .get_user_by_name(&body.user)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("User \"{}\" not found", body.user)))?;

    state
        .repo
        .remove_team_member(team_row.id, user.id)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(
        action = "team_rm_user";
        "scope={scope} team={team} user={} actor={}",
        body.user, auth.user.name
    );

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    tag = "team",
    path = "/-/org/{scope}/team",
    params(
        ("scope" = String, Path, description = "Organization scope (without @)"),
        ("format" = Option<String>, Query, description = "Use \"cli\" for flat array output"),
    ),
    responses(
        (status = OK, description = "Teams in scope", body = serde_json::Value),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn list_teams(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path(scope): Path<String>,
    Query(query): Query<FormatQuery>,
) -> WebResult<Json<serde_json::Value>> {
    let org = require_org_member(&state, &auth, &scope).await?;
    let teams = state
        .repo
        .list_teams_in_org(org.id)
        .await
        .map_err(WebError::CustomApiError)?;

    if query.format.as_deref() == Some("cli") {
        let names: Vec<String> = teams.into_iter().map(|t| format!("{scope}:{}", t.name)).collect();
        Ok(Json(serde_json::to_value(names).unwrap()))
    } else {
        let mut res: BTreeMap<String, String> = BTreeMap::new();
        for t in teams {
            res.insert(t.name, t.description.unwrap_or_default());
        }
        Ok(Json(serde_json::to_value(res).unwrap()))
    }
}

#[utoipa::path(
    get,
    tag = "team",
    path = "/-/team/{scope}/{team}/user",
    params(
        ("scope" = String, Path, description = "Organization scope (without @)"),
        ("team" = String, Path, description = "Team name"),
        ("format" = Option<String>, Query, description = "Use \"cli\" for flat array output"),
    ),
    responses(
        (status = OK, description = "Team members", body = serde_json::Value),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn list_users(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path((scope, team)): Path<(String, String)>,
    Query(query): Query<FormatQuery>,
) -> WebResult<Json<serde_json::Value>> {
    let org = require_org_member(&state, &auth, &scope).await?;
    let team_row = resolve_team_by_org(&state, org.id, &scope, &team).await?;
    let mut names = state
        .repo
        .list_team_member_names(team_row.id)
        .await
        .map_err(WebError::CustomApiError)?;
    names.sort();

    if query.format.as_deref() == Some("cli") {
        Ok(Json(serde_json::to_value(names).unwrap()))
    } else {
        let map: BTreeMap<String, String> = names.into_iter().map(|n| (n, String::new())).collect();
        Ok(Json(serde_json::to_value(map).unwrap()))
    }
}
