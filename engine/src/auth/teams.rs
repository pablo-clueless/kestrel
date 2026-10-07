//! Shared workspaces (HANDOFF → Accounts → Shared workspaces): several users in one workspace, each
//! an admin, or with write or read access. A workspace always has at least one admin. Invites are emailed links; a link only works for the address
//! it was sent to. Who may do what is decided here; `db::teams` is the SQL.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::{Accounts, AuthedUser, Client, internal, link_token, mail, normalize_email, session};
use crate::{
    db::teams::{Accept, InviteRow, MemberChange, Role, WorkspaceRow},
    error::ApiError,
};

/// How long an invite link works.
const INVITE_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const INVITE_PATH: &str = "/invite";
const NAME_MAX: usize = 80;
/// Workspaces one user may create (counted as those they're an admin of): plenty for teams and
/// projects, few enough that one account can't fill the database with schemas.
const MAX_ADMIN_OF: usize = 20;
const NOT_FOUND: ApiError = ApiError::NotFound("workspace not found");
const BAD_INVITE: &str = "this invite is invalid or has expired";
const ADMINS_ONLY: ApiError = ApiError::Forbidden("only the workspace's admins can do that");
const LAST_ADMIN: &str = "a workspace needs at least one admin; make someone else an admin first";

/// One of the signed-in user's workspaces.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspaceInfo {
    pub id: Uuid,
    /// Its name, or "<admin>'s workspace" for one that was never named.
    pub name: String,
    /// The signed-in user's role in it.
    pub role: Role,
    pub members: u32,
    /// Its admins' emails, longest-standing first.
    pub admin_emails: Vec<String>,
}

