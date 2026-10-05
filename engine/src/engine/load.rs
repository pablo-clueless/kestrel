//! Load test: closed model (fixed concurrency) or open model (fixed arrival rate).
//! HANDOFF → Test catalogue → Load test.
//!
//! Request tasks send their results to one aggregator task over a channel; the aggregator owns the
//! histograms and emits a bucket every 250 ms. In the open model, latency is measured from the
//! **scheduled** send time, so scheduler lateness and a stalled server both show up in the numbers
//! (no coordinated omission), and sends beyond the in-flight cap are counted as dropped.
//!
//! The breakpoint mode is the open model with a rate that steps up; `breakpoint.rs` judges each
//! step, and stops the schedule once one breaks a limit.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    },
    time::Duration,
};

use reqwest::Client;
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use super::phases::{PhaseTimes, Phases};
use super::soak::{self, SoakAnalyzer};
use super::{
    BUCKET_INTERVAL,
    breakpoint::StepJudge,
    client::{self, ClientOptions, Target},
    registry::{Run, now_ms},
    sample,
    spike::SpikeAnalyzer,
    types::{
        BreakpointResult, Bucket, ErrorClass, ErrorCounts, LoadConfig, LoadMode, MixStats, RateLimitResult, RunEvent,
        RunReport, RunStatus, Sample, SoakResult, SpikeResult, StartedEvent, StatusCount, TargetInfo,
    },
};
use crate::{
    contract::{Contract, ContractCheck, ContractSummary},
    platform,
    redact::Redactor,
    stats::Recorder,
    template::request::CompiledRequest,
};

/// Under load, validate at most this many responses per second against the spec, so schema
/// validation never becomes the bottleneck.
const CONTRACT_CHECKS_PER_SEC: u32 = 50;

const MAX_ERROR_SAMPLES: usize = 5;
const SAMPLE_BODY_BYTES: usize = 8 * 1024;
/// Scheduler lateness above this (p99) means the generator, not the target, limited the test.
const LAG_WARNING: Duration = Duration::from_millis(5);

pub struct Prepared {
    pub cfg: LoadConfig,
    pub request: CompiledRequest,
    pub target: Target,
    pub target_info: TargetInfo,
    pub max_in_flight: u32,
    /// The rate cap. The open model is validated against it up front; closed-model users are paced
    /// to it, since their rate otherwise depends only on how fast the target answers.
    pub max_rps: u32,
    /// Set by the caller for endpoints imported from a spec.
    pub contract: Option<Arc<Contract>>,
    /// Multi-endpoint mix, set with [`Prepared::with_mix`]. Empty: every request is `request`.
    pub mix: Vec<MixTarget>,
    /// Keeps the run's token fresh; set by the caller (see `refresh.rs`).
    pub refresh: Option<super::refresh::Refresher>,
}

/// One endpoint of a multi-endpoint mix, compiled and ready to send.
pub struct MixTarget {
    pub endpoint_id: uuid::Uuid,
    pub name: String,
    pub weight: u32,
    pub request: CompiledRequest,
    pub contract: Option<Arc<Contract>>,
}

impl Prepared {
    /// Spreads the run over `mix`. Every endpoint must be on the target's origin: the run's client is
    /// pinned to that host's address.
    pub fn with_mix(mut self, mix: Vec<MixTarget>) -> Result<Self, String> {
        let origin = |r: &CompiledRequest| r.preview().map(|req| req.url.origin());
        let primary = origin(&self.request)?;
        for m in &mix {
            if origin(&m.request)? != primary {
                return Err(format!(
                    "`{}` is on a different host; every endpoint in a mix must be on {}",
                    m.name,
                    primary.ascii_serialization()
                ));
            }
        }
        self.mix = mix;
        Ok(self)
    }
}

pub enum PrepareError {
    Invalid(String),
    /// The target isn't loopback and the user hasn't confirmed they may load-test it.
    HostNotConfirmed(String),
}

pub async fn prepare(
    cfg: LoadConfig,
    request: CompiledRequest,
    confirmed_hosts: &[String],
    max_in_flight: u32,
    max_rps: u32,
) -> Result<Prepared, PrepareError> {
    let first = request.preview().map_err(PrepareError::Invalid)?;
    let host = first.url.host_str().unwrap_or_default().to_ascii_lowercase();
    // Redirects are never followed under load; 3xx responses are recorded as-is.
    let opts = ClientOptions { keep_alive: cfg.keep_alive, follow_redirects: false, confirmed_hosts: vec![] };
    let target = client::connect(&first.url, &opts).await.map_err(PrepareError::Invalid)?;
    // "Localhost" means the resolved IP, not the name.
    if !target.pinned.is_loopback() && !confirmed_hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
        return Err(PrepareError::HostNotConfirmed(host));
    }
    let target_info = TargetInfo { host, pinned_ip: target.pinned.to_string(), loopback: target.pinned.is_loopback() };
    Ok(Prepared {
        cfg,
        request,
        target,
        target_info,
        max_in_flight,
        max_rps,
        contract: None,
        mix: Vec::new(),
        refresh: None,
    })
}

/// What request tasks report to the aggregator.
enum Msg {
    Done(Record),
    /// Open model: a scheduled send was skipped because the in-flight cap was reached.
    Dropped,
    /// Open model: how late the scheduler was for one send.
    Lag(Duration),
}

struct Record {
    /// Which endpoint of the mix (0 without one).
    endpoint: usize,
    /// When the request was scheduled, from the start of the run.
    offset: Duration,
    latency: Duration,
    ttfb: Duration,
    /// None when the request was never sent (it couldn't be rendered).
    phases: Option<PhaseTimes>,
    /// Rate-limit discovery only: the response's rate-limit headers, if it had any.
    rate_headers: Option<Vec<(String, String)>>,
    status: Option<u16>,
    error: Option<ErrorClass>,
    sample: Option<Box<Sample>>,
    contract: Option<ContractCheck>,
}

/// Shared by every request task. The channel closes when the last clone is dropped.
struct Shared {
    /// When the run started; records carry their offset from it.
    started: Instant,
    /// Keep responses' rate-limit headers (rate-limit discovery).
    rate_headers: bool,
    client: Client,
    /// What to send: one target, or a multi-endpoint mix picked by weight. Swapped mid-run when a
    /// token refresh compiles the requests again.
    targets: Arc<Targets>,
    timeout: Duration,
    ok_statuses: Vec<u16>,
    tx: mpsc::UnboundedSender<Msg>,
    in_flight: Arc<AtomicU32>,
    success_sampled: AtomicBool,
    error_samples: AtomicUsize,
    /// (window start, checks in window) for the contract sampling rate.
    contract_gate: Mutex<(Instant, u32)>,
}

/// The run's endpoints, behind a lock so a token refresh can swap in requests compiled with the new
/// token. Kept apart from `Shared`: the aggregator ends when the last `Shared` is dropped, and the
/// refresher mustn't hold that up.
struct Targets {
    picks: std::sync::RwLock<Arc<Vec<Pick>>>,
    total_weight: u32,
    /// Woken by a 401, so the refresher can fetch a token early.
    unauthorized: tokio::sync::Notify,
}

impl Targets {
    fn current(&self) -> Arc<Vec<Pick>> {
        Arc::clone(&self.picks.read().unwrap())
    }

    /// New requests for the same endpoints, in the same order; contracts and weights are kept.
    fn replace(&self, requests: Vec<CompiledRequest>) {
        let old = self.current();
        let new: Vec<Pick> = old
            .iter()
            .zip(requests)
            .map(|(p, request)| Pick { request, contract: p.contract.clone(), weight: p.weight })
            .collect();
        *self.picks.write().unwrap() = Arc::new(new);
    }
}

