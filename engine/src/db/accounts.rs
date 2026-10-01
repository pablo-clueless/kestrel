//! Users, sessions and memberships, in the shared `auth` schema. Policy (hashing, rate limits,
//! cookies) lives in `crate::auth`; this is only the SQL.

use std::time::Duration;

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::Db;

pub struct UserRow {
    pub id: Uuid,
    pub password_hash: String,
    pub email_verified: bool,
}

/// The user behind a valid session.
#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user_id: Uuid,
    pub email: String,
    pub email_verified: bool,
    /// Whether `last_seen_at` is old enough to be bumped (at most once a minute).
    pub stale: bool,
}

pub struct SessionRow {
    pub id_hash: Vec<u8>,
    pub created_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub expires_at_ms: i64,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
}

/// What an emailed link is for. Each kind is only accepted by its own route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailPurpose {
    Verify,
    Reset,
}

impl EmailPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Reset => "reset",
        }
    }
}

/// What signing up can run into, besides a database error.
pub enum CreateUser {
    Created(Uuid),
    EmailTaken,
}

impl Db {
    pub async fn user_by_email(&self, email: &str) -> anyhow::Result<Option<UserRow>> {
        let row: Option<(Uuid, String, bool)> = sqlx::query_as(
            self.auth_sql("SELECT id, password_hash, email_verified_at IS NOT NULL FROM auth.users WHERE email = $1"),
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, password_hash, email_verified)| UserRow { id, password_hash, email_verified }))
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
        let row: Option<(Uuid, String, bool, bool)> = sqlx::query_as(self.auth_sql(
            "SELECT s.user_id, u.email, u.email_verified_at IS NOT NULL,
                    s.last_seen_at < now() - interval '1 minute'
             FROM auth.sessions s JOIN auth.users u ON u.id = s.user_id
             WHERE s.id_hash = $1 AND s.expires_at > now()",
        ))
        .bind(id_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(user_id, email, email_verified, stale)| SessionUser { user_id, email, email_verified, stale }))
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

    pub async fn password_hash(&self, user: Uuid) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar(self.auth_sql("SELECT password_hash FROM auth.users WHERE id = $1"))
            .bind(user)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn set_password_hash(&self, user: Uuid, hash: &str) -> anyhow::Result<()> {
        sqlx::query(self.auth_sql("UPDATE auth.users SET password_hash = $2 WHERE id = $1"))
            .bind(user)
            .bind(hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Sets a new password and ends every session of `user` except `keep`, in one transaction: a
    /// stolen session must not survive the password change meant to lock its thief out.
    pub async fn change_password(&self, user: Uuid, hash: &str, keep: &[u8]) -> anyhow::Result<u64> {
        let mut tx = self.begin().await?;
        sqlx::query(self.auth_sql("UPDATE auth.users SET password_hash = $2 WHERE id = $1"))
            .bind(user)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
        let ended = sqlx::query(self.auth_sql("DELETE FROM auth.sessions WHERE user_id = $1 AND id_hash <> $2"))
            .bind(user)
            .bind(keep)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(ended)
    }

    /// `user`'s live sessions, most recently used first.
    pub async fn sessions_of(&self, user: Uuid) -> anyhow::Result<Vec<SessionRow>> {
        /// id_hash, created, last seen, expires (ms), user agent, ip.
        type Row = (Vec<u8>, i64, i64, i64, Option<String>, Option<String>);
        let rows: Vec<Row> = sqlx::query_as(self.auth_sql(
            "SELECT id_hash,
                    (extract(epoch FROM created_at) * 1000)::bigint,
                    (extract(epoch FROM last_seen_at) * 1000)::bigint,
                    (extract(epoch FROM expires_at) * 1000)::bigint,
                    user_agent, ip
             FROM auth.sessions WHERE user_id = $1 AND expires_at > now()
             ORDER BY last_seen_at DESC",
        ))
        .bind(user)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id_hash, created_at_ms, last_seen_at_ms, expires_at_ms, user_agent, ip)| SessionRow {
                id_hash,
                created_at_ms,
                last_seen_at_ms,
                expires_at_ms,
                user_agent,
                ip,
            })
            .collect())
    }

    /// Ends one of `user`'s sessions. `false` if they have no such session (someone else's counts
    /// as none).
    pub async fn delete_session_of(&self, user: Uuid, id_hash: &[u8]) -> anyhow::Result<bool> {
        let deleted = sqlx::query(self.auth_sql("DELETE FROM auth.sessions WHERE user_id = $1 AND id_hash = $2"))
            .bind(user)
            .bind(id_hash)
            .execute(&self.pool)
            .await?;
        Ok(deleted.rows_affected() == 1)
    }

    /// Deletes sessions that have expired. They're already refused; this only reclaims the rows.
    pub async fn delete_expired_sessions(&self) -> anyhow::Result<u64> {
        Ok(sqlx::query(self.auth_sql("DELETE FROM auth.sessions WHERE expires_at <= now()"))
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    /// Stores an emailed link's token (its hash) for `user`, valid for `lifetime`.
    pub async fn create_email_token(
        &self,
        token_hash: &[u8],
        user: Uuid,
        purpose: EmailPurpose,
        lifetime: Duration,
    ) -> anyhow::Result<()> {
        sqlx::query(self.auth_sql(
            "INSERT INTO auth.email_tokens (token_hash, user_id, purpose, expires_at)
             VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
        ))
        .bind(token_hash)
        .bind(user)
        .bind(purpose.as_str())
        .bind(lifetime.as_secs_f64())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Uses up a live token for `purpose`. Deleting it is what makes it single-use, so two clicks
    /// racing can't both succeed. The user it belonged to, or `None` if it's unknown, expired,
    /// already used, or for the other purpose.
    async fn take_email_token(
        &self,
        tx: &mut Transaction<'static, Postgres>,
        token_hash: &[u8],
        purpose: EmailPurpose,
    ) -> anyhow::Result<Option<Uuid>> {
        Ok(sqlx::query_scalar(self.auth_sql(
            "DELETE FROM auth.email_tokens WHERE token_hash = $1 AND purpose = $2 AND expires_at > now()
             RETURNING user_id",
        ))
        .bind(token_hash)
        .bind(purpose.as_str())
        .fetch_optional(&mut **tx)
        .await?)
    }

    /// Marks the email verified with a `verify` token, and drops the user's other verify links.
    /// The user, or `None` if the token isn't valid.
    pub async fn verify_email(&self, token_hash: &[u8]) -> anyhow::Result<Option<Uuid>> {
        let mut tx = self.begin().await?;
        let Some(user) = self.take_email_token(&mut tx, token_hash, EmailPurpose::Verify).await? else {
            return Ok(None);
        };
        self.mark_verified(&mut tx, user).await?;
        self.delete_email_tokens(&mut tx, user, EmailPurpose::Verify).await?;
        tx.commit().await?;
        Ok(Some(user))
    }

    /// Sets a new password with a `reset` token, in one transaction: the password changes, every
    /// session ends (whoever is signed in may not be the owner), the user's other reset links stop
    /// working, and the email counts as verified, since following the link proved the user reads
    /// it. The user, or `None` if the token isn't valid.
    pub async fn reset_password(&self, token_hash: &[u8], password_hash: &str) -> anyhow::Result<Option<Uuid>> {
        let mut tx = self.begin().await?;
        let Some(user) = self.take_email_token(&mut tx, token_hash, EmailPurpose::Reset).await? else {
            return Ok(None);
        };
        sqlx::query(self.auth_sql("UPDATE auth.users SET password_hash = $2 WHERE id = $1"))
            .bind(user)
            .bind(password_hash)
            .execute(&mut *tx)
            .await?;
        self.mark_verified(&mut tx, user).await?;
        sqlx::query(self.auth_sql("DELETE FROM auth.sessions WHERE user_id = $1")).bind(user).execute(&mut *tx).await?;
        self.delete_email_tokens(&mut tx, user, EmailPurpose::Reset).await?;
        tx.commit().await?;
        Ok(Some(user))
    }

    async fn mark_verified(&self, tx: &mut Transaction<'static, Postgres>, user: Uuid) -> anyhow::Result<()> {
        sqlx::query(
            self.auth_sql("UPDATE auth.users SET email_verified_at = coalesce(email_verified_at, now()) WHERE id = $1"),
        )
        .bind(user)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn delete_email_tokens(
        &self,
        tx: &mut Transaction<'static, Postgres>,
        user: Uuid,
        purpose: EmailPurpose,
    ) -> anyhow::Result<()> {
        sqlx::query(self.auth_sql("DELETE FROM auth.email_tokens WHERE user_id = $1 AND purpose = $2"))
            .bind(user)
            .bind(purpose.as_str())
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    /// Deletes email tokens that have expired, alongside the session sweep.
    pub async fn delete_expired_email_tokens(&self) -> anyhow::Result<u64> {
        Ok(sqlx::query(self.auth_sql("DELETE FROM auth.email_tokens WHERE expires_at <= now()"))
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    /// Ends an emailed link's validity as of now. Tests use it to simulate expiry.
    #[cfg(test)]
    pub async fn expire_email_token(&self, token_hash: &[u8]) {
        sqlx::query(
            self.auth_sql(
                "UPDATE auth.email_tokens SET expires_at = now() - interval '1 second' WHERE token_hash = $1",
            ),
        )
        .bind(token_hash)
        .execute(&self.pool)
        .await
        .unwrap();
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
