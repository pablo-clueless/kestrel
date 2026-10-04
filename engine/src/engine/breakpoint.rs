//! Judging the steps of a breakpoint run (HANDOFF → v1.x → Stress / breakpoint). The load runner
//! schedules the rising rate; this collects each step's results and decides whether it held.
//!
//! A step is judged `grace` after it ends, while the next one is already running, so the run can
//! stop as soon as one breaks. Requests of the step still in flight by then are counted as slow
//! (at `grace`): a server that has stopped answering mustn't pass just because its requests
//! haven't come back yet.

use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::types::{BreakpointMode, BreakpointResult, BreakpointStep};
use crate::stats::Recorder;

struct StepStats {
    /// Everything the scheduler planned to send in this step, drops included.
    scheduled: u64,
    dropped: u64,
    done: u64,
    errors: u64,
    latency: Recorder,
}

pub struct StepJudge {
    mode: BreakpointMode,
    rates: Vec<u32>,
    step: Duration,
    grace: Duration,
    stats: Vec<StepStats>,
    judged: usize,
    steps: Vec<BreakpointStep>,
    /// Cancelled when a step breaks, which ends the schedule.
    stop: CancellationToken,
    /// What the failures are called in reasons, e.g. "errors" or "429s".
    failures: &'static str,
}

impl StepJudge {
    pub fn new(mode: BreakpointMode, timeout: Duration, stop: CancellationToken) -> Self {
        let rates = mode.rates();
        let step = Duration::from_millis(mode.step_ms.into());
        let stats = rates
            .iter()
            .map(|_| StepStats { scheduled: 0, dropped: 0, done: 0, errors: 0, latency: Recorder::new(timeout) })
            .collect();
        Self {
            mode,
            rates,
            step,
            grace: timeout.min(Duration::from_secs(2)).min(step),
            stats,
            judged: 0,
            steps: Vec::new(),
            stop,
            failures: "errors",
        }
    }

    /// Names the failures in reasons (rate-limit discovery counts only 429s).
    pub fn counting(mut self, failures: &'static str) -> Self {
        self.failures = failures;
        self
    }

    fn index(&self, at: Duration) -> usize {
        ((at.as_secs_f64() / self.step.as_secs_f64()) as usize).min(self.rates.len() - 1)
    }

    /// A send was due at `at` (since the run started), whether or not it went out.
    pub fn scheduled(&mut self, at: Duration) {
        let i = self.index(at);
        self.stats[i].scheduled += 1;
    }

    pub fn dropped(&mut self, at: Duration) {
        let i = self.index(at);
        self.stats[i].dropped += 1;
    }

    /// A request scheduled at `at` finished.
    pub fn done(&mut self, at: Duration, latency: Duration, failed: bool) {
        let i = self.index(at);
        let s = &mut self.stats[i];
        s.done += 1;
        s.errors += u64::from(failed);
        s.latency.record(latency);
    }

    /// Judges every step that ended at least `grace` before `now`. Returns true once one broke.
    pub fn tick(&mut self, now: Duration) -> bool {
        while !self.broke()
            && self.judged < self.rates.len()
            && now >= self.step * (self.judged as u32 + 1) + self.grace
        {
            self.judge();
        }
        self.broke()
    }

    /// At the end of the run: judges the steps that ran and haven't been judged yet.
    pub fn finish(mut self) -> BreakpointResult {
        while !self.broke() && self.judged < self.rates.len() && self.stats[self.judged].scheduled > 0 {
            self.judge();
        }
        BreakpointResult {
            held_rate: self.steps.iter().take_while(|s| s.passed).last().map(|s| s.rate),
            broke_at_rate: self.steps.iter().find(|s| !s.passed).map(|s| s.rate),
            steps: self.steps,
        }
    }

    fn broke(&self) -> bool {
        self.steps.last().is_some_and(|s| !s.passed)
    }

