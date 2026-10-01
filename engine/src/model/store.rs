//! One workspace's data: the workspace itself, secrets, uploaded files, finished run reports and
//! confirmed hosts. It lives in the workspace's own Postgres schema (see `db`), and every query
//! here runs in a transaction pinned to that schema.
//!
//! The workspace, secrets and confirmed hosts are also cached in memory: requests compile against
//! them on every send, so reads never touch the database. Writes are serialised per workspace, go
//! to the database first, and update the cache only once committed, so a failed write leaves both
//! as they were.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, RwLock},
};

use anyhow::Context;
use bytes::Bytes;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::{Body, Endpoint, Environment, FieldKind, Secrets, Workspace};
use crate::{
    db::Db,
    engine::types::{RunReport, RunSummary},
};

/// Finished runs kept; older ones are pruned as new ones are saved.
const RUN_HISTORY: i64 = 500;

pub struct WorkspaceStore {
    id: Uuid,
    schema: String,
    db: Arc<Db>,
    /// Held across a write's database round trip, which a `std` lock can't be.
    write: tokio::sync::Mutex<()>,
    workspace: RwLock<Workspace>,
    secrets: RwLock<Secrets>,
    hosts: RwLock<BTreeSet<String>>,
}

/// Where compiled requests read uploaded files from. Compiling is synchronous, so files are
/// fetched beforehand ([`WorkspaceStore::files_for`]) and handed over as [`Files`].
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

/// An endpoint's uploaded files, fetched ahead of compiling it.
#[derive(Default)]
pub struct Files(HashMap<Uuid, Bytes>);

impl FileSource for Files {
    fn read_file(&self, id: Uuid) -> anyhow::Result<Bytes> {
        self.0.get(&id).cloned().with_context(|| format!("file {id} not found"))
    }
}

