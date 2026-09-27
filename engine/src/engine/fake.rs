//! A runner that sends no traffic and emits plausible-looking buckets. It exists to prove the
//! run → SSE → UI loop (M0) before real HTTP is involved.

use std::{sync::Arc, time::Duration};

use rand::RngExt;
use tokio::time::{Instant, MissedTickBehavior};

use super::{
    BUCKET_INTERVAL,
    registry::{Run, now_ms},
    types::{Bucket, FakeConfig, RunEvent, RunReport, RunStatus, StartedEvent},
};

pub async fn run(run: Arc<Run>, cfg: FakeConfig) {
    run.publish(RunEvent::Started(StartedEvent {
        run_id: run.id,
        config: run.config.clone(),
        started_at_ms: run.started_at_ms,
    }));

    let started = Instant::now();
    let duration = Duration::from_millis(cfg.duration_ms.into());
    let mut ticks = tokio::time::interval_at(started + BUCKET_INTERVAL, BUCKET_INTERVAL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let (mut total_requests, mut total_errors, mut max_p99_ms) = (0u64, 0u64, 0f64);

    let status = loop {
        tokio::select! {
            _ = run.cancel.cancelled() => break RunStatus::Cancelled,
            _ = ticks.tick() => {
                let elapsed = started.elapsed();
                let bucket = synth_bucket(elapsed);
                total_requests += u64::from(bucket.requests);
                total_errors += u64::from(bucket.errors);
                max_p99_ms = max_p99_ms.max(bucket.p99_ms);
                run.publish(RunEvent::Bucket(bucket));
                if elapsed >= duration {
                    break RunStatus::Completed;
                }
            }
        }
    };

    let elapsed_s = started.elapsed().as_secs_f64();
    run.finish(RunReport {
        total_requests,
        total_errors,
        mean_rps: if elapsed_s > 0.0 { total_requests as f64 / elapsed_s } else { 0.0 },
        max_p99_ms,
        ..RunReport::base(run.id, run.config.clone(), status, run.started_at_ms, now_ms())
    });
}

/// ~1,000 rps with a slow latency wave and an occasional p99 spike.
fn synth_bucket(elapsed: Duration) -> Bucket {
    let mut rng = rand::rng();
    let t = elapsed.as_secs_f64();
    let requests: u32 = rng.random_range(230..=270);
    let errors = (0..requests).filter(|_| rng.random_bool(0.005)).count() as u32;
    let p50_ms = 12.0 + 3.0 * (t / 2.0).sin() + rng.random_range(-0.8..0.8);
    let spike = if rng.random_bool(0.04) { rng.random_range(80.0..200.0) } else { 0.0 };
    let p99_ms = p50_ms * 3.2 + rng.random_range(-4.0..4.0) + spike;
    Bucket {
        t_ms: elapsed.as_millis() as u32,
        requests,
        errors,
        rps: f64::from(requests) / BUCKET_INTERVAL.as_secs_f64(),
        p50_ms,
        p99_ms,
        dropped: 0,
        in_flight: 0,
        lag_ms: 0.0,
    }
}
