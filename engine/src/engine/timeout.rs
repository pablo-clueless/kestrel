//! Timeout behaviour: does the server fail fast, or hang? And does it keep working on requests the
//! client has given up on? HANDOFF → Test catalogue → v1.x → Timeout behaviour.
//!
//! Three phases. **Before:** `samples` sequential probes with a generous timeout, as a baseline; a
//! probe that runs into it means the server never answered. **Burst:** `abandoned` requests,
//! `concurrency` at a time, each given up on after `tight_ms`, so the client disconnects mid-request.
//! **After:** the same probes again. A server that cancels work when the client leaves (or has room
//! to spare) answers them as before; one that keeps working queues them behind the abandoned work.

use std::{sync::Arc, time::Duration};

use tokio::{sync::Semaphore, task::JoinSet, time::Instant};

use super::{
    client::{self, ClientOptions, Outcome, Target},
    load::PrepareError,
    phases::Phases,
    registry::{Run, now_ms},
    sample,
    types::{
        Bucket, ErrorClass, ErrorCounts, Finding, FindingLevel, LatencySummary, RunEvent, RunReport, RunStatus, Sample,
        StartedEvent, StatusCount, TargetInfo, TimeoutConfig, TimeoutResult,
    },
};
use crate::{redact::Redactor, stats::Recorder, template::request::CompiledRequest};

pub const MAX_ABANDONED: u32 = 1_000;
pub const MAX_BURST_CONCURRENCY: u32 = 200;
const SAMPLE_BODY_BYTES: usize = 8 * 1024;
const MAX_ERROR_SAMPLES: usize = 5;

pub struct Prepared {
    pub cfg: TimeoutConfig,
    pub request: CompiledRequest,
    pub target: Target,
    pub target_info: TargetInfo,
    pub max_duration: Duration,
}

/// The burst is load, so like a load test the host must be confirmed unless it's this machine.
pub async fn prepare(
    cfg: TimeoutConfig,
    request: CompiledRequest,
    confirmed_hosts: &[String],
    max_duration: Duration,
) -> Result<Prepared, PrepareError> {
    let first = request.preview().map_err(PrepareError::Invalid)?;
    let host = first.url.host_str().unwrap_or_default().to_ascii_lowercase();
    let opts = ClientOptions { keep_alive: true, follow_redirects: false, confirmed_hosts: vec![] };
    let target = client::connect(&first.url, &opts).await.map_err(PrepareError::Invalid)?;
    if !target.pinned.is_loopback() && !confirmed_hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
        return Err(PrepareError::HostNotConfirmed(host));
    }
    let target_info = TargetInfo { host, pinned_ip: target.pinned.to_string(), loopback: target.pinned.is_loopback() };
    Ok(Prepared { cfg, request, target, target_info, max_duration })
}

/// Everything the report needs, gathered across the phases.
struct Totals {
    all: Recorder,
    success: Recorder,
    ttfb: Recorder,
    phases: Phases,
    statuses: std::collections::BTreeMap<u16, u64>,
    errors: ErrorCounts,
    total_errors: u64,
    samples: Vec<Sample>,
    have_success_sample: bool,
    error_samples: usize,
}

impl Totals {
    fn record(&mut self, p: &Prepared, req: &crate::template::request::RenderedRequest, o: &Outcome) {
        self.all.record(o.total);
        self.ttfb.record(o.ttfb);
        self.phases.record(o.into());
        if let Some(code) = o.status {
            *self.statuses.entry(code).or_default() += 1;
        }
        match &o.error {
            None => self.success.record(o.total),
            Some((class, _)) => {
                self.errors.add(*class);
                self.total_errors += 1;
            }
        }
        let want = if o.is_success() { !self.have_success_sample } else { self.error_samples < MAX_ERROR_SAMPLES };
        if want {
            let redactor = Redactor::new(p.request.api_key_header.as_deref(), &p.request.secret_values, Some(req));
            self.samples.push(sample::build(req, o, &redactor, SAMPLE_BODY_BYTES));
            if o.is_success() { self.have_success_sample = true } else { self.error_samples += 1 }
        }
    }
}

/// One phase of sequential probes: their latencies, and when each finished (from the phase start).
struct Probes {
    latency: Recorder,
    /// (finished at, took) per probe, in order.
    times: Vec<(Duration, Duration)>,
    hung: u32,
    fast_failures: u32,
}

enum Stop {
    Cancelled,
    Capped,
}

