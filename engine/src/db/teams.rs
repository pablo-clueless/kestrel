//! Shared workspaces: roles, names, members and invites, in the `auth` schema. Policy (who may do
//! what, emailing invites) lives in `crate::auth::teams`; this is only the SQL.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, Executor};
use ts_rs::TS;
use uuid::Uuid;

use super::{Db, quote_ident};

/// What a member may do in a workspace. Any number of admins, but always at least one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Role {
    /// Everything `write` can, plus members (invites, roles, removing), renaming and deleting the
    /// workspace.
    Admin,
    /// Reads and changes collections, environments and secrets, sends requests and runs tests.
    Write,
    /// Reads the workspace and run results; changes nothing and sends nothing.
    Read,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Write => "write",
            Self::Read => "read",
        }
    }

    fn parse(s: &str) -> anyhow::Result<Self> {
        Ok(match s {
            "admin" => Self::Admin,
            "write" => Self::Write,
            "read" => Self::Read,
            other => anyhow::bail!("unknown role `{other}` in auth.memberships"),
        })
    }

    pub fn can_edit(self) -> bool {
        matches!(self, Self::Admin | Self::Write)
    }
}

pub struct WorkspaceRow {
    pub id: Uuid,
    pub name: Option<String>,
    /// Admins' emails, longest-standing first.
    pub admin_emails: Vec<String>,
    pub role: Role,
    pub members: i64,
}

pub struct MemberRow {
    pub user_id: Uuid,
    pub email: String,
    pub role: Role,
    pub joined_at_ms: i64,
}

pub struct InviteRow {
    pub id: Uuid,
    pub email: String,
    pub role: Role,
    pub invited_by: Option<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
}

/// What changing or removing a member came to.
pub enum MemberChange {
    Done,
    NotMember,
    /// Refused: it would leave the workspace without an admin.
    LastAdmin,
}

/// What following an invite link came to.
pub enum Accept {
    Joined(Uuid),
    /// Already a member (perhaps invited twice); the invite is used up, the role left as it was.
    AlreadyMember(Uuid),
    /// The invite was for another address. It stays valid for the right person.
    WrongEmail,
    /// Unknown, used or expired.
    Invalid,
}

impl Db {
    /// `user`'s role in `workspace`, or `None` if they aren't a member.
    pub async fn role_in(&self, user: Uuid, workspace: Uuid) -> anyhow::Result<Option<Role>> {
        let role: Option<String> = sqlx::query_scalar(
            self.auth_sql("SELECT role FROM auth.memberships WHERE user_id = $1 AND workspace_id = $2"),
        )
        .bind(user)
        .bind(workspace)
        .fetch_optional(&self.pool)
        .await?;
        role.as_deref().map(Role::parse).transpose()
    }

