//! Analysing a soak run (HANDOFF → v1.x → Soak): a steady rate for a long time, read for drift.
//!
//! Requests go into windows by when they were scheduled. The window is sized from the planned length
//! (about [`TARGET_WINDOWS`] per run, 1 s to 60 s each), so memory stays flat however long the run
//! is, and the windows double as the report's downsampled timeline. The first tenth of the run is
//! warm-up. Drift compares the early fifth that follows with the last fifth; 401/403s that start
//! partway through are called out, since static tokens expiring mid-run look exactly like that.

use std::time::Duration;

use super::types::{Bucket, Finding, FindingLevel, SoakResult, SoakWindow};
use crate::stats::Recorder;

const TARGET_WINDOWS: u64 = 120;
const MIN_WINDOW: Duration = Duration::from_secs(1);
const MAX_WINDOW: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Window {
    scheduled: u64,
    done: u64,
    failed: u64,
    auth_failures: u64,
    latency: Option<Recorder>,
}

pub struct SoakAnalyzer {
    timeout: Duration,
    width: Duration,
    windows: Vec<Window>,
}

/// Window width for a run planned to last `planned`.
pub fn window_for(planned: Duration) -> Duration {
    let ms = (planned.as_millis() as u64 / TARGET_WINDOWS).max(1);
    Duration::from_millis(ms).clamp(MIN_WINDOW, MAX_WINDOW)
}

impl SoakAnalyzer {
    pub fn new(planned: Duration, timeout: Duration) -> Self {
        Self { timeout, width: window_for(planned), windows: Vec::new() }
    }

    fn window(&mut self, at: Duration) -> &mut Window {
        let i = (at.as_millis() / self.width.as_millis()) as usize;
        if self.windows.len() <= i {
            self.windows.resize_with(i + 1, Window::default);
        }
        &mut self.windows[i]
    }

    pub fn scheduled(&mut self, at: Duration) {
        self.window(at).scheduled += 1;
    }

    /// A send dropped at the in-flight cap counts as a failure: the target couldn't keep up.
    pub fn dropped(&mut self, at: Duration) {
        self.window(at).failed += 1;
    }

    pub fn done(&mut self, at: Duration, latency: Duration, failed: bool, status: Option<u16>) {
        let timeout = self.timeout;
        let w = self.window(at);
        w.done += 1;
        w.failed += u64::from(failed);
        w.auth_failures += u64::from(matches!(status, Some(401 | 403)));
        w.latency.get_or_insert_with(|| Recorder::new(timeout)).record(latency);
    }

    pub fn finish(self) -> SoakResult {
        let width_ms = self.width.as_millis() as f64;
        let windows: Vec<SoakWindow> = self
            .windows
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let pct = |n: u64| if w.scheduled > 0 { n as f64 * 100.0 / w.scheduled as f64 } else { 0.0 };
                let p = |q| w.latency.as_ref().map_or(0.0, |r| r.percentile_ms(q));
                SoakWindow {
                    t_ms: (i as f64 * width_ms) as u64,
                    requests: w.done,
                    error_pct: pct(w.failed),
                    auth_failure_pct: pct(w.auth_failures),
                    p50_ms: p(50.0),
                    p99_ms: p(99.0),
                }
            })
            .collect();
        let findings = findings(&self.windows, &windows, self.width, self.timeout);
        SoakResult { window_ms: self.width.as_millis() as u32, windows, findings }
    }
}

/// The soak windows as the report's timeline, so charts and the CSV cover the whole run.
pub fn timeline(result: &SoakResult) -> Vec<Bucket> {
    let width_s = f64::from(result.window_ms) / 1000.0;
    result
        .windows
        .iter()
        .map(|w| Bucket {
            t_ms: (w.t_ms + u64::from(result.window_ms)) as u32,
            requests: w.requests as u32,
            errors: (w.error_pct / 100.0 * w.requests as f64).round() as u32,
            rps: w.requests as f64 / width_s,
            p50_ms: w.p50_ms,
            p99_ms: w.p99_ms,
            dropped: 0,
            in_flight: 0,
            lag_ms: 0.0,
        })
        .collect()
}

/// p50, p99 and error % over a stretch of windows.
struct Stretch {
    p50: f64,
    p99: f64,
    error_pct: f64,
}

/// `timeout` must be the one the windows' recorders were made with, so their bounds match.
fn stretch(raw: &[Window], timeout: Duration) -> Option<Stretch> {
    let mut all = Recorder::new(timeout);
    let (mut scheduled, mut failed) = (0, 0);
    for w in raw {
        if let Some(r) = &w.latency {
            all.merge(r);
        }
        scheduled += w.scheduled;
        failed += w.failed;
    }
    (all.len() > 0).then(|| Stretch {
        p50: all.percentile_ms(50.0),
        p99: all.percentile_ms(99.0),
        error_pct: if scheduled > 0 { failed as f64 * 100.0 / scheduled as f64 } else { 0.0 },
    })
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}h{:02}m", s / 3600, s / 60 % 60) } else { format!("{}m{:02}s", s / 60, s % 60) }
}