pub async fn run(run: Arc<Run>, p: Prepared) {
    run.publish(RunEvent::Started(StartedEvent {
        run_id: run.id,
        config: run.config.clone(),
        started_at_ms: run.started_at_ms,
    }));

    let timeout = Duration::from_millis(p.cfg.timeout_ms.into());
    let mut t = Totals {
        all: Recorder::new(timeout),
        success: Recorder::new(timeout),
        ttfb: Recorder::new(timeout),
        phases: Phases::new(timeout),
        statuses: Default::default(),
        errors: ErrorCounts::default(),
        total_errors: 0,
        samples: Vec::new(),
        have_success_sample: false,
        error_samples: 0,
    };
    let started = Instant::now();
    let mut notes = Vec::new();
    let mut status = RunStatus::Completed;

    let before = probes(&run, &p, &mut t, started, timeout).await;
    let before = match before {
        Ok(b) => b,
        Err(stop) => {
            stopped(&mut status, &mut notes, stop, &p, "the first probes");
            return finish(run, p, t, status, notes, started, None);
        }
    };
    publish(&run, started, &before.latency, before.hung + before.fast_failures);

    // A quarter of the typical answer time abandons most requests mid-flight.
    let tight_ms =
        if p.cfg.tight_ms > 0 { p.cfg.tight_ms } else { ((before.latency.percentile_ms(50.0) / 4.0) as u32).max(1) };
    let burst = burst(&run, &p, Duration::from_millis(tight_ms.into())).await;
    let Some((abandoned, answered_in_time)) = burst else {
        status = RunStatus::Cancelled;
        return finish(run, p, t, status, notes, started, None);
    };

    let after = match probes(&run, &p, &mut t, started, timeout).await {
        Ok(a) => Some(a),
        Err(stop) => {
            stopped(&mut status, &mut notes, stop, &p, "the probes after the burst");
            None
        }
    };
    if let Some(a) = &after {
        publish(&run, started, &a.latency, a.hung + a.fast_failures);
    }

    let result = analyse(&before, after.as_ref(), tight_ms, abandoned, answered_in_time, timeout);
    finish(run, p, t, status, notes, started, Some(result));
}

fn stopped(status: &mut RunStatus, notes: &mut Vec<String>, stop: Stop, p: &Prepared, during: &str) {
    match stop {
        Stop::Cancelled => *status = RunStatus::Cancelled,
        Stop::Capped => {
            notes.push(format!("Stopped at the {} s duration cap during {during}.", p.max_duration.as_secs()))
        }
    }
}

async fn probes(run: &Run, p: &Prepared, t: &mut Totals, started: Instant, timeout: Duration) -> Result<Probes, Stop> {
    let mut out = Probes { latency: Recorder::new(timeout), times: Vec::new(), hung: 0, fast_failures: 0 };
    let phase = Instant::now();
    for _ in 0..p.cfg.samples {
        if started.elapsed() >= p.max_duration {
            return Err(Stop::Capped);
        }
        let Ok(req) = p.request.render() else { continue };
        let o = tokio::select! {
            _ = run.cancel.cancelled() => return Err(Stop::Cancelled),
            o = client::execute(&p.target.client, &req, timeout) => o,
        };
        t.record(p, &req, &o);
        out.latency.record(o.total);
        out.times.push((phase.elapsed(), o.total));
        let timed_out = matches!(o.error, Some((ErrorClass::Timeout, _)));
        let failed = o.status.is_none_or(|s| s >= 500);
        if timed_out {
            out.hung += 1;
        } else if failed && o.total < timeout / 2 {
            out.fast_failures += 1;
        }
    }
    Ok(out)
}

/// Sends the burst; returns (sent, answered within the tight timeout), or None if cancelled. Burst
/// requests are cut short on purpose, so they're counted here and kept out of the latency numbers.
async fn burst(run: &Run, p: &Prepared, tight: Duration) -> Option<(u32, u32)> {
    let permits = Arc::new(Semaphore::new(p.cfg.concurrency.max(1) as usize));
    let mut tasks = JoinSet::new();
    let mut sent = 0;
    for _ in 0..p.cfg.abandoned {
        let permit = tokio::select! {
            _ = run.cancel.cancelled() => { tasks.abort_all(); return None; }
            permit = Arc::clone(&permits).acquire_owned() => permit.ok()?,
        };
        let Ok(req) = p.request.render() else { continue };
        let client = p.target.client.clone();
        sent += 1;
        tasks.spawn(async move {
            let o = client::execute(&client, &req, tight).await;
            drop(permit);
            o
        });
    }
    let mut answered = 0;
    while let Some(next) = tokio::select! {
        _ = run.cancel.cancelled() => { tasks.abort_all(); return None; }
        next = tasks.join_next() => next,
    } {
        if next.is_ok_and(|o| o.status.is_some()) {
            answered += 1;
        }
    }
    Some((sent, answered))
}