/// Refreshes the token every `every`, and early (at most every [`MIN_GAP`]) when the run gets 401s,
/// until `stop`. Returns what it did, for the notes.
async fn refresh_loop(
    mut refresher: super::refresh::Refresher,
    targets: Arc<Targets>,
    stop: CancellationToken,
    mut tally: super::refresh::Tally,
) -> (String, Duration, super::refresh::Tally) {
    use super::refresh::MIN_GAP;
    let every = refresher.every;
    let mut last = Instant::now();
    let mut tick = tokio::time::interval_at(last + every, every);
    loop {
        let early = tokio::select! {
            _ = stop.cancelled() => break,
            _ = tick.tick() => false,
            _ = targets.unauthorized.notified() => true,
        };
        if early && last.elapsed() < MIN_GAP {
            continue;
        }
        match refresher.refresh().await {
            Ok(requests) => {
                targets.replace(requests);
                tally.refreshed += 1;
                tally.after_401s += u32::from(early);
            }
            Err(e) => tally.failures.push(e),
        }
        last = Instant::now();
        if early {
            tick.reset();
        }
    }
    (refresher.name().to_owned(), every, tally)
}

/// One endpoint the run sends to.
struct Pick {
    request: CompiledRequest,
    contract: Option<Arc<Contract>>,
    weight: u32,
}

impl Shared {
    /// The endpoint for the next request, by weight.
    fn pick(&self) -> (usize, Arc<Vec<Pick>>) {
        let picks = self.targets.current();
        let mut index = 0;
        if picks.len() > 1 {
            let mut r = rand::random_range(0..self.targets.total_weight.max(1));
            for (i, t) in picks.iter().enumerate() {
                if r < t.weight {
                    index = i;
                    break;
                }
                r -= t.weight;
            }
        }
        (index, picks)
    }

    /// True if this response may be contract-checked (at most `CONTRACT_CHECKS_PER_SEC`).
    fn contract_slot(&self) -> bool {
        let mut gate = self.contract_gate.lock().unwrap();
        if gate.0.elapsed() >= Duration::from_secs(1) {
            *gate = (Instant::now(), 0);
        }
        gate.1 += 1;
        gate.1 <= CONTRACT_CHECKS_PER_SEC
    }

    /// Sends one request. `scheduled` is when it should have gone out; latency is measured from there.
    async fn fire(&self, scheduled: Instant) {
        let _guard = InFlight::enter(&self.in_flight);
        let (endpoint, picks) = self.pick();
        let target = &picks[endpoint];
        let req = match target.request.render() {
            Ok(req) => req,
            Err(msg) => {
                tracing::warn!("render failed mid-run: {msg}");
                let _ = self.tx.send(Msg::Done(Record {
                    endpoint,
                    offset: Instant::now().saturating_duration_since(self.started),
                    latency: Duration::ZERO,
                    ttfb: Duration::ZERO,
                    phases: None,
                    rate_headers: None,
                    status: None,
                    error: Some(ErrorClass::InvalidRequest),
                    sample: None,
                    contract: None,
                }));
                return;
            }
        };
        let outcome = client::execute(&self.client, &req, self.timeout).await.classify_with(&self.ok_statuses);
        if outcome.status == Some(401) {
            self.targets.unauthorized.notify_one();
        }
        let latency = scheduled.elapsed();

        let check = match (&target.contract, outcome.status) {
            (Some(contract), Some(status)) if self.contract_slot() => Some(contract.check(status, &outcome.body)),
            _ => None,
        };

        let clean = outcome.is_success() && check.as_ref().is_none_or(|c| c.passed);
        let want_sample = if clean {
            !self.success_sampled.swap(true, Ordering::Relaxed)
        } else {
            self.error_samples.fetch_add(1, Ordering::Relaxed) < MAX_ERROR_SAMPLES
        };
        let redactor =
            Redactor::new(target.request.api_key_header.as_deref(), &target.request.secret_values, Some(&req));
        let check = check.map(|c| ContractCheck { message: redactor.text(&c.message), ..c });
        let sample = want_sample.then(|| {
            let mut s = sample::build(&req, &outcome, &redactor, SAMPLE_BODY_BYTES);
            s.contract = check.clone();
            Box::new(s)
        });

        let _ = self.tx.send(Msg::Done(Record {
            endpoint,
            offset: scheduled.saturating_duration_since(self.started),
            latency,
            ttfb: outcome.ttfb,
            phases: Some((&outcome).into()),
            rate_headers: self.rate_headers.then(|| rate_limit_headers(&outcome.response_headers)).flatten(),
            status: outcome.status,
            error: outcome.error.map(|(class, _)| class),
            sample,
            contract: check,
        }));
    }
}

/// Holds closed-model users to the rate cap. Each send takes the next free slot, `interval` after the
/// previous one; slots never bunch up behind the present, so there are no catch-up bursts.
struct Pacer {
    start: Instant,
    interval_ns: u64,
    next_ns: Mutex<u64>,
    throttled: AtomicBool,
}

impl Pacer {
    fn new(max_rps: u32) -> Self {
        Self {
            start: Instant::now(),
            interval_ns: 1_000_000_000 / u64::from(max_rps.max(1)),
            next_ns: Mutex::new(0),
            throttled: AtomicBool::new(false),
        }
    }

    async fn wait(&self) {
        let now = self.start.elapsed().as_nanos() as u64;
        let slot = {
            let mut next = self.next_ns.lock().unwrap();
            let slot = (*next).max(now);
            *next = slot + self.interval_ns;
            slot
        };
        if slot > now {
            self.throttled.store(true, Ordering::Relaxed);
            tokio::time::sleep_until(self.start + Duration::from_nanos(slot)).await;
        }
    }
}

/// Counts a request as in flight until dropped (including when its task is aborted).
struct InFlight<'a>(&'a AtomicU32);

