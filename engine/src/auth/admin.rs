//! The admin pages (HANDOFF → Accounts → Admin): every user and workspace on this engine, for the
//! accounts listed in `KESTREL_ADMIN_EMAILS`. The API checks [`Accounts::is_admin`] before any of
//! this runs; `db::admin` is the SQL.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::{Accounts, AuthedUser, internal, teams::MemberInfo};
use crate::{
    config::{CapsInfo, Config, SmtpTls},
    db::admin::{AdminUserRow, AdminWorkspaceRow},
    error::ApiError,
};

const USER_NOT_FOUND: ApiError = ApiError::NotFound("user not found");
const RECENT_SIGNUPS: i64 = 5;

/// `GET /api/admin/overview`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AdminOverview {
    pub users: u32,
    pub verified_users: u32,
    pub two_factor_users: u32,
    pub disabled_users: u32,
    pub signups_last_7d: u32,
    pub workspaces: u32,
    /// Workspaces with more than one member.
    pub shared_workspaces: u32,
    /// Signed-in sessions that haven't expired.
    pub live_sessions: u32,
    /// Tests running on this engine now.
    pub running_tests: u32,
    /// The newest accounts, newest first.
    pub recent_signups: Vec<AdminUser>,
}

/// One account, as the admin pages list it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AdminUser {
    pub id: Uuid,
    pub email: String,
    pub name: Option<String>,
    pub email_verified: bool,
    pub two_factor: bool,
    pub disabled: bool,
    /// Listed in `KESTREL_ADMIN_EMAILS`.
    pub admin: bool,
    /// The signed-in admin looking at the list.
    pub you: bool,
    #[ts(type = "number")]
    pub created_at_ms: i64,
    /// The latest use of a session still on record; `null` once they've all expired.
    #[ts(type = "number | null")]
    pub last_seen_at_ms: Option<i64>,
    pub workspaces: u32,
    pub live_sessions: u32,
}

/// One workspace, as the admin pages list it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AdminWorkspace {
    pub id: Uuid,
    /// Its name, or "<admin>'s workspace" for one that was never named.
    pub name: String,
    pub admin_emails: Vec<String>,
    pub members: u32,
    #[ts(type = "number")]
    pub created_at_ms: i64,
    /// What its data takes up in the database.
    #[ts(type = "number")]
    pub size_bytes: i64,
}

/// `GET /api/admin/settings`: how this engine is configured. Read-only (it's all environment
/// variables), and without secrets: no token, database URL or SMTP credentials.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AdminSettings {
    pub version: String,
    pub signup_open: bool,
    pub admin_emails: Vec<String>,
    pub public_url: Option<String>,
    /// `None` when email is off.
    pub mail: Option<AdminMail>,
    /// The address and port the engine listens on.
    pub listen: String,
    /// Extra `Host` values accepted besides localhost.
    pub allowed_hosts: Vec<String>,
    pub trusted_proxy: bool,
    pub db_max_connections: u32,
    pub caps: CapsInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AdminMail {
    pub host: String,
    pub port: u16,
    /// `tls`, `starttls` or `none`.
    pub tls: String,
    pub from: String,
    /// Whether it signs in to the SMTP server (the credentials themselves aren't shown).
    pub authenticated: bool,
}

/// `PUT /api/admin/users/{id}/disabled`.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetDisabledRequest {
    pub disabled: bool,
}

impl AdminSettings {
    pub fn of(config: &Config) -> Self {
        // localhost and 127.0.0.1 are always accepted; only list what the operator added.
        let defaults = [format!("127.0.0.1:{}", config.port), format!("localhost:{}", config.port)];
        Self {
            version: env!("CARGO_PKG_VERSION").into(),
            signup_open: config.signup_open,
            admin_emails: config.admin_emails.clone(),
            public_url: config.public_url.clone(),
            mail: config.smtp.as_ref().map(|smtp| AdminMail {
                host: smtp.host.clone(),
                port: smtp.port,
                tls: match smtp.tls {
                    SmtpTls::Tls => "tls",
                    SmtpTls::StartTls => "starttls",
                    SmtpTls::None => "none",
                }
                .into(),
                from: smtp.from.clone(),
                authenticated: smtp.username.is_some(),
            }),
            listen: std::net::SocketAddr::new(config.bind, config.port).to_string(),
            allowed_hosts: config.allowed_hosts.iter().filter(|h| !defaults.contains(h)).cloned().collect(),
            trusted_proxy: config.trusted_proxy,
            db_max_connections: config.db_max_connections,
            caps: config.caps.info(),
        }
    }
}

impl Accounts {
    fn admin_user(&self, row: AdminUserRow, viewer: &AuthedUser) -> AdminUser {
        AdminUser {
            admin: self.enabled && self.admin_emails.contains(&row.email),
            you: row.id == viewer.id,
            id: row.id,
            email: row.email,
            name: row.name,
            email_verified: row.email_verified,
            two_factor: row.two_factor,
            disabled: row.disabled,
            created_at_ms: row.created_at_ms,
            last_seen_at_ms: row.last_seen_at_ms,
            workspaces: row.workspaces as u32,
            live_sessions: row.live_sessions as u32,
        }
    }

    pub async fn admin_overview(&self, viewer: &AuthedUser, running_tests: usize) -> Result<AdminOverview, ApiError> {
        let counts = self.db.admin_overview().await.map_err(internal)?;
        let recent = self.db.admin_users(Some(RECENT_SIGNUPS)).await.map_err(internal)?;
        Ok(AdminOverview {
            users: counts.users as u32,
            verified_users: counts.verified_users as u32,
            two_factor_users: counts.two_factor_users as u32,
            disabled_users: counts.disabled_users as u32,
            signups_last_7d: counts.signups_last_7d as u32,
            workspaces: counts.workspaces as u32,
            shared_workspaces: counts.shared_workspaces as u32,
            live_sessions: counts.live_sessions as u32,
            running_tests: running_tests as u32,
            recent_signups: recent.into_iter().map(|row| self.admin_user(row, viewer)).collect(),
        })
    }

