//! Collects each request's phases (connect, waiting, download) into a [`PhaseSummary`].

use std::time::Duration;

use super::{client::Outcome, types::PhaseSummary};
use crate::stats::Recorder;

/// One request's phases, small enough to send between tasks.
#[derive(Debug, Clone, Copy)]
pub struct PhaseTimes {
    pub connect: Option<Duration>,
    pub ttfb: Duration,
    pub total: Duration,
    /// Whether response headers arrived. Without them there's no waiting/download split.
    pub responded: bool,
}

impl From<&Outcome> for PhaseTimes {
    fn from(o: &Outcome) -> Self {
        Self { connect: o.connect, ttfb: o.ttfb, total: o.total, responded: o.status.is_some() }
    }
}

pub struct Phases {
    connect: Recorder,
    waiting: Recorder,
    download: Recorder,
    new_connections: u64,
    requests: u64,
}

impl Phases {
    pub fn new(timeout: Duration) -> Self {
        Self {
            connect: Recorder::new(timeout),
            waiting: Recorder::new(timeout),
            download: Recorder::new(timeout),
            new_connections: 0,
            requests: 0,
        }
    }

    pub fn record(&mut self, t: PhaseTimes) {
        self.requests += 1;
        if let Some(connect) = t.connect {
            self.new_connections += 1;
            self.connect.record(connect);
        }
        if t.responded {
            self.waiting.record(t.ttfb.saturating_sub(t.connect.unwrap_or_default()));
            self.download.record(t.total.saturating_sub(t.ttfb));
        }
    }

    /// None when nothing was recorded.
    pub fn summary(&self, dns: Option<Duration>) -> Option<PhaseSummary> {
        (self.requests > 0).then(|| PhaseSummary {
            dns_ms: dns.map(|d| d.as_secs_f64() * 1000.0),
            new_connections: self.new_connections,
            requests: self.requests,
            connect: self.connect.summary(),
            waiting: self.waiting.summary(),
            download: self.download.summary(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn splits_requests_into_phases() {
        let mut p = Phases::new(Duration::from_secs(1));
        // A first request that opened a connection, then two that reused it, then a failure.
        p.record(PhaseTimes { connect: Some(30 * MS), ttfb: 50 * MS, total: 60 * MS, responded: true });
        p.record(PhaseTimes { connect: None, ttfb: 20 * MS, total: 25 * MS, responded: true });
        p.record(PhaseTimes { connect: None, ttfb: 20 * MS, total: 25 * MS, responded: true });
        p.record(PhaseTimes { connect: Some(10 * MS), ttfb: 1000 * MS, total: 1000 * MS, responded: false });

        let s = p.summary(Some(4 * MS)).unwrap();
        assert_eq!((s.requests, s.new_connections), (4, 2));
        assert_eq!(s.dns_ms, Some(4.0));
        assert_eq!(s.connect.unwrap().count, 2);
        let waiting = s.waiting.unwrap();
        assert_eq!(waiting.count, 3, "no waiting time for a request that never got a response");
        assert!((waiting.max_ms - 20.0).abs() < 0.1, "connect is taken out of waiting: {waiting:?}");
        assert!((s.download.unwrap().max_ms - 10.0).abs() < 0.1);
        assert!(Phases::new(Duration::from_secs(1)).summary(None).is_none());
    }
}
