pub mod access;
pub mod auth;
pub mod cdn;
pub mod data_api;
pub mod dist_tags;
pub mod downloads;
pub mod fast_meta;
pub mod home;
pub mod jsdelivr_util;
pub mod orgs;
pub mod package_dispatch;
pub mod profile;
pub mod publish;
pub mod registry;
pub mod search;
pub mod sso;
pub mod sync;
pub mod tarball;
pub mod teams;
pub mod tokens;
pub mod web_login;

use crate::error::{WebError, WebResult};
use crate::state::{AppState, LockOwner, UnlockGuard};
use std::collections::HashMap;

pub(crate) fn lock_package<'a>(
    state: &'a AppState,
    fullname: &str,
    owner: LockOwner,
) -> WebResult<UnlockGuard<'a>> {
    state
        .package_lock
        .try_guard(fullname, owner)
        .map_err(|owner| {
            WebError::Conflict(format!("package {fullname} is currently being {owner}"))
        })
}

pub(crate) fn ensure_local_package(source: Option<&str>, fullname: &str) -> WebResult<()> {
    if let Some(source) = source {
        return Err(WebError::Forbidden(format!(
            "package {fullname} was synced from upstream ({source}), mutation is not allowed"
        )));
    }
    Ok(())
}

pub(crate) async fn load_tag_map(
    state: &AppState,
    package_id: i64,
) -> WebResult<HashMap<String, String>> {
    let tags = state
        .repo
        .list_tags(package_id)
        .await
        .map_err(WebError::CustomApiError)?;
    Ok(tags.into_iter().map(|tag| (tag.tag, tag.version)).collect())
}