    /// Every workspace `user` belongs to, in the order they joined (the first is their default).
    pub async fn workspaces_of(&self, user: Uuid) -> anyhow::Result<Vec<WorkspaceRow>> {
        /// id, name, admins' emails, role, member count.
        type Row = (Uuid, Option<String>, Vec<String>, String, i64);
        let rows: Vec<Row> = sqlx::query_as(self.auth_sql(
            "SELECT w.id, w.name,
                    ARRAY(SELECT u.email FROM auth.memberships a JOIN auth.users u ON u.id = a.user_id
                          WHERE a.workspace_id = w.id AND a.role = 'admin' ORDER BY a.created_at, u.email),
                    m.role,
                    (SELECT count(*) FROM auth.memberships c WHERE c.workspace_id = w.id)
             FROM auth.memberships m JOIN auth.workspaces w ON w.id = m.workspace_id
             WHERE m.user_id = $1
             ORDER BY m.created_at, m.workspace_id",
        ))
        .bind(user)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(id, name, admin_emails, role, members)| {
                Ok(WorkspaceRow { id, name, admin_emails, role: Role::parse(&role)?, members })
            })
            .collect()
    }

    /// A new, empty workspace with `user` as its admin, created in one transaction with its schema.
    pub async fn create_workspace(&self, user: Uuid, name: &str) -> anyhow::Result<Uuid> {
        let mut tx = self.begin().await?;
        let id = Uuid::new_v4();
        self.ensure_workspace_in(&mut tx, id).await?;
        sqlx::query(self.auth_sql("UPDATE auth.workspaces SET name = $2 WHERE id = $1"))
            .bind(id)
            .bind(name)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            self.auth_sql("INSERT INTO auth.memberships (workspace_id, user_id, role) VALUES ($1, $2, 'admin')"),
        )
        .bind(id)
        .bind(user)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn rename_workspace(&self, workspace: Uuid, name: &str) -> anyhow::Result<()> {
        sqlx::query(self.auth_sql("UPDATE auth.workspaces SET name = $2 WHERE id = $1"))
            .bind(workspace)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Deletes the workspace and all of its data: its registry row (memberships and invites go with
    /// it) and its schema, in one transaction.
    pub async fn delete_workspace(&self, workspace: Uuid) -> anyhow::Result<()> {
        let schema = self.schema_of(workspace);
        let mut tx = self.begin().await?;
        // The same lock `ensure_workspace_in` takes, so a concurrent open can't recreate it halfway.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))").bind(&schema).execute(&mut *tx).await?;
        sqlx::query(self.auth_sql("DELETE FROM auth.workspaces WHERE id = $1"))
            .bind(workspace)
            .execute(&mut *tx)
            .await?;
        tx.execute(AssertSqlSafe(format!("DROP SCHEMA IF EXISTS {} CASCADE", quote_ident(&schema)))).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The workspace's members, admins first, then in the order they joined.
    pub async fn members(&self, workspace: Uuid) -> anyhow::Result<Vec<MemberRow>> {
        let rows: Vec<(Uuid, String, String, i64)> = sqlx::query_as(self.auth_sql(
            "SELECT u.id, u.email, m.role, (extract(epoch FROM m.created_at) * 1000)::bigint
             FROM auth.memberships m JOIN auth.users u ON u.id = m.user_id
             WHERE m.workspace_id = $1
             ORDER BY m.role <> 'admin', m.created_at, u.email",
        ))
        .bind(workspace)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(user_id, email, role, joined_at_ms)| {
                Ok(MemberRow { user_id, email, role: Role::parse(&role)?, joined_at_ms })
            })
            .collect()
    }

    /// Whether someone signed up as `email` is already in the workspace.
    pub async fn is_member_email(&self, workspace: Uuid, email: &str) -> anyhow::Result<bool> {
        let found: Option<i32> = sqlx::query_scalar(self.auth_sql(
            "SELECT 1 FROM auth.memberships m JOIN auth.users u ON u.id = m.user_id
             WHERE m.workspace_id = $1 AND u.email = $2",
        ))
        .bind(workspace)
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(found.is_some())
    }

    /// The workspace's live invites, newest first.
    pub async fn invites(&self, workspace: Uuid) -> anyhow::Result<Vec<InviteRow>> {
        type Row = (Uuid, String, String, Option<String>, i64, i64);
        let rows: Vec<Row> = sqlx::query_as(self.auth_sql(
            "SELECT i.id, i.email, i.role, u.email,
                    (extract(epoch FROM i.created_at) * 1000)::bigint,
                    (extract(epoch FROM i.expires_at) * 1000)::bigint
             FROM auth.invites i LEFT JOIN auth.users u ON u.id = i.invited_by
             WHERE i.workspace_id = $1 AND i.expires_at > now()
             ORDER BY i.created_at DESC",
        ))
        .bind(workspace)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(id, email, role, invited_by, created_at_ms, expires_at_ms)| {
                Ok(InviteRow { id, email, role: Role::parse(&role)?, invited_by, created_at_ms, expires_at_ms })
            })
            .collect()
    }

    /// Invites `email` with a fresh token. Inviting the same address again replaces the earlier
    /// invite (new role, new link, new expiry), so an old link stops working. Returns its id.
    pub async fn upsert_invite(
        &self,
        workspace: Uuid,
        email: &str,
        role: Role,
        token_hash: &[u8],
        invited_by: Uuid,
        lifetime: Duration,
    ) -> anyhow::Result<Uuid> {
        Ok(sqlx::query_scalar(self.auth_sql(
            "INSERT INTO auth.invites (id, token_hash, workspace_id, email, role, invited_by, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, now() + make_interval(secs => $7))
             ON CONFLICT (workspace_id, email) DO UPDATE
             SET token_hash = excluded.token_hash, role = excluded.role, invited_by = excluded.invited_by,
                 created_at = now(), expires_at = excluded.expires_at
             RETURNING id",
        ))
        .bind(Uuid::new_v4())
        .bind(token_hash)
        .bind(workspace)
        .bind(email)
        .bind(role.as_str())
        .bind(invited_by)
        .bind(lifetime.as_secs_f64())
        .fetch_one(&self.pool)
        .await?)
    }

    /// Withdraws an invite. `false` if the workspace has no such invite.
    pub async fn delete_invite(&self, workspace: Uuid, id: Uuid) -> anyhow::Result<bool> {
        let deleted = sqlx::query(self.auth_sql("DELETE FROM auth.invites WHERE workspace_id = $1 AND id = $2"))
            .bind(workspace)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(deleted.rows_affected() == 1)
    }

    /// Joins `user` (signed in as `email`) to the invite's workspace with its role, and uses the
    /// invite up, in one transaction. The row is locked first, so two clicks can't both join.
    pub async fn accept_invite(&self, token_hash: &[u8], user: Uuid, email: &str) -> anyhow::Result<Accept> {
        let mut tx = self.begin().await?;
        let invite: Option<(Uuid, Uuid, String, String)> = sqlx::query_as(self.auth_sql(
            "SELECT id, workspace_id, email, role FROM auth.invites
             WHERE token_hash = $1 AND expires_at > now() FOR UPDATE",
        ))
        .bind(token_hash)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((id, workspace, invited, role)) = invite else { return Ok(Accept::Invalid) };
        if invited != email {
            return Ok(Accept::WrongEmail);
        }
        let joined = sqlx::query(self.auth_sql(
            "INSERT INTO auth.memberships (workspace_id, user_id, role) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        ))
        .bind(workspace)
        .bind(user)
        .bind(role)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        sqlx::query(self.auth_sql("DELETE FROM auth.invites WHERE id = $1")).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(if joined { Accept::Joined(workspace) } else { Accept::AlreadyMember(workspace) })
    }

    /// Changes a member's role (`Some`) or removes them (`None`), unless that would leave the
    /// workspace without an admin. The workspace's row is locked first, so two admins demoting each
    /// other at once can't both succeed and leave none.
    pub async fn change_member(&self, workspace: Uuid, user: Uuid, role: Option<Role>) -> anyhow::Result<MemberChange> {
        let mut tx = self.begin().await?;
        sqlx::query(self.auth_sql("SELECT 1 FROM auth.workspaces WHERE id = $1 FOR UPDATE"))
            .bind(workspace)
            .execute(&mut *tx)
            .await?;
        let current: Option<String> = sqlx::query_scalar(
            self.auth_sql("SELECT role FROM auth.memberships WHERE workspace_id = $1 AND user_id = $2"),
        )
        .bind(workspace)
        .bind(user)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(current) = current.as_deref().map(Role::parse).transpose()? else {
            return Ok(MemberChange::NotMember);
        };
        if current == Role::Admin && role != Some(Role::Admin) {
            let admins: i64 = sqlx::query_scalar(
                self.auth_sql("SELECT count(*) FROM auth.memberships WHERE workspace_id = $1 AND role = 'admin'"),
            )
            .bind(workspace)
            .fetch_one(&mut *tx)
            .await?;
            if admins <= 1 {
                return Ok(MemberChange::LastAdmin);
            }
        }
        match role {
            Some(role) => sqlx::query(
                self.auth_sql("UPDATE auth.memberships SET role = $3 WHERE workspace_id = $1 AND user_id = $2"),
            )
            .bind(workspace)
            .bind(user)
            .bind(role.as_str()),
            None => sqlx::query(self.auth_sql("DELETE FROM auth.memberships WHERE workspace_id = $1 AND user_id = $2"))
                .bind(workspace)
                .bind(user),
        }
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(MemberChange::Done)
    }
    /// Deletes invites that have expired, alongside the session sweep.
    pub async fn delete_expired_invites(&self) -> anyhow::Result<u64> {
        Ok(sqlx::query(self.auth_sql("DELETE FROM auth.invites WHERE expires_at <= now()"))
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    /// Ends an invite's validity as of now. Tests use it to simulate expiry.
    #[cfg(test)]
    pub async fn expire_invite(&self, id: Uuid) {
        sqlx::query(self.auth_sql("UPDATE auth.invites SET expires_at = now() - interval '1 second' WHERE id = $1"))
            .bind(id)
            .execute(&self.pool)
            .await
            .unwrap();
    }
}
