//! Users, sessions and memberships, in the shared `auth` schema. Policy (hashing, rate limits,
//! cookies) lives in `crate::auth`; this is only the SQL.

use std::time::Duration;

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::Db;

pub struct UserRow {
    pub id: Uuid,
    pub password_hash: String,
}

/// The user behind a valid session.
#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user_id: Uuid,
    pub email: String,
    /// Whether `last_seen_at` is old enough to be bumped (at most once a minute).
    pub stale: bool,
}

/// What signing up can run into, besides a database error.
pub enum CreateUser {
    Created(Uuid),
    EmailTaken,
}

impl Db {
    pub async fn user_by_email(&self, email: &str) -> anyhow::Result<Option<UserRow>> {
        let row: Option<(Uuid, String)> =
            sqlx::query_as(self.auth_sql("SELECT id, password_hash FROM auth.users WHERE email = $1"))
                .bind(email)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(id, password_hash)| UserRow { id, password_hash }))
    }

    /// Creates a user and gives them a workspace in one transaction: `claim` (a browser's existing
    /// workspace) if it exists and nobody owns it yet, else a new one.
    pub async fn create_user(
        &self,
        email: &str,
        password_hash: &str,
        claim: Option<Uuid>,
    ) -> anyhow::Result<CreateUser> {
        let mut tx = self.begin().await?;
        let id = Uuid::new_v4();
        let inserted = sqlx::query(self.auth_sql(
            "INSERT INTO auth.users (id, email, password_hash) VALUES ($1, $2, $3) ON CONFLICT (email) DO NOTHING",
        ))
        .bind(id)
        .bind(email)
        .bind(password_hash)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            return Ok(CreateUser::EmailTaken);
        }
        self.give_workspace(&mut tx, id, claim).await?;
        tx.commit().await?;
        Ok(CreateUser::Created(id))
    }

    /// Makes sure `user` owns a workspace, claiming `claim` if possible (see [`Self::create_user`]).
    /// A user who already has one keeps it, and `claim` is ignored. Returns the workspace.
    pub async fn ensure_user_workspace(&self, user: Uuid, claim: Option<Uuid>) -> anyhow::Result<Uuid> {
        let mut tx = self.begin().await?;
        let workspace = self.give_workspace(&mut tx, user, claim).await?;
        tx.commit().await?;
        Ok(workspace)
    }

    async fn give_workspace(
        &self,
        tx: &mut Transaction<'static, Postgres>,
        user: Uuid,
        claim: Option<Uuid>,
    ) -> anyhow::Result<Uuid> {
        // Serialise per user, so two first logins at once can't each create a workspace.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(format!("user:{user}"))
            .execute(&mut **tx)
            .await?;
        if let Some(existing) = self.default_workspace_in(tx, user).await? {
            return Ok(existing);
        }
        if let Some(claim) = claim
            && self.claim_workspace(tx, user, claim).await?
        {
            return Ok(claim);
        }
        let workspace = Uuid::new_v4();
        self.ensure_workspace_in(tx, workspace).await?;
        self.add_owner(tx, workspace, user).await?;
        Ok(workspace)
    }

    /// Makes `user` the owner of existing workspace `id` if it has no owner. The unique index on
    /// owners settles a race between two users claiming the same id: only one insert lands.
    async fn claim_workspace(
        &self,
        tx: &mut Transaction<'static, Postgres>,
        user: Uuid,
        id: Uuid,
    ) -> anyhow::Result<bool> {
        let claimed = sqlx::query(self.auth_sql(
            "INSERT INTO auth.memberships (workspace_id, user_id, role)
             SELECT id, $2, 'owner' FROM auth.workspaces WHERE id = $1
             ON CONFLICT DO NOTHING",
        ))
        .bind(id)
        .bind(user)
        .execute(&mut **tx)
        .await?;
        Ok(claimed.rows_affected() == 1)
    }

    async fn add_owner(
        &self,
        tx: &mut Transaction<'static, Postgres>,
        workspace: Uuid,
        user: Uuid,
    ) -> anyhow::Result<()> {
        sqlx::query(
            self.auth_sql("INSERT INTO auth.memberships (workspace_id, user_id, role) VALUES ($1, $2, 'owner')"),
        )
        .bind(workspace)
        .bind(user)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// The workspace a user lands in: the first one they joined.
    async fn default_workspace_in(
        &self,
        tx: &mut Transaction<'static, Postgres>,
        user: Uuid,
    ) -> anyhow::Result<Option<Uuid>> {
        Ok(sqlx::query_scalar(self.auth_sql(
            "SELECT workspace_id FROM auth.memberships WHERE user_id = $1 ORDER BY created_at, workspace_id LIMIT 1",
        ))
        .bind(user)
        .fetch_optional(&mut **tx)
        .await?)
    }

    pub async fn is_member(&self, user: Uuid, workspace: Uuid) -> anyhow::Result<bool> {
        let found: Option<i32> = sqlx::query_scalar(
            self.auth_sql("SELECT 1 FROM auth.memberships WHERE user_id = $1 AND workspace_id = $2"),
        )
        .bind(user)
        .bind(workspace)
        .fetch_optional(&self.pool)
        .await?;
        Ok(found.is_some())
    }

    pub async fn create_session(
        &self,
        id_hash: &[u8],
        user: Uuid,
        lifetime: Duration,
        user_agent: Option<&str>,
        ip: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(self.auth_sql(
            "INSERT INTO auth.sessions (id_hash, user_id, expires_at, user_agent, ip)
             VALUES ($1, $2, now() + make_interval(secs => $3), $4, $5)",
        ))
        .bind(id_hash)
        .bind(user)
        .bind(lifetime.as_secs_f64())
        .bind(user_agent)
        .bind(ip)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The live session with this hash, if any. Expired sessions count as absent.
    pub async fn session(&self, id_hash: &[u8]) -> anyhow::Result<Option<SessionUser>> {
        let row: Option<(Uuid, String, bool)> = sqlx::query_as(self.auth_sql(
            "SELECT s.user_id, u.email, s.last_seen_at < now() - interval '1 minute'
             FROM auth.sessions s JOIN auth.users u ON u.id = s.user_id
             WHERE s.id_hash = $1 AND s.expires_at > now()",
        ))
        .bind(id_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(user_id, email, stale)| SessionUser { user_id, email, stale }))
    }

    /// Slides a session's expiry forward to `lifetime` from now.
    pub async fn touch_session(&self, id_hash: &[u8], lifetime: Duration) -> anyhow::Result<()> {
        sqlx::query(self.auth_sql(
            "UPDATE auth.sessions SET last_seen_at = now(), expires_at = now() + make_interval(secs => $2)
             WHERE id_hash = $1",
        ))
        .bind(id_hash)
        .bind(lifetime.as_secs_f64())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_session(&self, id_hash: &[u8]) -> anyhow::Result<()> {
        sqlx::query(self.auth_sql("DELETE FROM auth.sessions WHERE id_hash = $1"))
            .bind(id_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Ends a session as of now. Tests use it to simulate expiry.
    #[cfg(test)]
    pub async fn expire_session(&self, id_hash: &[u8]) {
        sqlx::query(
            self.auth_sql("UPDATE auth.sessions SET expires_at = now() - interval '1 second' WHERE id_hash = $1"),
        )
        .bind(id_hash)
        .execute(&self.pool)
        .await
        .unwrap();
    }
}
