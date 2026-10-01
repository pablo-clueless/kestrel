//! `engine import-sqlite <dir>`: moves workspaces from the SQLite layout (`<dir>/<id>/kestrel.db`,
//! one database per browser workspace) into Postgres, as `ws_<id>` with the same id, so a browser
//! keeps its workspace across the upgrade. Secrets are encrypted on the way in.
//!
//! Idempotent: a workspace already in Postgres is skipped, never overwritten. Each workspace is
//! imported in one transaction, so a failure leaves nothing half-imported.

use std::path::Path;

use anyhow::Context;
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use uuid::Uuid;

use super::Db;
use crate::engine::types::{RunReport, RunSummary};

const DB_FILE: &str = "kestrel.db";
/// The last SQLite schema version (`user_version`) the engine wrote.
const SQLITE_VERSION: i64 = 2;

#[derive(Debug, Default)]
pub struct Summary {
    pub imported: Vec<Uuid>,
    pub skipped: Vec<Uuid>,
    pub failed: Vec<(Uuid, anyhow::Error)>,
}

pub async fn import_dir(db: &Db, dir: &Path) -> anyhow::Result<Summary> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    let mut summary = Summary::default();
    for entry in entries {
        let path = entry?.path();
        let Some(id) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse::<Uuid>().ok()) else {
            continue;
        };
        let file = path.join(DB_FILE);
        if !file.is_file() {
            continue;
        }
        if db.workspace_exists(id).await? {
            summary.skipped.push(id);
            continue;
        }
        match import_one(db, id, &file).await {
            Ok(()) => summary.imported.push(id),
            Err(err) => summary.failed.push((id, err)),
        }
    }
    Ok(summary)
}

