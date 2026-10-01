//! Postgres. See HANDOFF.md → Accounts → Storage.
//!
//! Each workspace's data lives in its own schema, `ws_<uuid hex>`, so one user's rows can't turn up
//! in another's results because a `WHERE` was forgotten: the tables simply aren't there. Tables
//! shared across workspaces live in the `auth` schema: the workspace registry, users, sessions and
//! memberships (`accounts.rs`).
//!
//! The rules that keep this sound:
//! - A workspace query runs only inside [`Db::pinned`], a transaction whose first statement is
//!   `SET LOCAL search_path = "ws_…"`. `SET LOCAL` ends with the transaction, so a pooled
//!   connection never carries one workspace's path into the next request. (A plain `SET` would.)
//! - Every pooled connection starts with an empty `search_path`, so a query that forgot to pin finds
//!   no tables and fails instead of reading some other schema.
//! - Schema names are built only from a parsed [`Uuid`], never from request text.
//! - Shared queries name their schema (`auth.workspaces`).

pub mod accounts;
pub mod crypto;
pub mod import_sqlite;

use anyhow::Context;
use sqlx::{
    AssertSqlSafe, Executor, PgConnection, PgPool, Postgres, Transaction,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use uuid::Uuid;

use crypto::SecretsCipher;

/// Bump with a new entry; never edit a shipped one. Applied to the `auth` schema.
const AUTH_MIGRATIONS: &[&str] = &[
    // 1: the workspace registry.
    r#"
    CREATE TABLE auth.workspaces (
        id uuid PRIMARY KEY,
        schema_name text NOT NULL UNIQUE,
        schema_version int NOT NULL DEFAULT 0,
        created_at timestamptz NOT NULL DEFAULT now()
    );
    "#,
    // 2: accounts (A1). Emails are stored trimmed and lowercased by the engine, so plain `text` with
    // a unique index does what `citext` would, without needing an extension.
    r#"
    CREATE TABLE auth.users (
        id uuid PRIMARY KEY,
        email text NOT NULL UNIQUE,
        password_hash text NOT NULL,
        created_at timestamptz NOT NULL DEFAULT now(),
        email_verified_at timestamptz
    );
    -- Only the SHA-256 of a session token is stored; the token itself is in the cookie.
    CREATE TABLE auth.sessions (
        id_hash bytea PRIMARY KEY,
        user_id uuid NOT NULL REFERENCES auth.users (id) ON DELETE CASCADE,
        created_at timestamptz NOT NULL DEFAULT now(),
        expires_at timestamptz NOT NULL,
        last_seen_at timestamptz NOT NULL DEFAULT now(),
        user_agent text,
        ip text
    );
    CREATE INDEX sessions_by_user ON auth.sessions (user_id);
    CREATE TABLE auth.memberships (
        workspace_id uuid NOT NULL REFERENCES auth.workspaces (id) ON DELETE CASCADE,
        user_id uuid NOT NULL REFERENCES auth.users (id) ON DELETE CASCADE,
        role text NOT NULL CHECK (role IN ('owner')),
        created_at timestamptz NOT NULL DEFAULT now(),
        PRIMARY KEY (workspace_id, user_id)
    );
    CREATE INDEX memberships_by_user ON auth.memberships (user_id);
    -- At most one owner per workspace: two users claiming the same browser workspace can't both win.
    CREATE UNIQUE INDEX one_owner_per_workspace ON auth.memberships (workspace_id) WHERE role = 'owner';
    "#,
    // 3: email links (A3). Like sessions, only a token's SHA-256 is stored. Using a token deletes
    // it, which is what makes it single-use.
    r#"
    CREATE TABLE auth.email_tokens (
        token_hash bytea PRIMARY KEY,
        user_id uuid NOT NULL REFERENCES auth.users (id) ON DELETE CASCADE,
        purpose text NOT NULL CHECK (purpose IN ('verify', 'reset')),
        created_at timestamptz NOT NULL DEFAULT now(),
        expires_at timestamptz NOT NULL
    );
    CREATE INDEX email_tokens_by_user ON auth.email_tokens (user_id, purpose);
    "#,
];

/// Bump with a new entry; never edit a shipped one. Applied to every workspace schema, with
/// `search_path` pinned to it, so table names are unqualified.
///
/// Documents (collections, reports, …) are `text` holding JSON, not `jsonb`: we never query inside
/// them, and `jsonb` rejects `\u0000`, which can appear in a sampled response body.
pub const WORKSPACE_MIGRATIONS: &[&str] = &[
    // 1: initial schema (the SQLite schema at v2).
    r#"
    -- Workspace fields other than collections and environments, as one JSON object.
    CREATE TABLE workspace_meta (id int PRIMARY KEY CHECK (id = 1), doc text NOT NULL);
    CREATE TABLE collections (id uuid PRIMARY KEY, position int NOT NULL, doc text NOT NULL);
    CREATE TABLE environments (name text PRIMARY KEY, position int NOT NULL, vars text NOT NULL);
    -- `value` is AES-256-GCM: nonce ‖ ciphertext ‖ tag. See `crypto`.
    CREATE TABLE secrets (
        environment text NOT NULL,
        key text NOT NULL,
        value bytea NOT NULL,
        PRIMARY KEY (environment, key)
    );
    CREATE TABLE files (id uuid PRIMARY KEY, bytes bytea NOT NULL, created_at timestamptz NOT NULL DEFAULT now());
    -- Times come from the report (engine clock, ms), not from the database.
    CREATE TABLE runs (
        id uuid PRIMARY KEY,
        kind text NOT NULL,
        status text NOT NULL,
        started_at_ms bigint NOT NULL,
        finished_at_ms bigint NOT NULL,
        report text NOT NULL,
        summary text NOT NULL
    );
    CREATE INDEX runs_by_start ON runs (started_at_ms DESC);
    CREATE TABLE confirmed_hosts (host text PRIMARY KEY, confirmed_at timestamptz NOT NULL DEFAULT now());
    "#,
];

pub struct Db {
    pool: PgPool,
    /// `auth` (tests use a per-run prefix so they can share one database).
    auth_schema: String,
    /// Prefix of workspace schema names: `ws_` (tests: `<run>_ws_`).
    workspace_prefix: String,
    pub cipher: SecretsCipher,
}

impl Db {
    /// Connects, and migrates the `auth` schema. Workspace schemas are migrated when opened.
    pub async fn connect(url: &str, max_connections: u32, cipher: SecretsCipher) -> anyhow::Result<Self> {
        let db = Self::lazy(url, max_connections, cipher, "")?;
        db.migrate_auth().await?;
        Ok(db)
    }

    /// A pool that connects on first use, with schema names prefixed by `prefix`.
    pub fn lazy(url: &str, max_connections: u32, cipher: SecretsCipher, prefix: &str) -> anyhow::Result<Self> {
        let options: PgConnectOptions = url.parse().context("KESTREL_DATABASE_URL is not a valid Postgres URL")?;
        // Names our sessions in `pg_stat_activity`; tests use it to find and end their own.
        let options = options.application_name(&format!("kestrel{}", prefix.trim_end_matches('_')));
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .after_connect(|conn, _| {
                Box::pin(async move {
                    // No schema is searched until a transaction pins one (pg_catalog always is).
                    conn.execute("SET search_path = ''").await?;
                    Ok(())
                })
            })
            .connect_lazy_with(options);
        Ok(Self { pool, auth_schema: format!("{prefix}auth"), workspace_prefix: format!("{prefix}ws_"), cipher })
    }

    /// `ws_<hex>`: only ever built from a `Uuid`, so it's always a safe identifier.
    pub fn schema_of(&self, id: Uuid) -> String {
        format!("{}{}", self.workspace_prefix, id.simple())
    }

    /// `sql` with every `auth.` table reference pointed at this database's auth schema. Only
    /// `auth.<table>` is rewritten: name the schema itself with `quote_ident(&self.auth_schema)`.
    fn auth_sql(&self, sql: &str) -> AssertSqlSafe<String> {
        AssertSqlSafe(sql.replace("auth.", &format!("{}.", quote_ident(&self.auth_schema))))
    }

    /// A transaction with nothing pinned yet, for [`Self::ensure_workspace_in`].
    pub async fn begin(&self) -> anyhow::Result<Transaction<'static, Postgres>> {
        self.pool.begin().await.context("connecting to the database")
    }

    /// A transaction whose unqualified table names resolve to `schema` only.
    pub async fn pinned(&self, schema: &str) -> anyhow::Result<Transaction<'static, Postgres>> {
        let mut tx = self.pool.begin().await.context("connecting to the database")?;
        pin(&mut tx, schema).await?;
        Ok(tx)
    }

    async fn migrate_auth(&self) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await.context("connecting to the database")?;
        // Engines starting at once migrate one at a time.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))").bind(&self.auth_schema).execute(&mut *tx).await?;
        tx.execute(AssertSqlSafe(format!("CREATE SCHEMA IF NOT EXISTS {}", quote_ident(&self.auth_schema)))).await?;
        tx.execute(self.auth_sql(
            "CREATE TABLE IF NOT EXISTS auth.schema_version (id int PRIMARY KEY CHECK (id = 1), version int NOT NULL)",
        ))
        .await?;
        let version: Option<i32> = sqlx::query_scalar(self.auth_sql("SELECT version FROM auth.schema_version"))
            .fetch_optional(&mut *tx)
            .await?;
        let version = version.unwrap_or(0) as usize;
        check_not_newer("auth", version, AUTH_MIGRATIONS.len())?;
        for (i, sql) in AUTH_MIGRATIONS.iter().enumerate().skip(version) {
            sqlx::raw_sql(self.auth_sql(sql))
                .execute(&mut *tx)
                .await
                .with_context(|| format!("migrating the auth schema to v{}", i + 1))?;
        }
        sqlx::query(self.auth_sql(
            "INSERT INTO auth.schema_version (id, version) VALUES (1, $1)
             ON CONFLICT (id) DO UPDATE SET version = excluded.version",
        ))
        .bind(AUTH_MIGRATIONS.len() as i32)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Creates workspace `id` if it doesn't exist and migrates its schema to the current version.
    /// Creating the registry row and the schema is one transaction, so a workspace is never
    /// half-made. Returns the schema name.
    pub async fn ensure_workspace(&self, id: Uuid) -> anyhow::Result<String> {
        let mut tx = self.pool.begin().await.context("connecting to the database")?;
        let schema = self.ensure_workspace_in(&mut tx, id).await?;
        tx.commit().await?;
        Ok(schema)
    }

    /// [`Self::ensure_workspace`] inside the caller's transaction, which ends up pinned to the
    /// workspace's schema. Used where filling the workspace must be atomic with creating it.
    pub async fn ensure_workspace_in(
        &self,
        tx: &mut Transaction<'static, Postgres>,
        id: Uuid,
    ) -> anyhow::Result<String> {
        let schema = self.schema_of(id);
        let tx = &mut **tx;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))").bind(&schema).execute(&mut *tx).await?;
        let version: Option<i32> =
            sqlx::query_scalar(self.auth_sql("SELECT schema_version FROM auth.workspaces WHERE id = $1"))
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let version = match version {
            Some(v) => v as usize,
            None => {
                tx.execute(AssertSqlSafe(format!("CREATE SCHEMA {}", quote_ident(&schema)))).await?;
                sqlx::query(self.auth_sql("INSERT INTO auth.workspaces (id, schema_name) VALUES ($1, $2)"))
                    .bind(id)
                    .bind(&schema)
                    .execute(&mut *tx)
                    .await?;
                0
            }
        };
        check_not_newer(&format!("workspace {id}"), version, WORKSPACE_MIGRATIONS.len())?;
        pin(tx, &schema).await?;
        if version < WORKSPACE_MIGRATIONS.len() {
            for (i, sql) in WORKSPACE_MIGRATIONS.iter().enumerate().skip(version) {
                sqlx::raw_sql(*sql)
                    .execute(&mut *tx)
                    .await
                    .with_context(|| format!("migrating workspace {id} to v{}", i + 1))?;
            }
            sqlx::query(self.auth_sql("UPDATE auth.workspaces SET schema_version = $2 WHERE id = $1"))
                .bind(id)
                .bind(WORKSPACE_MIGRATIONS.len() as i32)
                .execute(&mut *tx)
                .await?;
        }
        Ok(schema)
    }

    /// Whether workspace `id` has been created.
    pub async fn workspace_exists(&self, id: Uuid) -> anyhow::Result<bool> {
        let found: Option<i32> = sqlx::query_scalar(self.auth_sql("SELECT 1 FROM auth.workspaces WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(found.is_some())
    }

    /// Migrates every workspace schema (`engine migrate --all`). One failing workspace doesn't stop
    /// the rest; returns how many were migrated and the failures.
    pub async fn migrate_all(&self) -> anyhow::Result<(usize, Vec<(Uuid, anyhow::Error)>)> {
        let ids: Vec<Uuid> = sqlx::query_scalar(self.auth_sql("SELECT id FROM auth.workspaces ORDER BY created_at"))
            .fetch_all(&self.pool)
            .await?;
        let mut failed = Vec::new();
        for &id in &ids {
            if let Err(err) = self.ensure_workspace(id).await {
                failed.push((id, err));
            }
        }
        Ok((ids.len() - failed.len(), failed))
    }

    #[cfg(test)]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// `SET LOCAL search_path = "<schema>"`, as one cached statement whatever the schema (`true` makes
/// it transaction-local).
async fn pin(conn: &mut PgConnection, schema: &str) -> anyhow::Result<()> {
    sqlx::query("SELECT set_config('search_path', $1, true)").bind(quote_ident(schema)).execute(conn).await?;
    Ok(())
}

fn check_not_newer(what: &str, version: usize, known: usize) -> anyhow::Result<()> {
    if version > known {
        anyhow::bail!(
            "{what} is at schema v{version}, newer than this engine understands (v{known}); upgrade the engine"
        );
    }
    Ok(())
}

/// `"name"`, with embedded quotes doubled. Our names are `[a-z0-9_]` anyway; this is belt and braces.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// A database for one test: its own schema prefix, so tests share one Postgres in parallel without
/// seeing each other. Its schemas are dropped when it goes out of scope, panics included. `None`
/// (with a note on stderr) when `KESTREL_TEST_DATABASE_URL` isn't set.
#[cfg(test)]
pub mod test {
    use std::{ops::Deref, sync::Arc};

    use sqlx::Connection;

    use super::*;

    pub const URL_VAR: &str = "KESTREL_TEST_DATABASE_URL";

    pub struct TestDb {
        db: Arc<Db>,
        url: String,
    }

    impl Deref for TestDb {
        type Target = Arc<Db>;
        fn deref(&self) -> &Arc<Db> {
            &self.db
        }
    }

    impl TestDb {
        /// The same database and schema names, over a pool of `size` connections.
        pub fn with_pool_size(&self, size: u32) -> Db {
            let prefix = self.db.auth_schema.strip_suffix("auth").unwrap();
            Db::lazy(&self.url, size, SecretsCipher::for_tests(), prefix).unwrap()
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            // Async cleanup from a sync `Drop`: a fresh connection on a throwaway runtime, so it
            // works whatever state the test's runtime is in. That runtime is blocked here, so a
            // transaction it dropped (sqlx rolls those back lazily) may still hold locks: end the
            // test's own sessions first, or `DROP SCHEMA` would wait on them forever.
            let (url, auth, prefix) = (self.url.clone(), self.db.auth_schema.clone(), self.db.workspace_prefix.clone());
            let app = format!("kestrel{}", auth.strip_suffix("_auth").unwrap_or(&auth));
            let what = format!("{auth}, {prefix}*");
            let cleanup = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                rt.block_on(async move {
                    let mut conn = PgConnection::connect(&url).await?;
                    sqlx::query(
                        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
                         WHERE application_name = $1 AND pid <> pg_backend_pid()",
                    )
                    .bind(&app)
                    .execute(&mut conn)
                    .await?;
                    let schemas: Vec<String> = sqlx::query_scalar(
                        "SELECT nspname::text FROM pg_namespace WHERE nspname = $1 OR starts_with(nspname, $2)",
                    )
                    .bind(&auth)
                    .bind(&prefix)
                    .fetch_all(&mut conn)
                    .await?;
                    for schema in schemas {
                        conn.execute(AssertSqlSafe(format!("DROP SCHEMA {} CASCADE", quote_ident(&schema)))).await?;
                    }
                    conn.close().await
                })
            });
            if let Ok(Err(err)) = cleanup.join() {
                eprintln!("couldn't drop test schemas {what}: {err}");
            }
        }
    }

    pub async fn db() -> Option<TestDb> {
        let _ = dotenvy::dotenv();
        let Ok(url) = std::env::var(URL_VAR) else {
            // In CI a missing database is a broken job, not a reason to skip half the suite.
            assert!(std::env::var_os("CI").is_none(), "{URL_VAR} must be set in CI");
            // Straight to stderr: the test harness captures `eprintln!`, which would hide the skip.
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "skipped a database test: set {URL_VAR} to run it (`docker compose up -d db`, see .env.example)"
            );
            return None;
        };
        // Unique per test, so tests run in parallel against one database without seeing each other.
        let prefix = format!("t{}_", &Uuid::new_v4().simple().to_string()[..10]);
        let db = Db::lazy(&url, 4, SecretsCipher::for_tests(), &prefix).unwrap();
        db.migrate_auth().await.expect("migrating the test database; is it running?");
        Some(TestDb { db: Arc::new(db), url })
    }

    /// Bails out of a test that needs a database when none is configured.
    macro_rules! require_db {
        () => {
            match $crate::db::test::db().await {
                Some(db) => db,
                None => return,
            }
        };
    }
    pub(crate) use require_db;
}

