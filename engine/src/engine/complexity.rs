//! Big-O: how does latency grow with input size? HANDOFF → Test catalogue → Big-O.
//!
//! Sizes are sampled in shuffled rounds (one request per size per round), not one size at a time
//! from small to large, so warm-up, GC and cache effects don't line up with n. Every request gets
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
    Ok(Prepared { cfg, request, target, target_info, sizes, budget })
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
        })
    }
}

pub async fn run(run: Arc<Run>, p: Prepared) {
    run.publish(RunEvent::Started(StartedEvent {
        run_id: run.id,
        config: run.config.clone(),
        started_at_ms: run.started_at_ms,
    }));

    let timeout = Duration::from_millis(p.cfg.timeout_ms.into());
    let slow = Duration::from_millis(p.cfg.slow_ms.into()).as_secs_f64() * 1000.0;
    let started = Instant::now();
    let mut data: BTreeMap<u64, SizeData> = p.sizes.iter().map(|&n| (n, SizeData::default())).collect();
    let mut active: Vec<u64> = p.sizes.clone();
    let mut notes = Vec::new();
    let mut statuses: BTreeMap<u16, u64> = BTreeMap::new();
    let mut errors = ErrorCounts::default();
    let (mut total, mut total_errors) = (0u64, 0u64);
    let mut samples: Vec<Sample> = Vec::new();
    let (mut have_success_sample, mut error_samples) = (false, 0);
    let mut last_progress = Instant::now();
    let mut status = RunStatus::Completed;

    // Warm up once, at the middle size.
    let mid = p.sizes[p.sizes.len() / 2];
    for _ in 0..p.cfg.warmup {
        let Ok(req) = p.request.render_sized(mid) else { break };
        tokio::select! {
            _ = run.cancel.cancelled() => { status = RunStatus::Cancelled; break; }
            _ = client::execute(&p.target.client, &req, timeout) => {}
        }
    }

    let rounds = p.cfg.samples;
    let mut round = 0;
    'rounds: while round < rounds && status == RunStatus::Completed {
        let mut order = active.clone();
        order.shuffle(&mut rand::rng());
        for n in order {
            // Dropped earlier in this round (too slow or failing).
            if !active.contains(&n) {
                continue;
            }
            if started.elapsed() >= p.budget {
                notes.push(format!(
                    "Stopped at the {} s time budget after {round} of {rounds} rounds.",
                    p.budget.as_secs()
                ));
                break 'rounds;
            }
            let req = match p.request.render_sized(n) {
                Ok(r) => r,
                Err(msg) => {
                    notes.push(format!("Couldn't render the request at n = {n}: {msg}"));
                    active.retain(|&m| m < n);
                    continue;
                }
            };
            let outcome = tokio::select! {
                _ = run.cancel.cancelled() => { status = RunStatus::Cancelled; break 'rounds; }
                o = client::execute(&p.target.client, &req, timeout) => o.classify_with(&p.cfg.ok_statuses),
            };

            total += 1;
            let d = data.get_mut(&n).expect("size is tracked");
            d.tries += 1;
            if let Some(code) = outcome.status {
                *statuses.entry(code).or_default() += 1;
            }
            match &outcome.error {
                None => {
                    d.latencies_ms.push(outcome.total.as_secs_f64() * 1000.0);
                    d.request_bytes.push(req.body.as_ref().map_or(0, |b| b.len() as u64));
                    d.response_bytes.push(outcome.body.len() as u64);
                }
                Some((class, _)) => {
                    d.errors += 1;
                    errors.add(*class);
                    total_errors += 1;
                }
            }
            let want_sample =
                if outcome.is_success() { !have_success_sample } else { error_samples < MAX_ERROR_SAMPLES };
            if want_sample {
                let redactor = Redactor::new(p.request.api_key_header.as_deref(), &p.request.secret_values, Some(&req));
                samples.push(sample::build(&req, &outcome, &redactor, SAMPLE_BODY_BYTES));
                if outcome.is_success() { have_success_sample = true } else { error_samples += 1 }
            }

            // Stop increasing n once a size fails too often.
            if d.tries as usize >= MIN_TRIES_BEFORE_DROPPING
                && f64::from(d.errors) / f64::from(d.tries) > MAX_ERROR_RATE
            {
                let reason = outcome.error.as_ref().map(|(_, m)| m.clone()).unwrap_or_default();
                notes.push(format!("Stopped increasing n at {n}: most requests failed ({reason})."));
                active.retain(|&m| m < n);
            }

            // Stop increasing n as soon as a size is slower than the per-request limit, rather than
            // waiting for more samples of something that takes seconds per request.
            if data[&n].point(n).is_some_and(|pt| pt.median_ms > slow) && active.iter().any(|&m| m >= n) {
                notes.push(format!(
                    "Stopped increasing n at {n}: its median exceeded the {} ms per-request limit.",
                    p.cfg.slow_ms
                ));
                active.retain(|&m| m < n);
            }

            if last_progress.elapsed() >= BUCKET_INTERVAL {
                publish_progress(&run, &data, round, rounds);
                last_progress = Instant::now();
            }
        }

        if active.is_empty() {
            break;
        }
        round += 1;
    }
    publish_progress(&run, &data, round, rounds);

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
        complexity: Some(ComplexityResult { points, analysis }),
        ..RunReport::base(run.id, run.config.clone(), status, run.started_at_ms, now_ms())
    });
}

fn publish_progress(run: &Run, data: &BTreeMap<u64, SizeData>, round: u32, rounds: u32) {
    let points = data.iter().filter_map(|(&n, d)| d.point(n)).collect();
    run.publish(RunEvent::Complexity(ComplexityProgress { round, rounds, points }));
}

/// Linear-interpolated quantile of sorted values.
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
    fn quantiles_interpolate() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(quantile(&v, 0.5), 2.5);
        assert_eq!(quantile(&v, 0.0), 1.0);
        assert_eq!(quantile(&[7.0], 0.75), 7.0);
    }
}
