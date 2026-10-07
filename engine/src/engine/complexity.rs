//! Big-O: how does latency grow with input size? HANDOFF → Test catalogue → Big-O.
//!
//! Sizes are sampled in shuffled rounds (one request per size per round), not one size at a time
//! from small to large, so warm-up, GC and cache effects don't line up with n. With a setup or teardown
//! request (server state, e.g. `seed n rows`) they can't be interleaved, so sizes go one at a time, in
//! random order. Every request gets
//! fresh random contents from the size generators. Medians per size are fitted in `stats::fit`.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use rand::seq::SliceRandom;
use tokio::time::Instant;

use super::{
    BUCKET_INTERVAL,
    client::{self, ClientOptions, Target},
    registry::{Run, now_ms},
    sample,
    types::{
        ComplexityConfig, ComplexityPoint, ComplexityProgress, ComplexityResult, ErrorCounts, RunEvent, RunReport,
        RunStatus, Sample, StartedEvent, StatusCount, TargetInfo,
    },
};
use crate::{
    redact::Redactor,
    stats::fit::{self, SizeStats},
    template::request::CompiledRequest,
};

const MAX_ERROR_SAMPLES: usize = 5;
const SAMPLE_BODY_BYTES: usize = 4 * 1024;
/// A size is dropped once this share of its requests failed (after a few tries).
const MAX_ERROR_RATE: f64 = 0.5;
const MIN_TRIES_BEFORE_DROPPING: usize = 3;

pub struct Prepared {
    pub cfg: ComplexityConfig,
    pub request: CompiledRequest,
    pub target: Target,
    pub target_info: TargetInfo,
    pub sizes: Vec<u64>,
    pub budget: Duration,
    /// Payload scaling: also read the points as bytes (`payload.rs`).
    pub payload: bool,
    /// Where each sample's body is also sent, as a payload baseline. See [`Prepared::with_baseline`].
    pub baseline: Option<Baseline>,
    /// Sent before each size's samples, rendered with that n. See [`Prepared::with_state`].
    pub setup: Option<StateRequest>,
    /// Sent after each size's samples.
    pub teardown: Option<StateRequest>,
}

/// A setup or teardown request of a server-state sweep.
pub struct StateRequest {
    pub name: String,
    pub request: CompiledRequest,
}

/// Unmeasured requests at each size after its setup, so the first measured one doesn't pay for cold
/// caches the setup left behind.
const STATE_WARMUP: u32 = 2;

/// The echo endpoint of a payload baseline: same body and headers, its own method and URL.
pub struct Baseline {
    pub name: String,
    pub method: crate::model::HttpMethod,
    pub url: url::Url,
}

impl Prepared {
    /// Adds a payload baseline. It must be on the endpoint's own origin, since the run's client is
    /// pinned to that host's address.
    pub fn with_baseline(mut self, name: String, request: &CompiledRequest) -> Result<Self, String> {
        let echo = request.preview()?;
        let main = self.request.preview()?;
        if echo.url.origin() != main.url.origin() {
            return Err(format!(
                "the baseline `{name}` is on a different host; it must be on {}",
                main.url.origin().ascii_serialization()
            ));
        }
        self.baseline = Some(Baseline { name, method: echo.method, url: echo.url });
        Ok(self)
    }

    /// Adds a setup or teardown request. Like the baseline, it must be on the endpoint's origin.
    pub fn with_state(mut self, setup: Option<StateRequest>, teardown: Option<StateRequest>) -> Result<Self, String> {
        let main = self.request.preview()?;
        for (what, state) in [("setup", &setup), ("teardown", &teardown)] {
            let Some(state) = state else { continue };
            if state.request.preview()?.url.origin() != main.url.origin() {
                return Err(format!(
                    "the {what} request `{}` is on a different host; it must be on {}",
                    state.name,
                    main.url.origin().ascii_serialization()
                ));
            }
        }
        self.setup = setup;
        self.teardown = teardown;
        Ok(self)
    }
}