impl<'a> InFlight<'a> {
    fn enter(counter: &'a AtomicU32) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub async fn run(run: Arc<Run>, mut p: Prepared) {
    platform::high_res_timer();
    run.publish(RunEvent::Started(StartedEvent {
        run_id: run.id,
        config: run.config.clone(),
        started_at_ms: run.started_at_ms,
    }));

    // A fresh token before the first request, so the run starts on one that will last.
    let mut refresher = p.refresh.take();
    let mut tally = super::refresh::Tally::default();
    let mut fresh = None;
    if let Some(r) = refresher.as_mut() {
        match r.refresh().await {
            Ok(requests) => {
                fresh = Some(requests);
                tally.refreshed += 1;
            }
            Err(e) => tally.failures.push(e),
        }
    }

    let timeout = Duration::from_millis(p.cfg.timeout_ms.into());
    let mut duration = Duration::from_millis(p.cfg.duration_ms.into());
    // A breakpoint run lasts as long as its steps, within the requested duration.
    let planned = match &p.cfg.mode {
        LoadMode::RateLimit(mode) => {
            let planned = Duration::from_millis(u64::from(mode.step_ms) * mode.as_breakpoint().rates().len() as u64);
            duration = duration.min(planned);
            Some(planned)
        }
        LoadMode::Breakpoint(mode) => {
            let planned = Duration::from_millis(u64::from(mode.step_ms) * mode.rates().len() as u64);
            duration = duration.min(planned);
            Some(planned)
        }
        LoadMode::Spike(mode) => {
            duration = duration.min(Duration::from_millis(mode.total_ms()));
            None
        }
        _ => None,
    };
    let ramp = Duration::from_millis(p.cfg.ramp_up_ms.into()).min(duration);
    let (tx, rx) = mpsc::unbounded_channel();
    let in_flight = Arc::new(AtomicU32::new(0));
    let started = Instant::now();
    // Ends the schedule early: a breakpoint step broke (or the user cancelled).
    let stop = run.cancel.child_token();
    let watch = match &p.cfg.mode {
        LoadMode::Breakpoint(mode) => Some(Watch::Breakpoint(StepJudge::new(mode.clone(), timeout, stop.clone()))),
        LoadMode::Spike(mode) => Some(Watch::Spike(SpikeAnalyzer::new(mode.clone(), timeout))),
        LoadMode::Soak { .. } => Some(Watch::Soak(SoakAnalyzer::new(duration, timeout))),
        LoadMode::RateLimit(mode) => Some(Watch::RateLimit {
            judge: StepJudge::new(mode.as_breakpoint(), timeout, stop.clone()).counting("429s"),
            limited_headers: None,
            ok_headers: None,
        }),
        _ => None,
    };

    // A mix's endpoints, for its per-endpoint report; empty for a single endpoint.
    let mix_meta: Vec<(uuid::Uuid, String, u32)> =
        p.mix.iter().map(|m| (m.endpoint_id, m.name.clone(), m.weight)).collect();
    let aggregator =
        tokio::spawn(aggregate(Arc::clone(&run), rx, Arc::clone(&in_flight), started, timeout, watch, mix_meta.len()));
    let dns = p.target.dns;

    let mut picks: Vec<Pick> = if p.mix.is_empty() {
        vec![Pick { request: p.request, contract: p.contract.clone(), weight: 1 }]
    } else {
        p.mix.into_iter().map(|m| Pick { request: m.request, contract: m.contract, weight: m.weight.max(1) }).collect()
    };
    if let Some(requests) = fresh {
        for (pick, request) in picks.iter_mut().zip(requests) {
            pick.request = request;
        }
    }
    let targets = Arc::new(Targets {
        total_weight: picks.iter().map(|t| t.weight).sum(),
        picks: std::sync::RwLock::new(Arc::new(picks)),
        unauthorized: tokio::sync::Notify::new(),
    });
    let refresh_stop = run.cancel.child_token();
    let refreshing =
        refresher.map(|r| tokio::spawn(refresh_loop(r, Arc::clone(&targets), refresh_stop.clone(), tally)));
    let shared = Arc::new(Shared {
        started,
        rate_headers: matches!(p.cfg.mode, LoadMode::RateLimit(_)),
        client: p.target.client,
        targets,
        timeout,
        ok_statuses: p.cfg.ok_statuses.clone(),
        tx,
        in_flight,
        success_sampled: AtomicBool::new(false),
        error_samples: AtomicUsize::new(0),
        contract_gate: Mutex::new((Instant::now(), 0)),
    });
    let pacer = Arc::new(Pacer::new(p.max_rps));
    let status = match p.cfg.mode {
        LoadMode::Closed { concurrency } => {
            closed(shared, Arc::clone(&pacer), concurrency, duration, ramp, &run.cancel).await
        }
        LoadMode::Open { rate } | LoadMode::Soak { rate } => {
            let cap = p.cfg.max_in_flight.unwrap_or(p.max_in_flight).clamp(1, p.max_in_flight);
            open(shared, Rates::Ramp { rate, ramp }, duration, cap as usize, &stop, &run.cancel).await
        }
        LoadMode::Breakpoint(ref mode) => {
            let cap = p.cfg.max_in_flight.unwrap_or(p.max_in_flight).clamp(1, p.max_in_flight);
            let step = Duration::from_millis(mode.step_ms.into());
            let rates = Rates::Segments(mode.rates().into_iter().map(|r| (step, r)).collect());
            open(shared, rates, duration, cap as usize, &stop, &run.cancel).await
        }
        LoadMode::RateLimit(ref mode) => {
            let cap = p.cfg.max_in_flight.unwrap_or(p.max_in_flight).clamp(1, p.max_in_flight);
            let step = Duration::from_millis(mode.step_ms.into());
            let rates = Rates::Segments(mode.as_breakpoint().rates().into_iter().map(|r| (step, r)).collect());
            open(shared, rates, duration, cap as usize, &stop, &run.cancel).await
        }
        LoadMode::Spike(ref mode) => {
            let cap = p.cfg.max_in_flight.unwrap_or(p.max_in_flight).clamp(1, p.max_in_flight);
            let ms = |v: u32| Duration::from_millis(v.into());
            let rates = Rates::Segments(vec![
                (ms(mode.before_ms), mode.base_rate),
                (ms(mode.spike_ms), mode.spike_rate),
                (ms(mode.after_ms), mode.base_rate),
            ]);
            open(shared, rates, duration, cap as usize, &stop, &run.cancel).await
        }
    };
    let active = started.elapsed().min(duration.max(Duration::from_millis(1)));
    refresh_stop.cancel();
    let refresh_notes = match refreshing {
        Some(handle) => match handle.await {
            Ok((name, every, tally)) => tally.note(&name, every),
            Err(_) => vec!["The token refresher stopped unexpectedly.".into()],
        },
        None => Vec::new(),
    };
    // Every `Shared` clone is gone now, so the channel is closed and the aggregator finishes.
    let totals = aggregator.await.expect("aggregator task panicked");

    let mut notes = refresh_notes;
    if let (Some(result), Some(planned)) = (&totals.breakpoint, planned) {
        notes.extend(breakpoint_note(result, planned, duration));
    }
    if let (Some(result), Some(planned)) = (&totals.rate_limit, planned) {
        notes.push(rate_limit_note(result, planned, duration));
    }
    if let (Some(result), LoadMode::Spike(mode)) = (&totals.spike, &p.cfg.mode) {
        notes.push(match result.recovery_ms {
            Some(0) => "The burst didn't knock the target off its baseline.".into(),
            Some(ms) => format!("Back to normal {:.0} s after the burst.", f64::from(ms) / 1000.0),
            None => format!(
                "Not back to normal {:.0} s after the burst (the end of the run). Lengthen \"after\" to see \
                 when it recovers.",
                f64::from(mode.after_ms) / 1000.0
            ),
        });
    }
    if totals.dropped > 0 {
        notes.push(format!(
            "{} scheduled sends were dropped at the in-flight cap. The target couldn't keep up with the rate; \
             these are not in the latency numbers.",
            totals.dropped
        ));
    }
    if let Some(lag) = totals.lag.summary()
        && lag.p99_ms > LAG_WARNING.as_secs_f64() * 1000.0
    {
        notes.push(format!(
            "Generator lag p99 was {:.1} ms: the load generator couldn't keep to schedule, so latency includes \
             that lateness. Lower the rate or run the target on another machine.",
            lag.p99_ms
        ));
    }
    if pacer.throttled.load(Ordering::Relaxed) {
        notes.push(format!(
            "Held to the {} req/s cap: the target answered faster than that. Raise the cap in local config \
             to go higher.",
            p.max_rps
        ));
    }
    if matches!(p.cfg.mode, LoadMode::Closed { .. }) {
        notes.push(
            "Closed model: throughput drops when the server slows, so stalls look milder than real arrival \
             traffic would see. Use the open model to measure a target rate."
                .into(),
        );
    }
    if p.target_info.loopback {
        notes.push("The target is on this machine, so it competes with the engine for CPU.".into());
    }

    let completed = totals.all.len();
    run.finish(RunReport {
        total_requests: completed,
        total_errors: totals.total_errors,
        mean_rps: completed as f64 / active.as_secs_f64(),
        max_p99_ms: totals.max_p99_ms,
        target: Some(p.target_info),
        latency: totals.all.summary(),
        latency_success: totals.success.summary(),
        ttfb: totals.ttfb.summary(),
        phases: totals.phases.summary(dns),
        histogram: totals.all.bins(),
        status_counts: totals.statuses.into_iter().map(|(status, count)| StatusCount { status, count }).collect(),
        error_counts: totals.errors,
        samples: totals.samples,
        notes,
        dropped: totals.dropped,
        generator_lag: totals.lag.summary(),
        contract: totals.contract,
        breakpoint: totals.breakpoint,
        spike: totals.spike,
        rate_limit: totals.rate_limit,
        // A soak outlasts the live history (about 17 minutes of buckets), so its windows are the
        // timeline. Empty for other modes, which `finish` fills from the history.
        timeline: totals.soak.as_ref().map(soak::timeline).unwrap_or_default(),
        soak: totals.soak,
        mix: (!mix_meta.is_empty()).then(|| {
            mix_meta
                .into_iter()
                .zip(totals.per_endpoint)
                .map(|((endpoint_id, name, weight), e)| MixStats {
                    endpoint_id,
                    name,
                    weight,
                    requests: e.latency.len(),
                    errors: e.errors,
                    latency: e.latency.summary(),
                    status_counts: e
                        .statuses
                        .into_iter()
                        .map(|(status, count)| StatusCount { status, count })
                        .collect(),
                })
                .collect()
        }),
        ..RunReport::base(run.id, run.config.clone(), status, run.started_at_ms, now_ms())
    });
}

/// `concurrency` virtual users, started evenly over `ramp`, each sending back to back until the end
/// (never faster, all together, than the rate cap).
async fn closed(
    shared: Arc<Shared>,
    pacer: Arc<Pacer>,
    concurrency: u32,
    duration: Duration,
    ramp: Duration,
    cancel: &CancellationToken,
) -> RunStatus {
    let start = Instant::now();
    let deadline = start + duration;
    let mut users = JoinSet::new();
    for i in 0..concurrency {
        let (shared, pacer) = (Arc::clone(&shared), Arc::clone(&pacer));
        let begin = start + ramp.mul_f64(f64::from(i) / f64::from(concurrency));
        users.spawn(async move {
            tokio::time::sleep_until(begin).await;
            loop {
                pacer.wait().await;
                if Instant::now() >= deadline {
                    break;
                }
                // Latency is measured from the actual send, after pacing.
                shared.fire(Instant::now()).await;
            }
        });
    }
    drop(shared);
    join_or_cancel(users, cancel).await
}

/// The open model's send rate over time.
enum Rates {
    /// `rate`/s, reached linearly over `ramp`.
    Ramp { rate: u32, ramp: Duration },
    /// Each rate for its duration, in order; the last one is held (breakpoint steps, spike phases).
    Segments(Vec<(Duration, u32)>),
}

impl Rates {
    fn at(&self, elapsed: Duration) -> f64 {
        match self {
            Self::Ramp { rate, ramp } => {
                let fraction = if ramp.is_zero() { 1.0 } else { (elapsed.as_secs_f64() / ramp.as_secs_f64()).min(1.0) };
                (f64::from(*rate) * fraction).max(1.0)
            }
            Self::Segments(segments) => {
                let mut end = Duration::ZERO;
                for &(length, rate) in segments {
                    end += length;
                    if elapsed < end {
                        return f64::from(rate.max(1));
                    }
                }
                f64::from(segments.last().map_or(1, |&(_, rate)| rate.max(1)))
            }
        }
    }
}

/// Sends at `rates`, never more than `max_in_flight` at once, until `duration` or `stop`.
///
/// The schedule runs on a blocking thread rather than a tokio timer: tokio's timer wheel has 1 ms
/// resolution, which made every send ~1 ms late, and in the open model that lateness is latency.
async fn open(
    shared: Arc<Shared>,
    rates: Rates,
    duration: Duration,
    max_in_flight: usize,
    stop: &CancellationToken,
    cancel: &CancellationToken,
) -> RunStatus {
    let token = stop.clone();
    let scheduler = tokio::task::spawn_blocking(move || schedule(shared, rates, duration, max_in_flight, &token));
    if let Some(requests) = scheduler.await.expect("scheduler thread panicked") {
        // Wait for the last requests, unless a step breaks meanwhile: then they're past the
        // breaking point and needn't finish.
        join_or_cancel(requests, stop).await;
    }
    // Stopping because a step broke is how a breakpoint run is meant to end.
    if cancel.is_cancelled() { RunStatus::Cancelled } else { RunStatus::Completed }
}

/// Returns the in-flight requests when the schedule completes, or `None` if cancelled (after
/// aborting them).
fn schedule(
    shared: Arc<Shared>,
    rates: Rates,
    duration: Duration,
    max_in_flight: usize,
    cancel: &CancellationToken,
) -> Option<JoinSet<()>> {
    let permits = Arc::new(Semaphore::new(max_in_flight));
    let start = Instant::now();
    let deadline = start + duration;
    let mut requests = JoinSet::new();
    let mut scheduled = start;

    loop {
        let elapsed = scheduled - start;
        scheduled += Duration::from_secs_f64(1.0 / rates.at(elapsed));
        if scheduled >= deadline {
            break;
        }
        if wait_until(scheduled.into_std(), cancel) {
            requests.abort_all();
            return None;
        }

        let _ = shared.tx.send(Msg::Lag(Instant::now().saturating_duration_since(scheduled)));
        match Arc::clone(&permits).try_acquire_owned() {
            Ok(permit) => {
                let shared = Arc::clone(&shared);
                let at = scheduled;
                requests.spawn(async move {
                    shared.fire(at).await;
                    drop(permit);
                });
            }
            Err(_) => {
                let _ = shared.tx.send(Msg::Dropped);
            }
        }
        // Reap finished tasks so the set doesn't grow for the whole run.
        while requests.try_join_next().is_some() {}
    }
    Some(requests)
}

/// Sleeps until shortly before `at`, then spins to hit it within microseconds. Costs up to one
/// core at high rates; accuracy is the point of the open model. Returns true if cancelled.
fn wait_until(at: std::time::Instant, cancel: &CancellationToken) -> bool {
    const SPIN: Duration = Duration::from_millis(2);
    const MAX_SLEEP: Duration = Duration::from_millis(50);
    loop {
        if cancel.is_cancelled() {
            return true;
        }
        let now = std::time::Instant::now();
        if now >= at {
            return false;
        }
        let remaining = at - now;
        if remaining > SPIN {
            std::thread::sleep((remaining - SPIN).min(MAX_SLEEP));
        } else {
            std::hint::spin_loop();
        }
    }
}

/// Waits for in-flight requests to finish (each is bounded by the timeout), unless cancelled.
async fn join_or_cancel(mut set: JoinSet<()>, cancel: &CancellationToken) -> RunStatus {
    tokio::select! {
        _ = cancel.cancelled() => {
            set.abort_all();
            while set.join_next().await.is_some() {}
            RunStatus::Cancelled
        }
        _ = async { while set.join_next().await.is_some() {} } => RunStatus::Completed,
    }
}

struct Totals {
    all: Recorder,
    success: Recorder,
    ttfb: Recorder,
    phases: Phases,
    lag: Recorder,
    statuses: BTreeMap<u16, u64>,
    errors: ErrorCounts,
    total_errors: u64,
    dropped: u64,
    max_p99_ms: f64,
    samples: Vec<Sample>,
    contract: Option<ContractSummary>,
    breakpoint: Option<BreakpointResult>,
    spike: Option<SpikeResult>,
    rate_limit: Option<RateLimitResult>,
    soak: Option<SoakResult>,
    /// Multi-endpoint runs: one per endpoint of the mix, in its order.
    per_endpoint: Vec<EndpointTotals>,
}

struct EndpointTotals {
    latency: Recorder,
    errors: u64,
    statuses: BTreeMap<u16, u64>,
}

/// What watches a shaped open-model run as results come in.
enum Watch {
    Breakpoint(StepJudge),
    Spike(SpikeAnalyzer),
    Soak(SoakAnalyzer),
    /// Only 429s count as failures; the first rate-limit headers of each kind are kept.
    RateLimit {
        judge: StepJudge,
        limited_headers: Option<Vec<(String, String)>>,
        ok_headers: Option<Vec<(String, String)>>,
    },
}

impl Watch {
    fn scheduled(&mut self, at: Duration) {
        match self {
            Self::Breakpoint(j) | Self::RateLimit { judge: j, .. } => j.scheduled(at),
            Self::Spike(a) => a.scheduled(at),
            Self::Soak(a) => a.scheduled(at),
        }
    }