impl WorkspaceInfo {
    fn of(row: WorkspaceRow) -> Self {
        let name = row.name.unwrap_or_else(|| match row.admin_emails.first().and_then(|e| e.split('@').next()) {
            Some(admin) => format!("{admin}'s workspace"),
            None => "Workspace".into(),
        });
        Self { id: row.id, name, role: row.role, members: row.members as u32, admin_emails: row.admin_emails }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MemberInfo {
    pub user_id: Uuid,
    pub email: String,
    pub role: Role,
    #[ts(type = "number")]
    pub joined_at_ms: i64,
    /// The signed-in user.
    pub you: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InviteInfo {
    pub id: Uuid,
    pub email: String,
    pub role: Role,
    pub invited_by: Option<String>,
    #[ts(type = "number")]
    pub created_at_ms: i64,
    #[ts(type = "number")]
    pub expires_at_ms: i64,
}

impl From<InviteRow> for InviteInfo {
    fn from(row: InviteRow) -> Self {
        Self {
            id: row.id,
            email: row.email,
            role: row.role,
            invited_by: row.invited_by,
            created_at_ms: row.created_at_ms,
            expires_at_ms: row.expires_at_ms,
        }
    }
}

/// `GET /api/workspaces/{id}/members`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MembersResponse {
    pub members: Vec<MemberInfo>,
    /// Invites not yet accepted. Only admins see these; empty for everyone else.
    pub invites: Vec<InviteInfo>,
}

/// `POST /api/workspaces` (create) and `PATCH /api/workspaces/{id}` (rename).
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspaceNameRequest {
    pub name: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InviteRequest {
    pub email: String,
    /// `admin`, `write` or `read`.
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InviteCreated {
    pub invite: InviteInfo,
    /// The link to join, for the admin to pass on by other means (always given, emailed or not).
    pub link: String,
    /// Whether it was emailed (only when this engine has SMTP set up, and the send worked).
    pub emailed: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcceptInviteRequest {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcceptInviteResponse {
    pub workspace_id: Uuid,
    /// Already a member before this invite; the role stayed as it was.
    pub already_member: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetRoleRequest {
    /// `admin`, `write` or `read`.
    pub role: Role,
}

impl Accounts {
    /// Every workspace `user` belongs to, the default (first joined) first.
    pub async fn workspaces(&self, user: &AuthedUser) -> Result<Vec<WorkspaceInfo>, ApiError> {
        Ok(self.db.workspaces_of(user.id).await.map_err(internal)?.into_iter().map(WorkspaceInfo::of).collect())
    }

    /// `user`'s role in `workspace`; a workspace they aren't in is a 404, as if it didn't exist.
    pub async fn role(&self, user: &AuthedUser, workspace: Uuid) -> Result<Role, ApiError> {
        self.db.role_in(user.id, workspace).await.map_err(internal)?.ok_or(NOT_FOUND)
    }

    async fn require_admin(&self, user: &AuthedUser, workspace: Uuid) -> Result<(), ApiError> {
        match self.role(user, workspace).await? {
            Role::Admin => Ok(()),
            _ => Err(ADMINS_ONLY),
        }
    }

    pub async fn create_workspace(&self, user: &AuthedUser, name: &str) -> Result<WorkspaceInfo, ApiError> {
        let name = workspace_name(name)?;
        let admin_of = self.workspaces(user).await?.iter().filter(|w| w.role == Role::Admin).count();
        if admin_of >= MAX_ADMIN_OF {
            return Err(ApiError::BadRequest(format!(
                "you're an admin of {MAX_ADMIN_OF} workspaces, the most allowed; delete or leave one first"
            )));
        }
        let id = self.db.create_workspace(user.id, &name).await.map_err(internal)?;
        tracing::info!("workspace {id} created by {}", user.email);
        self.workspaces(user).await?.into_iter().find(|w| w.id == id).ok_or(NOT_FOUND)
    }

    pub async fn rename_workspace(&self, user: &AuthedUser, workspace: Uuid, name: &str) -> Result<(), ApiError> {
        self.require_admin(user, workspace).await?;
        self.db.rename_workspace(workspace, &workspace_name(name)?).await.map_err(internal)
    }

    /// Checks that `user` may delete `workspace`: they're an admin of it, and it isn't their only workspace.
    /// The API stops its runs and drops it from the cache, then calls [`Self::delete_workspace`].
    pub async fn check_delete_workspace(&self, user: &AuthedUser, workspace: Uuid) -> Result<(), ApiError> {
        self.require_admin(user, workspace).await?;
        if self.workspaces(user).await?.len() < 2 {
            return Err(ApiError::BadRequest(
                "this is your only workspace; create or join another before deleting it".into(),
            ));
        }
        Ok(())
    }

    pub async fn delete_workspace(&self, user: &AuthedUser, workspace: Uuid) -> Result<(), ApiError> {
        self.db.delete_workspace(workspace).await.map_err(internal)?;
        tracing::info!("workspace {workspace} deleted by {}", user.email);
        Ok(())
    }

    /// Members, and for admins the open invites. Any member may look.
    pub async fn members(&self, user: &AuthedUser, workspace: Uuid) -> Result<MembersResponse, ApiError> {
        let role = self.role(user, workspace).await?;
        let members = self.db.members(workspace).await.map_err(internal)?;
        let invites = match role {
            Role::Admin => self.db.invites(workspace).await.map_err(internal)?,
            _ => Vec::new(),
        };
        Ok(MembersResponse {
            members: members
                .into_iter()
                .map(|m| MemberInfo {
                    you: m.user_id == user.id,
                    user_id: m.user_id,
                    email: m.email,
                    role: m.role,
                    joined_at_ms: m.joined_at_ms,
                })
                .collect(),
            invites: invites.into_iter().map(InviteInfo::from).collect(),
        })
    }

    /// Invites `email` with any role, admin included. The link is emailed when this engine can send
    /// email, and returned either way so the admin can pass it on themselves.
    pub async fn invite(
        &self,
        user: &AuthedUser,
        workspace: Uuid,
        req: InviteRequest,
        client: Client<'_>,
    ) -> Result<InviteCreated, ApiError> {
        self.require_admin(user, workspace).await?;
        let email = normalize_email(&req.email)?;
        if self.db.is_member_email(workspace, &email).await.map_err(internal)? {
            return Err(ApiError::BadRequest(format!("{email} is already a member")));
        }
        self.invites.check(&workspace.to_string()).map_err(ApiError::TooManyRequests)?;
        let base = self.link_base(client.origin)?;
        let token = session::new_token();
        let id = self
            .db
            .upsert_invite(workspace, &email, req.role, &session::hash(&token), user.id, INVITE_LIFETIME)
            .await
            .map_err(internal)?;
        let link = format!("{base}{INVITE_PATH}?token={token}");

        let mut emailed = false;
        // The per-address limit only guards the inbox, so a link handed over by hand doesn't use it.
        if let Some(mailer) = &self.mailer
            && self.mail_per_address.check(&email).is_ok()
        {
            let name = self.workspaces(user).await?.into_iter().find(|w| w.id == workspace).map(|w| w.name);
            let message =
                mail::invite(&email, &user.email, name.as_deref().unwrap_or("Workspace"), access(req.role), &link);
            match mailer.send(message).await {
                Ok(()) => emailed = true,
                Err(err) => tracing::warn!("couldn't email the invite to {email}: {err:#}"),
            }
        }
        let invite = self
            .db
            .invites(workspace)
            .await
            .map_err(internal)?
            .into_iter()
            .find(|i| i.id == id)
            .map(InviteInfo::from)
            .ok_or_else(|| ApiError::Internal("the invite vanished as it was made".into()))?;
        Ok(InviteCreated { invite, link, emailed })
    }

    pub async fn revoke_invite(&self, user: &AuthedUser, workspace: Uuid, id: Uuid) -> Result<(), ApiError> {
        self.require_admin(user, workspace).await?;
        match self.db.delete_invite(workspace, id).await.map_err(internal)? {
            true => Ok(()),
            false => Err(ApiError::NotFound("invite not found")),
        }
    }

    /// Joins the invite's workspace, if it was sent to the signed-in user's address.
    pub async fn accept_invite(
        &self,
        user: &AuthedUser,
        token: &str,
        client: Client<'_>,
    ) -> Result<AcceptInviteResponse, ApiError> {
        if let Some(ip) = client.ip {
            self.by_ip.check(ip).map_err(ApiError::TooManyRequests)?;
        }
        let hash = link_token(token).map_err(|_| ApiError::BadRequest(BAD_INVITE.into()))?;
        match self.db.accept_invite(&hash, user.id, &user.email).await.map_err(internal)? {
            Accept::Joined(workspace_id) => {
                tracing::info!("{} joined workspace {workspace_id}", user.email);
                Ok(AcceptInviteResponse { workspace_id, already_member: false })
            }
            Accept::AlreadyMember(workspace_id) => Ok(AcceptInviteResponse { workspace_id, already_member: true }),
            Accept::WrongEmail => Err(ApiError::Forbidden(
                "this invite was sent to a different email address; sign in with that one to accept it",
            )),
            Accept::Invalid => Err(ApiError::BadRequest(BAD_INVITE.into())),
        }
    }

    /// Gives a member another role. Admins only, their own role included, as long as the workspace
    /// keeps an admin.
    pub async fn set_role(&self, user: &AuthedUser, workspace: Uuid, member: Uuid, role: Role) -> Result<(), ApiError> {
        self.require_admin(user, workspace).await?;
        self.change_member(workspace, member, Some(role)).await
    }

    /// An admin removes anyone, or a member leaves (their own id). Either way the workspace keeps an
    /// admin: the last one has to hand over first (or delete the workspace).
    pub async fn remove_member(&self, user: &AuthedUser, workspace: Uuid, member: Uuid) -> Result<(), ApiError> {
        if member != user.id {
            self.require_admin(user, workspace).await?;
        } else {
            self.role(user, workspace).await?;
        }
        self.change_member(workspace, member, None).await
    }

    async fn change_member(&self, workspace: Uuid, member: Uuid, role: Option<Role>) -> Result<(), ApiError> {
        match self.db.change_member(workspace, member, role).await.map_err(internal)? {
            MemberChange::Done => Ok(()),
            MemberChange::NotMember => Err(ApiError::NotFound("member not found")),
            MemberChange::LastAdmin => Err(ApiError::BadRequest(LAST_ADMIN.into())),
        }
    }
}
/// How an invite email describes the role.
fn access(role: Role) -> &'static str {
    match role {
        Role::Admin => "as an admin",
        Role::Write => "with write access",
        Role::Read => "with read access",
    }
}

/// Trimmed; 1–80 characters, no control characters (it goes in emails and the UI).
fn workspace_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("give the workspace a name".into()));
    }
    if name.chars().count() > NAME_MAX {
        return Err(ApiError::BadRequest(format!("keep the name to {NAME_MAX} characters")));
    }
    if name.chars().any(char::is_control) {
        return Err(ApiError::BadRequest("the name can't contain line breaks or control characters".into()));
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_workspace_names() {
        assert_eq!(workspace_name("  Payments team ").unwrap(), "Payments team");
        for bad in ["", "   ", "a\nb", &"x".repeat(81)] {
            assert!(workspace_name(bad).is_err(), "{bad:?}");
        }
        assert!(workspace_name(&"é".repeat(80)).is_ok(), "counted in characters, not bytes");
    }
}
