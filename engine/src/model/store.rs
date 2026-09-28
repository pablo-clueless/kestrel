//! Loads and saves `kestrel.json` and `kestrel.secrets.json`. Writes are atomic (temp file, fsync,
//! rename) so a crash never leaves a half-written workspace.

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::RwLock,
};

use anyhow::Context;

use super::{Secrets, Workspace};

pub const WORKSPACE_FILE: &str = "kestrel.json";
pub const SECRETS_FILE: &str = "kestrel.secrets.json";

pub struct WorkspaceStore {
    dir: PathBuf,
    workspace: RwLock<Workspace>,
    secrets: RwLock<Secrets>,
}

impl WorkspaceStore {
    /// Reads both files from `dir`, treating missing files as empty.
    pub fn open(dir: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let dir = dir.into();
        let path = dir.join(WORKSPACE_FILE);
        let workspace = match read_json::<serde_json::Value>(&path)? {
            Some(raw) => serde_json::from_value(migrate(raw))
                .with_context(|| format!("{} is not a valid workspace file", path.display()))?,
            None => Workspace::default(),
        };
        let secrets = read_json(&dir.join(SECRETS_FILE))?.unwrap_or_default();
        Ok(Self { dir, workspace: RwLock::new(workspace), secrets: RwLock::new(secrets) })
    }

    /// A store that never touches disk, for tests.
    #[cfg(test)]
    pub fn in_memory(workspace: Workspace, secrets: Secrets) -> Self {
        Self { dir: PathBuf::new(), workspace: RwLock::new(workspace), secrets: RwLock::new(secrets) }
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

    pub fn save_workspace(&self, workspace: Workspace) -> anyhow::Result<()> {
        let mut guard = self.workspace.write().unwrap();
        self.persist(WORKSPACE_FILE, &workspace)?;
        *guard = workspace;
        Ok(())
    }

    pub fn set_secret(&self, environment: &str, key: &str, value: Option<String>) -> anyhow::Result<()> {
        let mut guard = self.secrets.write().unwrap();
        let mut next = guard.clone();
        match value {
            Some(v) => {
                next.entry(environment.to_owned()).or_default().insert(key.to_owned(), v);
            }
            None => {
                if let Some(env) = next.get_mut(environment) {
                    env.remove(key);
                }
                next.retain(|_, kv| !kv.is_empty());
            }
        }
        self.persist(SECRETS_FILE, &next)?;
        self.ensure_secrets_ignored();
        *guard = next;
        Ok(())
    }

    fn persist<T: serde::Serialize>(&self, file: &str, value: &T) -> anyhow::Result<()> {
        if self.dir.as_os_str().is_empty() {
            return Ok(());
        }
        write_atomic(&self.dir.join(file), &serde_json::to_vec_pretty(value)?)
    }

    /// Adds the secrets file to a `.gitignore` next to it, if there is one and it isn't listed.
    fn ensure_secrets_ignored(&self) {
        let path = self.dir.join(".gitignore");
        let Ok(contents) = fs::read_to_string(&path) else { return };
        if contents.lines().any(|l| l.trim().trim_start_matches('/') == SECRETS_FILE) {
            return;
        }
        let sep = if contents.ends_with('\n') || contents.is_empty() { "" } else { "\n" };
        if let Err(err) = fs::write(&path, format!("{contents}{sep}{SECRETS_FILE}\n")) {
            tracing::warn!("couldn't add {SECRETS_FILE} to .gitignore: {err}");
        }
    }
}

/// Pre-collections files kept endpoints at the top level; move them into a "Default" collection.
fn migrate(mut raw: serde_json::Value) -> serde_json::Value {
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
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .with_context(|| format!("{} is not a valid workspace file", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kestrel-store-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trips_and_gitignores_secrets() {
        let dir = temp_dir();
        fs::write(dir.join(".gitignore"), "/target").unwrap();

        let store = WorkspaceStore::open(&dir).unwrap();
        store.save_workspace(Workspace { active_environment: Some("local".into()), ..Default::default() }).unwrap();
        store.set_secret("local", "token", Some("s3cret".into())).unwrap();

        let reopened = WorkspaceStore::open(&dir).unwrap();
        assert_eq!(reopened.workspace().active_environment.as_deref(), Some("local"));
        assert_eq!(reopened.secrets()["local"]["token"], "s3cret");
        assert_eq!(reopened.secret_keys()["local"], vec!["token".to_string()]);
        assert!(fs::read_to_string(dir.join(".gitignore")).unwrap().contains(SECRETS_FILE));
        assert!(!fs::read_to_string(dir.join(WORKSPACE_FILE)).unwrap().contains("s3cret"));

        reopened.set_secret("local", "token", None).unwrap();
        assert!(reopened.secret_keys().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migrates_top_level_endpoints_into_a_default_collection() {
        let dir = temp_dir();
        let id = uuid::Uuid::new_v4();
        fs::write(
            dir.join(WORKSPACE_FILE),
            format!(r#"{{"endpoints":[{{"id":"{id}","method":"GET","url":"{{{{base}}}}/x"}}],"environments":[]}}"#),
        )
        .unwrap();

        let ws = WorkspaceStore::open(&dir).unwrap().workspace();
        assert_eq!(ws.collections.len(), 1);
        assert_eq!(ws.collections[0].name, "Default");
        assert_eq!(ws.active_collection, Some(ws.collections[0].id));
        assert!(ws.endpoint(id).is_some());
        fs::remove_dir_all(dir).unwrap();
    }
}