fn publish(run: &Run, started: Instant, latency: &Recorder, errors: u32) {
    if latency.len() == 0 {
        return;
    }
    run.publish(RunEvent::Bucket(Bucket {
        t_ms: started.elapsed().as_millis() as u32,
        requests: latency.len() as u32,
        errors,
        rps: 0.0,
        p50_ms: latency.percentile_ms(50.0),
        p99_ms: latency.percentile_ms(99.0),
        dropped: 0,
        in_flight: 0,
        lag_ms: 0.0,
    }));
}

fn finish(
    run: Arc<Run>,
    p: Prepared,
    t: Totals,
    status: RunStatus,
    mut notes: Vec<String>,
    started: Instant,
    result: Option<TimeoutResult>,
) {
    if p.target_info.loopback {
        notes.push("The target is on this machine, so it competes with the engine for CPU.".into());
    }
    let elapsed_s = started.elapsed().as_secs_f64();
    run.finish(RunReport {
        total_requests: t.all.len(),
        total_errors: t.total_errors,
        mean_rps: if elapsed_s > 0.0 { t.all.len() as f64 / elapsed_s } else { 0.0 },
        max_p99_ms: t.all.percentile_ms(99.0),
        target: Some(p.target_info),
        latency: t.all.summary(),
        latency_success: t.success.summary(),
        ttfb: t.ttfb.summary(),
        phases: t.phases.summary(p.target.dns),
        histogram: t.all.bins(),
        status_counts: t.statuses.into_iter().map(|(status, count)| StatusCount { status, count }).collect(),
        error_counts: t.errors,
        samples: t.samples,
        notes,
        timeout: result,
        ..RunReport::base(run.id, run.config.clone(), status, run.started_at_ms, now_ms())
    });
}

/// A probe after the burst is "normal" within 1.5× the baseline p99 plus 5 ms (loopback jitter).
fn normal_limit(before: &LatencySummary) -> f64 {
    before.p99_ms * 1.5 + 5.0
}

