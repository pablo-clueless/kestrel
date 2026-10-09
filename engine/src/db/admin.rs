//! The admin pages' queries: every user and workspace on the engine, across the `auth` schema.
//! Who may run them (`KESTREL_ADMIN_EMAILS`) is decided in `crate::auth::admin`; this is only the SQL.

use uuid::Uuid;

use super::Db;

pub struct OverviewRow {
    pub users: i64,
    pub verified_users: i64,
    pub two_factor_users: i64,
    pub disabled_users: i64,
    pub signups_last_7d: i64,
    pub workspaces: i64,
    /// Workspaces with more than one member.
    pub shared_workspaces: i64,
    pub live_sessions: i64,
}

pub struct AdminUserRow {
    pub id: Uuid,
    pub email: String,
    pub name: Option<String>,
    pub email_verified: bool,
    pub two_factor: bool,
    pub disabled: bool,
    pub created_at_ms: i64,
    /// The most recent use of any of its sessions still on record; `None` once they've all expired.
    pub last_seen_at_ms: Option<i64>,
    pub workspaces: i64,
    pub live_sessions: i64,
}

pub struct AdminWorkspaceRow {
    pub id: Uuid,
    pub name: Option<String>,
    pub admin_emails: Vec<String>,
    pub members: i64,
    pub created_at_ms: i64,
    /// Tables, indexes and TOAST of its schema.
    pub size_bytes: i64,
}

/// The columns [`AdminUserRow`] is read from, in order.
const USER_COLUMNS: &str = "u.id, u.email, u.name, u.email_verified_at IS NOT NULL, u.totp_enabled_at IS NOT NULL,
    u.disabled_at IS NOT NULL, (extract(epoch FROM u.created_at) * 1000)::bigint,
    (SELECT (extract(epoch FROM max(s.last_seen_at)) * 1000)::bigint FROM auth.sessions s WHERE s.user_id = u.id),
    (SELECT count(*) FROM auth.memberships m WHERE m.user_id = u.id),
    (SELECT count(*) FROM auth.sessions s WHERE s.user_id = u.id AND s.expires_at > now())";

type UserTuple = (Uuid, String, Option<String>, bool, bool, bool, i64, Option<i64>, i64, i64);

fn user_row(
    (id, email, name, email_verified, two_factor, disabled, created_at_ms, last_seen_at_ms, workspaces, live_sessions): UserTuple,
) -> AdminUserRow {
    AdminUserRow {
        id,
        email,
        name,
        email_verified,
        two_factor,
        disabled,
        created_at_ms,
        last_seen_at_ms,
        workspaces,
        live_sessions,
    }
}

impl Db {
    pub async fn admin_overview(&self) -> anyhow::Result<OverviewRow> {
        type Row = (i64, i64, i64, i64, i64, i64, i64, i64);
        let row: Row = sqlx::query_as(self.auth_sql(
            "SELECT (SELECT count(*) FROM auth.users),
                    (SELECT count(*) FROM auth.users WHERE email_verified_at IS NOT NULL),
                    (SELECT count(*) FROM auth.users WHERE totp_enabled_at IS NOT NULL),
                    (SELECT count(*) FROM auth.users WHERE disabled_at IS NOT NULL),
                    (SELECT count(*) FROM auth.users WHERE created_at > now() - interval '7 days'),
                    (SELECT count(*) FROM auth.workspaces),
                    (SELECT count(*) FROM (SELECT 1 FROM auth.memberships GROUP BY workspace_id HAVING count(*) > 1) s),
                    (SELECT count(*) FROM auth.sessions WHERE expires_at > now())",
        ))
        .fetch_one(&self.pool)
        .await?;
        let (users, verified_users, two_factor_users, disabled_users, signups_last_7d, workspaces, shared, live) = row;
        Ok(OverviewRow {
            users,
            verified_users,
            two_factor_users,
            disabled_users,
            signups_last_7d,
            workspaces,
            shared_workspaces: shared,
            live_sessions: live,
        })
    }