async fn import_one(db: &Db, id: Uuid, file: &Path) -> anyhow::Result<()> {
    let options = SqliteConnectOptions::new().filename(file).read_only(true);
    let mut lite =
        SqliteConnection::connect_with(&options).await.with_context(|| format!("opening {}", file.display()))?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version").fetch_one(&mut lite).await?;
    anyhow::ensure!(
        (1..=SQLITE_VERSION).contains(&version),
        "{} is SQLite schema v{version}; only v1–v{SQLITE_VERSION} can be imported",
        file.display()
    );

    let meta: Option<String> = sqlx::query_scalar("SELECT doc FROM workspace_meta").fetch_optional(&mut lite).await?;
    let collections: Vec<(String, i64, String)> =
        sqlx::query_as("SELECT id, position, doc FROM collections").fetch_all(&mut lite).await?;
    let environments: Vec<(String, i64, String)> =
        sqlx::query_as("SELECT name, position, vars FROM environments").fetch_all(&mut lite).await?;
    let secrets: Vec<(String, String, String)> =
        sqlx::query_as("SELECT environment, key, value FROM secrets").fetch_all(&mut lite).await?;
    let files: Vec<(String, Vec<u8>)> = sqlx::query_as("SELECT id, bytes FROM files").fetch_all(&mut lite).await?;
    // v1 had no `summary` column; it's rebuilt from the report.
    let runs: Vec<(String, String, String, i64, i64, String)> =
        sqlx::query_as("SELECT id, kind, status, started_at_ms, finished_at_ms, report FROM runs")
            .fetch_all(&mut lite)
            .await?;
    let hosts: Vec<String> = sqlx::query_scalar("SELECT host FROM confirmed_hosts").fetch_all(&mut lite).await?;
    lite.close().await?;

    // Creating the workspace and copying into it is one transaction: if the copy failed after the
    // workspace had been committed, a re-run would skip it as already imported.
    let mut tx = db.begin().await?;
    db.ensure_workspace_in(&mut tx, id).await?;
    if let Some(doc) = meta {
        sqlx::query("INSERT INTO workspace_meta (id, doc) VALUES (1, $1)").bind(doc).execute(&mut *tx).await?;
    }
    for (cid, position, doc) in collections {
        let cid: Uuid = cid.parse().with_context(|| format!("collection id `{cid}`"))?;
        sqlx::query("INSERT INTO collections (id, position, doc) VALUES ($1, $2, $3)")
            .bind(cid)
            .bind(position as i32)
            .bind(doc)
            .execute(&mut *tx)
            .await?;
    }
    for (name, position, vars) in environments {
        sqlx::query("INSERT INTO environments (name, position, vars) VALUES ($1, $2, $3)")
            .bind(name)
            .bind(position as i32)
            .bind(vars)
            .execute(&mut *tx)
            .await?;
    }
    for (env, key, value) in secrets {
        let sealed = db.cipher.encrypt(id, &env, &key, &value)?;
        sqlx::query("INSERT INTO secrets (environment, key, value) VALUES ($1, $2, $3)")
            .bind(env)
            .bind(key)
            .bind(sealed)
            .execute(&mut *tx)
            .await?;
    }
    for (fid, bytes) in files {
        let fid: Uuid = fid.parse().with_context(|| format!("file id `{fid}`"))?;
        sqlx::query("INSERT INTO files (id, bytes) VALUES ($1, $2)").bind(fid).bind(bytes).execute(&mut *tx).await?;
    }
    for (rid, kind, status, started, finished, report) in runs {
        let rid: Uuid = rid.parse().with_context(|| format!("run id `{rid}`"))?;
        let parsed: RunReport = serde_json::from_str(&report).with_context(|| format!("run {rid}'s report"))?;
        let summary = serde_json::to_string(&RunSummary::of_report(&parsed))?;
        sqlx::query(
            "INSERT INTO runs (id, kind, status, started_at_ms, finished_at_ms, report, summary)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(rid)
        .bind(kind)
        .bind(status)
        .bind(started)
        .bind(finished)
        .bind(report)
        .bind(summary)
        .execute(&mut *tx)
        .await?;
    }
    for host in hosts {
        sqlx::query("INSERT INTO confirmed_hosts (host) VALUES ($1)").bind(host).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::Executor;

    use super::*;
    use crate::{db::test::require_db, model::store::WorkspaceStore};

    /// Writes a v2 SQLite workspace database the way the SQLite engine did.
    async fn sqlite_workspace(dir: &Path, id: Uuid) {
        let ws_dir = dir.join(id.to_string());
        std::fs::create_dir_all(&ws_dir).unwrap();
        let options = SqliteConnectOptions::new().filename(ws_dir.join(DB_FILE)).create_if_missing(true);
        let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
        let collection = Uuid::new_v4();
        conn.execute(sqlx::AssertSqlSafe(format!(
            r#"
            CREATE TABLE workspace_meta (id INTEGER PRIMARY KEY CHECK (id = 1), doc TEXT NOT NULL);
            CREATE TABLE collections (id TEXT PRIMARY KEY, position INTEGER NOT NULL, doc TEXT NOT NULL);
            CREATE TABLE environments (name TEXT PRIMARY KEY, position INTEGER NOT NULL, vars TEXT NOT NULL);
            CREATE TABLE secrets (environment TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
                                  PRIMARY KEY (environment, key));
            CREATE TABLE files (id TEXT PRIMARY KEY, bytes BLOB NOT NULL, created_at_ms INTEGER NOT NULL);
            CREATE TABLE runs (id TEXT PRIMARY KEY, kind TEXT NOT NULL, status TEXT NOT NULL,
                               started_at_ms INTEGER NOT NULL, finished_at_ms INTEGER NOT NULL,
                               report TEXT NOT NULL, summary TEXT);
            CREATE TABLE confirmed_hosts (host TEXT PRIMARY KEY, confirmed_at_ms INTEGER NOT NULL);
            INSERT INTO workspace_meta VALUES (1, '{{"activeEnvironment":"local","activeCollection":"{collection}"}}');
            INSERT INTO collections VALUES ('{collection}', 0,
                '{{"id":"{collection}","name":"Imported","vars":{{}},"endpoints":[]}}');
            INSERT INTO environments VALUES ('local', 0, '{{"base":"http://localhost:8089"}}');
            INSERT INTO secrets VALUES ('local', 'token', 's3cret');
            INSERT INTO confirmed_hosts VALUES ('api.example.com', 0);
            PRAGMA user_version = 2;
            "#
        )))
        .await
        .unwrap();
        conn.close().await.unwrap();
    }

    #[tokio::test]
    async fn imports_each_workspace_once_with_secrets_encrypted() {
        let db = require_db!();
        let dir = std::env::temp_dir().join(format!("kestrel-import-{}", Uuid::new_v4()));
        let id = Uuid::new_v4();
        sqlite_workspace(&dir, id).await;
        std::fs::create_dir_all(dir.join("not-a-workspace")).unwrap();

        let summary = import_dir(&db, &dir).await.unwrap();
        assert_eq!(summary.imported, [id]);
        assert!(summary.failed.is_empty(), "{:?}", summary.failed);

        let store = WorkspaceStore::open(Arc::clone(&db), id).await.unwrap();
        let ws = store.workspace();
        assert_eq!(ws.collections[0].name, "Imported");
        assert_eq!(ws.active_collection, Some(ws.collections[0].id));
        assert_eq!(ws.environments[0].vars["base"], "http://localhost:8089");
        assert_eq!(store.secrets()["local"]["token"], "s3cret");
        assert_eq!(store.confirmed_hosts(), ["api.example.com"]);

        let again = import_dir(&db, &dir).await.unwrap();
        assert_eq!((again.imported.len(), again.skipped.as_slice()), (0, &[id][..]), "never overwrites");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