fn analyse(
    before: &Probes,
    after: Option<&Probes>,
    tight_ms: u32,
    abandoned: u32,
    answered_in_time: u32,
    timeout: Duration,
) -> TimeoutResult {
    let before_summary = before.latency.summary();
    let after_summary = after.and_then(|a| a.latency.summary());
    let hung = before.hung + after.map_or(0, |a| a.hung);
    let fast_failures = before.fast_failures + after.map_or(0, |a| a.fast_failures);

    // Recovery: the first probe from which every later one is normal.
    let recovered_after_ms = match (&before_summary, after) {
        (Some(b), Some(a)) => {
            let limit = normal_limit(b);
            let first_good_run =
                (0..a.times.len()).find(|&i| a.times[i..].iter().all(|(_, took)| took.as_secs_f64() * 1000.0 <= limit));
            match first_good_run {
                Some(0) | None => None,
                Some(i) => Some(a.times[i - 1].0.as_secs_f64() * 1000.0),
            }
        }
        _ => None,
    };

    let mut findings = Vec::new();
    let mut add = |level, message: String| findings.push(Finding { level, message });
    let timeout_s = timeout.as_secs_f64();
    let probes = before.times.len() + after.map_or(0, |a| a.times.len());
    if hung > 0 {
        add(
            FindingLevel::Bad,
            format!(
                "{hung} of {probes} probes got no answer before the {timeout_s} s timeout. The server doesn't give \
                 up on its own, so a slow dependency holds connections open; answering 503 or 504 after a set \
                 time fails faster."
            ),
        );
    } else if fast_failures > 0 {
        add(
            FindingLevel::Good,
            format!(
                "{fast_failures} probe{} failed, but quickly (well inside the timeout): the server fails fast \
                 rather than hanging.",
                if fast_failures == 1 { "" } else { "s" }
            ),
        );
    }

    if answered_in_time == abandoned && abandoned > 0 {
        add(
            FindingLevel::Info,
            format!(
                "Every burst request was answered within {tight_ms} ms, so none were abandoned and the burst \
                 tested nothing. Set a tighter burst timeout."
            ),
        );
    } else if let (Some(b), Some(a)) = (&before_summary, &after_summary) {
        let gave_up = abandoned - answered_in_time;
        let slower = a.p50_ms > b.p50_ms * 1.5 + 5.0 || a.p99_ms > normal_limit(b);
        if slower {
            let recovery = match recovered_after_ms {
                Some(ms) => format!(" and took {:.1} s to recover", ms / 1000.0),
                None => " and hadn't recovered by the last probe".into(),
            };
            add(
                FindingLevel::Warn,
                format!(
                    "After the client gave up on {gave_up} requests, later ones were slower (p50 {:.1} → {:.1} ms, \
                     p99 {:.1} → {:.1} ms){recovery}. The server likely keeps working on requests nobody is \
                     waiting for, so client timeouts and retries pile up load. Cancelling work when the client \
                     disconnects avoids that.",
                    b.p50_ms, a.p50_ms, b.p99_ms, a.p99_ms
                ),
            );
        } else {
            add(
                FindingLevel::Good,
                format!(
                    "Giving up on {gave_up} requests didn't slow the ones after (p50 {:.1} → {:.1} ms): the server \
                     cancels abandoned work, or had room to spare.",
                    b.p50_ms, a.p50_ms
                ),
            );
        }
    }
    findings.sort_by_key(|f| f.level);

    TimeoutResult {
        tight_ms,
        before: before_summary,
        after: after_summary,
        abandoned,
        answered_in_time,
        hung,
        fast_failures,
        recovered_after_ms,
        findings,
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, http::StatusCode, routing::get};

    use super::*;
    use crate::{
        engine::{registry::RunRegistry, types::RunConfig},
        model::{Endpoint, HttpMethod, Workspace},
    };

    /// One worker, 20 ms a job. `/queue` hands the job to a background task, so it runs even after
    /// the client leaves; `/cancellable` does it in the handler, which is dropped on disconnect.
    /// `/hang` never answers in time; `/unavailable` refuses at once.
    async fn server() -> String {
        let queue = Arc::new(Semaphore::new(1));
        let cancellable = Arc::new(Semaphore::new(1));
        let app = Router::new()
            .route(
                "/queue",
                get(move || {
                    let queue = Arc::clone(&queue);
                    async move {
                        let job = tokio::spawn(async move {
                            let _worker = queue.acquire().await.unwrap();
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        });
                        job.await.unwrap();
                        "done"
                    }
                }),
            )
            .route(
                "/cancellable",
                get(move || {
                    let cancellable = Arc::clone(&cancellable);
                    async move {
                        let _worker = cancellable.acquire().await.unwrap();
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        "done"
                    }
                }),
            )
            .route(
                "/hang",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    "late"
                }),
            )
            .route("/unavailable", get(|| async { (StatusCode::SERVICE_UNAVAILABLE, "busy") }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn probe(url: String, samples: u32, abandoned: u32, timeout_ms: u32) -> RunReport {
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
        let cfg = TimeoutConfig {
            endpoint_id: uuid::Uuid::nil(),
            environment: None,
            samples,
            abandoned,
            concurrency: 20,
            // Long enough to reach the server, far shorter than a 20 ms job.
            tight_ms: 10,
            timeout_ms,
        };
        let Ok(prepared) = prepare(cfg.clone(), request, &[], Duration::from_secs(60)).await else {
            panic!("prepare failed")
        };
        let registry = Arc::new(RunRegistry::default());
        let r = registry.create(uuid::Uuid::nil(), RunConfig::Timeout(cfg));
        run(Arc::clone(&r), prepared).await;
        r.report().unwrap()
    }

    fn first(report: &RunReport) -> &Finding {
        &report.timeout.as_ref().unwrap().findings[0]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn work_that_outlives_the_client_slows_what_comes_after() {
        let report = probe(format!("{}/queue", server().await), 5, 50, 5_000).await;
        let t = report.timeout.as_ref().unwrap();
        assert_eq!(t.answered_in_time, 0, "{t:?}");
        assert_eq!(first(&report).level, FindingLevel::Warn, "{:?}", t.findings);
        // The probes after queue behind abandoned jobs.
        let (before, after) = (t.before.as_ref().unwrap(), t.after.as_ref().unwrap());
        assert!(after.max_ms > before.max_ms * 2.0, "{t:?}");
        assert!(t.recovered_after_ms.is_some(), "{t:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancelled_work_does_not() {
        let report = probe(format!("{}/cancellable", server().await), 5, 50, 5_000).await;
        let t = report.timeout.as_ref().unwrap();
        assert_eq!(first(&report).level, FindingLevel::Good, "{:?}", t.findings);
        assert!(t.after.as_ref().unwrap().max_ms < 200.0, "{t:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hanging_is_bad_and_failing_fast_is_good() {
        let hang = probe(format!("{}/hang", server().await), 2, 1, 300).await;
        assert_eq!(hang.timeout.as_ref().unwrap().hung, 4);
        assert_eq!(first(&hang).level, FindingLevel::Bad);

        let busy = probe(format!("{}/unavailable", server().await), 3, 5, 5_000).await;
        let t = busy.timeout.as_ref().unwrap();
        assert_eq!((t.hung, t.fast_failures), (0, 6), "{t:?}");
        assert!(t.findings.iter().any(|f| f.level == FindingLevel::Good && f.message.contains("fails fast")));
    }
}