    fn dropped(&mut self, at: Duration) {
        match self {
            Self::Breakpoint(j) => j.dropped(at),
            Self::Spike(a) => a.dropped(at),
            Self::Soak(a) => a.dropped(at),
            // A drop at the in-flight cap is the engine's limit, not the target's 429. It counts as
            // answered-without-429 so it doesn't hold the step open as "pending".
            Self::RateLimit { judge, .. } => judge.done(at, Duration::ZERO, false),
        }
    }

    fn done(&mut self, rec: &Record) {
        let failed = rec.error.is_some();
        match self {
            Self::Breakpoint(j) => j.done(rec.offset, rec.latency, failed),
            Self::Spike(a) => a.done(rec.offset, rec.latency, failed),
            Self::Soak(a) => a.done(rec.offset, rec.latency, failed, rec.status),
            Self::RateLimit { judge, limited_headers, ok_headers } => {
                let limited = rec.status == Some(429);
                judge.done(rec.offset, rec.latency, limited);
                let slot = if limited { limited_headers } else { ok_headers };
                if slot.is_none() {
                    slot.clone_from(&rec.rate_headers);
                }
            }
        }
    }

    fn tick(&mut self, now: Duration) {
        if let Self::Breakpoint(j) | Self::RateLimit { judge: j, .. } = self {
            j.tick(now);
        }
    }