fn findings(raw: &[Window], windows: &[SoakWindow], width: Duration, timeout: Duration) -> Vec<Finding> {
    let mut out = Vec::new();
    // The last window is usually partial; leave it out of the comparison.
    let complete = raw.len().saturating_sub(1);
    if complete < 5 {
        out.push(Finding {
            level: FindingLevel::Info,
            message: "The run was too short to read drift; a soak needs several minutes at least.".into(),
        });
        return out;
    }
    let warmup = (complete / 10).max(1);
    let fifth = ((complete - warmup) / 5).max(1);
    let early = stretch(&raw[warmup..warmup + fifth], timeout);
    let late = stretch(&raw[complete - fifth..complete], timeout);

    // Credentials expiring: 401/403s absent early, then the bulk of a window.
    let onset = windows[..complete].iter().position(|w| w.auth_failure_pct >= 50.0);
    let early_auth = windows[..warmup + fifth].iter().any(|w| w.auth_failure_pct > 0.0);
    if let Some(i) = onset.filter(|&i| i >= warmup && !early_auth) {
        out.push(Finding {
            level: FindingLevel::Bad,
            message: format!(
                "From about {} in, most responses were 401 or 403. The credentials probably expired mid-run: \
                 soak runs use the token they started with, so use one that outlives the run.",
                clock(windows[i].t_ms)
            ),
        });
    }

    if let (Some(e), Some(l)) = (early, late) {
        let grew = |from: f64, to: f64| to > from * 1.25 + 5.0;
        let pct = |from: f64, to: f64| if from > 0.0 { (to - from) / from * 100.0 } else { 0.0 };
        if grew(e.p99, l.p99) || grew(e.p50, l.p50) {
            out.push(Finding {
                level: FindingLevel::Warn,
                message: format!(
                    "Latency crept up at a steady rate: p50 {:.1} → {:.1} ms, p99 {:.1} → {:.1} ms ({:+.0}%) from \
                     early in the run to the end. That points at something growing: a leak, a queue, a cache or \
                     connection pool filling up.",
                    e.p50,
                    l.p50,
                    e.p99,
                    l.p99,
                    pct(e.p99, l.p99)
                ),
            });
        }
        if l.error_pct > e.error_pct + 1.0 {
            out.push(Finding {
                level: FindingLevel::Warn,
                message: format!("Errors rose from {:.1}% early on to {:.1}% at the end.", e.error_pct, l.error_pct),
            });
        }
        if out.is_empty() {
            out.push(Finding {
                level: FindingLevel::Good,
                message: format!(
                    "Held steady for {}: p99 {:.1} → {:.1} ms, errors {:.1}% → {:.1}%.",
                    clock((complete as u128 * width.as_millis()) as u64),
                    e.p99,
                    l.p99,
                    e.error_pct,
                    l.error_pct
                ),
            });
        }
    }
    out.sort_by_key(|f| f.level);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    /// 60 one-second windows (a 60 s run), 10 requests each, latency from `latency(second)`.
    fn soak(latency: impl Fn(u64) -> u64, status: impl Fn(u64) -> u16) -> SoakResult {
        let mut a = SoakAnalyzer::new(Duration::from_secs(60), ms(5_000));
        assert_eq!(a.width, Duration::from_secs(1));
        for s in 0..60 {
            for k in 0..10 {
                let at = ms(s * 1000 + k * 100);
                a.scheduled(at);
                let code = status(s);
                a.done(at, ms(latency(s)), code >= 400, Some(code));
            }
        }
        a.finish()
    }

    #[test]
    fn window_size_follows_the_planned_length() {
        assert_eq!(window_for(Duration::from_secs(60)), Duration::from_secs(1));
        assert_eq!(window_for(Duration::from_secs(1800)), Duration::from_secs(15));
        assert_eq!(window_for(Duration::from_secs(4 * 3600)), Duration::from_secs(60));
    }

    #[test]
    fn steady_is_good() {
        let r = soak(|_| 20, |_| 200);
        assert_eq!(r.windows.len(), 60);
        assert_eq!(r.findings[0].level, FindingLevel::Good, "{:?}", r.findings);
        assert_eq!(timeline(&r).len(), 60);
    }

    #[test]
    fn creeping_latency_is_flagged() {
        let r = soak(|s| 20 + s * 2, |_| 200);
        assert_eq!(r.findings[0].level, FindingLevel::Warn, "{:?}", r.findings);
        assert!(r.findings[0].message.contains("crept up"), "{:?}", r.findings);
    }

    #[test]
    fn expiring_credentials_are_called_out() {
        let r = soak(|_| 20, |s| if s >= 40 { 401 } else { 200 });
        assert_eq!(r.findings[0].level, FindingLevel::Bad, "{:?}", r.findings);
        assert!(r.findings[0].message.contains("From about 0m40s"), "{:?}", r.findings);
        assert!(r.findings.iter().any(|f| f.message.contains("Errors rose")), "{:?}", r.findings);
    }

    #[test]
    fn short_runs_say_so() {
        let mut a = SoakAnalyzer::new(Duration::from_secs(3), ms(1_000));
        a.scheduled(ms(10));
        a.done(ms(10), ms(5), false, Some(200));
        assert!(a.finish().findings[0].message.contains("too short"));
    }
}
