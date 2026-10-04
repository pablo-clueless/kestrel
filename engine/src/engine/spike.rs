//! Analysing a spike run (HANDOFF → v1.x → Spike): a baseline rate, a burst, then the baseline
//! again. Requests are bucketed into one-second windows by when they were scheduled; the baseline's
//! windows say what "normal" is, and recovery is when the windows after the burst are normal again
//! and stay that way.

use std::time::Duration;

use super::types::{SpikeMode, SpikePhase, SpikeResult};
use crate::stats::Recorder;

const WINDOW: Duration = Duration::from_secs(1);
/// The first second of the baseline is warm-up (connections, caches) and isn't part of "normal".
const WARMUP: Duration = Duration::from_secs(1);

struct Window {
    scheduled: u64,
    failed: u64,
    done: u64,
    latency: Recorder,
}

pub struct SpikeAnalyzer {
    mode: SpikeMode,
    timeout: Duration,
    windows: Vec<Window>,
}

impl SpikeAnalyzer {
    pub fn new(mode: SpikeMode, timeout: Duration) -> Self {
        Self { mode, timeout, windows: Vec::new() }
    }

    fn window(&mut self, at: Duration) -> &mut Window {
        let i = (at.as_secs_f64() / WINDOW.as_secs_f64()) as usize;
        while self.windows.len() <= i {
            self.windows.push(Window { scheduled: 0, failed: 0, done: 0, latency: Recorder::new(self.timeout) });
        }
        &mut self.windows[i]
    }

    pub fn scheduled(&mut self, at: Duration) {
        self.window(at).scheduled += 1;
    }

    /// A send dropped at the in-flight cap: it counts as a failure.
    pub fn dropped(&mut self, at: Duration) {
        self.window(at).failed += 1;
    }

    pub fn done(&mut self, at: Duration, latency: Duration, failed: bool) {
        let w = self.window(at);
        w.done += 1;
        w.failed += u64::from(failed);
        w.latency.record(latency);
    }

    pub fn finish(self) -> SpikeResult {
        let ms = |v: u32| Duration::from_millis(v.into());
        let spike_start = ms(self.mode.before_ms);
        let spike_end = spike_start + ms(self.mode.spike_ms);
        let end = spike_end + ms(self.mode.after_ms);
        let range = |from: Duration, to: Duration| {
            let first = (from.as_secs_f64() / WINDOW.as_secs_f64()).ceil() as usize;
            let last = (to.as_secs_f64() / WINDOW.as_secs_f64()) as usize;
            first..last.min(self.windows.len())
        };

        let baseline = self.phase(range(WARMUP.min(spike_start), spike_start));
        let spike = self.phase(range(spike_start, spike_end));
        let after = self.phase(range(spike_end, end));

        // Normal: p99 within 1.5× the baseline's (at least +5 ms, for very fast targets), and no more
        // than one point more errors.
        let normal = |w: &Window| {
            let p99 = if w.latency.len() > 0 { w.latency.percentile_ms(99.0) } else { 0.0 };
            let err = if w.scheduled > 0 { w.failed as f64 * 100.0 / w.scheduled as f64 } else { 0.0 };
            baseline.as_ref().is_none_or(|b| p99 <= (b.p99_ms * 1.5).max(b.p99_ms + 5.0) && err <= b.error_pct + 1.0)
        };
        let after_windows: Vec<&Window> =
            self.windows[range(spike_end, end)].iter().filter(|w| w.scheduled > 0).collect();
        // The first window from which every later one is normal.
        let settled = (0..=after_windows.len()).find(|&i| after_windows[i..].iter().all(|w| normal(w)));
        let recovery_ms = match settled {
            Some(i) if i < after_windows.len() => Some(i as u32 * WINDOW.as_millis() as u32),
            // Every window after the spike was abnormal: not recovered within the run.
            _ => None,
        };

        SpikeResult { baseline, spike, after, recovery_ms }
    }

    /// Stats over a range of windows.
    fn phase(&self, windows: std::ops::Range<usize>) -> Option<SpikePhase> {
        let ws = self.windows.get(windows.clone())?;
        let scheduled: u64 = ws.iter().map(|w| w.scheduled).sum();
        if scheduled == 0 {
            return None;
        }
        let mut latency = Recorder::new(self.timeout);
        for w in ws {
            latency.merge(&w.latency);
        }
        let failed: u64 = ws.iter().map(|w| w.failed).sum();
        let done: u64 = ws.iter().map(|w| w.done).sum();
        let seconds = ws.len() as f64 * WINDOW.as_secs_f64();
        Some(SpikePhase {
            requests: scheduled,
            achieved_rps: done.saturating_sub(failed.min(done)) as f64 / seconds,
            error_pct: failed as f64 * 100.0 / scheduled as f64,
            p50_ms: if latency.len() > 0 { latency.percentile_ms(50.0) } else { 0.0 },
            p99_ms: if latency.len() > 0 { latency.percentile_ms(99.0) } else { 0.0 },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode() -> SpikeMode {
        SpikeMode { base_rate: 10, spike_rate: 100, before_ms: 4_000, spike_ms: 2_000, after_ms: 6_000 }
    }

    /// One second of traffic starting at `second`: `n` requests at `latency_ms`, `failed` of them failing.
    fn second(a: &mut SpikeAnalyzer, second: u64, n: u64, latency_ms: u64, failed: u64) {
        for i in 0..n {
            let at = Duration::from_millis(second * 1_000 + i * 1_000 / n);
            a.scheduled(at);
            a.done(at, Duration::from_millis(latency_ms), i < failed);
        }
    }

    #[test]
    fn measures_how_long_the_target_takes_to_recover() {
        let mut a = SpikeAnalyzer::new(mode(), Duration::from_secs(5));
        for s in 0..4 {
            second(&mut a, s, 10, 20, 0); // baseline
        }
        second(&mut a, 4, 100, 400, 5); // the burst
        second(&mut a, 5, 100, 900, 20);
        second(&mut a, 6, 10, 600, 0); // still slow after the burst
        second(&mut a, 7, 10, 200, 0);
        for s in 8..12 {
            second(&mut a, s, 10, 21, 0); // back to normal from 8 s, i.e. 2 s after the burst
        }
        let r = a.finish();
        let (baseline, spike) = (r.baseline.unwrap(), r.spike.unwrap());
        assert_eq!(baseline.requests, 30, "the first second is warm-up");
        assert!((baseline.p99_ms - 20.0).abs() < 0.5, "{baseline:?}");
        assert!(spike.p99_ms >= 800.0 && spike.error_pct > 10.0, "{spike:?}");
        assert_eq!(r.recovery_ms, Some(2_000));
    }

    #[test]
    fn a_target_that_never_recovers_says_so() {
        let mut a = SpikeAnalyzer::new(mode(), Duration::from_secs(5));
        for s in 0..4 {
            second(&mut a, s, 10, 20, 0);
        }
        second(&mut a, 4, 100, 500, 0);
        second(&mut a, 5, 100, 500, 0);
        for s in 6..12 {
            second(&mut a, s, 10, 300, 0);
        }
        assert_eq!(a.finish().recovery_ms, None);
    }

    #[test]
    fn a_target_that_shrugs_the_spike_off_recovers_at_once() {
        let mut a = SpikeAnalyzer::new(mode(), Duration::from_secs(5));
        for s in 0..12 {
            second(&mut a, s, if (4..6).contains(&s) { 100 } else { 10 }, 20, 0);
        }
        assert_eq!(a.finish().recovery_ms, Some(0));
    }
}
