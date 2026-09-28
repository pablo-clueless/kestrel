//! Types shared with the UI. `cargo test -p engine export_bindings` writes them to
//! `kestrel/src/types/engine/` (see `.cargo/config.toml`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Body of `POST /api/runs`. Tagged by `kind`; real tests are added as variants.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum RunConfig {
    /// Emits synthetic buckets without sending any traffic. Proves the live-update loop (M0).
    Fake(FakeConfig),
    /// Sequential requests against one endpoint (HANDOFF → Test catalogue → Latency probe).
    Latency(LatencyConfig),
    /// Concurrent load, closed or open model (HANDOFF → Test catalogue → Load test).
    Load(LoadConfig),
    /// How latency grows with input size (HANDOFF → Test catalogue → Big-O).
    Complexity(ComplexityConfig),
}

impl RunConfig {
    pub fn kind(&self) -> RunKind {
        match self {
            Self::Fake(_) => RunKind::Fake,
            Self::Latency(_) => RunKind::Latency,
            Self::Load(_) => RunKind::Load,
            Self::Complexity(_) => RunKind::Complexity,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LoadConfig {
    pub endpoint_id: Uuid,
    #[serde(default)]
    pub environment: Option<String>,
    pub mode: LoadMode,
    pub duration_ms: u32,
    /// Linear ramp from ~0 to full concurrency/rate over this long. 0 = no ramp.
    #[serde(default)]
    pub ramp_up_ms: u32,
    pub timeout_ms: u32,
    pub keep_alive: bool,
    /// Open model only: sends beyond this many in flight are dropped (and counted). Defaults to the cap.
    #[serde(default)]
    pub max_in_flight: Option<u32>,
    /// Status codes that count as success even though they're 4xx/5xx (e.g. an expected 404).
    #[serde(default)]
    pub ok_statuses: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum LoadMode {
    /// N virtual users, each sending as soon as its previous request finished.
    Closed { concurrency: u32 },
    /// Requests scheduled at a fixed rate regardless of responses. Latency is measured from the
    /// scheduled send time, so a stalled server can't hide its own slowness.
    Open { rate: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LatencyConfig {
    /// An endpoint saved in the workspace.
    pub endpoint_id: Uuid,
    /// Overrides the workspace's active environment.
    #[serde(default)]
    pub environment: Option<String>,
    pub warmup: u32,
    pub samples: u32,
    pub keep_alive: bool,
    pub timeout_ms: u32,
    /// Status codes that count as success even though they're 4xx/5xx.
    #[serde(default)]
    pub ok_statuses: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FakeConfig {
    pub duration_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RunKind {
    Fake,
    Latency,
    Load,
    Complexity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RunStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartRunResponse {
    pub run_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunSummary {
    pub run_id: Uuid,
    pub kind: RunKind,
    pub status: RunStatus,
    #[ts(type = "number")]
    pub started_at_ms: u64,
}

/// One SSE `data:` payload. The SSE `id:` line carries the event id, not this struct.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum RunEvent {
    Started(StartedEvent),
    Bucket(Bucket),
    /// Big-O runs: medians per size so far. Sent at most every 250 ms.
    Complexity(ComplexityProgress),
    /// The client missed events that are no longer in history. Clear local state; the retained
    /// history follows immediately.
    Resync,
    /// Always the last event. The client then fetches `GET /api/runs/:id/report`.
    Finished(FinishedEvent),
}

impl RunEvent {
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Finished(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartedEvent {
    pub run_id: Uuid,
    pub config: RunConfig,
    #[ts(type = "number")]
    pub started_at_ms: u64,
}

/// Aggregated results for one 250 ms window.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Bucket {
    /// Window end, in ms since the run started.
    pub t_ms: u32,
    pub requests: u32,
    pub errors: u32,
    pub rps: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    /// Open model: scheduled sends skipped because the in-flight cap was reached.
    pub dropped: u32,
    /// Requests in flight at the end of the window.
    pub in_flight: u32,
    /// Worst lateness of the scheduler in this window (open model). High values mean the load
    /// generator, not the target, is the bottleneck.
    pub lag_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FinishedEvent {
    pub status: RunStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunReport {
    pub run_id: Uuid,
    pub config: RunConfig,
    pub status: RunStatus,
    #[ts(type = "number")]
    pub started_at_ms: u64,
    #[ts(type = "number")]
    pub finished_at_ms: u64,
    #[ts(type = "number")]
    pub total_requests: u64,
    #[ts(type = "number")]
    pub total_errors: u64,
    pub mean_rps: f64,
    pub max_p99_ms: f64,
    /// Target host and the IP it was pinned to.
    pub target: Option<TargetInfo>,
    /// All requests, failures and timeouts included. Headline numbers use this.
    pub latency: Option<LatencySummary>,
    /// Successful requests only.
    pub latency_success: Option<LatencySummary>,
    /// Time to first byte (response headers), all requests.
    pub ttfb: Option<LatencySummary>,
    /// Distribution of all requests.
    pub histogram: Vec<HistogramBin>,
    /// The very first request of the run (a warm-up if any), reported apart from the rest.
    pub cold_ms: Option<f64>,
    pub status_counts: Vec<StatusCount>,
    pub error_counts: ErrorCounts,
    /// A few redacted request/response pairs: the first success and the first errors.
    pub samples: Vec<Sample>,
    /// Things the reader should know, e.g. "stopped at the duration cap".
    pub notes: Vec<String>,
    /// Open model: scheduled sends that were never issued because of the in-flight cap.
    #[ts(type = "number")]
    pub dropped: u64,
    /// Open model: how late the scheduler issued sends.
    pub generator_lag: Option<LatencySummary>,
    /// Per-window results, as streamed during the run.
    pub timeline: Vec<Bucket>,
    /// Contract checks against the spec, when the endpoint was imported from one.
    pub contract: Option<crate::contract::ContractSummary>,
    /// Big-O runs only.
    pub complexity: Option<ComplexityResult>,
}

impl RunReport {
    /// A report with only the common fields set; runners fill in the rest.
    pub fn base(run_id: Uuid, config: RunConfig, status: RunStatus, started_at_ms: u64, finished_at_ms: u64) -> Self {
        Self {
            run_id,
            config,
            status,
            started_at_ms,
            finished_at_ms,
            total_requests: 0,
            total_errors: 0,
            mean_rps: 0.0,
            max_p99_ms: 0.0,
            target: None,
            latency: None,
            latency_success: None,
            ttfb: None,
            histogram: vec![],
            cold_ms: None,
            status_counts: vec![],
            error_counts: ErrorCounts::default(),
            samples: vec![],
            notes: vec![],
            dropped: 0,
            generator_lag: None,
            timeline: vec![],
            contract: None,
            complexity: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TargetInfo {
    pub host: String,
    pub pinned_ip: String,
    pub loopback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LatencySummary {
    #[ts(type = "number")]
    pub count: u64,
    pub min_ms: f64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub stddev_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistogramBin {
    pub lo_ms: f64,
    pub hi_ms: f64,
    #[ts(type = "number")]
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StatusCount {
    pub status: u16,
    #[ts(type = "number")]
    pub count: u64,
}

/// Why a request counted as an error (HANDOFF → Test catalogue → Error classification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorClass {
    /// DNS, connect, TLS, reset.
    Transport,
    Timeout,
    /// 4xx / 5xx other than 429.
    Http,
    RateLimited,
    /// The load generator ran out of something (e.g. ephemeral ports), not the target.
    ClientResource,
    /// The request couldn't be built (e.g. an invalid header value).
    InvalidRequest,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ErrorCounts {
    #[ts(type = "number")]
    pub transport: u64,
    #[ts(type = "number")]
    pub timeout: u64,
    #[ts(type = "number")]
    pub http: u64,
    #[ts(type = "number")]
    pub rate_limited: u64,
    #[ts(type = "number")]
    pub client_resource: u64,
    #[ts(type = "number")]
    pub invalid_request: u64,
}

impl ErrorCounts {
    pub fn add(&mut self, class: ErrorClass) {
        let slot = match class {
            ErrorClass::Transport => &mut self.transport,
            ErrorClass::Timeout => &mut self.timeout,
            ErrorClass::Http => &mut self.http,
            ErrorClass::RateLimited => &mut self.rate_limited,
            ErrorClass::ClientResource => &mut self.client_resource,
            ErrorClass::InvalidRequest => &mut self.invalid_request,
        };
        *slot += 1;
    }
}

/// A redacted request/response pair.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Sample {
    pub request: SentRequest,
    pub status: Option<u16>,
    pub error: Option<String>,
    pub error_class: Option<ErrorClass>,
    pub ttfb_ms: f64,
    pub total_ms: f64,
    pub response_headers: Vec<(String, String)>,
    /// UTF-8 (lossy), truncated.
    pub body: String,
    #[ts(type = "number")]
    pub body_bytes: u64,
    pub body_truncated: bool,
    /// Whether this response matches the spec, when the endpoint was imported from one.
    pub contract: Option<crate::contract::ContractCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SentRequest {
    pub method: crate::model::HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ComplexityConfig {
    /// The endpoint must use a size generator (`{{n}}`, `{{n:int_array}}`, …) somewhere.
    pub endpoint_id: Uuid,
    #[serde(default)]
    pub environment: Option<String>,
    pub min_n: u32,
    pub max_n: u32,
    /// How many sizes, spaced geometrically from `min_n` to `max_n`.
    pub points: u32,
    /// Samples per size; each is one shuffled round over all sizes.
    pub samples: u32,
    /// Requests at the middle size before measuring.
    pub warmup: u32,
    pub timeout_ms: u32,
    pub keep_alive: bool,
    #[serde(default)]
    pub ok_statuses: Vec<u16>,
    /// Once a size's median exceeds this, larger sizes are dropped.
    pub slow_ms: u32,
    /// Total time for the sweep.
    pub budget_ms: u32,
}

/// Latency at one size. Medians use successful requests only.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ComplexityPoint {
    #[ts(type = "number")]
    pub n: u64,
    pub median_ms: f64,
    pub p25_ms: f64,
    pub p75_ms: f64,
    pub samples: u32,
    pub errors: u32,
    /// Median request body size at this n, to see how much growth is just transfer.
    #[ts(type = "number")]
    pub request_bytes: u64,
    #[ts(type = "number")]
    pub response_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ComplexityProgress {
    pub round: u32,
    pub rounds: u32,
    pub points: Vec<ComplexityPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ComplexityResult {
    pub points: Vec<ComplexityPoint>,
    pub analysis: crate::stats::fit::Analysis,
}
