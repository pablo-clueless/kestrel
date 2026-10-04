//! Concurrency correctness: the same request released `requests` times at once, for `rounds`
//! rounds. HANDOFF → Test catalogue → v1.x → Concurrency correctness.
//!
//! Each round renders the request once and sends that exact request from every task, all waiting
//! on one barrier, so they reach the server together. What comes back is the point: one success
//! and the rest refused (409 and friends) means the server serialises the write; several successes
//! on a create mean it doesn't; 5xx means it falls over. `findings` reads the rounds that way,
//! taking the method into account (several successful PUTs are normal; several POSTs may not be).

use std::{
    collections::{BTreeMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use tokio::{sync::Barrier, task::JoinSet, time::Instant};

use super::{
    client::{self, ClientOptions, Outcome, Target},
    load::PrepareError,
    phases::Phases,
    registry::{Run, now_ms},
    sample,
    types::{
        Bucket, ConcurrencyConfig, ConcurrencyResult, ConcurrencyRound, ErrorCounts, Finding, FindingLevel, RunEvent,
        RunReport, RunStatus, Sample, StartedEvent, StatusCount, TargetInfo,
    },
};
use crate::{model::HttpMethod, redact::Redactor, stats::Recorder, template::request::CompiledRequest};

/// Most requests in one burst, whatever the in-flight cap: past this, task start-up spreads the
/// burst out more than it adds to the race.
pub const MAX_BURST: u32 = 1_000;
/// Most rounds in one run.
pub const MAX_ROUNDS: u32 = 50;
const SAMPLE_BODY_BYTES: usize = 8 * 1024;
/// One sample per status code, up to this many codes.
const MAX_SAMPLES: usize = 8;

pub struct Prepared {
    pub cfg: ConcurrencyConfig,
    pub request: CompiledRequest,
    pub target: Target,
    pub target_info: TargetInfo,
    pub max_duration: Duration,
}

/// Like a load test, a burst of writes needs the host confirmed unless it's this machine.
pub async fn prepare(
    cfg: ConcurrencyConfig,
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

/// One request of a round: when it was sent and answered, from the round's release.
struct Shot {
    sent: Duration,
    done: Duration,
    outcome: Outcome,
}

pub async fn run(run: Arc<Run>, p: Prepared) {
    run.publish(RunEvent::Started(StartedEvent {
        run_id: run.id,
        config: run.config.clone(),
        started_at_ms: run.started_at_ms,
    }));

    let timeout = Duration::from_millis(p.cfg.timeout_ms.into());
    let n = p.cfg.requests.max(1) as usize;
    let mut all = Recorder::new(timeout);
    let mut success = Recorder::new(timeout);
    let mut ttfb = Recorder::new(timeout);
    let mut phases = Phases::new(timeout);
    let mut statuses: BTreeMap<u16, u64> = BTreeMap::new();
    let mut errors = ErrorCounts::default();
    let (mut total_errors, mut max_p99_ms) = (0u64, 0f64);
    let mut samples: Vec<Sample> = Vec::new();
    let mut sampled: HashSet<Option<u16>> = HashSet::new();
    let mut rounds = Vec::new();
    let mut notes = Vec::new();
    let mut status = RunStatus::Completed;
    let started = Instant::now();

    'rounds: for round in 1..=p.cfg.rounds {
        if started.elapsed() >= p.max_duration {
            notes.push(format!(
                "Stopped at the {} s duration cap after {} of {} rounds.",
                p.max_duration.as_secs(),
                rounds.len(),
                p.cfg.rounds
            ));
            break;
        }
        if round > 1 && p.cfg.pause_ms > 0 {
            tokio::select! {
                _ = run.cancel.cancelled() => { status = RunStatus::Cancelled; break; }
                _ = tokio::time::sleep(Duration::from_millis(p.cfg.pause_ms.into())) => {}
            }
        }
        let req = match p.request.render() {
            Ok(req) => Arc::new(req),
            Err(msg) => {
                notes.push(format!("Round {round} couldn't be rendered: {msg}"));
                continue;
            }
        };

        // Every task parks on the barrier; the last arrival (this task) releases them together.
        let barrier = Arc::new(Barrier::new(n + 1));
        let release: Arc<std::sync::OnceLock<Instant>> = Arc::default();
        let mut tasks = JoinSet::new();
        for _ in 0..n {
            let (client, req, barrier, release) =
                (p.target.client.clone(), Arc::clone(&req), Arc::clone(&barrier), Arc::clone(&release));
            tasks.spawn(async move {
                barrier.wait().await;
                let t0 = *release.get_or_init(Instant::now);
                let sent = t0.elapsed();
                let outcome = client::execute(&client, &req, timeout).await;
                Shot { sent, done: t0.elapsed(), outcome }
            });
        }
        barrier.wait().await;
        release.get_or_init(Instant::now);

        let mut shots = Vec::with_capacity(n);
        loop {
            tokio::select! {
                _ = run.cancel.cancelled() => {
                    tasks.abort_all();
                    status = RunStatus::Cancelled;
                    break 'rounds;
                }
                next = tasks.join_next() => match next {
                    Some(Ok(shot)) => shots.push(shot),
                    Some(Err(_)) => {}
                    None => break,
                },
            }
        }

        let mut window = Recorder::new(timeout);
        for shot in &shots {
            let o = &shot.outcome;
            all.record(o.total);
            window.record(o.total);
            ttfb.record(o.ttfb);
            phases.record(o.into());
            if let Some(code) = o.status {
                *statuses.entry(code).or_default() += 1;
            }
            if let Some((class, _)) = &o.error {
                errors.add(*class);
            }
            if is_error(o) {
                total_errors += 1;
            } else {
                success.record(o.total);
            }
            if samples.len() < MAX_SAMPLES && sampled.insert(o.status) {
                let redactor = Redactor::new(p.request.api_key_header.as_deref(), &p.request.secret_values, Some(&req));
                samples.push(sample::build(&req, o, &redactor, SAMPLE_BODY_BYTES));
            }
        }
        let summary = summarize(round, &shots);
        let bucket = Bucket {
            t_ms: started.elapsed().as_millis() as u32,
            requests: shots.len() as u32,
            errors: shots.iter().filter(|s| is_error(&s.outcome)).count() as u32,
            rps: 0.0,
            p50_ms: window.percentile_ms(50.0),
            p99_ms: window.percentile_ms(99.0),
            dropped: 0,
            in_flight: 0,
            lag_ms: 0.0,
        };
        max_p99_ms = max_p99_ms.max(bucket.p99_ms);
        run.publish(RunEvent::Bucket(bucket));
        rounds.push(summary);
    }

    if p.target_info.loopback {
        notes.push("The target is on this machine, so it competes with the engine for CPU.".into());
    }
    let elapsed_s = started.elapsed().as_secs_f64();
    let findings = findings(p.request.method, n as u32, &rounds);
    run.finish(RunReport {
        total_requests: all.len(),
        total_errors,
        mean_rps: if elapsed_s > 0.0 { all.len() as f64 / elapsed_s } else { 0.0 },
        max_p99_ms,
        target: Some(p.target_info),
        latency: all.summary(),
        latency_success: success.summary(),
        ttfb: ttfb.summary(),
        phases: phases.summary(p.target.dns),
        histogram: all.bins(),
        status_counts: statuses.into_iter().map(|(status, count)| StatusCount { status, count }).collect(),
        error_counts: errors,
        samples,
        notes,
        concurrency: Some(ConcurrencyResult { rounds, findings }),
        ..RunReport::base(run.id, run.config.clone(), status, run.started_at_ms, now_ms())
    });
}

/// Here, an error is the server failing (5xx) or no answer at all. A 409 or 422 is the server
/// refusing a duplicate, which is often the right answer.
fn is_error(o: &Outcome) -> bool {
    o.status.is_none_or(|s| s >= 500)
}

fn summarize(round: u32, shots: &[Shot]) -> ConcurrencyRound {
    let mut statuses: BTreeMap<u16, u64> = BTreeMap::new();
    let mut bodies = HashSet::new();
    let (mut failed, mut succeeded) = (0, 0);
    for shot in shots {
        match shot.outcome.status {
            None => failed += 1,
            Some(code) => {
                *statuses.entry(code).or_default() += 1;
                if (200..300).contains(&code) {
                    succeeded += 1;
                    let mut h = DefaultHasher::new();
                    shot.outcome.body.hash(&mut h);
                    bodies.insert(h.finish());
                }
            }
        }
    }
    let last_sent = shots.iter().map(|s| s.sent).max().unwrap_or_default();
    let first_done = shots.iter().map(|s| s.done).min().unwrap_or_default();
    let last_done = shots.iter().map(|s| s.done).max().unwrap_or_default();
    ConcurrencyRound {
        round,
        statuses: statuses.into_iter().map(|(status, count)| StatusCount { status, count }).collect(),
        failed,
        succeeded,
        distinct_bodies: bodies.len() as u32,
        overlap_ms: first_done.saturating_sub(last_sent).as_secs_f64() * 1000.0,
        spread_ms: (last_done - first_done).as_secs_f64() * 1000.0,
    }
}

/// Statuses that mean "refused because of another request": conflicts, failed preconditions,
/// validation (e.g. "already exists"), locks, rate limits; and 404/410 for a resource already gone.
const REFUSALS: &[u16] = &[404, 409, 410, 412, 422, 423, 428, 429];

fn codes(rounds: &[ConcurrencyRound], keep: impl Fn(u16) -> bool) -> String {
    let set: std::collections::BTreeSet<u16> =
        rounds.iter().flat_map(|r| &r.statuses).map(|s| s.status).filter(|s| keep(*s)).collect();
    set.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
}

/// Reads the rounds: worst first, then what went right, then context.
pub fn findings(method: HttpMethod, n: u32, rounds: &[ConcurrencyRound]) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut add = |level, message: String| out.push(Finding { level, message });
    let total = rounds.len();
    if total == 0 {
        return out;
    }
    let count = |f: &dyn Fn(&ConcurrencyRound) -> bool| rounds.iter().filter(|r| f(r)).count();
    let of = |k: usize| if total == 1 { String::new() } else { format!(" in {k} of {total} rounds") };

    let server_errors: u64 = rounds.iter().flat_map(|r| &r.statuses).filter(|s| s.status >= 500).map(|s| s.count).sum();
    if server_errors > 0 {
        let k = count(&|r| r.statuses.iter().any(|s| s.status >= 500));
        add(
            FindingLevel::Bad,
            format!(
                "{server_errors} request{} got a server error ({}){}. Simultaneous identical requests make the \
                 server fail: look for unhandled unique-constraint violations, deadlocks or lock timeouts.",
                if server_errors == 1 { "" } else { "s" },
                codes(rounds, |s| s >= 500),
                of(k)
            ),
        );
    }
    let failed: u32 = rounds.iter().map(|r| r.failed).sum();
    if failed > 0 {
        add(
            FindingLevel::Warn,
            format!(
                "{failed} request{} got no response (timeouts or connection errors).",
                if failed == 1 { "" } else { "s" }
            ),
        );
    }

    let multi = count(&|r| r.succeeded > 1);
    let most = rounds.iter().map(|r| r.succeeded).max().unwrap_or(0);
    let differing = count(&|r| r.succeeded > 1 && r.distinct_bodies > 1);
    let refused_only = |r: &ConcurrencyRound| {
        r.failed == 0 && r.statuses.iter().all(|s| (200..300).contains(&s.status) || REFUSALS.contains(&s.status))
    };
    let one_through = count(&|r| r.succeeded == 1 && refused_only(r));
    let none_through = count(&|r| r.succeeded == 0 && refused_only(r));
    let refusals = codes(rounds, |s| REFUSALS.contains(&s));

    match method {
        HttpMethod::Post => {
            if multi > 0 {
                let bodies = if differing > 0 {
                    " The successful responses differed, so separate records were probably created."
                } else {
                    " The successful responses were identical, which is what an idempotent create looks like."
                };
                add(
                    FindingLevel::Warn,
                    format!(
                        "More than one identical request succeeded{} (up to {most} of {n}). If this should \
                         happen once (a unique field, an idempotency key, a payment), the server isn't \
                         enforcing it.{bodies}",
                        of(multi)
                    ),
                );
            } else if one_through > 0 && one_through + none_through == total {
                add(
                    FindingLevel::Good,
                    format!(
                        "Exactly one request succeeded{} and the rest were refused ({refusals}): the server \
                         lets only one of the simultaneous creates through.",
                        if total == 1 { String::new() } else { format!(" in {one_through} of {total} rounds") }
                    ),
                );
                if none_through > 0 {
                    add(
                        FindingLevel::Info,
                        format!(
                            "In {none_through} round{} every request was refused, probably because an earlier \
                             round already made the record. Put {{{{uuid}}}} or {{{{seq}}}} in the unique \
                             field so each round is a fresh attempt.",
                            if none_through == 1 { "" } else { "s" }
                        ),
                    );
                }
            }
        }
        HttpMethod::Put | HttpMethod::Patch => {
            let conflicts = codes(rounds, |s| matches!(s, 409 | 412 | 428));
            if !conflicts.is_empty() && rounds.iter().any(|r| r.succeeded > 0) {
                add(
                    FindingLevel::Good,
                    format!("Some updates were refused ({conflicts}): the server detects conflicting writes."),
                );
            } else if multi > 0 {
                add(
                    FindingLevel::Info,
                    "Every update succeeded, as expected for PUT and PATCH, unless you send a version \
                     (If-Match with an ETag) and expect conflicts to be refused."
                        .into(),
                );
            }
            if differing > 0 {
                add(
                    FindingLevel::Info,
                    format!(
                        "Successful responses differed within a round{}, so the requests didn't all see the \
                         same final state.",
                        of(differing)
                    ),
                );
            }
        }
        HttpMethod::Delete => {
            if multi > 0 {
                add(
                    FindingLevel::Info,
                    format!(
                        "More than one delete reported success{} (up to {most} of {n}). Fine if deletes are \
                         idempotent; otherwise the server doesn't notice the resource is already gone.",
                        of(multi)
                    ),
                );
            } else if one_through > 0 && one_through + none_through == total {
                add(
                    FindingLevel::Good,
                    format!("One delete succeeded and the rest were told it was gone or locked ({refusals})."),
                );
            }
        }
        HttpMethod::Get | HttpMethod::Head | HttpMethod::Options => add(
            FindingLevel::Info,
            "This is a read. The test is meant for writes (POST, PUT, PATCH, DELETE), but it still shows \
             whether simultaneous reads fail."
                .into(),
        ),
    }

    let apart = count(&|r| r.overlap_ms <= 0.0);
    if apart > 0 {
        add(
            FindingLevel::Warn,
            format!(
                "The requests didn't all overlap{}: some were answered before others were sent, so the race \
                 may not have happened. Fewer requests, or a slower endpoint, overlap better.",
                of(apart)
            ),
        );
    }
    if total > 1 {
        let shapes: HashSet<Vec<(u16, u64)>> =
            rounds.iter().map(|r| r.statuses.iter().map(|s| (s.status, s.count)).collect()).collect();
        if shapes.len() > 1 {
            add(
                FindingLevel::Info,
                "Rounds ended differently, so the outcome depends on timing, which is the mark of a race.".into(),
            );
        }
    }
    if !out.iter().any(|f| f.level != FindingLevel::Info) {
        let message = format!("No problems seen in {total} round{}.", if total == 1 { "" } else { "s" });
        out.push(Finding { level: FindingLevel::Info, message });
    }
    out.sort_by_key(|f| f.level);
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::{Router, http::StatusCode, routing::post};

    use super::*;
    use crate::{
        engine::{registry::RunRegistry, types::RunConfig},
        model::{Body, Endpoint, Workspace},
    };

    /// `/racy`: checks for the name, waits, then inserts, so simultaneous requests all get in.
    /// `/locked`: the same, under a lock, so one gets in and the rest get 409.
    /// `/crash`: 500 for everything after the first.
    async fn server() -> String {
        let racy: Arc<Mutex<HashSet<String>>> = Arc::default();
        let locked: Arc<tokio::sync::Mutex<HashSet<String>>> = Arc::default();
        let crashed: Arc<Mutex<u32>> = Arc::default();
        let app = Router::new()
            .route(
                "/racy",
                post(move |body: String| {
                    let racy = Arc::clone(&racy);
                    async move {
                        let exists = racy.lock().unwrap().contains(&body);
                        tokio::time::sleep(Duration::from_millis(30)).await;
                        if exists {
                            return (StatusCode::CONFLICT, "exists".to_owned());
                        }
                        racy.lock().unwrap().insert(body);
                        (StatusCode::CREATED, uuid::Uuid::new_v4().to_string())
                    }
                }),
            )
            .route(
                "/locked",
                post(move |body: String| {
                    let locked = Arc::clone(&locked);
                    async move {
                        let mut names = locked.lock().await;
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        if !names.insert(body) {
                            return (StatusCode::CONFLICT, "exists".to_owned());
                        }
                        (StatusCode::CREATED, uuid::Uuid::new_v4().to_string())
                    }
                }),
            )
            .route(
                "/crash",
                post(move || {
                    let crashed = Arc::clone(&crashed);
                    async move {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        let mut n = crashed.lock().unwrap();
                        *n += 1;
                        if *n == 1 { StatusCode::CREATED } else { StatusCode::INTERNAL_SERVER_ERROR }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn fire(url: String, body: &str, requests: u32, rounds: u32) -> RunReport {
        let endpoint = Endpoint {
            id: uuid::Uuid::nil(),
            name: String::new(),
            group: None,
            method: HttpMethod::Post,
            url,
            headers: vec![],
            query: vec![],
            body: Body::Raw { content_type: "text/plain".into(), content: body.into() },
            auth: Default::default(),
            expect: None,
            extract: vec![],
        };
        let request =
            CompiledRequest::compile(&endpoint, &Workspace::default(), &Default::default(), None, false).unwrap();
        let cfg = ConcurrencyConfig {
            endpoint_id: uuid::Uuid::nil(),
            environment: None,
            requests,
            rounds,
            pause_ms: 0,
            timeout_ms: 5_000,
        };
        let Ok(prepared) = prepare(cfg.clone(), request, &[], Duration::from_secs(60)).await else {
            panic!("prepare failed")
        };
        let registry = Arc::new(RunRegistry::default());
        let r = registry.create(uuid::Uuid::nil(), RunConfig::Concurrency(cfg));
        run(Arc::clone(&r), prepared).await;
        r.report().unwrap()
    }

    fn levels(report: &RunReport) -> Vec<FindingLevel> {
        report.concurrency.as_ref().unwrap().findings.iter().map(|f| f.level).collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_racy_create_lets_duplicates_through() {
        let report = fire(format!("{}/racy", server().await), "ada", 8, 1).await;
        let c = report.concurrency.as_ref().unwrap();
        let round = &c.rounds[0];
        assert_eq!(round.succeeded, 8, "{round:?}");
        assert_eq!(round.distinct_bodies, 8, "each got its own id");
        assert!(round.overlap_ms > 0.0, "they raced: {round:?}");
        assert_eq!(levels(&report)[0], FindingLevel::Warn, "{:?}", c.findings);
        assert!(c.findings[0].message.contains("separate records"), "{:?}", c.findings);
        assert_eq!(report.total_requests, 8);
        assert_eq!(report.total_errors, 0, "409s and 201s aren't errors here");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_locked_create_lets_one_through_and_later_rounds_are_explained() {
        let report = fire(format!("{}/locked", server().await), "ada", 6, 2).await;
        let c = report.concurrency.as_ref().unwrap();
        assert_eq!((c.rounds[0].succeeded, c.rounds[1].succeeded), (1, 0), "{:?}", c.rounds);
        assert_eq!(c.rounds[0].statuses.iter().map(|s| (s.status, s.count)).collect::<Vec<_>>(), [(201, 1), (409, 5)]);
        assert_eq!(levels(&report)[0], FindingLevel::Good, "{:?}", c.findings);
        assert!(c.findings.iter().any(|f| f.message.contains("{{uuid}}")), "{:?}", c.findings);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn server_errors_are_the_worst_finding() {
        let report = fire(format!("{}/crash", server().await), "x", 5, 1).await;
        let c = report.concurrency.as_ref().unwrap();
        assert_eq!(levels(&report)[0], FindingLevel::Bad, "{:?}", c.findings);
        assert!(c.findings[0].message.starts_with("4 requests got a server error (500)"), "{:?}", c.findings);
        assert_eq!(report.total_errors, 4);
    }

    #[test]
    fn reads_and_updates_are_read_differently() {
        let round = |statuses: &[(u16, u64)], succeeded, distinct_bodies| ConcurrencyRound {
            round: 1,
            statuses: statuses.iter().map(|&(status, count)| StatusCount { status, count }).collect(),
            failed: 0,
            succeeded,
            distinct_bodies,
            overlap_ms: 5.0,
            spread_ms: 1.0,
        };
        let get = findings(HttpMethod::Get, 4, &[round(&[(200, 4)], 4, 1)]);
        assert!(get[0].message.starts_with("This is a read"), "{get:?}");
        let put = findings(HttpMethod::Put, 4, &[round(&[(200, 1), (412, 3)], 1, 1)]);
        assert_eq!(put[0].level, FindingLevel::Good, "{put:?}");
        let delete = findings(HttpMethod::Delete, 3, &[round(&[(204, 1), (404, 2)], 1, 1)]);
        assert_eq!(delete[0].level, FindingLevel::Good, "{delete:?}");
        let apart = findings(
            HttpMethod::Post,
            2,
            &[ConcurrencyRound { overlap_ms: 0.0, ..round(&[(201, 1), (409, 1)], 1, 1) }],
        );
        assert!(apart.iter().any(|f| f.level == FindingLevel::Warn && f.message.contains("overlap")), "{apart:?}");
    }
}