    /// Every account, newest first. `limit` caps it (the overview's recent sign-ups).
    pub async fn admin_users(&self, limit: Option<i64>) -> anyhow::Result<Vec<AdminUserRow>> {
        let sql = format!("SELECT {USER_COLUMNS} FROM auth.users u ORDER BY u.created_at DESC, u.email LIMIT $1");
        let rows: Vec<UserTuple> = sqlx::query_as(self.auth_sql(&sql)).bind(limit).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(user_row).collect())
    }

    pub async fn admin_user(&self, user: Uuid) -> anyhow::Result<Option<AdminUserRow>> {
        let sql = format!("SELECT {USER_COLUMNS} FROM auth.users u WHERE u.id = $1");
        let row: Option<UserTuple> = sqlx::query_as(self.auth_sql(&sql)).bind(user).fetch_optional(&self.pool).await?;
        Ok(row.map(user_row))
    }

    /// Every workspace, newest first, with how much space its schema takes.
    pub async fn admin_workspaces(&self) -> anyhow::Result<Vec<AdminWorkspaceRow>> {
        type Row = (Uuid, Option<String>, Vec<String>, i64, i64, i64);
        let rows: Vec<Row> = sqlx::query_as(self.auth_sql(
            "SELECT w.id, w.name,
                    ARRAY(SELECT u.email FROM auth.memberships a JOIN auth.users u ON u.id = a.user_id
                          WHERE a.workspace_id = w.id AND a.role = 'admin' ORDER BY a.created_at, u.email),
                    (SELECT count(*) FROM auth.memberships c WHERE c.workspace_id = w.id),
                    (extract(epoch FROM w.created_at) * 1000)::bigint,
                    coalesce((SELECT sum(pg_catalog.pg_total_relation_size(c.oid))
                              FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                              WHERE n.nspname = w.schema_name AND c.relkind IN ('r', 'm')), 0)::bigint
             FROM auth.workspaces w
             ORDER BY w.created_at DESC, w.id",
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, name, admin_emails, members, created_at_ms, size_bytes)| AdminWorkspaceRow {
                id,
                name,
                admin_emails,
                members,
                created_at_ms,
                size_bytes,
            })
            .collect())
    }

    /// Marks the email verified, as following a verification link would. `false` if there's no such user.
    pub async fn admin_verify_email(&self, user: Uuid) -> anyhow::Result<bool> {
        let updated = sqlx::query(
            self.auth_sql("UPDATE auth.users SET email_verified_at = coalesce(email_verified_at, now()) WHERE id = $1"),
        )
        .bind(user)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Ends every session of `user`, and any half-finished two-factor sign-in. How many sessions ended.
    pub async fn end_all_sessions(&self, user: Uuid) -> anyhow::Result<u64> {
        let mut tx = self.begin().await?;
        let ended = sqlx::query(self.auth_sql("DELETE FROM auth.sessions WHERE user_id = $1"))
            .bind(user)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        sqlx::query(self.auth_sql("DELETE FROM auth.login_challenges WHERE user_id = $1"))
            .bind(user)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(ended)
    }

    /// Disables `user` (signing them out everywhere, in the same transaction) or enables them again.
    /// `false` if there's no such user.
    pub async fn set_disabled(&self, user: Uuid, disabled: bool) -> anyhow::Result<bool> {
        let mut tx = self.begin().await?;
        let updated = sqlx::query(self.auth_sql(
            "UPDATE auth.users SET disabled_at = CASE WHEN $2 THEN coalesce(disabled_at, now()) END WHERE id = $1",
        ))
        .bind(user)
        .bind(disabled)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if disabled {
            for table in ["auth.sessions", "auth.login_challenges"] {
                sqlx::query(self.auth_sql(&format!("DELETE FROM {table} WHERE user_id = $1")))
                    .bind(user)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(updated == 1)
    }
}
