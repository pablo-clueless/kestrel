//! Load test: closed model (fixed concurrency) or open model (fixed arrival rate).
//! HANDOFF → Test catalogue → Load test.
//!
//! Request tasks send their results to one aggregator task over a channel; the aggregator owns the
//! histograms and emits a bucket every 250 ms. In the open model, latency is measured from the
//! **scheduled** send time, so scheduler lateness and a stalled server both show up in the numbers
//! (no coordinated omission), and sends beyond the in-flight cap are counted as dropped.

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

use super::{
    BUCKET_INTERVAL,
    client::{self, ClientOptions, Target},
    registry::{Run, now_ms},
    sample,
    types::{
        Bucket, ErrorClass, ErrorCounts, LoadConfig, LoadMode, RunEvent, RunReport, RunStatus, Sample, StartedEvent,
        StatusCount, TargetInfo,
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
    Ok(Prepared { cfg, request, target, target_info, max_in_flight, max_rps, contract: None })
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
    latency: Duration,
    ttfb: Duration,
    status: Option<u16>,
    error: Option<ErrorClass>,
    sample: Option<Box<Sample>>,
    contract: Option<ContractCheck>,
}

/// Shared by every request task. The channel closes when the last clone is dropped.
struct Shared {
    client: Client,
    request: CompiledRequest,
    timeout: Duration,
    ok_statuses: Vec<u16>,
    tx: mpsc::UnboundedSender<Msg>,
    in_flight: Arc<AtomicU32>,
    success_sampled: AtomicBool,
    error_samples: AtomicUsize,
    contract: Option<Arc<Contract>>,
    /// (window start, checks in window) for the contract sampling rate.
    contract_gate: Mutex<(Instant, u32)>,
}

impl Shared {
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
        let req = match self.request.render() {
            Ok(req) => req,
            Err(msg) => {
                tracing::warn!("render failed mid-run: {msg}");
                let _ = self.tx.send(Msg::Done(Record {
                    latency: Duration::ZERO,
                    ttfb: Duration::ZERO,
                    status: None,
                    error: Some(ErrorClass::InvalidRequest),
                    sample: None,
                    contract: None,
                }));
                return;
            }
        };
        let outcome = client::execute(&self.client, &req, self.timeout).await.classify_with(&self.ok_statuses);
        let latency = scheduled.elapsed();

        let check = match (&self.contract, outcome.status) {
            (Some(contract), Some(status)) if self.contract_slot() => Some(contract.check(status, &outcome.body)),
            _ => None,
        };

        let clean = outcome.is_success() && check.as_ref().is_none_or(|c| c.passed);
        let want_sample = if clean {
            !self.success_sampled.swap(true, Ordering::Relaxed)
        } else {
            self.error_samples.fetch_add(1, Ordering::Relaxed) < MAX_ERROR_SAMPLES
        };
        let redactor = Redactor::new(self.request.api_key_header.as_deref(), &self.request.secret_values, Some(&req));
        let check = check.map(|c| ContractCheck { message: redactor.text(&c.message), ..c });
        let sample = want_sample.then(|| {
            let mut s = sample::build(&req, &outcome, &redactor, SAMPLE_BODY_BYTES);
            s.contract = check.clone();
            Box::new(s)
        });

        let _ = self.tx.send(Msg::Done(Record {
            latency,
            ttfb: outcome.ttfb,
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

pub async fn run(run: Arc<Run>, p: Prepared) {
    platform::high_res_timer();
    run.publish(RunEvent::Started(StartedEvent {
        run_id: run.id,
        config: run.config.clone(),
        started_at_ms: run.started_at_ms,
    }));

    let timeout = Duration::from_millis(p.cfg.timeout_ms.into());
    let duration = Duration::from_millis(p.cfg.duration_ms.into());
    let ramp = Duration::from_millis(p.cfg.ramp_up_ms.into()).min(duration);
    let (tx, rx) = mpsc::unbounded_channel();
    let in_flight = Arc::new(AtomicU32::new(0));
    let started = Instant::now();

    let aggregator = tokio::spawn(aggregate(Arc::clone(&run), rx, Arc::clone(&in_flight), started, timeout));

    let shared = Arc::new(Shared {
        client: p.target.client,
        request: p.request,
        timeout,
        ok_statuses: p.cfg.ok_statuses.clone(),
        tx,
        in_flight,
        success_sampled: AtomicBool::new(false),
        error_samples: AtomicUsize::new(0),
        contract: p.contract.clone(),
        contract_gate: Mutex::new((Instant::now(), 0)),
    });
    let pacer = Arc::new(Pacer::new(p.max_rps));
    let status = match p.cfg.mode {
        LoadMode::Closed { concurrency } => {
            closed(shared, Arc::clone(&pacer), concurrency, duration, ramp, &run.cancel).await
        }
        LoadMode::Open { rate } => {
            let cap = p.cfg.max_in_flight.unwrap_or(p.max_in_flight).clamp(1, p.max_in_flight);
            open(shared, rate, duration, ramp, cap as usize, &run.cancel).await
        }
    };
    let active = started.elapsed().min(duration.max(Duration::from_millis(1)));
    // Every `Shared` clone is gone now, so the channel is closed and the aggregator finishes.
    let totals = aggregator.await.expect("aggregator task panicked");

    let mut notes = Vec::new();
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
        histogram: totals.all.bins(),
        status_counts: totals.statuses.into_iter().map(|(status, count)| StatusCount { status, count }).collect(),
        error_counts: totals.errors,
        samples: totals.samples,
        notes,
        dropped: totals.dropped,
        generator_lag: totals.lag.summary(),
        contract: totals.contract,
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

/// Sends at `rate`/s (ramping linearly over `ramp`), never more than `max_in_flight` at once.
///
/// The schedule runs on a blocking thread rather than a tokio timer: tokio's timer wheel has 1 ms
/// resolution, which made every send ~1 ms late, and in the open model that lateness is latency.
async fn open(
    shared: Arc<Shared>,
    rate: u32,
    duration: Duration,
    ramp: Duration,
    max_in_flight: usize,
    cancel: &CancellationToken,
) -> RunStatus {
    let token = cancel.clone();
    let scheduler = tokio::task::spawn_blocking(move || schedule(shared, rate, duration, ramp, max_in_flight, &token));
    match scheduler.await.expect("scheduler thread panicked") {
        Some(requests) => join_or_cancel(requests, cancel).await,
        None => RunStatus::Cancelled,
    }
}

/// Returns the in-flight requests when the schedule completes, or `None` if cancelled (after
/// aborting them).
fn schedule(
    shared: Arc<Shared>,
    rate: u32,
    duration: Duration,
    ramp: Duration,
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
        let ramp_fraction = if ramp.is_zero() { 1.0 } else { (elapsed.as_secs_f64() / ramp.as_secs_f64()).min(1.0) };
        let current_rate = (f64::from(rate) * ramp_fraction).max(1.0);
        scheduled += Duration::from_secs_f64(1.0 / current_rate);
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
    lag: Recorder,
    statuses: BTreeMap<u16, u64>,
    errors: ErrorCounts,
    total_errors: u64,
    dropped: u64,
    max_p99_ms: f64,
    samples: Vec<Sample>,
    contract: Option<ContractSummary>,
}

async fn aggregate(
    run: Arc<Run>,
    mut rx: mpsc::UnboundedReceiver<Msg>,
    in_flight: Arc<AtomicU32>,
    started: Instant,
    timeout: Duration,
) -> Totals {
    let mut t = Totals {
        all: Recorder::new(timeout),
        success: Recorder::new(timeout),
        ttfb: Recorder::new(timeout),
        lag: Recorder::new(timeout),
        statuses: BTreeMap::new(),
        errors: ErrorCounts::default(),
        total_errors: 0,
        dropped: 0,
        max_p99_ms: 0.0,
        samples: Vec::new(),
        contract: None,
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
                }
                Some(Msg::Lag(lag)) => {
                    t.lag.record(lag);
                    w_lag = w_lag.max(lag);
                }
                Some(Msg::Done(rec)) => {
                    t.all.record(rec.latency);
                    t.ttfb.record(rec.ttfb);
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
            _ = ticks.tick() => flush(&mut t, &mut window, &mut w_errors, &mut w_dropped, &mut w_lag),
        }
    }
    flush(&mut t, &mut window, &mut w_errors, &mut w_dropped, &mut w_lag);
    t
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
        let r = registry.create(super::super::types::RunConfig::Load(cfg));
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

    #[tokio::test(flavor = "multi_thread")]
    async fn redirects_are_not_followed_under_load() {
        let base = server().await;
        let report = run_load(format!("{base}/redir"), config(LoadMode::Closed { concurrency: 2 }, 300)).await;
        assert!(report.total_requests > 0);
        assert!(report.status_counts.iter().all(|s| s.status == 303), "{:?}", report.status_counts);
    }
}
