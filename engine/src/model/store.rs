//! Everything the engine keeps lives in one SQLite database, `kestrel.db`: the workspace, secrets,
//! uploaded files, finished run reports and confirmed hosts. One file on one volume, so a container
//! only needs `/data` mounted.
//!
//! The workspace and secrets are also cached in memory: requests compile against them on every
//! send, and reads never touch the database. Writes go to the database first and update the cache
//! only once committed, so a failed write leaves both as they were.
//!
//! A fresh database imports the older file layout (`kestrel.json`, `kestrel.secrets.json`,
//! `kestrel-files/`) once. Those files are left in place but no longer read.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
    time::Duration,
};

use anyhow::Context;
use bytes::Bytes;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use super::{Secrets, Workspace};
use crate::engine::{
    registry::now_ms,
    types::{RunReport, RunSummary},
};

pub const DB_FILE: &str = "kestrel.db";
/// Legacy files, imported into a fresh database.
pub const WORKSPACE_FILE: &str = "kestrel.json";
pub const SECRETS_FILE: &str = "kestrel.secrets.json";
pub const FILES_DIR: &str = "kestrel-files";

/// Finished runs kept; older ones are pruned as new ones are saved.
const RUN_HISTORY: i64 = 500;

/// Bump with a new entry in [`MIGRATIONS`]; never edit a shipped one.
const MIGRATIONS: &[&str] = &[
    // 1: initial schema.
    r#"
    -- Workspace fields other than collections and environments, as one JSON object.
    CREATE TABLE workspace_meta (id INTEGER PRIMARY KEY CHECK (id = 1), doc TEXT NOT NULL);
    CREATE TABLE collections (id TEXT PRIMARY KEY, position INTEGER NOT NULL, doc TEXT NOT NULL);
    CREATE TABLE environments (name TEXT PRIMARY KEY, position INTEGER NOT NULL, vars TEXT NOT NULL);
    CREATE TABLE secrets (
        environment TEXT NOT NULL,
        key TEXT NOT NULL,
        value TEXT NOT NULL,
        PRIMARY KEY (environment, key)
    );
    CREATE TABLE files (id TEXT PRIMARY KEY, bytes BLOB NOT NULL, created_at_ms INTEGER NOT NULL);
    CREATE TABLE runs (
        id TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        status TEXT NOT NULL,
        started_at_ms INTEGER NOT NULL,
        finished_at_ms INTEGER NOT NULL,
        report TEXT NOT NULL
    );
    CREATE INDEX runs_by_start ON runs (started_at_ms DESC);
    CREATE TABLE confirmed_hosts (host TEXT PRIMARY KEY, confirmed_at_ms INTEGER NOT NULL);
    "#,
    // 2: a list-sized summary per run, so listing doesn't parse every report. Backfilled in Rust.
    "ALTER TABLE runs ADD COLUMN summary TEXT;",
];

pub struct WorkspaceStore {
    db: Mutex<Connection>,
    workspace: RwLock<Workspace>,
    secrets: RwLock<Secrets>,
    hosts: RwLock<BTreeSet<String>>,
}

/// Where compiled requests read uploaded files from.
pub trait FileSource {
    fn read_file(&self, id: Uuid) -> anyhow::Result<Bytes>;
}

/// For requests that can't reference files.
#[cfg(test)]
pub struct NoFiles;

#[cfg(test)]
impl FileSource for NoFiles {
    fn read_file(&self, id: Uuid) -> anyhow::Result<Bytes> {
        anyhow::bail!("file {id} not found")
    }
}

impl FileSource for WorkspaceStore {
    fn read_file(&self, id: Uuid) -> anyhow::Result<Bytes> {
        let db = self.db.lock().unwrap();
        let bytes: Option<Vec<u8>> =
            db.query_row("SELECT bytes FROM files WHERE id = ?1", [id.to_string()], |r| r.get(0)).optional()?;
        bytes.map(Bytes::from).with_context(|| format!("file {id} not found"))
    }
}

impl WorkspaceStore {
    /// Opens (or creates) `kestrel.db` in `dir`, migrating it to the current schema.
    pub fn open(dir: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(DB_FILE);
        let mut conn = Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
        // WAL: reads don't block the writer. NORMAL is durable across app crashes, and loses at most
        // the last commit on power loss.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        migrate(&mut conn, Some(&dir))?;
        ensure_ignored(&dir);
        Self::load(conn)
    }

