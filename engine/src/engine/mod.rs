pub mod client;
pub mod fake;
pub mod latency;
pub mod load;
pub mod registry;
pub mod sample;
pub mod types;

use std::{sync::Arc, time::Duration};

use registry::{RETENTION, Run, RunRegistry};
use types::FakeConfig;

/// Every runner aggregates into windows of this length, so the UI gets ≤ 4 events/s.
pub const BUCKET_INTERVAL: Duration = Duration::from_millis(250);

/// A validated run, ready to start. Built by the API handler so config problems are a 400.
pub enum Prepared {
    Fake(FakeConfig),
    Latency(Box<latency::Prepared>),
    Load(Box<load::Prepared>),
}

/// Spawns the runner for `run` and schedules its eviction once it finishes.
pub fn spawn(registry: &Arc<RunRegistry>, run: Arc<Run>, prepared: Prepared) {
    let registry = Arc::clone(registry);
    tokio::spawn(async move {
        match prepared {
            Prepared::Fake(cfg) => fake::run(Arc::clone(&run), cfg).await,
            Prepared::Latency(p) => latency::run(Arc::clone(&run), *p).await,
            Prepared::Load(p) => load::run(Arc::clone(&run), *p).await,
        }
        registry.evict_after(run.id, RETENTION);
    });
}