pub async fn prepare(
    cfg: ComplexityConfig,
    request: CompiledRequest,
    budget_cap: Duration,
    confirmed_hosts: Vec<String>,
) -> Result<Prepared, String> {
    if !request.uses_size() {
        return Err("mark the input size in the request with {{n}}, {{n:int_array}}, {{n:string}} or \
                    {{n:object_array}} (URL, query, headers or body)"
            .into());
    }
    let first = request.preview()?;
    let opts = ClientOptions { keep_alive: cfg.keep_alive, follow_redirects: true, confirmed_hosts };
    let target = client::connect(&first.url, &opts).await?;
    let target_info = TargetInfo {
        host: first.url.host_str().unwrap_or_default().to_owned(),
        pinned_ip: target.pinned.to_string(),
        loopback: target.pinned.is_loopback(),
    };
    let sizes = geometric_sizes(cfg.min_n, cfg.max_n, cfg.points);
    let budget = Duration::from_millis(cfg.budget_ms.into()).min(budget_cap);
    Ok(Prepared {
        cfg,
        request,
        target,
        target_info,
        sizes,
        budget,
        payload: false,
        baseline: None,
        setup: None,
        teardown: None,
    })
}

/// `points` sizes from `min` to `max`, evenly spaced on a log scale, rounded and deduplicated.
pub fn geometric_sizes(min: u32, max: u32, points: u32) -> Vec<u64> {
    let (min, max) = (min.max(1) as f64, max.max(min.max(1)) as f64);
    let steps = points.max(2) - 1;
    let mut sizes: Vec<u64> =
        (0..=steps).map(|i| (min * (max / min).powf(f64::from(i) / f64::from(steps))).round() as u64).collect();
    sizes.dedup();
    sizes
}

#[derive(Default)]
struct SizeData {
    latencies_ms: Vec<f64>,
    /// The same bodies sent to the baseline endpoint (successes only).
    baseline_ms: Vec<f64>,
    request_bytes: Vec<u64>,
    response_bytes: Vec<u64>,
    errors: u32,
    tries: u32,
}

impl SizeData {
    fn point(&self, n: u64) -> Option<ComplexityPoint> {
        let mut l = self.latencies_ms.clone();
        if l.is_empty() {
            return None;
        }
        l.sort_by(f64::total_cmp);
        Some(ComplexityPoint {
            n,
            median_ms: quantile(&l, 0.5),
            p25_ms: quantile(&l, 0.25),
            p75_ms: quantile(&l, 0.75),
            samples: l.len() as u32,
            errors: self.errors,
            request_bytes: median_u64(&self.request_bytes),
            response_bytes: median_u64(&self.response_bytes),
            baseline_median_ms: (!self.baseline_ms.is_empty()).then(|| {
                let mut b = self.baseline_ms.clone();
                b.sort_by(f64::total_cmp);
                quantile(&b, 0.5)
            }),
        })
    }
}

/// What the sweep should do after one measured request.
enum Flow {
    Next,
    Cancelled,
}

/// Everything a sweep accumulates, whichever order it samples sizes in.
struct Sweep<'a> {
    run: &'a Run,
    p: &'a Prepared,
    timeout: Duration,
    slow_ms: f64,
    data: BTreeMap<u64, SizeData>,
    /// Sizes still being sampled. Dropping a size drops every larger one too.
    active: Vec<u64>,
    notes: Vec<String>,
    statuses: BTreeMap<u16, u64>,
    errors: ErrorCounts,
    total: u64,
    total_errors: u64,
    samples: Vec<Sample>,
    have_success_sample: bool,
    error_samples: usize,
    last_progress: Instant,
    baseline_failures: u32,
}

