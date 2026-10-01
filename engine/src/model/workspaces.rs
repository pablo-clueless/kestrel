//! One [`WorkspaceStore`] per browser. The UI makes up a random id, keeps it in `localStorage` and
//! sends it with every call; each id gets its own Postgres schema (`ws_<id>`, see `db`). There is
//! no login yet (HANDOFF → Accounts, A1): knowing an id is what grants access to that workspace,
//! so ids are v4 UUIDs (122 random bits) and are never listed.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use uuid::Uuid;

use super::store::WorkspaceStore;
use crate::db::Db;

/// Stores kept open at once. Past this, the least recently used ones that nothing else holds
/// (e.g. a run in progress) are closed; they reload from the database on their next request.
const MAX_OPEN: usize = 256;

pub struct Workspaces {
    db: Arc<Db>,
    open: Mutex<Open>,
}

#[derive(Default)]
struct Open {
    tick: u64,
    stores: HashMap<Uuid, (Arc<WorkspaceStore>, u64)>,
}

impl Workspaces {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db, open: Mutex::default() }
    }

    /// Workspace `id`, opened (or created) if it isn't already.
    pub async fn get(&self, id: Uuid) -> anyhow::Result<Arc<WorkspaceStore>> {
        if let Some(store) = self.open.lock().unwrap().touch(id) {
            return Ok(store);
        }
        // Opened without holding the lock, so a slow database doesn't stall every other workspace.
        let store = Arc::new(WorkspaceStore::open(Arc::clone(&self.db), id).await?);
        let mut open = self.open.lock().unwrap();
        // Another request may have opened it meanwhile; keep theirs, so there's one cache per id.
        if let Some(existing) = open.touch(id) {
            return Ok(existing);
        }
        open.evict();
        open.tick += 1;
        let tick = open.tick;
        open.stores.insert(id, (Arc::clone(&store), tick));
        Ok(store)
    }
}

impl Open {
    fn touch(&mut self, id: Uuid) -> Option<Arc<WorkspaceStore>> {
        self.tick += 1;
        let tick = self.tick;
        let (store, used) = self.stores.get_mut(&id)?;
        *used = tick;
        Some(Arc::clone(store))
    }

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
    use crate::{db::test::require_db, model::Workspace};

    #[tokio::test]
    async fn keeps_workspaces_apart_and_reloads_them() {
        let db = require_db!();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        {
            let workspaces = Workspaces::new(Arc::clone(&db));
            let ws = Workspace { active_environment: Some("a-only".into()), ..Default::default() };
            workspaces.get(a).await.unwrap().save_workspace(ws).await.unwrap();
            assert_eq!(workspaces.get(b).await.unwrap().workspace().active_environment, None);
        }
        let reopened = Workspaces::new(Arc::clone(&db));
        assert_eq!(reopened.get(a).await.unwrap().workspace().active_environment.as_deref(), Some("a-only"));
    }

    #[tokio::test]
    async fn returns_one_store_per_id() {
        let db = require_db!();
        let workspaces = Workspaces::new(Arc::clone(&db));
        let id = Uuid::new_v4();
        let (x, y) = tokio::join!(workspaces.get(id), workspaces.get(id));
        assert!(Arc::ptr_eq(&x.unwrap(), &y.unwrap()), "concurrent opens share one cache");
    }

    #[tokio::test]
    async fn evicts_only_idle_stores() {
        // Eviction is pure bookkeeping; exercise it without a database (the pool never connects).
        let url = "postgres://unused@localhost/unused";
        let db = Arc::new(Db::lazy(url, 1, crate::db::crypto::SecretsCipher::for_tests(), "x_").unwrap());
        let mut open = Open::default();
        let fake = || Arc::new(WorkspaceStore::empty_for_tests(Arc::clone(&db)));
        let held = fake();
        open.stores.insert(Uuid::new_v4(), (Arc::clone(&held), 0));
        for i in 0..MAX_OPEN + 10 {
            open.evict();
            open.stores.insert(Uuid::new_v4(), (fake(), i as u64 + 1));
        }
        assert!(open.stores.len() <= MAX_OPEN);
        assert!(open.stores.values().any(|(s, _)| Arc::ptr_eq(s, &held)));
    }
}
