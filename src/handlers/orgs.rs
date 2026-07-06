use crate::error::{WebError, WebResult};
use crate::middleware::auth::{AuthContext, OptionalAuth, RequireAuth, is_admin};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

const DEVELOPERS_TEAM: &str = "developers";

#[derive(Debug, Deserialize, ToSchema)]
pub struct OrgMemberRequest {
    pub user: String,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrgSummary {
    pub name: String,
    pub size: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MembershipDetail {
    pub org: OrgSummary,
    pub user: String,
    pub role: String,
}

fn validate_role(role: &str) -> WebResult<()> {
    match role {
        "owner" | "admin" | "developer" => Ok(()),
        other => Err(WebError::BadRequest(format!(
            "invalid role: \"{other}\", must be one of: owner, admin, developer"
        ))),
    }
}

pub async fn require_org_manager(
    state: &AppState,
    auth: &AuthContext,
    org_name: &str,
) -> WebResult<()> {
    if is_admin(&auth.user, &state.config.auth.admins) {
        return Ok(());
    }
    let org_row = state
        .repo
        .get_org_by_name(org_name)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Organization \"{org_name}\" not found")))?;
    let row = state
        .repo
        .get_org_member(org_row.id, auth.user.id)
        .await
        .map_err(WebError::CustomApiError)?;
    match row.as_ref().map(|r| r.role.as_str()) {
        Some("owner") | Some("admin") => Ok(()),
        _ => Err(WebError::Forbidden(format!(
            "\"{}\" is not an owner or admin of organization \"{org_name}\"",
            auth.user.name
        ))),
    }
}

pub async fn auto_add_to_developers(
    state: &AppState,
    org_id: i64,
    user_id: i64,
) -> WebResult<()> {
    if let Some(team) = state
        .repo
        .get_team_by_org_name(org_id, DEVELOPERS_TEAM)
        .await
        .map_err(WebError::CustomApiError)?
    {
        state
            .repo
            .add_team_member(team.id, user_id)
            .await
            .map_err(WebError::CustomApiError)?;
    }
    Ok(())
}

#[utoipa::path(
    get,
    tag = "org",
    path = "/-/org/{org}/user",
    params(
        ("org" = String, Path, description = "Organization name (scope without @)"),
    ),
    responses(
        (status = OK, description = "Roster: username → role", body = serde_json::Value),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn roster(
    State(state): State<AppState>,
    OptionalAuth(_auth): OptionalAuth,
    Path(org): Path<String>,
) -> WebResult<Json<serde_json::Value>> {
    let org_row = state
        .repo
        .get_org_by_name(&org)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Organization \"{org}\" not found")))?;

    let members = state
        .repo
        .list_org_member_roster(org_row.id)
        .await
        .map_err(WebError::CustomApiError)?;

    let res: BTreeMap<String, String> = members.into_iter().collect();
    Ok(Json(serde_json::to_value(res).unwrap()))
}

#[utoipa::path(
    put,
    tag = "org",
    path = "/-/org/{org}/user",
    params(
        ("org" = String, Path, description = "Organization name (scope without @)"),
    ),
    request_body = OrgMemberRequest,
    responses(
        (status = CREATED, description = "Member added/updated", body = MembershipDetail),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn set_member(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path(org_name): Path<String>,
    Json(body): Json<OrgMemberRequest>,
) -> WebResult<(StatusCode, Json<MembershipDetail>)> {
    require_org_manager(&state, &auth, &org_name).await?;

    let role = body.role.as_deref().unwrap_or("developer");
    validate_role(role)?;

    let org_row = state
        .repo
        .get_org_by_name(&org_name)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Organization \"{org_name}\" not found")))?;

    let user = state
        .repo
        .get_user_by_name(&body.user)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| {
            WebError::BadRequest(format!(
                "User \"{}\" does not exist; users must register before being added to an org",
                body.user
            ))
        })?;

    let existed = state
        .repo
        .get_org_member(org_row.id, user.id)
        .await
        .map_err(WebError::CustomApiError)?
        .is_some();

    state
        .repo
        .add_org_member(org_row.id, user.id, role)
        .await
        .map_err(WebError::CustomApiError)?;

    if !existed {
        auto_add_to_developers(&state, org_row.id, user.id).await?;
    }

    let size = state
        .repo
        .count_org_members(org_row.id)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(
        action = "org_set_member";
        "org={org_name} user={} role={role} actor={}",
        body.user, auth.user.name
    );

    Ok((
        StatusCode::CREATED,
        Json(MembershipDetail {
            org: OrgSummary {
                name: org_name,
                size,
            },
            user: body.user,
            role: role.to_string(),
        }),
    ))
}

