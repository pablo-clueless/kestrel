//! Two-factor sign-in, in the `auth` schema: the TOTP secret on `auth.users`, recovery codes, and the
//! challenge between the password step and the code step. Policy lives in `crate::auth::two_factor`.

use std::time::Duration;

use uuid::Uuid;

use super::Db;

/// Where a user's two-factor setup stands.
pub struct TwoFactorRow {
    /// Encrypted; present from setup on, confirmed or not.
    pub sealed_secret: Option<Vec<u8>>,
    pub enabled: bool,
    pub last_step: Option<i64>,
    pub recovery_codes_left: i64,
}

/// Failed code attempts allowed on one challenge before it stops working.
pub const MAX_CHALLENGE_ATTEMPTS: i32 = 5;

impl Db {
    pub async fn two_factor(&self, user: Uuid) -> anyhow::Result<TwoFactorRow> {
        let (sealed_secret, enabled, last_step, recovery_codes_left): (Option<Vec<u8>>, bool, Option<i64>, i64) =
            sqlx::query_as(self.auth_sql(
                "SELECT totp_secret, totp_enabled_at IS NOT NULL, totp_last_step,
                        (SELECT count(*) FROM auth.recovery_codes r WHERE r.user_id = u.id)
                 FROM auth.users u WHERE id = $1",
            ))
            .bind(user)
            .fetch_one(&self.pool)
            .await?;
        Ok(TwoFactorRow { sealed_secret, enabled, last_step, recovery_codes_left })
    }

    /// Stores a new, unconfirmed secret. `false` (nothing changed) if two-factor is already on:
    /// it has to be turned off first, so a stolen session can't swap in its own authenticator.
    pub async fn set_pending_totp(&self, user: Uuid, sealed: &[u8]) -> anyhow::Result<bool> {
        let updated = sqlx::query(self.auth_sql(
            "UPDATE auth.users SET totp_secret = $2, totp_last_step = NULL
             WHERE id = $1 AND totp_enabled_at IS NULL",
        ))
        .bind(user)
        .bind(sealed)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Turns two-factor on (the pending secret was confirmed with a code for `step`) and stores fresh
    /// recovery codes, in one transaction.
    pub async fn enable_totp(&self, user: Uuid, step: i64, recovery_hashes: &[Vec<u8>]) -> anyhow::Result<bool> {
        let mut tx = self.begin().await?;
        let enabled = sqlx::query(self.auth_sql(
            "UPDATE auth.users SET totp_enabled_at = now(), totp_last_step = $2
             WHERE id = $1 AND totp_secret IS NOT NULL AND totp_enabled_at IS NULL",
        ))
        .bind(user)
        .bind(step)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if !enabled {
            return Ok(false);
        }
        self.replace_recovery_codes_in(&mut tx, user, recovery_hashes).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Turns two-factor off: the secret and recovery codes go.
    pub async fn disable_totp(&self, user: Uuid) -> anyhow::Result<()> {
        let mut tx = self.begin().await?;
        sqlx::query(self.auth_sql(
            "UPDATE auth.users SET totp_secret = NULL, totp_enabled_at = NULL, totp_last_step = NULL WHERE id = $1",
        ))
        .bind(user)
        .execute(&mut *tx)
        .await?;
        sqlx::query(self.auth_sql("DELETE FROM auth.recovery_codes WHERE user_id = $1"))
            .bind(user)
            .execute(&mut *tx)
            .await?;
        sqlx::query(self.auth_sql("DELETE FROM auth.login_challenges WHERE user_id = $1"))
            .bind(user)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn replace_recovery_codes(&self, user: Uuid, hashes: &[Vec<u8>]) -> anyhow::Result<()> {
        let mut tx = self.begin().await?;
        self.replace_recovery_codes_in(&mut tx, user, hashes).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn replace_recovery_codes_in(
        &self,
        tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
        user: Uuid,
        hashes: &[Vec<u8>],
    ) -> anyhow::Result<()> {
        sqlx::query(self.auth_sql("DELETE FROM auth.recovery_codes WHERE user_id = $1"))
            .bind(user)
            .execute(&mut **tx)
            .await?;
        sqlx::query(self.auth_sql(
            "INSERT INTO auth.recovery_codes (user_id, code_hash) SELECT $1, unnest($2::bytea[]) ON CONFLICT DO NOTHING",
        ))
        .bind(user)
        .bind(hashes)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Records that a code for `step` was accepted. `false` if one for this step (or a later one) was
    /// already used: checked and set in one statement, so two sign-ins racing with one code can't
    /// both pass.
    pub async fn use_totp_step(&self, user: Uuid, step: i64) -> anyhow::Result<bool> {
        let updated = sqlx::query(self.auth_sql(
            "UPDATE auth.users SET totp_last_step = $2
             WHERE id = $1 AND (totp_last_step IS NULL OR totp_last_step < $2)",
        ))
        .bind(user)
        .bind(step)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Uses up a recovery code. `false` if it isn't one of the user's (or was used already).
    pub async fn use_recovery_code(&self, user: Uuid, hash: &[u8]) -> anyhow::Result<bool> {
        let deleted =
            sqlx::query(self.auth_sql("DELETE FROM auth.recovery_codes WHERE user_id = $1 AND code_hash = $2"))
                .bind(user)
                .bind(hash)
                .execute(&self.pool)
                .await?;
        Ok(deleted.rows_affected() == 1)
    }

    /// The password step passed for `user`; the code step must follow within `lifetime`.
    pub async fn create_login_challenge(
        &self,
        token_hash: &[u8],
        user: Uuid,
        lifetime: Duration,
    ) -> anyhow::Result<()> {
        sqlx::query(self.auth_sql(
            "INSERT INTO auth.login_challenges (token_hash, user_id, expires_at)
             VALUES ($1, $2, now() + make_interval(secs => $3))",
        ))
        .bind(token_hash)
        .bind(user)
        .bind(lifetime.as_secs_f64())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Counts an attempt against a live challenge and returns its user. `None` once it's unknown,
    /// expired or out of attempts.
    pub async fn attempt_login_challenge(&self, token_hash: &[u8]) -> anyhow::Result<Option<Uuid>> {
        Ok(sqlx::query_scalar(self.auth_sql(
            "UPDATE auth.login_challenges SET attempts = attempts + 1
             WHERE token_hash = $1 AND expires_at > now() AND attempts < $2
             RETURNING user_id",
        ))
        .bind(token_hash)
        .bind(MAX_CHALLENGE_ATTEMPTS)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn delete_login_challenge(&self, token_hash: &[u8]) -> anyhow::Result<()> {
        sqlx::query(self.auth_sql("DELETE FROM auth.login_challenges WHERE token_hash = $1"))
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Deletes challenges that have expired, alongside the session sweep.
    pub async fn delete_expired_login_challenges(&self) -> anyhow::Result<u64> {
        Ok(sqlx::query(self.auth_sql("DELETE FROM auth.login_challenges WHERE expires_at <= now()"))
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    /// Turns two-factor off for the account with this email (the `engine user reset-2fa` command, for
    /// someone who lost their authenticator and their recovery codes). `false` if there's no such
    /// account.
    pub async fn reset_two_factor_by_email(&self, email: &str) -> anyhow::Result<bool> {
        let id: Option<Uuid> = sqlx::query_scalar(self.auth_sql("SELECT id FROM auth.users WHERE email = $1"))
            .bind(email)
            .fetch_optional(&self.pool)
            .await?;
        let Some(id) = id else { return Ok(false) };
        self.disable_totp(id).await?;
        Ok(true)
    }
}