    /// A store in an in-memory database, for tests.
    #[cfg(test)]
    pub fn in_memory(workspace: Workspace, secrets: Secrets) -> Self {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, None).unwrap();
        let store = Self::load(conn).unwrap();
        store.save_workspace(workspace).unwrap();
        for (env, kv) in secrets {
            for (key, value) in kv {
                store.set_secret(&env, &key, Some(value)).unwrap();
            }
        }
        store
    }

    fn load(conn: Connection) -> anyhow::Result<Self> {
        let workspace = read_workspace(&conn)?;
        let mut secrets = Secrets::new();
        {
            let mut stmt = conn.prepare("SELECT environment, key, value FROM secrets")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?)))?;
            for row in rows {
                let (env, key, value) = row?;
                secrets.entry(env).or_default().insert(key, value);
            }
        }
        let hosts =
            conn.prepare("SELECT host FROM confirmed_hosts")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(Self {
            db: Mutex::new(conn),
            workspace: RwLock::new(workspace),
            secrets: RwLock::new(secrets),
            hosts: RwLock::new(hosts),
        })
    }

    /// Stores an uploaded file and returns its id.
    pub fn save_file(&self, bytes: Bytes) -> anyhow::Result<Uuid> {
        let id = Uuid::new_v4();
        self.db.lock().unwrap().execute(
            "INSERT INTO files (id, bytes, created_at_ms) VALUES (?1, ?2, ?3)",
            params![id.to_string(), &bytes[..], now_ms() as i64],
        )?;
        Ok(id)
    }

    pub fn workspace(&self) -> Workspace {
        self.workspace.read().unwrap().clone()
    }

    pub fn secrets(&self) -> Secrets {
        self.secrets.read().unwrap().clone()
    }

    pub fn secret_keys(&self) -> BTreeMap<String, Vec<String>> {
        let secrets = self.secrets.read().unwrap();
        secrets.iter().map(|(env, kv)| (env.clone(), kv.keys().cloned().collect())).collect()
    }

    /// Replaces the workspace. Only collections and environments whose content changed are
    /// rewritten; removed ones are deleted.
    pub fn save_workspace(&self, workspace: Workspace) -> anyhow::Result<()> {
        let mut guard = self.workspace.write().unwrap();
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        write_workspace(&tx, &workspace)?;
        tx.commit()?;
        *guard = workspace;
        Ok(())
    }

    pub fn set_secret(&self, environment: &str, key: &str, value: Option<String>) -> anyhow::Result<()> {
        let mut guard = self.secrets.write().unwrap();
        let db = self.db.lock().unwrap();
        match &value {
            Some(v) => db.execute(
                "INSERT INTO secrets (environment, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT (environment, key) DO UPDATE SET value = excluded.value",
                params![environment, key, v],
            )?,
            None => db.execute("DELETE FROM secrets WHERE environment = ?1 AND key = ?2", params![environment, key])?,
        };
        match value {
            Some(v) => {
                guard.entry(environment.to_owned()).or_default().insert(key.to_owned(), v);
            }
            None => {
                if let Some(env) = guard.get_mut(environment) {
                    env.remove(key);
                }
                guard.retain(|_, kv| !kv.is_empty());
            }
        }
        Ok(())
    }

    /// Hosts the user confirmed they may load test.
    pub fn confirmed_hosts(&self) -> Vec<String> {
        self.hosts.read().unwrap().iter().cloned().collect()
    }

    pub fn confirm_host(&self, host: &str) -> anyhow::Result<()> {
        let mut guard = self.hosts.write().unwrap();
        self.db.lock().unwrap().execute(
            "INSERT INTO confirmed_hosts (host, confirmed_at_ms) VALUES (?1, ?2) ON CONFLICT (host) DO NOTHING",
            params![host, now_ms() as i64],
        )?;
        guard.insert(host.to_owned());
        Ok(())
    }

    /// Keeps a finished run's report, pruning the oldest beyond the history limit.
    pub fn save_run(&self, report: &RunReport) -> anyhow::Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO runs (id, kind, status, started_at_ms, finished_at_ms, report, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                report.run_id.to_string(),
                enum_str(&report.config.kind())?,
                enum_str(&report.status)?,
                report.started_at_ms as i64,
                report.finished_at_ms as i64,
                serde_json::to_string(report)?,
                serde_json::to_string(&RunSummary::of_report(report))?,
            ],
        )?;
        tx.execute(
            "DELETE FROM runs WHERE id NOT IN (SELECT id FROM runs ORDER BY started_at_ms DESC LIMIT ?1)",
            [RUN_HISTORY],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Saved runs, newest first.
    pub fn runs(&self) -> anyhow::Result<Vec<RunSummary>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT summary FROM runs ORDER BY started_at_ms DESC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|doc| serde_json::from_str(&doc?).context("a stored run summary is invalid")).collect()
    }

    pub fn run_report(&self, id: Uuid) -> anyhow::Result<Option<RunReport>> {
        let db = self.db.lock().unwrap();
        let doc: Option<String> =
            db.query_row("SELECT report FROM runs WHERE id = ?1", [id.to_string()], |r| r.get(0)).optional()?;
        doc.map(|d| serde_json::from_str(&d).context("stored run report is invalid")).transpose()
    }
}