#[utoipa::path(
    delete,
    tag = "org",
    path = "/-/org/{org}/user",
    params(
        ("org" = String, Path, description = "Organization name (scope without @)"),
    ),
    request_body = OrgMemberRequest,
    responses(
        (status = NO_CONTENT, description = "Member removed"),
        (status = BAD_REQUEST, body = crate::error::ApiErrorDetail),
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = CONFLICT, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn rm_member(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path(org_name): Path<String>,
    Json(body): Json<OrgMemberRequest>,
) -> WebResult<StatusCode> {
    require_org_manager(&state, &auth, &org_name).await?;

    let org_row = state
        .repo
        .get_org_by_name(&org_name)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Organization \"{org_name}\" not found")))?;

    let user = state
        .repo
        .get_user_by_name(&body.user)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("User \"{}\" not found", body.user)))?;

    let existing = state
        .repo
        .get_org_member(org_row.id, user.id)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| {
            WebError::NotFound(format!(
                "User \"{}\" is not a member of organization \"{org_name}\"",
                body.user
            ))
        })?;

    if existing.role == "owner" {
        let owner_count = state
            .repo
            .count_org_owners(org_row.id)
            .await
            .map_err(WebError::CustomApiError)?;
        if owner_count <= 1 {
            return Err(WebError::Conflict(format!(
                "cannot remove the last owner of organization \"{org_name}\""
            )));
        }
    }

    state
        .repo
        .remove_org_member_cascade(org_row.id, user.id)
        .await
        .map_err(WebError::CustomApiError)?;

    log::info!(
        action = "org_rm_member";
        "org={org_name} user={} actor={}",
        body.user, auth.user.name
    );

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    tag = "org",
    path = "/-/org/{org}/package",
    params(
        ("org" = String, Path, description = "Organization name (scope without @)"),
    ),
    responses(
        (status = OK, description = "Packages in the org viewable by current user", body = serde_json::Value),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail, description = "Org not found (triggers client fallback to /-/user/{x}/package)"),
    ),
)]
pub async fn org_packages(
    State(state): State<AppState>,
    OptionalAuth(auth): OptionalAuth,
    Path(org): Path<String>,
) -> WebResult<Json<serde_json::Value>> {
    let org_row = match state
        .repo
        .get_org_by_name(&org)
        .await
        .map_err(WebError::CustomApiError)?
    {
        Some(o) => o,
        None => {
            return Err(WebError::NotFound(format!(
                "Organization \"{org}\" not found"
            )));
        }
    };

    let viewer_id = auth.as_ref().map(|a| a.user.id).unwrap_or(0);

    let pkgs = state
        .repo
        .list_packages_in_org_viewable(org_row.id, viewer_id)
        .await
        .map_err(WebError::CustomApiError)?;

    let pkg_ids: Vec<i64> = pkgs.iter().map(|p| p.id).collect();
    let perm_map = state
        .repo
        .list_package_max_permissions(&pkg_ids)
        .await
        .map_err(WebError::CustomApiError)?;

    let mut res: BTreeMap<String, String> = BTreeMap::new();
    for pkg in pkgs {
        let level = if perm_map.get(&pkg.id).copied().unwrap_or(false) {
            "write"
        } else {
            "read"
        };
        res.insert(pkg.name, level.to_string());
    }
    Ok(Json(serde_json::to_value(res).unwrap()))
}