    fn finish(self, t: &mut Totals) {
        match self {
            Self::Breakpoint(j) => t.breakpoint = Some(j.finish()),
            Self::Spike(a) => t.spike = Some(a.finish()),
            Self::Soak(a) => t.soak = Some(a.finish()),
            Self::RateLimit { judge, limited_headers, ok_headers } => {
                let result = judge.finish();
                t.rate_limit = Some(RateLimitResult {
                    held_rate: result.held_rate,
                    limited_at_rate: result.broke_at_rate,
                    steps: result.steps,
                    limited_headers: limited_headers.unwrap_or_default(),
                    ok_headers: ok_headers.unwrap_or_default(),
                });
            }
        }
    }
}

async fn aggregate(
    run: Arc<Run>,
    mut rx: mpsc::UnboundedReceiver<Msg>,
    in_flight: Arc<AtomicU32>,
    started: Instant,
    timeout: Duration,
    mut watch: Option<Watch>,
    endpoints: usize,
) -> Totals {
    let mut t = Totals {
        all: Recorder::new(timeout),
        success: Recorder::new(timeout),
        ttfb: Recorder::new(timeout),
        phases: Phases::new(timeout),
        lag: Recorder::new(timeout),
        statuses: BTreeMap::new(),
        errors: ErrorCounts::default(),
        total_errors: 0,
        dropped: 0,
        max_p99_ms: 0.0,
        samples: Vec::new(),
        contract: None,
        breakpoint: None,
        spike: None,
        rate_limit: None,
        soak: None,
        per_endpoint: (0..endpoints)
            .map(|_| EndpointTotals { latency: Recorder::new(timeout), errors: 0, statuses: BTreeMap::new() })
            .collect(),
    };
    let mut window = Recorder::new(timeout);
    let (mut w_errors, mut w_dropped, mut w_lag) = (0u32, 0u32, Duration::ZERO);
    let mut w_opened = Instant::now();
    let mut ticks = tokio::time::interval_at(started + BUCKET_INTERVAL, BUCKET_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut flush = |t: &mut Totals, window: &mut Recorder, errors: &mut u32, dropped: &mut u32, lag: &mut Duration| {
        let requests = window.len() as u32;
        if requests == 0 && *dropped == 0 {
            return;
        }
        let (p50, p99) =
            if requests > 0 { (window.percentile_ms(50.0), window.percentile_ms(99.0)) } else { (0.0, 0.0) };
        t.max_p99_ms = t.max_p99_ms.max(p99);
        run.publish(RunEvent::Bucket(Bucket {
            t_ms: started.elapsed().as_millis() as u32,
            requests,
            errors: *errors,
            rps: f64::from(requests) / w_opened.elapsed().as_secs_f64().max(1e-6),
            p50_ms: p50,
            p99_ms: p99,
            dropped: *dropped,
            in_flight: in_flight.load(Ordering::Relaxed),
            lag_ms: lag.as_secs_f64() * 1000.0,
        }));
        window.reset();
        (*errors, *dropped, *lag) = (0, 0, Duration::ZERO);
        w_opened = Instant::now();
    };

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                None => break,
                Some(Msg::Dropped) => {
                    t.dropped += 1;
                    w_dropped += 1;
                    if let Some(watch) = watch.as_mut() {
                        watch.dropped(started.elapsed());
                    }
                }
                Some(Msg::Lag(lag)) => {
                    t.lag.record(lag);
                    w_lag = w_lag.max(lag);
                    // One per scheduled send, sent or dropped.
                    if let Some(watch) = watch.as_mut() {
                        watch.scheduled(started.elapsed().saturating_sub(lag));
                    }
                }
                Some(Msg::Done(rec)) => {
                    if let Some(watch) = watch.as_mut() {
                        watch.done(&rec);
                    }
                    t.all.record(rec.latency);
                    t.ttfb.record(rec.ttfb);
                    if let Some(e) = t.per_endpoint.get_mut(rec.endpoint) {
                        e.latency.record(rec.latency);
                        e.errors += u64::from(rec.error.is_some());
                        if let Some(code) = rec.status {
                            *e.statuses.entry(code).or_default() += 1;
                        }
                    }
                    if let Some(phases) = rec.phases {
                        t.phases.record(phases);
                    }
                    window.record(rec.latency);
                    if let Some(code) = rec.status {
                        *t.statuses.entry(code).or_default() += 1;
                    }
                    match rec.error {
                        None => t.success.record(rec.latency),
                        Some(class) => {
                            t.errors.add(class);
                            t.total_errors += 1;
                            w_errors += 1;
                        }
                    }
                    if let Some(check) = &rec.contract {
                        let summary = t.contract.get_or_insert_with(|| ContractSummary { sampled: true, ..Default::default() });
                        summary.add(check, str::to_owned);
                    }
                    if let Some(s) = rec.sample {
                        t.samples.push(*s);
                    }
                }
            },
            _ = ticks.tick() => {
                flush(&mut t, &mut window, &mut w_errors, &mut w_dropped, &mut w_lag);
                if let Some(watch) = watch.as_mut() {
                    watch.tick(started.elapsed());
                }
            }
        }
    }
    flush(&mut t, &mut window, &mut w_errors, &mut w_dropped, &mut w_lag);
    if let Some(watch) = watch {
        watch.finish(&mut t);
    }
    t
}