/// Applies pending migrations. The first one also imports the legacy files from `legacy_dir`.
fn migrate(conn: &mut Connection, legacy_dir: Option<&Path>) -> anyhow::Result<()> {
    let version = conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize;
    if version > MIGRATIONS.len() {
        anyhow::bail!(
            "{DB_FILE} is schema v{version}, newer than this engine understands (v{}); upgrade the engine",
            MIGRATIONS.len()
        );
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql).with_context(|| format!("migrating {DB_FILE} to v{}", i + 1))?;
        if i == 0
            && let Some(dir) = legacy_dir
        {
            import_legacy(&tx, dir)?;
        }
        if i == 1 {
            backfill_run_summaries(&tx)?;
        }
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

fn backfill_run_summaries(tx: &Transaction) -> anyhow::Result<()> {
    let reports: Vec<(String, String)> = tx
        .prepare("SELECT id, report FROM runs")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut update = tx.prepare("UPDATE runs SET summary = ?2 WHERE id = ?1")?;
    for (id, report) in reports {
        let report: RunReport = serde_json::from_str(&report).context("stored run report is invalid")?;
        update.execute(params![id, serde_json::to_string(&RunSummary::of_report(&report))?])?;
    }
    Ok(())
}

/// Copies `kestrel.json`, `kestrel.secrets.json` and `kestrel-files/` into a new database.
fn import_legacy(tx: &Transaction, dir: &Path) -> anyhow::Result<()> {
    let workspace_path = dir.join(WORKSPACE_FILE);
    if let Some(raw) = read_json::<serde_json::Value>(&workspace_path)? {
        let workspace: Workspace = serde_json::from_value(migrate_legacy_workspace(raw))
            .with_context(|| format!("{} is not a valid workspace file", workspace_path.display()))?;
        write_workspace(tx, &workspace)?;
        tracing::info!("imported {} into {DB_FILE}; it is no longer read", workspace_path.display());
    }

    let secrets_path = dir.join(SECRETS_FILE);
    if let Some(secrets) = read_json::<Secrets>(&secrets_path)? {
        for (env, kv) in &secrets {
            for (key, value) in kv {
                tx.execute(
                    "INSERT INTO secrets (environment, key, value) VALUES (?1, ?2, ?3)",
                    params![env, key, value],
                )?;
            }
        }
        tracing::info!("imported {} into {DB_FILE}; it can be deleted", secrets_path.display());
    }

    let files_dir = dir.join(FILES_DIR);
    if let Ok(entries) = fs::read_dir(&files_dir) {
        let mut count = 0;
        for entry in entries {
            let path = entry?.path();
            // Files are named by id; anything else (e.g. a leftover temp file) isn't referenced.
            let Some(id) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse::<Uuid>().ok()) else {
                continue;
            };
            let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            tx.execute(
                "INSERT INTO files (id, bytes, created_at_ms) VALUES (?1, ?2, ?3)",
                params![id.to_string(), bytes, now_ms() as i64],
            )?;
            count += 1;
        }
        if count > 0 {
            tracing::info!("imported {count} file(s) from {} into {DB_FILE}", files_dir.display());
        }
    }
    Ok(())
}

fn read_workspace(conn: &Connection) -> anyhow::Result<Workspace> {
    let meta: Option<String> = conn.query_row("SELECT doc FROM workspace_meta", [], |r| r.get(0)).optional()?;
    let mut workspace: Workspace = match meta {
        Some(doc) => serde_json::from_str(&doc).context("stored workspace settings are invalid")?,
        None => Workspace::default(),
    };
    let mut stmt = conn.prepare("SELECT doc FROM collections ORDER BY position")?;
    for doc in stmt.query_map([], |r| r.get::<_, String>(0))? {
        workspace.collections.push(serde_json::from_str(&doc?).context("a stored collection is invalid")?);
    }
    let mut stmt = conn.prepare("SELECT name, vars FROM environments ORDER BY position")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (name, vars) = row?;
        let vars = serde_json::from_str(&vars).context("stored environment variables are invalid")?;
        workspace.environments.push(super::Environment { name, vars });
    }
    Ok(workspace)
}