impl Sweep<'_> {
    /// Stops sampling `n` and everything larger.
    fn drop_from(&mut self, n: u64, note: String) {
        self.notes.push(note);
        self.active.retain(|&m| m < n);
    }

    /// One measured request at size `n` (plus its baseline echo), with the bookkeeping that drops
    /// sizes that fail or are too slow.
    async fn measure(&mut self, n: u64, round: u32, rounds: u32) -> Flow {
        let p = self.p;
        let req = match p.request.render_sized(n) {
            Ok(r) => r,
            Err(msg) => {
                self.drop_from(n, format!("Couldn't render the request at n = {n}: {msg}"));
                return Flow::Next;
            }
        };
        let outcome = tokio::select! {
            _ = self.run.cancel.cancelled() => return Flow::Cancelled,
            o = client::execute(&p.target.client, &req, self.timeout) => o.classify_with(&p.cfg.ok_statuses),
        };

        self.total += 1;
        let d = self.data.get_mut(&n).expect("size is tracked");
        d.tries += 1;
        if let Some(code) = outcome.status {
            *self.statuses.entry(code).or_default() += 1;
        }
        match &outcome.error {
            None => {
                d.latencies_ms.push(outcome.total.as_secs_f64() * 1000.0);
                d.request_bytes.push(req.body.as_ref().map_or(0, |b| b.len() as u64));
                d.response_bytes.push(outcome.body.len() as u64);
            }
            Some((class, _)) => {
                d.errors += 1;
                self.errors.add(*class);
                self.total_errors += 1;
            }
        }
        // The same body to the echo endpoint, straight after, so both see the same conditions.
        if let (Some(base), None) = (&p.baseline, &outcome.error) {
            let mut echo = req.clone();
            echo.method = base.method;
            echo.url = base.url.clone();
            let echoed = tokio::select! {
                _ = self.run.cancel.cancelled() => return Flow::Cancelled,
                o = client::execute(&p.target.client, &echo, self.timeout) => o,
            };
            let d = self.data.get_mut(&n).expect("size is tracked");
            if echoed.is_success() {
                d.baseline_ms.push(echoed.total.as_secs_f64() * 1000.0);
            } else {
                self.baseline_failures += 1;
            }
        }

        let want_sample =
            if outcome.is_success() { !self.have_success_sample } else { self.error_samples < MAX_ERROR_SAMPLES };
        if want_sample {
            let redactor = Redactor::new(p.request.api_key_header.as_deref(), &p.request.secret_values, Some(&req));
            self.samples.push(sample::build(&req, &outcome, &redactor, SAMPLE_BODY_BYTES));
            if outcome.is_success() { self.have_success_sample = true } else { self.error_samples += 1 }
        }

        // Stop increasing n once a size fails too often.
        let d = &self.data[&n];
        if d.tries as usize >= MIN_TRIES_BEFORE_DROPPING && f64::from(d.errors) / f64::from(d.tries) > MAX_ERROR_RATE {
            let reason = outcome.error.as_ref().map(|(_, m)| m.clone()).unwrap_or_default();
            self.drop_from(n, format!("Stopped increasing n at {n}: most requests failed ({reason})."));
        }

        // Stop increasing n as soon as a size is slower than the per-request limit, rather than
        // waiting for more samples of something that takes seconds per request.
        if self.data[&n].point(n).is_some_and(|pt| pt.median_ms > self.slow_ms) && self.active.iter().any(|&m| m >= n) {
            let limit = p.cfg.slow_ms;
            self.drop_from(
                n,
                format!("Stopped increasing n at {n}: its median exceeded the {limit} ms per-request limit."),
            );
        }

        if self.last_progress.elapsed() >= BUCKET_INTERVAL {
            publish_progress(self.run, &self.data, round, rounds);
            self.last_progress = Instant::now();
        }
        Flow::Next
    }

    /// Sends a setup or teardown request at size `n`. `Err` with why it failed.
    async fn state(&self, state: &StateRequest, n: u64) -> Result<(), String> {
        let req = state.request.render_sized(n)?;
        let outcome = tokio::select! {
            _ = self.run.cancel.cancelled() => return Ok(()),
            o = client::execute(&self.p.target.client, &req, self.timeout) => o,
        };
        match (&outcome.error, outcome.status) {
            (Some((_, msg)), _) => Err(msg.clone()),
            (None, Some(code)) if !(200..300).contains(&code) => Err(format!("status {code}")),
            _ => Ok(()),
        }
    }
}