/// A response's rate-limit headers, or None if it had none.
fn rate_limit_headers(headers: &[(String, String)]) -> Option<Vec<(String, String)>> {
    let found: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| {
            let name = name.to_ascii_lowercase();
            name == "retry-after"
                || name.starts_with("x-ratelimit")
                || name.starts_with("x-rate-limit")
                || name.starts_with("ratelimit")
        })
        .cloned()
        .collect();
    (!found.is_empty()).then_some(found)
}

/// The headline of a rate-limit discovery run, for the notes.
fn rate_limit_note(result: &RateLimitResult, planned: Duration, ran: Duration) -> String {
    let limited = result.steps.iter().find(|s| !s.passed);
    match (limited, result.steps.last()) {
        (Some(step), _) => format!(
            "429s started at {} req/s ({:.1}% of that step).{}",
            step.rate,
            step.error_pct,
            match result.held_rate {
                Some(held) => format!(" {held} req/s went through without them."),
                None => " Even the first step got them; start lower.".into(),
            }
        ),
        (None, Some(last)) if ran < planned => format!(
            "No 429s up to {} req/s, where the duration cap stopped the run. Raise KESTREL_MAX_DURATION_S, or \
             use bigger or shorter steps, to go further.",
            last.rate
        ),
        (None, Some(last)) => format!("No 429s up to {} req/s: no rate limit at or below that rate.", last.rate),
        (None, None) => "No requests were sent.".into(),
    }
}