/// Upserts each collection and environment (SQLite skips rows whose values are unchanged) and
/// deletes the ones no longer present.
fn write_workspace(tx: &Transaction, workspace: &Workspace) -> anyhow::Result<()> {
    let meta = Workspace { collections: vec![], environments: vec![], ..workspace.clone() };
    tx.execute(
        "INSERT INTO workspace_meta (id, doc) VALUES (1, ?1) ON CONFLICT (id) DO UPDATE SET doc = excluded.doc",
        [serde_json::to_string(&meta)?],
    )?;

    let mut upsert = tx.prepare_cached(
        "INSERT INTO collections (id, position, doc) VALUES (?1, ?2, ?3)
         ON CONFLICT (id) DO UPDATE SET position = excluded.position, doc = excluded.doc
         WHERE position != excluded.position OR doc != excluded.doc",
    )?;
    for (i, c) in workspace.collections.iter().enumerate() {
        upsert.execute(params![c.id.to_string(), i as i64, serde_json::to_string(c)?])?;
    }
    let keep: HashSet<String> = workspace.collections.iter().map(|c| c.id.to_string()).collect();
    delete_missing(tx, "collections", "id", &keep)?;

    let mut upsert = tx.prepare_cached(
        "INSERT INTO environments (name, position, vars) VALUES (?1, ?2, ?3)
         ON CONFLICT (name) DO UPDATE SET position = excluded.position, vars = excluded.vars
         WHERE position != excluded.position OR vars != excluded.vars",
    )?;
    for (i, e) in workspace.environments.iter().enumerate() {
        upsert.execute(params![e.name, i as i64, serde_json::to_string(&e.vars)?])?;
    }
    let keep: HashSet<String> = workspace.environments.iter().map(|e| e.name.clone()).collect();
    delete_missing(tx, "environments", "name", &keep)?;
    Ok(())
}

/// Deletes rows of `table` whose `key` column isn't in `keep`. Table and column are constants.
fn delete_missing(tx: &Transaction, table: &str, key: &str, keep: &HashSet<String>) -> anyhow::Result<()> {
    let existing: Vec<String> =
        tx.prepare(&format!("SELECT {key} FROM {table}"))?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut delete = tx.prepare(&format!("DELETE FROM {table} WHERE {key} = ?1"))?;
    for gone in existing.iter().filter(|k| !keep.contains(*k)) {
        delete.execute([gone])?;
    }
    Ok(())
}

/// A unit enum's serde name, e.g. `RunStatus::Completed` → `"completed"`.
fn enum_str<T: serde::Serialize>(value: &T) -> anyhow::Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(s) => Ok(s),
        other => anyhow::bail!("expected a string, got {other}"),
    }
}

/// Keeps the database (which holds secrets) out of git when the workspace dir is in a repo.
fn ensure_ignored(dir: &Path) {
    const PATTERN: &str = "kestrel.db*";
    let path = dir.join(".gitignore");
    let Ok(contents) = fs::read_to_string(&path) else { return };
    if contents.lines().any(|l| l.trim().trim_start_matches('/') == PATTERN) {
        return;
    }
    let sep = if contents.ends_with('\n') || contents.is_empty() { "" } else { "\n" };
    if let Err(err) = fs::write(&path, format!("{contents}{sep}{PATTERN}\n")) {
        tracing::warn!("couldn't add {PATTERN} to .gitignore: {err}");
    }
}