pub async fn run(run: Arc<Run>, p: Prepared) {
    run.publish(RunEvent::Started(StartedEvent {
        run_id: run.id,
        config: run.config.clone(),
        started_at_ms: run.started_at_ms,
    }));

    let started = Instant::now();
    let mut s = Sweep {
        run: &run,
        p: &p,
        timeout: Duration::from_millis(p.cfg.timeout_ms.into()),
        slow_ms: Duration::from_millis(p.cfg.slow_ms.into()).as_secs_f64() * 1000.0,
        data: p.sizes.iter().map(|&n| (n, SizeData::default())).collect(),
        active: p.sizes.clone(),
        notes: Vec::new(),
        statuses: BTreeMap::new(),
        errors: ErrorCounts::default(),
        total: 0,
        total_errors: 0,
        samples: Vec::new(),
        have_success_sample: false,
        error_samples: 0,
        last_progress: Instant::now(),
        baseline_failures: 0,
    };
    let mut status = RunStatus::Completed;
    let rounds = p.cfg.samples;
    let budget_note = |done: String| format!("Stopped at the {} s time budget after {done}.", p.budget.as_secs());

    if p.setup.is_some() || p.teardown.is_some() {
        // Server state: each size needs its own state in place while it's measured, so sizes can't
        // be interleaved. They're taken one at a time, in random order, so slow drift on the server
        // still doesn't line up with n.
        let mut order = p.sizes.clone();
        order.shuffle(&mut rand::rng());
        let mut done = 0;
        'sizes: for n in order {
            if !s.active.contains(&n) {
                continue;
            }
            if started.elapsed() >= p.budget {
                s.notes.push(budget_note(format!("{done} of {} sizes", p.sizes.len())));
                break;
            }
            if let Some(setup) = &p.setup
                && let Err(why) = s.state(setup, n).await
            {
                s.drop_from(
                    n,
                    format!("Stopped increasing n at {n}: the setup request `{}` failed ({why}).", setup.name),
                );
                continue;
            }
            for _ in 0..p.cfg.warmup.min(STATE_WARMUP) {
                let Ok(req) = p.request.render_sized(n) else { break };
                tokio::select! {
                    _ = run.cancel.cancelled() => { status = RunStatus::Cancelled; break 'sizes; }
                    _ = client::execute(&p.target.client, &req, s.timeout) => {}
                }
            }
            for _ in 0..rounds {
                if !s.active.contains(&n) || started.elapsed() >= p.budget {
                    break;
                }
                if let Flow::Cancelled = s.measure(n, done, p.sizes.len() as u32).await {
                    status = RunStatus::Cancelled;
                    break 'sizes;
                }
            }
            if let Some(teardown) = &p.teardown
                && let Err(why) = s.state(teardown, n).await
            {
                s.notes.push(format!(
                    "The teardown request `{}` failed at n = {n} ({why}); later sizes may have started from \
                     leftover state.",
                    teardown.name
                ));
            }
            done += 1;
        }
        let setup = p.setup.as_ref().map(|r| format!("`{}` before", r.name));
        let teardown = p.teardown.as_ref().map(|r| format!("`{}` after", r.name));
        s.notes.push(format!(
            "Server state: {} each size's samples, one size at a time in random order.",
            [setup, teardown].into_iter().flatten().collect::<Vec<_>>().join(" and ")
        ));
        publish_progress(&run, &s.data, done, p.sizes.len() as u32);
    } else {
        // Warm up once, at the middle size.
        let mid = p.sizes[p.sizes.len() / 2];
        for _ in 0..p.cfg.warmup {
            let Ok(req) = p.request.render_sized(mid) else { break };
            tokio::select! {
                _ = run.cancel.cancelled() => { status = RunStatus::Cancelled; break; }
                _ = client::execute(&p.target.client, &req, s.timeout) => {}
            }
        }

        let mut round = 0;
        'rounds: while round < rounds && status == RunStatus::Completed {
            let mut order = s.active.clone();
            order.shuffle(&mut rand::rng());
            for n in order {
                // Dropped earlier in this round (too slow or failing).
                if !s.active.contains(&n) {
                    continue;
                }
                if started.elapsed() >= p.budget {
                    s.notes.push(budget_note(format!("{round} of {rounds} rounds")));
                    break 'rounds;
                }
                if let Flow::Cancelled = s.measure(n, round, rounds).await {
                    status = RunStatus::Cancelled;
                    break 'rounds;
                }
            }
            if s.active.is_empty() {
                break;
            }
            round += 1;
        }
        publish_progress(&run, &s.data, round, rounds);
    }

    let Sweep { data, mut notes, statuses, errors, total, total_errors, samples, baseline_failures, .. } = s;
    let points: Vec<ComplexityPoint> = data.iter().filter_map(|(&n, d)| d.point(n)).collect();
    let stats: Vec<SizeStats> =
        points.iter().map(|p| SizeStats { n: p.n as f64, median: p.median_ms, p25: p.p25_ms, p75: p.p75_ms }).collect();
    let analysis = fit::analyse(&stats);

    if let (Some(first), Some(last)) = (points.first(), points.last())
        && last.request_bytes > 0
    {
        notes.push(format!(
            "Request bodies grew from {} to {} bytes, so part of the growth is transfer and parsing.",
            first.request_bytes, last.request_bytes
        ));
    }
    let payload = p.payload.then(|| super::payload::analyse(&points, &analysis, p.target_info.loopback));
    if let Some(base) = &p.baseline {
        notes.extend(baseline_note(&base.name, &points));
        if baseline_failures > 0 {
            notes.push(format!(
                "{baseline_failures} baseline requests to `{}` failed and aren't in its curve.",
                base.name
            ));
        }
    }
    if p.target_info.loopback {
        notes.push("The target is on this machine, so it competes with the engine for CPU.".into());
    }
    notes.push(
        "Measures server + network + serialisation, not the algorithm alone; results hold only for the n range \
         tested."
            .into(),
    );

    run.finish(RunReport {
        total_requests: total,
        total_errors,
        mean_rps: total as f64 / started.elapsed().as_secs_f64().max(1e-6),
        target: Some(p.target_info),
        status_counts: statuses.into_iter().map(|(status, count)| StatusCount { status, count }).collect(),
        error_counts: errors,
        samples,
        notes,
        complexity: Some(ComplexityResult { points, analysis, payload }),
        ..RunReport::base(run.id, run.config.clone(), status, run.started_at_ms, now_ms())
    });
}
fn publish_progress(run: &Run, data: &BTreeMap<u64, SizeData>, round: u32, rounds: u32) {
    let points = data.iter().filter_map(|(&n, d)| d.point(n)).collect();
    run.publish(RunEvent::Complexity(ComplexityProgress { round, rounds, points }));
}

