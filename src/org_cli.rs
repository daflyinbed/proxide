use crate::config::Config;
use crate::state::AppState;
use anyhow::{Context, Result, bail};
use clap::Subcommand;

const DEVELOPERS_TEAM: &str = "developers";

#[derive(Debug, Subcommand)]
pub enum OrgAction {
    /// Create a new organization with the given scope name and bootstrap an owner.
    Create {
        /// Organization name (also the scope, with or without leading @).
        name: String,
        /// Username of the initial owner (must already exist in the users table).
        owner: String,
    },
    /// Add a user to an organization with a given role (default: developer).
    AddMember {
        /// Organization name.
        org: String,
        /// Username to add.
        user: String,
        /// Role: owner, admin, or developer (default: developer).
        #[arg(long)]
        role: Option<String>,
    },
    /// Remove a user from an organization (cascades to all teams).
    RmMember {
        /// Organization name.
        org: String,
        /// Username to remove.
        user: String,
    },
    /// List members of an organization.
    Ls {
        /// Organization name.
        org: String,
    },
    /// Delete an organization entirely (cascades to all teams and permissions).
    Delete {
        /// Organization name.
        org: String,
    },
}

fn normalize_scope(name: &str) -> String {
    name.trim().trim_start_matches('@').to_string()
}

pub async fn run_org(config: Config, action: OrgAction) -> Result<()> {
    let state = AppState::new(config).await?;
    state.repo.migrate().await?;
    match action {
        OrgAction::Create { name, owner } => create_org(&state, &name, &owner).await,
        OrgAction::AddMember { org, user, role } => {
            add_member(&state, &org, &user, role.as_deref()).await
        }
        OrgAction::RmMember { org, user } => rm_member(&state, &org, &user).await,
        OrgAction::Ls { org } => ls_org(&state, &org).await,
        OrgAction::Delete { org } => delete_org(&state, &org).await,
    }
}

async fn create_org(state: &AppState, raw_name: &str, owner_name: &str) -> Result<()> {
    let name = normalize_scope(raw_name);
    if name.is_empty() {
        bail!("organization name cannot be empty");
    }
    let owner = state
        .repo
        .get_user_by_name(owner_name)
        .await
        .context("failed to look up owner user")?
        .ok_or_else(|| anyhow::anyhow!("user \"{owner_name}\" does not exist; create the user first (e.g. via `npm adduser` or CAS login)"))?;

    if state.repo.get_org_by_name(&name).await?.is_some() {
        bail!("organization \"{name}\" already exists");
    }

    let org_id = state.repo.create_org(&name, None).await?;
    state
        .repo
        .create_team(org_id, DEVELOPERS_TEAM, None)
        .await
        .context("failed to create developers team")?;
    state
        .repo
        .add_org_member(org_id, owner.id, "owner")
        .await
        .context("failed to add owner as org member")?;
    let dev_team = state
        .repo
        .get_team_by_org_name(org_id, DEVELOPERS_TEAM)
        .await?
        .ok_or_else(|| anyhow::anyhow!("developers team disappeared after create"))?;
    state
        .repo
        .add_team_member(dev_team.id, owner.id)
        .await
        .context("failed to add owner to developers team")?;

    println!(
        "created organization \"{}\" (id={org_id}) with owner \"{}\"; developers team initialized",
        name, owner_name
    );
    Ok(())
}

async fn add_member(
    state: &AppState,
    raw_org: &str,
    user_name: &str,
    role: Option<&str>,
) -> Result<()> {
    let org_name = normalize_scope(raw_org);
    let role = role.unwrap_or("developer");
    if !matches!(role, "owner" | "admin" | "developer") {
        bail!("invalid role \"{role}\", must be one of: owner, admin, developer");
    }
    let org = state
        .repo
        .get_org_by_name(&org_name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("organization \"{org_name}\" not found"))?;
    let user = state
        .repo
        .get_user_by_name(user_name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("user \"{user_name}\" does not exist"))?;

    let existed = state.repo.get_org_member(org.id, user.id).await?.is_some();
    let applied = state
        .repo
        .set_org_member_role_guarded(org.id, user.id, role)
        .await?;
    if !applied {
        bail!("cannot demote the last owner of organization \"{org_name}\"");
    }
    if !existed {
        if let Some(dev_team) = state.repo.get_team_by_org_name(org.id, DEVELOPERS_TEAM).await? {
            state.repo.add_team_member(dev_team.id, user.id).await?;
        }
    }
    println!(
        "set \"{}\" as {role} of organization \"{}\"",
        user_name, org_name
    );
    Ok(())
}

async fn rm_member(state: &AppState, raw_org: &str, user_name: &str) -> Result<()> {
    let org_name = normalize_scope(raw_org);
    let org = state
        .repo
        .get_org_by_name(&org_name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("organization \"{org_name}\" not found"))?;
    let user = state
        .repo
        .get_user_by_name(user_name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("user \"{user_name}\" does not exist"))?;

    let existing = state
        .repo
        .get_org_member(org.id, user.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("user \"{user_name}\" is not a member of organization \"{org_name}\""))?;

    if existing.role == "owner" {
        let owner_count = state.repo.count_org_owners(org.id).await?;
        if owner_count <= 1 {
            bail!("cannot remove the last owner of organization \"{org_name}\"");
        }
    }

    let removed = state
        .repo
        .remove_org_member_cascade(org.id, user.id)
        .await?;
    if !removed {
        bail!("cannot remove the last owner of organization \"{org_name}\"");
    }
    println!(
        "removed \"{}\" from organization \"{}\"",
        user_name, org_name
    );
    Ok(())
}

async fn ls_org(state: &AppState, raw_org: &str) -> Result<()> {
    let org_name = normalize_scope(raw_org);
    let org = state
        .repo
        .get_org_by_name(&org_name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("organization \"{org_name}\" not found"))?;
    let members = state.repo.list_org_members(org.id).await?;

    println!("organization: {} (id={})", org.name, org.id);
    println!("members:");
    println!("{:<32}  {}", "user", "role");
    println!("{:-<32}  {:-<10}", "", "");
    for m in members {
        let name = state
            .repo
            .get_user_by_id(m.user_id)
            .await?
            .map(|u| u.name)
            .unwrap_or_else(|| format!("<unknown:{}>", m.user_id));
        println!("{:<32}  {}", name, m.role);
    }
    Ok(())
}

async fn delete_org(state: &AppState, raw_org: &str) -> Result<()> {
    let org_name = normalize_scope(raw_org);
    let org = state
        .repo
        .get_org_by_name(&org_name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("organization \"{org_name}\" not found"))?;
    state.repo.delete_org(org.id).await?;
    println!("deleted organization \"{}\"", org_name);
    Ok(())
}
