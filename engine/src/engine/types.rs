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
    /// The same request fired several times at once (HANDOFF → Test catalogue → v1.x →
    /// Concurrency correctness).
    Concurrency(ConcurrencyConfig),
}

impl RunConfig {
    pub fn kind(&self) -> RunKind {
        match self {
            Self::Fake(_) => RunKind::Fake,
            Self::Latency(_) => RunKind::Latency,
            Self::Load(_) => RunKind::Load,
            Self::Complexity(_) => RunKind::Complexity,
            Self::Concurrency(_) => RunKind::Concurrency,
        }
    }

    /// The endpoint under test; none for synthetic runs.
    pub fn endpoint_id(&self) -> Option<Uuid> {
        match self {
            Self::Fake(_) => None,
            Self::Latency(c) => Some(c.endpoint_id),
            Self::Load(c) => Some(c.endpoint_id),
            Self::Complexity(c) => Some(c.endpoint_id),
            Self::Concurrency(c) => Some(c.endpoint_id),
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
    /// Open model whose rate steps up (`start_rate`, then +`step_percent`% every `step_ms`) until a
    /// step breaks a limit or `max_rate` is reached (HANDOFF → v1.x → Stress / breakpoint).
    Breakpoint(BreakpointMode),
    /// Open model at `base_rate`, a burst at `spike_rate`, then `base_rate` again, to see how the
    /// target copes and how long it takes to recover (HANDOFF → v1.x → Spike).
    Spike(SpikeMode),
    /// Steps the rate up like `Breakpoint` until 429s appear, and reports the threshold and the
    /// rate-limit headers seen (HANDOFF → v1.x → Rate-limit discovery).
    RateLimit(RateLimitMode),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RateLimitMode {
    pub start_rate: u32,
    pub step_percent: u32,
    pub step_ms: u32,
    pub max_rate: u32,
}

impl RateLimitMode {
    /// A step is limited once more than this share of its requests get 429 (a stray one isn't a limit).
    pub const LIMITED_PCT: f64 = 1.0;

    /// The same steps as a breakpoint run that only counts 429s.
    pub fn as_breakpoint(&self) -> BreakpointMode {
        BreakpointMode {
            start_rate: self.start_rate,
            step_percent: self.step_percent,
            step_ms: self.step_ms,
            max_rate: self.max_rate,
            max_error_pct: Self::LIMITED_PCT,
            max_p99_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RateLimitResult {
    /// `error_pct` is the share of 429s.
    pub steps: Vec<BreakpointStep>,
    /// The highest step without 429s (to within 1%).
    pub held_rate: Option<u32>,
    /// The first step with 429s. None if none had them.
    pub limited_at_rate: Option<u32>,
    /// Rate-limit headers (`Retry-After`, `X-RateLimit-*`, `RateLimit-*`) on the first 429.
    pub limited_headers: Vec<(String, String)>,
    /// The same headers on the first other response: many APIs announce their limits on every
    /// response.
    pub ok_headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpikeMode {
    pub base_rate: u32,
    pub spike_rate: u32,
    /// Baseline before the burst. Its first second is warm-up and isn't part of "normal".
    pub before_ms: u32,
    pub spike_ms: u32,
    /// Baseline after the burst: the window recovery is measured in.
    pub after_ms: u32,
}

impl SpikeMode {
    pub fn total_ms(&self) -> u64 {
        u64::from(self.before_ms) + u64::from(self.spike_ms) + u64::from(self.after_ms)
    }
}

/// Numbers for one part of a spike run.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpikePhase {
    #[ts(type = "number")]
    pub requests: u64,
    /// Successful responses per second.
    pub achieved_rps: f64,
    /// Failures plus drops, as a percentage of everything scheduled.
    pub error_pct: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpikeResult {
    /// The baseline before the burst, without its first (warm-up) second.
    pub baseline: Option<SpikePhase>,
    pub spike: Option<SpikePhase>,
    pub after: Option<SpikePhase>,
    /// How long after the burst ended the target was back to normal (one-second windows with p99
    /// within 1.5× the baseline's and no more than one point more errors) and stayed there. None if
    /// it hadn't recovered by the end of the run.
    pub recovery_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BreakpointMode {
    pub start_rate: u32,
    /// How much each step raises the rate, in percent (at least +1 req/s).
    pub step_percent: u32,
    /// How long each step lasts.
    pub step_ms: u32,
    /// The last step runs at this rate.
    pub max_rate: u32,
    /// A step breaks when more than this share of its requests fail (drops at the in-flight cap
    /// count as failures).
    pub max_error_pct: f64,
    /// A step also breaks when its p99 is above this.
    #[serde(default)]
    pub max_p99_ms: Option<u32>,
}

impl BreakpointMode {
    /// The rate of each step, from `start_rate` up to `max_rate`.
    pub fn rates(&self) -> Vec<u32> {
        let mut rates = vec![self.start_rate.max(1)];
        while let Some(&last) = rates.last()
            && last < self.max_rate
        {
            let next = (f64::from(last) * (1.0 + f64::from(self.step_percent) / 100.0)).ceil() as u32;
            rates.push(next.max(last + 1).min(self.max_rate));
        }
        rates
    }
}

/// One step of a breakpoint run.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BreakpointStep {
    /// Scheduled rate.
    pub rate: u32,
    /// What the target actually answered, per second.
    pub achieved_rps: f64,
    #[ts(type = "number")]
    pub requests: u64,
    /// Failed requests plus drops, as a percentage of everything scheduled.
    pub error_pct: f64,
    #[ts(type = "number")]
    pub dropped: u64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub passed: bool,
    /// Why it failed, e.g. "p99 812 ms > 500 ms".
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BreakpointResult {
    pub steps: Vec<BreakpointStep>,
    /// The highest rate whose step passed.
    pub held_rate: Option<u32>,
    /// The rate of the first step that failed. None if none did.
    pub broke_at_rate: Option<u32>,
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
pub struct ConcurrencyConfig {
    pub endpoint_id: Uuid,
    #[serde(default)]
    pub environment: Option<String>,
    /// Identical requests released together in each round.
    pub requests: u32,
    /// Bursts to fire. The request is rendered once per round, so `{{uuid}}` or `{{seq}}` differ
    /// between rounds but not within one: each round is a fresh attempt at the same race.
    pub rounds: u32,
    /// Wait between rounds.
    pub pause_ms: u32,
    pub timeout_ms: u32,
}

/// Concurrency runs: what happened in each round, and what that suggests.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConcurrencyResult {
    pub rounds: Vec<ConcurrencyRound>,
    /// Plain-language reading of the rounds, worst first.
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConcurrencyRound {
    /// 1-based.
    pub round: u32,
    pub statuses: Vec<StatusCount>,
    /// Requests that got no response: timeouts, connection errors.
    pub failed: u32,
    /// 2xx responses.
    pub succeeded: u32,
    /// Different bodies among the 2xx responses. Several usually means several records were made.
    pub distinct_bodies: u32,
    /// How long every request was in flight at once (from the last send to the first response).
    /// 0 means some finished before others were sent, so they didn't all race.
    pub overlap_ms: f64,
    /// From the first response to the last.
    pub spread_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Finding {
    pub level: FindingLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FindingLevel {
    Bad,
    Warn,
    Good,
    Info,
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
    Concurrency,
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
    /// The endpoint under test; none for synthetic runs. It may since have been deleted.
    pub endpoint_id: Option<Uuid>,
    /// Headline numbers, once the run has finished.
    pub result: Option<RunResult>,
}

/// The few report numbers a run list shows.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunResult {
    #[ts(type = "number")]
    pub finished_at_ms: u64,
    #[ts(type = "number")]
    pub total_requests: u64,
    #[ts(type = "number")]
    pub total_errors: u64,
    pub mean_rps: f64,
    /// All requests, as in the report's headline latency.
    pub p50_ms: Option<f64>,
    pub p99_ms: Option<f64>,
}

impl RunSummary {
    pub fn of_report(report: &RunReport) -> Self {
        Self {
            run_id: report.run_id,
            kind: report.config.kind(),
            status: report.status,
            started_at_ms: report.started_at_ms,
            endpoint_id: report.config.endpoint_id(),
            result: Some(RunResult {
                finished_at_ms: report.finished_at_ms,
                total_requests: report.total_requests,
                total_errors: report.total_errors,
                mean_rps: report.mean_rps,
                p50_ms: report.latency.as_ref().map(|l| l.p50_ms),
                p99_ms: report.latency.as_ref().map(|l| l.p99_ms),
            }),
        }
    }
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
    /// Where the time went: DNS, opening connections, waiting for the server, downloading.
    /// Latency and load runs only.
    pub phases: Option<PhaseSummary>,
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
    /// Breakpoint load runs only.
    pub breakpoint: Option<BreakpointResult>,
    /// Spike load runs only.
    pub spike: Option<SpikeResult>,
    /// Rate-limit discovery runs only.
    pub rate_limit: Option<RateLimitResult>,
    /// Concurrency runs only.
    pub concurrency: Option<ConcurrencyResult>,
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
            phases: None,
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
            breakpoint: None,
            spike: None,
            rate_limit: None,
            concurrency: None,
        }
    }
}

/// A request's time split into phases. Measured from when the request was sent, so in an open-model
/// load run it leaves out time spent queued behind the in-flight cap.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PhaseSummary {
    /// Resolving the host, once per run (the address is then pinned). None for an IP in the URL.
    pub dns_ms: Option<f64>,
    /// Requests that opened a new connection, out of `requests`. With keep-alive this is usually
    /// just the first few; without it, every request.
    #[ts(type = "number")]
    pub new_connections: u64,
    #[ts(type = "number")]
    pub requests: u64,
    /// Opening a connection (TCP + TLS), for the requests that opened one.
    pub connect: Option<LatencySummary>,
    /// From sending (after any connect) until the response headers arrived: the server's work plus
    /// a network round trip.
    pub waiting: Option<LatencySummary>,
    /// Reading the body after the headers arrived.
    pub download: Option<LatencySummary>,
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
    /// Opening a new connection (TCP + TLS), part of `ttfb_ms`. None when a pooled one was reused.
    pub connect_ms: Option<f64>,
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