/// The headline of a breakpoint run, for the notes.
fn breakpoint_note(result: &BreakpointResult, planned: Duration, ran: Duration) -> Option<String> {
    let broke = result.steps.iter().find(|s| !s.passed);
    Some(match (broke, result.steps.last()) {
        (Some(step), _) => format!(
            "Broke at {} req/s ({}).{}",
            step.rate,
            step.reason.as_deref().unwrap_or("over a limit"),
            match result.held_rate {
                Some(held) => format!(" The last step that held was {held} req/s."),
                None => " Even the first step broke; start lower.".into(),
            }
        ),
        (None, Some(last)) if ran < planned => format!(
            "Stopped at the duration cap at {} req/s without breaking. Raise KESTREL_MAX_DURATION_S, or use \
             bigger or shorter steps, to go further.",
            last.rate
        ),
        (None, Some(last)) => format!("Didn't break: every step held, up to {} req/s.", last.rate),
        (None, None) => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use axum::{Router, response::Redirect, routing::get};

    use super::*;
    use crate::{
        engine::registry::RunRegistry,
        model::{Endpoint, HttpMethod, Workspace},
    };

    /// `/pause`: every request arriving between 0.5 s and 1.5 s after the first one waits until 1.5 s
    /// (a global stall, like a GC pause). `/slow`: 300 ms. `/redir`: 303 to `/ok`.
    async fn server() -> String {
        let first: Arc<OnceLock<std::time::Instant>> = Arc::default();
        let app = Router::new()
            .route(
                "/pause",
                get(move || {
                    let first = Arc::clone(&first);
                    async move {
                        let t0 = *first.get_or_init(std::time::Instant::now);
                        let (from, until) = (t0 + Duration::from_millis(500), t0 + Duration::from_millis(1500));
                        let now = std::time::Instant::now();
                        if now >= from && now < until {
                            tokio::time::sleep(until - now).await;
                        }
                        "ok"
                    }
                }),
            )
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    "ok"
                }),
            )
            .route("/redir", get(|| async { Redirect::to("/ok") }))
            // 30 requests per second, then 429s; announces the limit on every response.
            .route(
                "/limited",
                get({
                    let window: Arc<std::sync::Mutex<(std::time::Instant, u32)>> =
                        Arc::new(std::sync::Mutex::new((std::time::Instant::now(), 0)));
                    move || {
                        let window = Arc::clone(&window);
                        async move {
                            use axum::{http::StatusCode, response::IntoResponse};
                            let over = {
                                let mut w = window.lock().unwrap();
                                if w.0.elapsed() >= Duration::from_secs(1) {
                                    *w = (std::time::Instant::now(), 0);
                                }
                                w.1 += 1;
                                w.1 > 30
                            };
                            let limit = [("X-RateLimit-Limit", "30")];
                            if over {
                                (StatusCode::TOO_MANY_REQUESTS, limit, [("Retry-After", "1")], "slow down")
                                    .into_response()
                            } else {
                                (limit, "ok").into_response()
                            }
                        }
                    }
                }),
            )
            // Serves one request at a time, 50 ms each: about 20 req/s before requests queue up.
            .route(
                "/capacity",
                get({
                    let one = Arc::new(tokio::sync::Semaphore::new(1));
                    move || {
                        let one = Arc::clone(&one);
                        async move {
                            let _permit = one.acquire().await.unwrap();
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            "ok"
                        }
                    }
                }),
            )
            .route("/ok", get(|| async { "ok" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn config(mode: LoadMode, duration_ms: u32) -> LoadConfig {
        LoadConfig {
            endpoint_id: uuid::Uuid::nil(),
            environment: None,
            mode,
            duration_ms,
            ramp_up_ms: 0,
            timeout_ms: 5_000,
            keep_alive: true,
            max_in_flight: None,
            ok_statuses: vec![],
            mix: vec![],
            token_refresh: None,
        }
    }

    async fn run_load(url: String, cfg: LoadConfig) -> RunReport {
        run_load_capped(url, cfg, 100_000).await
    }

    async fn run_load_capped(url: String, cfg: LoadConfig, max_rps: u32) -> RunReport {
        let endpoint = Endpoint {
            id: uuid::Uuid::nil(),
            name: String::new(),
            group: None,
            method: HttpMethod::Get,
            url,
            headers: vec![],
            query: vec![],
            body: Default::default(),
            auth: Default::default(),
            expect: None,
            extract: vec![],
        };
        let request =
            CompiledRequest::compile(&endpoint, &Workspace::default(), &Default::default(), None, false).unwrap();
        let prepared = match prepare(cfg.clone(), request, &[], 10_000, max_rps).await {
            Ok(p) => p,
            Err(_) => panic!("prepare failed"),
        };
        let registry = Arc::new(RunRegistry::default());
        let r = registry.create(uuid::Uuid::nil(), super::super::types::RunConfig::Load(cfg));
        super::run(Arc::clone(&r), prepared).await;
        r.report().unwrap()
    }

    /// The whole point of the open model: a 1 s stall hits every request scheduled during it.
    /// A single closed-model user sends one request into the stall and waits, so the stall is
    /// nearly invisible in its percentiles.
    #[tokio::test(flavor = "multi_thread")]
    async fn open_model_is_free_of_coordinated_omission() {
        let base = server().await;
        let open = run_load(format!("{base}/pause"), config(LoadMode::Open { rate: 100 }, 2_000)).await;
        let base = server().await;
        let closed = run_load(format!("{base}/pause"), config(LoadMode::Closed { concurrency: 1 }, 2_000)).await;

        let (open, closed) = (open.latency.unwrap(), closed.latency.unwrap());
        assert!(open.p90_ms >= 400.0, "open p90 should show the stall: {open:?}");
        assert!(closed.p99_ms < 200.0, "closed p99 hides the stall (that's the bias): {closed:?}");
        assert!(closed.max_ms >= 900.0, "the one stalled request is still recorded: {closed:?}");
    }

    /// Phases: with keep-alive, connections are reused and nearly all the time is waiting; without
    /// it, every request opens its own connection and pays for it.
    #[tokio::test(flavor = "multi_thread")]
    async fn phases_split_connect_from_waiting() {
        let base = server().await;
        let kept = run_load(format!("{base}/slow"), config(LoadMode::Closed { concurrency: 2 }, 1_000)).await;
        let p = kept.phases.unwrap();
        assert!(p.requests >= 4, "{p:?}");
        assert!(p.new_connections >= 1 && p.new_connections <= 3, "reused: {p:?}");
        assert!(p.connect.as_ref().unwrap().count == p.new_connections, "{p:?}");
        let waiting = p.waiting.unwrap();
        assert!(waiting.p50_ms >= 290.0 && waiting.p50_ms < 600.0, "the server's 300 ms: {waiting:?}");
        assert_eq!(p.dns_ms, None, "an IP in the URL needs no lookup");

        let mut cfg = config(LoadMode::Closed { concurrency: 2 }, 1_000);
        cfg.keep_alive = false;
        let fresh = run_load(format!("{base}/slow"), cfg).await.phases.unwrap();
        assert_eq!(fresh.new_connections, fresh.requests, "no keep-alive: every request connects: {fresh:?}");
    }

    /// Breakpoint: steps up until queueing pushes p99 over the limit, then stops early.
    #[tokio::test(flavor = "multi_thread")]
    async fn breakpoint_stops_at_the_step_the_target_cannot_keep_up_with() {
        use crate::engine::types::BreakpointMode;
        let base = server().await;
        let mode = BreakpointMode {
            start_rate: 5,
            step_percent: 100,
            step_ms: 1_000,
            max_rate: 80,
            max_error_pct: 5.0,
            max_p99_ms: Some(250),
        };
        let report = run_load(format!("{base}/capacity"), config(LoadMode::Breakpoint(mode), 30_000)).await;
        let result = report.breakpoint.as_ref().unwrap();
        let rates: Vec<u32> = result.steps.iter().map(|s| s.rate).collect();
        assert!(matches!(result.broke_at_rate, Some(20 | 40)), "breaks around its ~20 req/s capacity: {result:?}");
        assert!(result.held_rate.is_some_and(|r| r >= 10), "{result:?}");
        assert!(rates.len() < 5, "stopped before the last step: {rates:?}");
        assert_eq!(report.status, RunStatus::Completed, "breaking is the expected end, not a cancel");
        assert!(report.notes.iter().any(|n| n.starts_with("Broke at")), "{:?}", report.notes);
        let elapsed_s = (report.finished_at_ms - report.started_at_ms) as f64 / 1000.0;
        assert!(elapsed_s < 8.0, "didn't run all five steps: {elapsed_s} s");
    }

    /// Spike: a burst over the target's capacity builds a queue; the baseline after it shows how
    /// long the queue takes to drain.
    #[tokio::test(flavor = "multi_thread")]
    async fn spike_measures_recovery_after_the_burst() {
        use crate::engine::types::SpikeMode;
        let base = server().await;
        let mode = SpikeMode { base_rate: 5, spike_rate: 60, before_ms: 3_000, spike_ms: 1_000, after_ms: 8_000 };
        let report = run_load(format!("{base}/capacity"), config(LoadMode::Spike(mode), 12_000)).await;
        let r = report.spike.as_ref().unwrap();
        let (baseline, spike) = (r.baseline.as_ref().unwrap(), r.spike.as_ref().unwrap());
        assert!(spike.p99_ms > baseline.p99_ms * 4.0, "the burst queues up: {r:?}");
        let recovery = r.recovery_ms.expect("drains well within 8 s");
        assert!((1_000..=6_000).contains(&recovery), "~40 queued at ~15/s net: {r:?}");
        assert!(report.notes.iter().any(|n| n.starts_with("Back to normal")), "{:?}", report.notes);
    }

    /// Rate-limit discovery: finds where 429s start and what the API says about its limit.
    #[tokio::test(flavor = "multi_thread")]
    async fn rate_limit_discovery_finds_the_threshold_and_headers() {
        use crate::engine::types::RateLimitMode;
        let base = server().await;
        let mode = RateLimitMode { start_rate: 10, step_percent: 100, step_ms: 1_000, max_rate: 80 };
        let report = run_load(format!("{base}/limited"), config(LoadMode::RateLimit(mode), 30_000)).await;
        let r = report.rate_limit.as_ref().unwrap();
        assert_eq!((r.held_rate, r.limited_at_rate), (Some(20), Some(40)), "{r:?}");
        let has = |hs: &[(String, String)], name: &str| hs.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
        assert!(has(&r.limited_headers, "retry-after") && has(&r.limited_headers, "x-ratelimit-limit"), "{r:?}");
        assert!(has(&r.ok_headers, "x-ratelimit-limit") && !has(&r.ok_headers, "retry-after"), "{r:?}");
        let limited = r.steps.iter().find(|s| !s.passed).unwrap();
        assert!(limited.reason.as_deref().unwrap().starts_with("429s"), "{limited:?}");
        assert!(report.notes.iter().any(|n| n.starts_with("429s started at 40 req/s")), "{:?}", report.notes);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sends_beyond_the_in_flight_cap_are_counted_as_dropped() {
        let base = server().await;
        let mut cfg = config(LoadMode::Open { rate: 100 }, 1_000);
        cfg.max_in_flight = Some(5);
        let report = run_load(format!("{base}/slow"), cfg).await;

        assert!(report.dropped >= 70, "dropped {}", report.dropped);
        let scheduled = report.total_requests + report.dropped;
        assert!((95..=100).contains(&scheduled), "completed + dropped = {scheduled}");
        assert!(report.notes.iter().any(|n| n.contains("dropped")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn closed_model_is_held_to_the_rate_cap() {
        let base = server().await;
        let report =
            run_load_capped(format!("{base}/ok"), config(LoadMode::Closed { concurrency: 20 }, 1_000), 200).await;
        assert!((180..=210).contains(&report.total_requests), "sent {} at a 200 rps cap", report.total_requests);
        assert!(report.notes.iter().any(|n| n.contains("cap")), "{:?}", report.notes);
    }

    /// A 3:1 mix of a fast and a slow endpoint: traffic splits by weight, each endpoint gets its own
    /// numbers, and an endpoint on another host is refused.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_mix_spreads_requests_by_weight_and_reports_each_endpoint() {
        use crate::engine::types::MixEntry;
        let base = server().await;
        let compile = |url: String| {
            let endpoint = Endpoint {
                id: uuid::Uuid::nil(),
                name: String::new(),
                group: None,
                method: HttpMethod::Get,
                url,
                headers: vec![],
                query: vec![],
                body: Default::default(),
                auth: Default::default(),
                expect: None,
                extract: vec![],
            };
            CompiledRequest::compile(&endpoint, &Workspace::default(), &Default::default(), None, false).unwrap()
        };
        let (fast, slow) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let mut cfg = config(LoadMode::Open { rate: 200 }, 2_000);
        cfg.endpoint_id = fast;
        cfg.mix = vec![MixEntry { endpoint_id: fast, weight: 3 }, MixEntry { endpoint_id: slow, weight: 1 }];
        let target = |id, name: &str, weight, url: String| MixTarget {
            endpoint_id: id,
            name: name.into(),
            weight,
            request: compile(url),
            contract: None,
        };
        let Ok(prepared) = prepare(cfg.clone(), compile(format!("{base}/ok")), &[], 10_000, 100_000).await else {
            panic!("prepare failed")
        };
        let elsewhere = prepared.with_mix(vec![target(slow, "other", 1, "http://127.0.0.2:9/x".into())]);
        assert!(elsewhere.is_err_and(|e| e.contains("different host")));

        let Ok(prepared) = prepare(cfg.clone(), compile(format!("{base}/ok")), &[], 10_000, 100_000).await else {
            panic!("prepare failed")
        };
        let prepared = prepared
            .with_mix(vec![
                target(fast, "ok", 3, format!("{base}/ok")),
                target(slow, "slow", 1, format!("{base}/slow")),
            ])
            .unwrap();
        let registry = Arc::new(RunRegistry::default());
        let r = registry.create(uuid::Uuid::nil(), super::super::types::RunConfig::Load(cfg));
        super::run(Arc::clone(&r), prepared).await;
        let report = r.report().unwrap();

        let mix = report.mix.as_ref().expect("per-endpoint stats");
        assert_eq!((mix[0].name.as_str(), mix[1].name.as_str()), ("ok", "slow"));
        let (a, b) = (mix[0].requests as f64, mix[1].requests as f64);
        assert_eq!(a + b, report.total_requests as f64);
        assert!((0.68..=0.82).contains(&(a / (a + b))), "about 3 in 4 to the first: {a} vs {b}");
        let (fast_p50, slow_p50) = (mix[0].latency.as_ref().unwrap().p50_ms, mix[1].latency.as_ref().unwrap().p50_ms);
        assert!(fast_p50 < 100.0 && slow_p50 >= 290.0, "each endpoint's own latency: {fast_p50} vs {slow_p50}");
    }

    /// Token refresh: tokens that expire after 1.5 s, a run that starts with a stale one. It's
    /// refreshed before the first request, and again early when 401s start.
    #[tokio::test(flavor = "multi_thread")]
    async fn token_refresh_keeps_a_short_lived_token_fresh() {
        use crate::{
            db::{Db, crypto::SecretsCipher},
            engine::refresh::Refresher,
            model::{Auth, Collection, Environment, Extract, ExtractSource, ExtractTarget, store::WorkspaceStore},
        };
        use axum::{http::StatusCode, http::header, routing::get};

        // The latest token and when it was issued; it works for 1.5 s.
        let issued: Arc<std::sync::Mutex<(u32, std::time::Instant)>> =
            Arc::new(std::sync::Mutex::new((0, std::time::Instant::now())));
        let (for_token, for_check) = (Arc::clone(&issued), Arc::clone(&issued));
        let app = Router::new()
            .route(
                "/token",
                get(move || {
                    let issued = Arc::clone(&for_token);
                    async move {
                        let mut i = issued.lock().unwrap();
                        *i = (i.0 + 1, std::time::Instant::now());
                        axum::Json(serde_json::json!({ "token": format!("tok-{}", i.0) }))
                    }
                }),
            )
            .route(
                "/protected",
                get(move |headers: axum::http::HeaderMap| {
                    let issued = Arc::clone(&for_check);
                    async move {
                        let (n, at) = *issued.lock().unwrap();
                        let sent = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
                        let ok = sent == format!("Bearer tok-{n}") && at.elapsed() < Duration::from_millis(1500);
                        if ok { StatusCode::OK } else { StatusCode::UNAUTHORIZED }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let endpoint = |path: &str, auth: Auth, extract: Vec<Extract>| Endpoint {
            id: uuid::Uuid::new_v4(),
            name: path.trim_start_matches('/').into(),
            group: None,
            method: HttpMethod::Get,
            url: format!("{{{{base}}}}{path}"),
            headers: vec![],
            query: vec![],
            body: Default::default(),
            auth,
            expect: None,
            extract,
        };
        let rule = Extract {
            source: ExtractSource::Body,
            path: "token".into(),
            target: ExtractTarget::Variable,
            name: "token".into(),
            enabled: true,
        };
        let token = endpoint("/token", Auth::None, vec![rule]);
        let protected = endpoint("/protected", Auth::Bearer { token: "{{token}}".into() }, vec![]);
        let workspace = Workspace {
            collections: vec![Collection {
                headers: vec![],
                id: uuid::Uuid::new_v4(),
                name: "c".into(),
                vars: Default::default(),
                endpoints: vec![token.clone(), protected.clone()],
                groups: vec![],
                source: None,
                schema_defs: None,
            }],
            environments: vec![Environment {
                name: "local".into(),
                vars: [("base".to_string(), base), ("token".to_string(), "stale".to_string())].into(),
            }],
            active_environment: Some("local".into()),
            ..Default::default()
        };
        // Variables only: the store never needs the database here.
        let db = Db::lazy("postgres://unused@127.0.0.1:1/unused", 1, SecretsCipher::for_tests(), "x_").unwrap();
        let store = Arc::new(WorkspaceStore::empty_for_tests(Arc::new(db)));
        store.set_workspace_for_tests(workspace.clone());

        let request =
            CompiledRequest::compile(&protected, &workspace, &Default::default(), Some("local"), false).unwrap();
        let cfg = config(LoadMode::Open { rate: 50 }, 3_000);
        let Ok(mut prepared) = prepare(cfg.clone(), request, &[], 10_000, 100_000).await else {
            panic!("prepare failed")
        };
        prepared.refresh = Some(Refresher::new(
            Duration::from_secs(60),
            token,
            vec![protected],
            "local".into(),
            store,
            Duration::from_secs(2),
        ));
        let registry = Arc::new(RunRegistry::default());
        let r = registry.create(uuid::Uuid::nil(), super::super::types::RunConfig::Load(cfg));
        super::run(Arc::clone(&r), prepared).await;
        let report = r.report().unwrap();

        let unauthorized = report.status_counts.iter().find(|s| s.status == 401).map_or(0, |s| s.count);
        assert!(
            unauthorized * 100 < report.total_requests * 15,
            "a few 401s while the new token comes in, not a wall: {unauthorized} of {}",
            report.total_requests
        );
        let note = report.notes.iter().find(|n| n.starts_with("Refreshed the token")).expect("a refresh note");
        assert!(note.contains("early because of 401s"), "{note}");
        assert!(!report.notes.iter().any(|n| n.contains("failed")), "{:?}", report.notes);
    }

    /// Soak: windows sized from the run, used as the timeline, and a steady target reads as steady.
    #[tokio::test(flavor = "multi_thread")]
    async fn soak_reports_windows_as_the_timeline() {
        let base = server().await;
        let report = run_load(format!("{base}/ok"), config(LoadMode::Soak { rate: 50 }, 6_000)).await;
        let soak = report.soak.as_ref().unwrap();
        assert_eq!(soak.window_ms, 1_000);
        assert!((6..=7).contains(&soak.windows.len()), "{:?}", soak.windows);
        assert_eq!(report.timeline.len(), soak.windows.len(), "the windows are the timeline");
        assert!((280..=310).contains(&report.total_requests), "{}", report.total_requests);
        assert_eq!(soak.findings[0].level, crate::engine::types::FindingLevel::Good, "{:?}", soak.findings);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn redirects_are_not_followed_under_load() {
        let base = server().await;
        let report = run_load(format!("{base}/redir"), config(LoadMode::Closed { concurrency: 2 }, 300)).await;
        assert!(report.total_requests > 0);
        assert!(report.status_counts.iter().all(|s| s.status == 303), "{:?}", report.status_counts);
    }
}
