use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::types::{FinishedEvent, RunConfig, RunEvent, RunReport, RunStatus, RunSummary};

/// Events kept per run for replay. ~17 min of 250 ms buckets; well above the 60 s default cap.
const HISTORY_CAP: usize = 4096;
/// Live-event buffer per subscriber before it counts as lagged.
const BROADCAST_CAP: usize = 256;
/// How long a finished run (and its report) stays available.
pub const RETENTION: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone)]
pub struct Envelope {
    /// Monotonic per run, starting at 1. Sent as the SSE `id:`.
    pub id: u64,
    pub event: Arc<RunEvent>,
}

pub struct Run {
    pub id: Uuid,
    /// The workspace that started it. Only that workspace can see or stop it.
    pub workspace: Uuid,
    pub config: RunConfig,
    pub started_at_ms: u64,
    pub cancel: CancellationToken,
    tx: broadcast::Sender<Envelope>,
    state: Mutex<RunState>,
}

struct RunState {
    next_id: u64,
    history: VecDeque<Envelope>,
    status: RunStatus,
    report: Option<RunReport>,
}

/// What a new subscriber gets: missed events to replay, then a live receiver.
pub struct Subscription {
    /// Events the client asked for were evicted from history. Send `Resync`, and `backlog` is the
    /// whole retained history.
    pub gap: bool,
    pub backlog: Vec<Envelope>,
    pub rx: broadcast::Receiver<Envelope>,
}

impl Run {
    fn new(workspace: Uuid, config: RunConfig) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
        Self {
            id: Uuid::new_v4(),
            workspace,
            config,
            started_at_ms: now_ms(),
            cancel: CancellationToken::new(),
            tx,
            state: Mutex::new(RunState {
                next_id: 1,
                history: VecDeque::new(),
                status: RunStatus::Running,
                report: None,
            }),
        }
    }

    pub fn publish(&self, event: RunEvent) {
        // Assigning the id, appending to history and broadcasting happen under one lock, so
        // `subscribe` sees each event exactly once: either in the backlog or on the receiver.
        let mut state = self.state.lock().unwrap();
        let envelope = Envelope { id: state.next_id, event: Arc::new(event) };
        state.next_id += 1;
        if state.history.len() == HISTORY_CAP {
            state.history.pop_front();
        }
        state.history.push_back(envelope.clone());
        // No receivers is fine: history covers late subscribers.
        let _ = self.tx.send(envelope);
    }

    /// Replay events after `after` (an SSE `Last-Event-ID`), then follow live.
    pub fn subscribe(&self, after: Option<u64>) -> Subscription {
        let state = self.state.lock().unwrap();
        let rx = self.tx.subscribe();
        let oldest = state.history.front().map(|e| e.id);
        let gap = matches!((after, oldest), (Some(a), Some(o)) if o > a + 1);
        let from = if gap { 0 } else { after.unwrap_or(0) };
        let backlog = state.history.iter().filter(|e| e.id > from).cloned().collect();
        Subscription { gap, backlog, rx }
    }

    /// Stores the report, then publishes `Finished`, so a client reacting to `Finished` can
    /// always fetch the report.
    pub fn finish(&self, mut report: RunReport) {
        let status = report.status;
        {
            let mut state = self.state.lock().unwrap();
            if report.timeline.is_empty() {
                report.timeline = state
                    .history
                    .iter()
                    .filter_map(|e| match e.event.as_ref() {
                        RunEvent::Bucket(b) => Some(b.clone()),
                        _ => None,
                    })
                    .collect();
            }
            state.status = status;
            state.report = Some(report);
        }
        self.publish(RunEvent::Finished(FinishedEvent { status }));
    }

    pub fn status(&self) -> RunStatus {
        self.state.lock().unwrap().status
    }

    pub fn report(&self) -> Option<RunReport> {
        self.state.lock().unwrap().report.clone()
    }

    pub fn summary(&self) -> RunSummary {
        match self.report() {
            Some(report) => RunSummary::of_report(&report),
            None => RunSummary {
                run_id: self.id,
                kind: self.config.kind(),
                status: self.status(),
                started_at_ms: self.started_at_ms,
                endpoint_id: self.config.endpoint_id(),
                result: None,
            },
        }
    }
}

#[derive(Default)]
pub struct RunRegistry {
    runs: RwLock<HashMap<Uuid, Arc<Run>>>,
}

impl RunRegistry {
    pub fn create(&self, workspace: Uuid, config: RunConfig) -> Arc<Run> {
        let run = Arc::new(Run::new(workspace, config));
        self.runs.write().unwrap().insert(run.id, run.clone());
        run
    }

    /// Run `id`, if it belongs to `workspace`.
    pub fn get(&self, workspace: Uuid, id: Uuid) -> Option<Arc<Run>> {
        self.runs.read().unwrap().get(&id).filter(|r| r.workspace == workspace).cloned()
    }

    /// `workspace`'s runs, newest first.
    pub fn list(&self, workspace: Uuid) -> Vec<RunSummary> {
        let runs = self.runs.read().unwrap();
        let mut runs: Vec<_> = runs.values().filter(|r| r.workspace == workspace).map(|r| r.summary()).collect();
        runs.sort_by_key(|r| std::cmp::Reverse(r.started_at_ms));
        runs
    }

    pub fn evict_after(self: &Arc<Self>, id: Uuid, delay: Duration) {
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            registry.runs.write().unwrap().remove(&id);
        });
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::{Bucket, FakeConfig};

    fn run() -> Run {
        Run::new(Uuid::new_v4(), RunConfig::Fake(FakeConfig { duration_ms: 1000 }))
    }

    fn bucket(t_ms: u32) -> RunEvent {
        RunEvent::Bucket(Bucket {
            t_ms,
            requests: 1,
            errors: 0,
            rps: 4.0,
            p50_ms: 1.0,
            p99_ms: 2.0,
            dropped: 0,
            in_flight: 0,
            lag_ms: 0.0,
        })
    }

    fn ids(envelopes: &[Envelope]) -> Vec<u64> {
        envelopes.iter().map(|e| e.id).collect()
    }

    #[test]
    fn replays_everything_to_a_fresh_subscriber() {
        let run = run();
        (0..3).for_each(|t| run.publish(bucket(t)));
        let sub = run.subscribe(None);
        assert!(!sub.gap);
        assert_eq!(ids(&sub.backlog), vec![1, 2, 3]);
    }

    #[test]
    fn resumes_after_last_event_id() {
        let run = run();
        (0..5).for_each(|t| run.publish(bucket(t)));
        let sub = run.subscribe(Some(3));
        assert!(!sub.gap);
        assert_eq!(ids(&sub.backlog), vec![4, 5]);
    }

    #[test]
    fn reports_a_gap_when_history_was_evicted() {
        let run = run();
        (0..HISTORY_CAP as u32 + 10).for_each(|t| run.publish(bucket(t)));
        let sub = run.subscribe(Some(2));
        assert!(sub.gap);
        assert_eq!(sub.backlog.len(), HISTORY_CAP);
        assert_eq!(sub.backlog[0].id, 11);
    }

    #[tokio::test]
    async fn events_after_subscribe_arrive_live_not_in_backlog() {
        let run = run();
        run.publish(bucket(0));
        let mut sub = run.subscribe(None);
        run.publish(bucket(1));
        assert_eq!(ids(&sub.backlog), vec![1]);
        assert_eq!(sub.rx.recv().await.unwrap().id, 2);
    }
}
