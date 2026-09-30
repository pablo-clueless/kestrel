//! One [`WorkspaceStore`] per browser. The UI makes up a random id, keeps it in `localStorage` and
//! sends it with every call; each id gets its own database under `workspaces/<id>/`. There is no
//! login: knowing an id is what grants access to that workspace, so ids are v4 UUIDs (122 random
//! bits) and are never listed.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use uuid::Uuid;

use super::store::WorkspaceStore;

/// Stores kept open at once. Past this, the least recently used ones that nothing else holds
/// (e.g. a run in progress) are closed; they reopen from disk on their next request.
const MAX_OPEN: usize = 256;

pub struct Workspaces {
    /// `None` keeps every workspace in memory (tests).
    root: Option<PathBuf>,
    open: Mutex<Open>,
}

#[derive(Default)]
struct Open {
    tick: u64,
    stores: HashMap<Uuid, (Arc<WorkspaceStore>, u64)>,
}

impl Workspaces {
    /// Workspaces in subdirectories of `root`, created on first use.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: Some(root.into()), open: Mutex::default() }
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self { root: None, open: Mutex::default() }
    }

    /// Puts `store` in place of workspace `id`, for tests.
    #[cfg(test)]
    pub fn insert(&self, id: Uuid, store: WorkspaceStore) {
        let mut open = self.open.lock().unwrap();
        open.tick += 1;
        let tick = open.tick;
        open.stores.insert(id, (Arc::new(store), tick));
    }

    /// Workspace `id`, opened (or created) if it isn't already.
    pub fn get(&self, id: Uuid) -> anyhow::Result<Arc<WorkspaceStore>> {
        let mut open = self.open.lock().unwrap();
        open.tick += 1;
        let tick = open.tick;
        if let Some((store, used)) = open.stores.get_mut(&id) {
            *used = tick;
            return Ok(Arc::clone(store));
        }
        let store = Arc::new(match &self.root {
            // `Uuid`'s hyphenated form is safe as a directory name.
            Some(root) => WorkspaceStore::open(root.join(id.to_string()))?,
            #[cfg(test)]
            None => WorkspaceStore::in_memory(Default::default(), Default::default()),
            #[cfg(not(test))]
            None => unreachable!("in-memory workspaces are test-only"),
        });
        open.evict();
        open.stores.insert(id, (Arc::clone(&store), tick));
        Ok(store)
    }
}

impl Open {
    /// Closes idle stores, least recently used first, until there's room for one more.
    fn evict(&mut self) {
        if self.stores.len() < MAX_OPEN {
            return;
        }
        let mut idle: Vec<(Uuid, u64)> = self
            .stores
            .iter()
            .filter(|(_, (store, _))| Arc::strong_count(store) == 1)
            .map(|(id, (_, used))| (*id, *used))
            .collect();
        idle.sort_by_key(|(_, used)| *used);
        let excess = self.stores.len() + 1 - MAX_OPEN;
        for (id, _) in idle.into_iter().take(excess) {
            self.stores.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Workspace;

    #[test]
    fn keeps_workspaces_apart_and_reopens_them_from_disk() {
        let dir = std::env::temp_dir().join(format!("kestrel-workspaces-{}", Uuid::new_v4()));
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        {
            let workspaces = Workspaces::new(&dir);
            let mut ws = Workspace::default();
            ws.active_environment = Some("a-only".into());
            workspaces.get(a).unwrap().save_workspace(ws).unwrap();
            assert_eq!(workspaces.get(b).unwrap().workspace().active_environment, None);
        }
        let reopened = Workspaces::new(&dir);
        assert_eq!(reopened.get(a).unwrap().workspace().active_environment.as_deref(), Some("a-only"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn evicts_only_idle_stores() {
        let workspaces = Workspaces::in_memory();
        let held = workspaces.get(Uuid::new_v4()).unwrap();
        for _ in 0..MAX_OPEN + 10 {
            workspaces.get(Uuid::new_v4()).unwrap();
        }
        let open = workspaces.open.lock().unwrap();
        assert!(open.stores.len() <= MAX_OPEN);
        assert!(open.stores.values().any(|(s, _)| Arc::ptr_eq(s, &held)));
    }
}