    fn judge(&mut self) {
        let k = self.judged;
        self.judged += 1;
        let rate = self.rates[k];
        let s = &mut self.stats[k];
        if s.scheduled == 0 {
            return;
        }
        let pending = s.scheduled.saturating_sub(s.dropped + s.done);
        s.latency.record_n(self.grace, pending);
        let failed = s.errors + s.dropped;
        let error_pct = failed as f64 * 100.0 / s.scheduled as f64;
        let (p50_ms, p99_ms) = if s.latency.len() > 0 {
            (s.latency.percentile_ms(50.0), s.latency.percentile_ms(99.0))
        } else {
            (0.0, 0.0)
        };

        let mut reasons = Vec::new();
        if error_pct > self.mode.max_error_pct {
            reasons.push(format!("{} {error_pct:.1}% > {}%", self.failures, self.mode.max_error_pct));
        }
        if let Some(limit) = self.mode.max_p99_ms
            && p99_ms > f64::from(limit)
        {
            reasons.push(format!("p99 {p99_ms:.0} ms > {limit} ms"));
        }
        let passed = reasons.is_empty();
        self.steps.push(BreakpointStep {
            rate,
            achieved_rps: (s.done - s.errors.min(s.done)) as f64 / self.step.as_secs_f64(),
            requests: s.scheduled,
            error_pct,
            dropped: s.dropped,
            p50_ms,
            p99_ms,
            passed,
            reason: (!passed).then(|| reasons.join(", ")),
        });
        if !passed {
            self.stop.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn mode() -> BreakpointMode {
        BreakpointMode {
            start_rate: 10,
            step_percent: 50,
            step_ms: 1_000,
            max_rate: 40,
            max_error_pct: 5.0,
            max_p99_ms: Some(200),
        }
    }

    /// Fills one step: `n` requests scheduled in it, `slow` of them taking 500 ms, `errors` failing.
    fn fill(judge: &mut StepJudge, step: u32, n: u32, slow: u32, errors: u32) {
        for i in 0..n {
            let at = Duration::from_millis(u64::from(step) * 1_000 + u64::from(i) * 1_000 / u64::from(n));
            judge.scheduled(at);
            let latency = if i < slow { 500 * MS } else { 20 * MS };
            judge.done(at, latency, i < errors);
        }
    }

    #[test]
    fn steps_rise_by_percent_up_to_the_max() {
        assert_eq!(mode().rates(), [10, 15, 23, 35, 40]);
        let tiny = BreakpointMode { start_rate: 1, step_percent: 10, max_rate: 3, ..mode() };
        assert_eq!(tiny.rates(), [1, 2, 3], "at least +1 req/s per step");
    }

    #[test]
    fn stops_at_the_first_step_over_a_limit() {
        let stop = CancellationToken::new();
        let mut judge = StepJudge::new(mode(), Duration::from_secs(1), stop.clone());
        fill(&mut judge, 0, 10, 0, 0);
        fill(&mut judge, 1, 15, 0, 0);
        fill(&mut judge, 2, 23, 3, 0); // 3 of 23 slow: p99 500 ms
        assert!(!judge.tick(Duration::from_millis(1_500)), "step 0 isn't judged before its grace");
        assert!(!judge.tick(Duration::from_millis(3_000)), "steps 0 and 1 hold");
        assert!(!stop.is_cancelled());
        assert!(judge.tick(Duration::from_millis(4_000)), "step 2 breaks on p99");
        assert!(stop.is_cancelled());

        let result = judge.finish();
        assert_eq!((result.held_rate, result.broke_at_rate), (Some(15), Some(23)));
        assert_eq!(result.steps.len(), 3, "nothing is judged after the break");
        assert_eq!(result.steps[2].reason.as_deref(), Some("p99 500 ms > 200 ms"));
        assert!((result.steps[0].achieved_rps - 10.0).abs() < 0.01);
    }

    #[test]
    fn errors_drops_and_unanswered_requests_count() {
        let mut judge = StepJudge::new(mode(), Duration::from_secs(1), CancellationToken::new());
        // 10 scheduled: 9 answered, 1 dropped at the in-flight cap: 10% failed.
        for i in 0..10u64 {
            let at = Duration::from_millis(i * 100);
            judge.scheduled(at);
            if i == 9 { judge.dropped(at) } else { judge.done(at, 10 * MS, false) }
        }
        assert!(judge.tick(Duration::from_secs(5)));
        let step = &judge.finish().steps[0];
        assert!(step.reason.as_deref().unwrap().starts_with("errors 10.0%"), "{step:?}");

        // A server that stops answering: the requests are still in flight when the step is judged.
        let mut silent = StepJudge::new(
            BreakpointMode { max_error_pct: 50.0, ..mode() },
            Duration::from_secs(1),
            CancellationToken::new(),
        );
        for i in 0..10u64 {
            silent.scheduled(Duration::from_millis(i * 100));
        }
        assert!(silent.tick(Duration::from_secs(5)), "unanswered requests count as slow");
    }

    #[test]
    fn a_run_that_never_breaks_holds_every_step() {
        let mut judge = StepJudge::new(mode(), Duration::from_secs(1), CancellationToken::new());
        for (step, rate) in [10, 15, 23, 35, 40].into_iter().enumerate() {
            fill(&mut judge, step as u32, rate, 0, 0);
        }
        let result = judge.finish();
        assert_eq!((result.held_rate, result.broke_at_rate, result.steps.len()), (Some(40), None, 5));
    }
}
