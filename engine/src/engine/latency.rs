//! Latency probe: `warmup` requests, then `samples` sequential requests to one endpoint.
//! HANDOFF → Test catalogue → Latency probe.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use tokio::time::Instant;

use super::{
    BUCKET_INTERVAL,
    client::{self, ClientOptions, Outcome, Target},
    registry::{Run, now_ms},
    sample,
    types::{
        Bucket, ErrorCounts, LatencyConfig, RunEvent, RunReport, RunStatus, Sample, StartedEvent, StatusCount,
        TargetInfo,
    },
};
use crate::{
    contract::{Contract, ContractSummary},
    redact::Redactor,
    stats::Recorder,
    template::request::CompiledRequest,
};

const MAX_ERROR_SAMPLES: usize = 5;
const SAMPLE_BODY_BYTES: usize = 8 * 1024;

/// Everything checked before the run is created, so config errors are a 400, not a failed run.
pub struct Prepared {
    pub cfg: LatencyConfig,
    pub request: CompiledRequest,
    pub target: Target,
    pub target_info: TargetInfo,
    pub max_duration: Duration,
    /// Set by the caller for endpoints imported from a spec.
    pub contract: Option<Arc<Contract>>,
}

pub async fn prepare(
    cfg: LatencyConfig,
    request: CompiledRequest,
    max_duration: Duration,
    confirmed_hosts: Vec<String>,
) -> Result<Prepared, String> {
    let first = request.preview()?;
    let opts = ClientOptions { keep_alive: cfg.keep_alive, follow_redirects: true, confirmed_hosts };
    let target = client::connect(&first.url, &opts).await?;
    let target_info = TargetInfo {
        host: first.url.host_str().unwrap_or_default().to_owned(),
        pinned_ip: target.pinned.to_string(),
        loopback: target.pinned.is_loopback(),
    };
    Ok(Prepared { cfg, request, target, target_info, max_duration, contract: None })
}

