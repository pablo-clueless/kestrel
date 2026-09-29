pub mod client;
pub mod complexity;
pub mod fake;
pub mod latency;
pub mod load;
pub mod registry;
pub mod sample;
pub mod types;

use std::{sync::Arc, time::Duration};

use registry::{RETENTION, Run, RunRegistry};

use crate::model::store::WorkspaceStore;
use types::FakeConfig;

/// Every runner aggregates into windows of this length, so the UI gets ≤ 4 events/s.
pub const BUCKET_INTERVAL: Duration = Duration::from_millis(250);

/// A validated run, ready to start. Built by the API handler so config problems are a 400.
pub enum Prepared {
    Fake(FakeConfig),
    Latency(Box<latency::Prepared>),
    Load(Box<load::Prepared>),
    Complexity(Box<complexity::Prepared>),
}

/// Spawns the runner for `run`. Once it finishes, saves the report and schedules the run's eviction
/// from memory; the report is served from the store after that.
pub fn spawn(registry: &Arc<RunRegistry>, store: &Arc<WorkspaceStore>, run: Arc<Run>, prepared: Prepared) {
    let registry = Arc::clone(registry);
    let store = Arc::clone(store);
    tokio::spawn(async move {
        match prepared {
            Prepared::Fake(cfg) => fake::run(Arc::clone(&run), cfg).await,
            Prepared::Latency(p) => latency::run(Arc::clone(&run), *p).await,
            Prepared::Load(p) => load::run(Arc::clone(&run), *p).await,
            Prepared::Complexity(p) => complexity::run(Arc::clone(&run), *p).await,
        }
        if let Some(report) = run.report() {
            let saved = tokio::task::spawn_blocking(move || store.save_run(&report)).await;
            match saved {
                Ok(Ok(())) => {}
                Ok(Err(err)) => tracing::warn!("couldn't save run {}: {err:#}", run.id),
                Err(err) => tracing::warn!("couldn't save run {}: {err}", run.id),
            }
        }
        registry.evict_after(run.id, RETENTION);
    });
}
