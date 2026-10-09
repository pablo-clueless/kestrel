//! The signed-in user's own account: their display name, signing out their other devices, and
//! deleting the account.

use uuid::Uuid;

use super::{Accounts, AuthedUser, DeleteAccountRequest, internal, session};
use crate::{db::teams::DeleteUser, error::ApiError};

const NAME_MAX: usize = 80;

impl Accounts {
    /// Sets the display name; an empty one clears it.
    pub async fn set_name(&self, user: &AuthedUser, raw: &str) -> Result<(), ApiError> {
        let name = display_name(raw)?;
        self.db.set_name(user.id, name.as_deref()).await.map_err(internal)
    }

    /// Signs out every session of `user` but `keep` (the one asking). How many ended.
    pub async fn sign_out_others(&self, user: &AuthedUser, keep: &str) -> Result<u64, ApiError> {
        let ended = self.db.delete_other_sessions(user.id, &session::hash(keep)).await.map_err(internal)?;
        tracing::info!("{} signed out {ended} other session(s)", user.email);
        Ok(ended)
    }

    /// Deletes the signed-in user's account after checking the password (and a code, with
    /// two-factor on). Returns the workspaces deleted with it, for the API to stop their runs and
    /// drop them from the cache.
    pub async fn delete_account(&self, user: &AuthedUser, req: DeleteAccountRequest) -> Result<Vec<Uuid>, ApiError> {
        self.confirm_password(user, req.password).await?;
        if self.db.two_factor(user.id).await.map_err(internal)?.enabled {
            let code = req.code.as_deref().map(str::trim).filter(|c| !c.is_empty());
            let Some(code) = code else {
                return Err(ApiError::BadRequest("enter a code from your authenticator app".into()));
            };
            if !self.check_code(user.id, code).await? {
                // A 400, not a 401: the UI treats 401 as "your session ended".
                return Err(ApiError::BadRequest("that code didn't work".into()));
            }
        }
        let deleted = self.delete_user(user.id, "you're").await?;
        tracing::info!("{} deleted their account, and {} workspace(s) with it", user.email, deleted.len());
        Ok(deleted)
    }

    /// [`crate::db::Db::delete_user`] with its refusal worded for `who` ("you're", or the email for
    /// an admin deleting someone).
    pub(super) async fn delete_user(&self, id: Uuid, who: &str) -> Result<Vec<Uuid>, ApiError> {
        // Named before deleting, for the refusal: afterwards the names are gone too.
        let names = self.db.workspaces_of(id).await.map_err(internal)?;
        match self.db.delete_user(id).await.map_err(internal)? {
            DeleteUser::Deleted(workspaces) => Ok(workspaces),
            DeleteUser::NotFound => Err(ApiError::NotFound("user not found")),
            DeleteUser::LastAdmin(blocked) => {
                let blocked: Vec<String> = names
                    .into_iter()
                    .filter(|w| blocked.contains(&w.id))
                    .map(|w| format!("\"{}\"", super::teams::WorkspaceInfo::of(w).name))
                    .collect();
                Err(ApiError::BadRequest(format!(
                    "{who} the only admin of {}, which other people are in; make one of them an admin, or \
                     delete the workspace, first",
                    blocked.join(", ")
                )))
            }
        }
    }
}

/// Trimmed; at most 80 characters and no control characters (it's shown in the UI and emails).
/// Empty means no name.
fn display_name(raw: &str) -> Result<Option<String>, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > NAME_MAX {
        return Err(ApiError::BadRequest(format!("keep your name to {NAME_MAX} characters")));
    }
    if name.chars().any(char::is_control) {
        return Err(ApiError::BadRequest("your name can't contain line breaks or control characters".into()));
    }
    Ok(Some(name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_are_trimmed_and_checked() {
        assert_eq!(display_name("  Ada Lovelace ").unwrap().as_deref(), Some("Ada Lovelace"));
        assert_eq!(display_name("   ").unwrap(), None);
        assert!(display_name(&"a".repeat(81)).is_err());
        assert!(display_name("Ada\nLovelace").is_err());
    }
}