/// Pre-collections files kept endpoints at the top level; move them into a "Default" collection.
fn migrate_legacy_workspace(mut raw: serde_json::Value) -> serde_json::Value {
    let Some(obj) = raw.as_object_mut() else { return raw };
    if obj.contains_key("collections") {
        return raw;
    }
    let endpoints = obj.remove("endpoints").unwrap_or_else(|| serde_json::json!([]));
    if endpoints.as_array().is_some_and(|a| !a.is_empty()) {
        let id = uuid::Uuid::new_v4();
        obj.insert(
            "collections".into(),
            serde_json::json!([{ "id": id, "name": "Default", "vars": {}, "endpoints": endpoints }]),
        );
        obj.insert("activeCollection".into(), serde_json::json!(id));
    }
    raw
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map(Some).with_context(|| format!("{} is not valid JSON", path.display()))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        engine::types::{FakeConfig, RunConfig, RunStatus},
        model::{Collection, Environment},
    };

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kestrel-store-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn collection(name: &str) -> Collection {
        Collection {
            id: Uuid::new_v4(),
            name: name.into(),
            vars: Default::default(),
            endpoints: vec![],
            source: None,
            schema_defs: None,
        }
    }

    #[test]
    fn round_trips_everything_and_gitignores_the_db() {
        let dir = temp_dir();
        fs::write(dir.join(".gitignore"), "/target").unwrap();

        let store = WorkspaceStore::open(&dir).unwrap();
        let (a, b) = (collection("a"), collection("b"));
        store
            .save_workspace(Workspace {
                collections: vec![a.clone(), b.clone()],
                environments: vec![Environment { name: "local".into(), vars: [("k".into(), "v".into())].into() }],
                active_environment: Some("local".into()),
                active_collection: Some(b.id),
            })
            .unwrap();
        store.set_secret("local", "token", Some("s3cret".into())).unwrap();
        store.confirm_host("api.example.com").unwrap();
        let file = store.save_file(Bytes::from_static(b"png")).unwrap();
        drop(store);

        let reopened = WorkspaceStore::open(&dir).unwrap();
        let ws = reopened.workspace();
        assert_eq!(ws.collections.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(ws.active_collection, Some(b.id));
        assert_eq!(ws.environments[0].vars["k"], "v");
        assert_eq!(reopened.secrets()["local"]["token"], "s3cret");
        assert_eq!(reopened.confirmed_hosts(), ["api.example.com"]);
        assert_eq!(&reopened.read_file(file).unwrap()[..], b"png");
        assert!(fs::read_to_string(dir.join(".gitignore")).unwrap().contains("kestrel.db*"));

        // Reordering and removal are saved too.
        reopened.save_workspace(Workspace { collections: vec![b.clone()], ..ws }).unwrap();
        reopened.set_secret("local", "token", None).unwrap();
        drop(reopened);
        let again = WorkspaceStore::open(&dir).unwrap();
        assert_eq!(again.workspace().collections.len(), 1);
        assert!(again.secret_keys().is_empty());
        drop(again);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn imports_legacy_files_once() {
        let dir = temp_dir();
        let id = uuid::Uuid::new_v4();
        fs::write(
            dir.join(WORKSPACE_FILE),
            format!(r#"{{"endpoints":[{{"id":"{id}","method":"GET","url":"{{{{base}}}}/x"}}],"environments":[]}}"#),
        )
        .unwrap();
        fs::write(dir.join(SECRETS_FILE), r#"{"local":{"token":"s3cret"}}"#).unwrap();
        let file = Uuid::new_v4();
        fs::create_dir_all(dir.join(FILES_DIR)).unwrap();
        fs::write(dir.join(FILES_DIR).join(file.to_string()), b"data").unwrap();

        let store = WorkspaceStore::open(&dir).unwrap();
        let ws = store.workspace();
        assert_eq!(ws.collections[0].name, "Default", "top-level endpoints move into a collection");
        assert_eq!(ws.active_collection, Some(ws.collections[0].id));
        assert!(ws.endpoint(id).is_some());
        assert_eq!(store.secrets()["local"]["token"], "s3cret");
        assert_eq!(&store.read_file(file).unwrap()[..], b"data");

        // Later edits win: the legacy file isn't imported again.
        store.save_workspace(Workspace::default()).unwrap();
        drop(store);
        assert!(WorkspaceStore::open(&dir).unwrap().workspace().collections.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn keeps_run_reports_newest_first() {
        let store = WorkspaceStore::in_memory(Workspace::default(), Secrets::default());
        let report = |started| {
            RunReport::base(
                Uuid::new_v4(),
                RunConfig::Fake(FakeConfig { duration_ms: 10 }),
                RunStatus::Completed,
                started,
                started + 10,
            )
        };
        let (old, new) = (report(1_000), report(2_000));
        store.save_run(&old).unwrap();
        store.save_run(&new).unwrap();

        let runs = store.runs().unwrap();
        assert_eq!(runs.iter().map(|r| r.run_id).collect::<Vec<_>>(), [new.run_id, old.run_id]);
        assert_eq!(runs[0].status, RunStatus::Completed);
        assert_eq!(runs[0].result.as_ref().unwrap().finished_at_ms, 2_010, "headline numbers come along");
        assert_eq!(runs[0].endpoint_id, None, "synthetic runs have no endpoint");
        assert_eq!(store.run_report(old.run_id).unwrap().unwrap().finished_at_ms, 1_010);
        assert!(store.run_report(Uuid::new_v4()).unwrap().is_none());
    }

    #[test]
    fn refuses_a_newer_schema() {
        let dir = temp_dir();
        drop(WorkspaceStore::open(&dir).unwrap());
        Connection::open(dir.join(DB_FILE)).unwrap().pragma_update(None, "user_version", 99).unwrap();
        let err = WorkspaceStore::open(&dir).err().unwrap();
        assert!(err.to_string().contains("newer"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }
}
