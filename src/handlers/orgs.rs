use crate::error::{WebError, WebResult};
use crate::middleware::auth::{AuthContext, OptionalAuth, RequireAuth, is_admin};
use crate::repository::OrganizationRow;
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
) -> WebResult<OrganizationRow> {
    let org_row = state
        .repo
        .get_org_by_name(org_name)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Organization \"{org_name}\" not found")))?;
    if is_admin(&auth.user, &state.config.auth.admins) {
        return Ok(org_row);
    }
    let row = state
        .repo
        .get_org_member(org_row.id, auth.user.id)
        .await
        .map_err(WebError::CustomApiError)?;
    match row.as_ref().map(|r| r.role.as_str()) {
        Some("owner") | Some("admin") => Ok(org_row),
        _ => Err(WebError::Forbidden(format!(
            "\"{}\" is not an owner or admin of organization \"{org_name}\"",
            auth.user.name
        ))),
    }
}

pub async fn require_org_member(
    state: &AppState,
    auth: &AuthContext,
    org_name: &str,
) -> WebResult<OrganizationRow> {
    let org_row = state
        .repo
        .get_org_by_name(org_name)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("Organization \"{org_name}\" not found")))?;
    if is_admin(&auth.user, &state.config.auth.admins) {
        return Ok(org_row);
    }
    let is_member = state
        .repo
        .get_org_member(org_row.id, auth.user.id)
        .await
        .map_err(WebError::CustomApiError)?
        .is_some();
    if is_member {
        Ok(org_row)
    } else {
        Err(WebError::Forbidden(format!(
            "\"{}\" is not a member of organization \"{org_name}\"",
            auth.user.name
        )))
    }
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
        (status = UNAUTHORIZED, body = crate::error::ApiErrorDetail),
        (status = FORBIDDEN, body = crate::error::ApiErrorDetail),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail),
    ),
)]
pub async fn roster(
    State(state): State<AppState>,
    RequireAuth(auth): RequireAuth,
    Path(org): Path<String>,
) -> WebResult<Json<serde_json::Value>> {
    let org_row = require_org_member(&state, &auth, &org).await?;

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
    let org_row = require_org_manager(&state, &auth, &org_name).await?;

    let role = body.role.as_deref().unwrap_or("developer");
    validate_role(role)?;

    if role == "owner" && !is_admin(&auth.user, &state.config.auth.admins) {
        let actor_member = state
            .repo
            .get_org_member(org_row.id, auth.user.id)
            .await
            .map_err(WebError::CustomApiError)?;
        if !matches!(actor_member.as_ref().map(|m| m.role.as_str()), Some("owner")) {
            return Err(WebError::Forbidden(
                "only owners can grant the owner role".to_string(),
            ));
        }
    }

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

    let applied = state
        .repo
        .set_org_member_role_and_join_developers(org_row.id, user.id, role, DEVELOPERS_TEAM)
        .await
        .map_err(WebError::CustomApiError)?;
    if !applied {
        return Err(WebError::Conflict(format!(
            "cannot demote the last owner of organization \"{org_name}\""
        )));
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
    let org_row = require_org_manager(&state, &auth, &org_name).await?;

    let user = state
        .repo
        .get_user_by_name(&body.user)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("User \"{}\" not found", body.user)))?;

    state
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

    let removed = state
        .repo
        .remove_org_member_cascade(org_row.id, user.id)
        .await
        .map_err(WebError::CustomApiError)?;
    if !removed {
        return Err(WebError::Conflict(format!(
            "cannot remove the last owner of organization \"{org_name}\""
        )));
    }

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
        (status = OK, description = "Packages in the org viewable by current user (delegates to per-user listing when the name is not an org)", body = serde_json::Value),
        (status = NOT_FOUND, body = crate::error::ApiErrorDetail, description = "Neither an org nor a user with that name exists"),
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
            return crate::handlers::access::build_user_packages(&state, auth.as_ref(), &org)
                .await;
        }
    };

    let is_admin_viewer = auth
        .as_ref()
        .is_some_and(|a| is_admin(&a.user, &state.config.auth.admins));

    let viewer_id = auth.as_ref().map(|a| a.user.id).unwrap_or(0);

    let pkgs = if is_admin_viewer {
        state
            .repo
            .list_all_packages_in_org(org_row.id)
            .await
            .map_err(WebError::CustomApiError)?
    } else {
        state
            .repo
            .list_packages_in_org_viewable(org_row.id, viewer_id)
            .await
            .map_err(WebError::CustomApiError)?
    };

    let write_map = if is_admin_viewer {
        pkgs.iter().map(|p| (p.id, true)).collect()
    } else {
        state
            .repo
            .list_org_package_viewer_permissions(org_row.id, viewer_id)
            .await
            .map_err(WebError::CustomApiError)?
    };

    let mut res: BTreeMap<String, String> = BTreeMap::new();
    for pkg in pkgs {
        let level = if write_map.get(&pkg.id).copied().unwrap_or(false) {
            "write"
        } else {
            "read"
        };
        res.insert(pkg.name, level.to_string());
    }
    Ok(Json(serde_json::to_value(res).unwrap()))
}