impl FromIterator<(Uuid, Bytes)> for Files {
    fn from_iter<I: IntoIterator<Item = (Uuid, Bytes)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl WorkspaceStore {
    /// Opens workspace `id`, creating and migrating its schema if needed, and loads the cache.
    pub async fn open(db: Arc<Db>, id: Uuid) -> anyhow::Result<Self> {
        let schema = db.ensure_workspace(id).await?;
        let mut tx = db.pinned(&schema).await?;
        let workspace = read_workspace(&mut tx).await?;

        let mut secrets = Secrets::new();
        let rows: Vec<(String, String, Vec<u8>)> =
            sqlx::query_as("SELECT environment, key, value FROM secrets").fetch_all(&mut *tx).await?;
        for (env, key, sealed) in rows {
            let value = db.cipher.decrypt(id, &env, &key, &sealed)?;
            secrets.entry(env).or_default().insert(key, value);
        }

        let hosts: Vec<String> = sqlx::query_scalar("SELECT host FROM confirmed_hosts").fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(Self {
            id,
            schema,
            db,
            write: tokio::sync::Mutex::new(()),
            workspace: RwLock::new(workspace),
            secrets: RwLock::new(secrets),
            hosts: RwLock::new(hosts.into_iter().collect()),
        })
    }

    /// A store that was never loaded, for tests that only need the type.
    #[cfg(test)]
    pub fn empty_for_tests(db: Arc<Db>) -> Self {
        Self {
            id: Uuid::nil(),
            schema: String::new(),
            db,
            write: tokio::sync::Mutex::new(()),
            workspace: RwLock::default(),
            secrets: RwLock::default(),
            hosts: RwLock::default(),
        }
    }

    async fn tx(&self) -> anyhow::Result<Transaction<'static, Postgres>> {
        self.db.pinned(&self.schema).await
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

    /// Hosts the user confirmed they may load test.
    pub fn confirmed_hosts(&self) -> Vec<String> {
        self.hosts.read().unwrap().iter().cloned().collect()
    }

    /// Replaces the workspace. Only collections and environments whose content changed are
    /// rewritten; removed ones are deleted.
    pub async fn save_workspace(&self, workspace: Workspace) -> anyhow::Result<()> {
        let _write = self.write.lock().await;
        let mut tx = self.tx().await?;
        write_workspace(&mut tx, &workspace).await?;
        tx.commit().await?;
        *self.workspace.write().unwrap() = workspace;
        Ok(())
    }

    pub async fn set_secret(&self, environment: &str, key: &str, value: Option<String>) -> anyhow::Result<()> {
        let _write = self.write.lock().await;
        let mut tx = self.tx().await?;
        match &value {
            Some(v) => {
                let sealed = self.db.cipher.encrypt(self.id, environment, key, v)?;
                sqlx::query(
                    "INSERT INTO secrets (environment, key, value) VALUES ($1, $2, $3)
                     ON CONFLICT (environment, key) DO UPDATE SET value = excluded.value",
                )
                .bind(environment)
                .bind(key)
                .bind(sealed)
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query("DELETE FROM secrets WHERE environment = $1 AND key = $2")
                    .bind(environment)
                    .bind(key)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;

        let mut cache = self.secrets.write().unwrap();
        match value {
            Some(v) => {
                cache.entry(environment.to_owned()).or_default().insert(key.to_owned(), v);
            }
            None => {
                if let Some(env) = cache.get_mut(environment) {
                    env.remove(key);
                }
                cache.retain(|_, kv| !kv.is_empty());
            }
        }
        Ok(())
    }

    pub async fn confirm_host(&self, host: &str) -> anyhow::Result<()> {
        let _write = self.write.lock().await;
        let mut tx = self.tx().await?;
        sqlx::query("INSERT INTO confirmed_hosts (host) VALUES ($1) ON CONFLICT (host) DO NOTHING")
            .bind(host)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.hosts.write().unwrap().insert(host.to_owned());
        Ok(())
    }

    /// Stores an uploaded file and returns its id.
    pub async fn save_file(&self, bytes: Bytes) -> anyhow::Result<Uuid> {
        let id = Uuid::new_v4();
        let mut tx = self.tx().await?;
        sqlx::query("INSERT INTO files (id, bytes) VALUES ($1, $2)")
            .bind(id)
            .bind(&bytes[..])
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// The uploaded files `endpoint`'s multipart body references. Missing ones are left out, so
    /// compiling reports them by name.
    pub async fn files_for(&self, endpoint: &Endpoint) -> anyhow::Result<Files> {
        let Body::Multipart { fields } = &endpoint.body else { return Ok(Files::default()) };
        let ids: Vec<Uuid> = fields
            .iter()
            .filter(|f| f.enabled && f.kind == FieldKind::File)
            .filter_map(|f| f.file.as_ref().map(|file| file.id))
            .collect();
        if ids.is_empty() {
            return Ok(Files::default());
        }
        let mut tx = self.tx().await?;
        let rows: Vec<(Uuid, Vec<u8>)> =
            sqlx::query_as("SELECT id, bytes FROM files WHERE id = ANY($1)").bind(&ids).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(Files(rows.into_iter().map(|(id, bytes)| (id, Bytes::from(bytes))).collect()))
    }

    /// Keeps a finished run's report, pruning the oldest beyond the history limit.
    pub async fn save_run(&self, report: &RunReport) -> anyhow::Result<()> {
        let mut tx = self.tx().await?;
        sqlx::query(
            "INSERT INTO runs (id, kind, status, started_at_ms, finished_at_ms, report, summary)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET kind = excluded.kind, status = excluded.status,
                 started_at_ms = excluded.started_at_ms, finished_at_ms = excluded.finished_at_ms,
                 report = excluded.report, summary = excluded.summary",
        )
        .bind(report.run_id)
        .bind(enum_str(&report.config.kind())?)
        .bind(enum_str(&report.status)?)
        .bind(report.started_at_ms as i64)
        .bind(report.finished_at_ms as i64)
        .bind(serde_json::to_string(report)?)
        .bind(serde_json::to_string(&RunSummary::of_report(report))?)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM runs WHERE id NOT IN (SELECT id FROM runs ORDER BY started_at_ms DESC LIMIT $1)")
            .bind(RUN_HISTORY)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Saved runs, newest first.
    pub async fn runs(&self) -> anyhow::Result<Vec<RunSummary>> {
        let mut tx = self.tx().await?;
        let docs: Vec<String> =
            sqlx::query_scalar("SELECT summary FROM runs ORDER BY started_at_ms DESC").fetch_all(&mut *tx).await?;
        tx.commit().await?;
        docs.iter().map(|doc| serde_json::from_str(doc).context("a stored run summary is invalid")).collect()
    }

    pub async fn run_report(&self, id: Uuid) -> anyhow::Result<Option<RunReport>> {
        let mut tx = self.tx().await?;
        let doc: Option<String> =
            sqlx::query_scalar("SELECT report FROM runs WHERE id = $1").bind(id).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        doc.map(|d| serde_json::from_str(&d).context("stored run report is invalid")).transpose()
    }
}

async fn read_workspace(tx: &mut Transaction<'static, Postgres>) -> anyhow::Result<Workspace> {
    let meta: Option<String> = sqlx::query_scalar("SELECT doc FROM workspace_meta").fetch_optional(&mut **tx).await?;
    let mut workspace: Workspace = match meta {
        Some(doc) => serde_json::from_str(&doc).context("stored workspace settings are invalid")?,
        None => Workspace::default(),
    };
    let docs: Vec<String> =
        sqlx::query_scalar("SELECT doc FROM collections ORDER BY position").fetch_all(&mut **tx).await?;
    for doc in docs {
        workspace.collections.push(serde_json::from_str(&doc).context("a stored collection is invalid")?);
    }
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT name, vars FROM environments ORDER BY position").fetch_all(&mut **tx).await?;
    for (name, vars) in rows {
        let vars = serde_json::from_str(&vars).context("stored environment variables are invalid")?;
        workspace.environments.push(Environment { name, vars });
    }
    Ok(workspace)
}

/// Upserts each collection and environment (skipping rows whose values are unchanged) and deletes
/// the ones no longer present.
async fn write_workspace(tx: &mut Transaction<'static, Postgres>, workspace: &Workspace) -> anyhow::Result<()> {
    let meta = Workspace { collections: vec![], environments: vec![], ..workspace.clone() };
    sqlx::query(
        "INSERT INTO workspace_meta (id, doc) VALUES (1, $1) ON CONFLICT (id) DO UPDATE SET doc = excluded.doc",
    )
    .bind(serde_json::to_string(&meta)?)
    .execute(&mut **tx)
    .await?;

    for (i, c) in workspace.collections.iter().enumerate() {
        sqlx::query(
            "INSERT INTO collections (id, position, doc) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET position = excluded.position, doc = excluded.doc
             WHERE collections.position <> excluded.position OR collections.doc <> excluded.doc",
        )
        .bind(c.id)
        .bind(i as i32)
        .bind(serde_json::to_string(c)?)
        .execute(&mut **tx)
        .await?;
    }
    let keep: Vec<Uuid> = workspace.collections.iter().map(|c| c.id).collect();
    sqlx::query("DELETE FROM collections WHERE id <> ALL($1)").bind(&keep).execute(&mut **tx).await?;

    for (i, e) in workspace.environments.iter().enumerate() {
        sqlx::query(
            "INSERT INTO environments (name, position, vars) VALUES ($1, $2, $3)
             ON CONFLICT (name) DO UPDATE SET position = excluded.position, vars = excluded.vars
             WHERE environments.position <> excluded.position OR environments.vars <> excluded.vars",
        )
        .bind(&e.name)
        .bind(i as i32)
        .bind(serde_json::to_string(&e.vars)?)
        .execute(&mut **tx)
        .await?;
    }
    let keep: Vec<String> = workspace.environments.iter().map(|e| e.name.clone()).collect();
    sqlx::query("DELETE FROM environments WHERE name <> ALL($1)").bind(&keep).execute(&mut **tx).await?;
    Ok(())
}

/// A unit enum's serde name, e.g. `RunStatus::Completed` → `"completed"`.
fn enum_str<T: serde::Serialize>(value: &T) -> anyhow::Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(s) => Ok(s),
        other => anyhow::bail!("expected a string, got {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::test::require_db,
        engine::types::{FakeConfig, RunConfig, RunStatus},
        model::{Collection, Environment},
    };

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

    #[tokio::test]
    async fn round_trips_everything() {
        let db = require_db!();
        let id = Uuid::new_v4();
        let store = WorkspaceStore::open(Arc::clone(&db), id).await.unwrap();
        let (a, b) = (collection("a"), collection("b"));
        store
            .save_workspace(Workspace {
                collections: vec![a.clone(), b.clone()],
                environments: vec![Environment { name: "local".into(), vars: [("k".into(), "v".into())].into() }],
                active_environment: Some("local".into()),
                active_collection: Some(b.id),
            })
            .await
            .unwrap();
        store.set_secret("local", "token", Some("s3cret".into())).await.unwrap();
        store.confirm_host("api.example.com").await.unwrap();
        let file = store.save_file(Bytes::from_static(b"png")).await.unwrap();
        drop(store);

        let reopened = WorkspaceStore::open(Arc::clone(&db), id).await.unwrap();
        let ws = reopened.workspace();
        assert_eq!(ws.collections.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(ws.active_collection, Some(b.id));
        assert_eq!(ws.environments[0].vars["k"], "v");
        assert_eq!(reopened.secrets()["local"]["token"], "s3cret");
        assert_eq!(reopened.confirmed_hosts(), ["api.example.com"]);

        let endpoint: Endpoint = serde_json::from_value(serde_json::json!({
            "id": Uuid::new_v4(), "method": "POST", "url": "http://x",
            "body": { "type": "multipart", "fields": [
                { "key": "f", "kind": "file", "file": { "id": file, "name": "a.png", "contentType": "image/png", "size": 3 } }
            ]}
        }))
        .unwrap();
        assert_eq!(&reopened.files_for(&endpoint).await.unwrap().read_file(file).unwrap()[..], b"png");

        // Reordering and removal are saved too.
        reopened.save_workspace(Workspace { collections: vec![b.clone()], ..ws }).await.unwrap();
        reopened.set_secret("local", "token", None).await.unwrap();
        drop(reopened);
        let again = WorkspaceStore::open(Arc::clone(&db), id).await.unwrap();
        assert_eq!(again.workspace().collections.len(), 1);
        assert!(again.secret_keys().is_empty());
    }

    #[tokio::test]
    async fn keeps_run_reports_newest_first() {
        let db = require_db!();
        let store = WorkspaceStore::open(Arc::clone(&db), Uuid::new_v4()).await.unwrap();
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
        store.save_run(&old).await.unwrap();
        store.save_run(&new).await.unwrap();

        let runs = store.runs().await.unwrap();
        assert_eq!(runs.iter().map(|r| r.run_id).collect::<Vec<_>>(), [new.run_id, old.run_id]);
        assert_eq!(runs[0].status, RunStatus::Completed);
        assert_eq!(runs[0].result.as_ref().unwrap().finished_at_ms, 2_010, "headline numbers come along");
        assert_eq!(runs[0].endpoint_id, None, "synthetic runs have no endpoint");
        assert_eq!(store.run_report(old.run_id).await.unwrap().unwrap().finished_at_ms, 1_010);
        assert!(store.run_report(Uuid::new_v4()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn secrets_are_encrypted_at_rest() {
        let db = require_db!();
        let id = Uuid::new_v4();
        let store = WorkspaceStore::open(Arc::clone(&db), id).await.unwrap();
        store.set_secret("local", "token", Some("hunter2-plaintext".into())).await.unwrap();
        let mut tx = db.pinned(&db.schema_of(id)).await.unwrap();
        let stored: Vec<u8> = sqlx::query_scalar("SELECT value FROM secrets").fetch_one(&mut *tx).await.unwrap();
        assert!(!stored.windows(7).any(|w| w == b"hunter2"), "secret stored in plaintext");
        tx.rollback().await.unwrap();
    }

    /// Two workspaces with identical ids, names and keys, written and read concurrently over a
    /// single pooled connection: a `search_path` leaking from one transaction into the next would
    /// show up here as one workspace reading the other's rows.
    #[tokio::test]
    async fn workspaces_sharing_one_connection_never_see_each_other() {
        let shared = require_db!();
        // Same database and schema names, but a pool of one connection.
        let db = Arc::new(shared.with_pool_size(1));
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let (sa, sb) = (
            Arc::new(WorkspaceStore::open(Arc::clone(&db), a).await.unwrap()),
            Arc::new(WorkspaceStore::open(Arc::clone(&db), b).await.unwrap()),
        );
        let same_collection = collection("same");
        let write = |store: Arc<WorkspaceStore>, tag: &'static str| {
            let mut c = same_collection.clone();
            c.name = tag.into();
            async move {
                for round in 0..20 {
                    let env = Environment { name: "local".into(), vars: [("who".into(), tag.into())].into() };
                    let ws = Workspace { collections: vec![c.clone()], environments: vec![env], ..Default::default() };
                    store.save_workspace(ws).await.unwrap();
                    store.set_secret("local", "token", Some(format!("{tag}-{round}"))).await.unwrap();
                    store.confirm_host(&format!("{tag}.example")).await.unwrap();
                }
            }
        };
        tokio::join!(write(Arc::clone(&sa), "a"), write(Arc::clone(&sb), "b"));
        drop((sa, sb));

        for (id, tag) in [(a, "a"), (b, "b")] {
            let store = WorkspaceStore::open(Arc::clone(&db), id).await.unwrap();
            let ws = store.workspace();
            assert_eq!(ws.collections.len(), 1);
            assert_eq!(ws.collections[0].name, tag);
            assert_eq!(ws.environments[0].vars["who"], tag);
            assert_eq!(store.secrets()["local"]["token"], format!("{tag}-19"));
            assert_eq!(store.confirmed_hosts(), [format!("{tag}.example")]);
        }
    }
}