    pub async fn admin_users(&self, viewer: &AuthedUser) -> Result<Vec<AdminUser>, ApiError> {
        let rows = self.db.admin_users(None).await.map_err(internal)?;
        Ok(rows.into_iter().map(|row| self.admin_user(row, viewer)).collect())
    }

    pub async fn admin_workspaces(&self) -> Result<Vec<AdminWorkspace>, ApiError> {
        let rows = self.db.admin_workspaces().await.map_err(internal)?;
        Ok(rows.into_iter().map(AdminWorkspace::of).collect())
    }

    /// Any workspace's members, whether or not the admin is one of them.
    pub async fn admin_members(&self, viewer: &AuthedUser, workspace: Uuid) -> Result<Vec<MemberInfo>, ApiError> {
        if !self.db.workspace_exists(workspace).await.map_err(internal)? {
            return Err(ApiError::NotFound("workspace not found"));
        }
        let members = self.db.members(workspace).await.map_err(internal)?;
        Ok(members
            .into_iter()
            .map(|m| MemberInfo {
                you: m.user_id == viewer.id,
                user_id: m.user_id,
                email: m.email,
                name: m.name,
                role: m.role,
                joined_at_ms: m.joined_at_ms,
            })
            .collect())
    }

    /// The account an admin action is aimed at. Admins can't aim the lockout actions at themselves
    /// or each other: `KESTREL_ADMIN_EMAILS` is what grants admin, so it's where that's changed.
    async fn target(&self, viewer: &AuthedUser, id: Uuid, protect_admins: bool) -> Result<AdminUserRow, ApiError> {
        let row = self.db.admin_user(id).await.map_err(internal)?.ok_or(USER_NOT_FOUND)?;
        if protect_admins && row.id == viewer.id {
            return Err(ApiError::BadRequest("you can't do that to your own account here; use your profile".into()));
        }
        if protect_admins && self.admin_emails.contains(&row.email) {
            return Err(ApiError::BadRequest(format!(
                "{} is an admin of this engine; remove them from KESTREL_ADMIN_EMAILS first",
                row.email
            )));
        }
        Ok(row)
    }

    pub async fn admin_verify_email(&self, viewer: &AuthedUser, id: Uuid) -> Result<(), ApiError> {
        let row = self.target(viewer, id, false).await?;
        self.db.admin_verify_email(id).await.map_err(internal)?;
        tracing::info!("{} marked {}'s email verified", viewer.email, row.email);
        Ok(())
    }

    /// Turns two-factor off, for someone who lost their authenticator and recovery codes.
    pub async fn admin_reset_two_factor(&self, viewer: &AuthedUser, id: Uuid) -> Result<(), ApiError> {
        let row = self.target(viewer, id, false).await?;
        self.db.disable_totp(id).await.map_err(internal)?;
        tracing::info!("{} turned off two-factor sign-in for {}", viewer.email, row.email);
        Ok(())
    }

    /// Signs the user out everywhere. How many sessions ended.
    pub async fn admin_sign_out(&self, viewer: &AuthedUser, id: Uuid) -> Result<u64, ApiError> {
        let row = self.target(viewer, id, true).await?;
        let ended = self.db.end_all_sessions(id).await.map_err(internal)?;
        tracing::info!("{} signed {} out of {ended} session(s)", viewer.email, row.email);
        Ok(ended)
    }

    pub async fn admin_set_disabled(&self, viewer: &AuthedUser, id: Uuid, disabled: bool) -> Result<(), ApiError> {
        let row = self.target(viewer, id, true).await?;
        if !self.db.set_disabled(id, disabled).await.map_err(internal)? {
            return Err(USER_NOT_FOUND);
        }
        let done = if disabled { "disabled" } else { "enabled" };
        tracing::info!("{} {done} the account {}", viewer.email, row.email);
        Ok(())
    }

    /// Deletes the account, as deleting it themselves would (the same refusal if it's the last admin
    /// of a shared workspace). Returns the workspaces deleted with it.
    pub async fn admin_delete_user(&self, viewer: &AuthedUser, id: Uuid) -> Result<Vec<Uuid>, ApiError> {
        let row = self.target(viewer, id, true).await?;
        let deleted = self.delete_user(id, &format!("{} is", row.email)).await?;
        tracing::info!(
            "{} deleted the account {}, and {} workspace(s) with it",
            viewer.email,
            row.email,
            deleted.len()
        );
        Ok(deleted)
    }

    /// Checks the workspace exists before the API stops its runs and deletes it.
    pub async fn admin_check_workspace(&self, workspace: Uuid) -> Result<(), ApiError> {
        match self.db.workspace_exists(workspace).await.map_err(internal)? {
            true => Ok(()),
            false => Err(ApiError::NotFound("workspace not found")),
        }
    }
}

impl AdminWorkspace {
    fn of(row: AdminWorkspaceRow) -> Self {
        let name = row.name.unwrap_or_else(|| match row.admin_emails.first().and_then(|e| e.split('@').next()) {
            Some(admin) => format!("{admin}'s workspace"),
            None => "Workspace".into(),
        });
        Self {
            id: row.id,
            name,
            admin_emails: row.admin_emails,
            members: row.members as u32,
            created_at_ms: row.created_at_ms,
            size_bytes: row.size_bytes,
        }
    }
}