pub async fn run(run: Arc<Run>, p: Prepared) {
    run.publish(RunEvent::Started(StartedEvent {
        run_id: run.id,
        config: run.config.clone(),
        started_at_ms: run.started_at_ms,
    }));

    let timeout = Duration::from_millis(p.cfg.timeout_ms.into());
    let total_planned = p.cfg.warmup + p.cfg.samples;
    let mut all = Recorder::new(timeout);
    let mut success = Recorder::new(timeout);
    let mut ttfb = Recorder::new(timeout);
    let mut window = Window::new(timeout);
    let mut statuses: BTreeMap<u16, u64> = BTreeMap::new();
    let mut errors = ErrorCounts::default();
    let (mut total_errors, mut max_p99_ms) = (0u64, 0f64);
    let mut cold_ms = None;
    let mut samples: Vec<Sample> = Vec::new();
    let (mut have_success_sample, mut error_samples) = (false, 0);
    let mut notes = Vec::new();
    let mut contract_summary = p.contract.as_ref().map(|_| ContractSummary::default());
    let run_redactor = Redactor::new(p.request.api_key_header.as_deref(), &p.request.secret_values, None);

    let started = Instant::now();
    let mut measured_since = started;
    let mut next_flush = started + BUCKET_INTERVAL;

    let mut status = RunStatus::Completed;
    for i in 0..total_planned {
        if started.elapsed() >= p.max_duration {
            notes.push(format!(
                "Stopped at the {} s duration cap after {} of {} measured requests.",
                p.max_duration.as_secs(),
                all.len(),
                p.cfg.samples
            ));
            break;
        }

        let (req, outcome) = match p.request.render() {
            Ok(req) => {
                let outcome = tokio::select! {
                    _ = run.cancel.cancelled() => { status = RunStatus::Cancelled; break; }
                    outcome = client::execute(&p.target.client, &req, timeout) => outcome.classify_with(&p.cfg.ok_statuses),
                };
                (req, outcome)
            }
            Err(msg) => {
                // Can't happen after `prepare` unless a generator produces an invalid URL.
                notes.push(format!("Request {i} couldn't be rendered: {msg}"));
                continue;
            }
        };

        if i == 0 {
            cold_ms = Some(outcome.total.as_secs_f64() * 1000.0);
        }
        if i < p.cfg.warmup {
            continue;
        }
        if i == p.cfg.warmup {
            measured_since = Instant::now() - outcome.total;
        }

        all.record(outcome.total);
        ttfb.record(outcome.ttfb);
        window.record(&outcome);
        if let Some(code) = outcome.status {
            *statuses.entry(code).or_default() += 1;
        }
        match &outcome.error {
            None => success.record(outcome.total),
            Some((class, _)) => {
                errors.add(*class);
                total_errors += 1;
            }
        }

        let check = match (&p.contract, outcome.status) {
            (Some(contract), Some(status)) => Some(contract.check(status, &outcome.body)),
            _ => None,
        };
        if let (Some(summary), Some(check)) = (contract_summary.as_mut(), &check) {
            summary.add(check, |m| run_redactor.text(m));
        }

        // Keep the first clean response, and the first few errors or contract violations.
        let clean = outcome.is_success() && check.as_ref().is_none_or(|c| c.passed);
        let want_sample = if clean { !have_success_sample } else { error_samples < MAX_ERROR_SAMPLES };
        if want_sample {
            let redactor = Redactor::new(p.request.api_key_header.as_deref(), &p.request.secret_values, Some(&req));
            let mut s = sample::build(&req, &outcome, &redactor, SAMPLE_BODY_BYTES);
            s.contract = check.map(|c| crate::contract::ContractCheck { message: redactor.text(&c.message), ..c });
            samples.push(s);
            if clean { have_success_sample = true } else { error_samples += 1 }
        }

        if Instant::now() >= next_flush {
            if let Some(bucket) = window.flush(started.elapsed()) {
                max_p99_ms = max_p99_ms.max(bucket.p99_ms);
                run.publish(RunEvent::Bucket(bucket));
            }
            while next_flush <= Instant::now() {
                next_flush += BUCKET_INTERVAL;
            }
        }
    }
    if let Some(bucket) = window.flush(started.elapsed()) {
        max_p99_ms = max_p99_ms.max(bucket.p99_ms);
        run.publish(RunEvent::Bucket(bucket));
    }

    if p.target_info.loopback {
        notes.push("The target is on this machine, so it competes with the engine for CPU.".into());
    }

    let measured_s = measured_since.elapsed().as_secs_f64();
    run.finish(RunReport {
        total_requests: all.len(),
        total_errors,
        mean_rps: if measured_s > 0.0 { all.len() as f64 / measured_s } else { 0.0 },
        max_p99_ms,
        target: Some(p.target_info),
        latency: all.summary(),
        latency_success: success.summary(),
        ttfb: ttfb.summary(),
        histogram: all.bins(),
        cold_ms,
        status_counts: statuses.into_iter().map(|(status, count)| StatusCount { status, count }).collect(),
        error_counts: errors,
        samples,
        notes,
        contract: contract_summary,
        ..RunReport::base(run.id, run.config.clone(), status, run.started_at_ms, now_ms())
    });
}

/// Requests completed since the last flush.
struct Window {
    hist: Recorder,
    errors: u32,
    opened: Instant,
}

impl Window {
    fn new(timeout: Duration) -> Self {
        Self { hist: Recorder::new(timeout), errors: 0, opened: Instant::now() }
    }

    fn record(&mut self, outcome: &Outcome) {
        self.hist.record(outcome.total);
        if !outcome.is_success() {
            self.errors += 1;
        }
    }

    fn flush(&mut self, elapsed: Duration) -> Option<Bucket> {
        let requests = self.hist.len() as u32;
        if requests == 0 {
            return None;
        }
        let window_s = self.opened.elapsed().as_secs_f64().max(1e-6);
        let bucket = Bucket {
            t_ms: elapsed.as_millis() as u32,
            requests,
            errors: self.errors,
            rps: f64::from(requests) / window_s,
            p50_ms: self.hist.percentile_ms(50.0),
            p99_ms: self.hist.percentile_ms(99.0),
            dropped: 0,
            in_flight: 0,
            lag_ms: 0.0,
        };
        self.hist.reset();
        self.errors = 0;
        self.opened = Instant::now();
        Some(bucket)
    }
}