#[cfg(test)]
mod tests {
    use super::{test::require_db, *};

    #[tokio::test]
    async fn creates_and_migrates_a_workspace_once() {
        let db = require_db!();
        let id = Uuid::new_v4();
        assert!(!db.workspace_exists(id).await.unwrap());
        let schema = db.ensure_workspace(id).await.unwrap();
        assert_eq!(db.ensure_workspace(id).await.unwrap(), schema, "idempotent");
        assert!(db.workspace_exists(id).await.unwrap());
        let (migrated, failed) = db.migrate_all().await.unwrap();
        assert_eq!((migrated, failed.len()), (1, 0));
    }

    #[tokio::test]
    async fn racing_opens_of_a_new_workspace_both_succeed() {
        let db = require_db!();
        let id = Uuid::new_v4();
        let (a, b) = tokio::join!(db.ensure_workspace(id), db.ensure_workspace(id));
        assert_eq!(a.unwrap(), b.unwrap());
    }

    #[tokio::test]
    async fn an_unpinned_query_finds_no_workspace_tables() {
        let db = require_db!();
        db.ensure_workspace(Uuid::new_v4()).await.unwrap();
        let err = sqlx::query("SELECT count(*) FROM collections").fetch_one(db.pool()).await.unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
    }

    #[tokio::test]
    async fn refuses_a_newer_workspace_schema() {
        let db = require_db!();
        let id = Uuid::new_v4();
        db.ensure_workspace(id).await.unwrap();
        sqlx::query(db.auth_sql("UPDATE auth.workspaces SET schema_version = 99 WHERE id = $1"))
            .bind(id)
            .execute(db.pool())
            .await
            .unwrap();
        let err = db.ensure_workspace(id).await.unwrap_err();
        assert!(err.to_string().contains("newer"), "{err}");
    }
}