/// Linear-interpolated quantile of sorted values.
/// How much of the endpoint's growth the echo endpoint shows too: that part is moving and parsing
/// the bytes, the rest is the endpoint's own work.
fn baseline_note(name: &str, points: &[ComplexityPoint]) -> Option<String> {
    let with: Vec<(&ComplexityPoint, f64)> =
        points.iter().filter_map(|p| p.baseline_median_ms.map(|b| (p, b))).collect();
    let (&(first, first_echo), &(last, last_echo)) = (with.first()?, with.last()?);
    if first.n == last.n {
        return None;
    }
    let grew = last.median_ms - first.median_ms;
    let echo_grew = (last_echo - first_echo).max(0.0);
    Some(if grew <= 0.0 {
        format!(
            "Payload baseline (`{name}`): it went from {first_echo:.1} to {last_echo:.1} ms from n = {} to n = {}, \
             while this endpoint didn't grow.",
            first.n, last.n
        )
    } else {
        let share = (echo_grew / grew * 100.0).clamp(0.0, 100.0);
        format!(
            "Payload baseline (`{name}`): from n = {} to n = {} this endpoint grew {grew:.1} ms and the echo grew \
             {echo_grew:.1} ms, so about {share:.0}% of the growth is moving and parsing the bytes; the rest is \
             the endpoint's own work.",
            first.n, last.n
        )
    })
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    let pos = q * (sorted.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

fn median_u64(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut v = values.to_vec();
    v.sort_unstable();
    v[v.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_geometric_and_deduplicated() {
        assert_eq!(geometric_sizes(1, 4096, 13), vec![1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096]);
        assert_eq!(geometric_sizes(1, 4, 10), vec![1, 2, 3, 4]);
        assert_eq!(geometric_sizes(10, 10, 5), vec![10]);
    }

    #[test]
    fn the_baseline_note_says_how_much_growth_is_transfer() {
        let point = |n, median_ms, echo| ComplexityPoint {
            n,
            median_ms,
            p25_ms: median_ms,
            p75_ms: median_ms,
            samples: 5,
            errors: 0,
            request_bytes: n * 10,
            response_bytes: 0,
            baseline_median_ms: Some(echo),
        };
        // The endpoint grows 40 ms, the echo 10 ms: a quarter is moving the bytes.
        let note = baseline_note("echo", &[point(1, 10.0, 2.0), point(1000, 50.0, 12.0)]).unwrap();
        assert!(note.contains("grew 40.0 ms") && note.contains("about 25%"), "{note}");
        assert!(baseline_note("echo", &[point(1, 10.0, 2.0)]).is_none(), "one size says nothing");
    }

    #[test]
    fn quantiles_interpolate() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(quantile(&v, 0.5), 2.5);
        assert_eq!(quantile(&v, 0.0), 1.0);
        assert_eq!(quantile(&[7.0], 0.75), 7.0);
    }
}
