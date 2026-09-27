//! Latency recording. Values are stored in microseconds in HDR histograms.

use std::time::Duration;

use hdrhistogram::Histogram;

use crate::engine::types::{HistogramBin, LatencySummary};

const SIGFIGS: u8 = 3;
const HISTOGRAM_BINS: usize = 30;

pub struct Recorder {
    hist: Histogram<u64>,
}

impl Recorder {
    /// `max` must exceed the largest configured timeout; timeouts are recorded at that value.
    pub fn new(max: Duration) -> Self {
        let max_us = (max.as_micros() as u64 * 2).max(10_000);
        Self { hist: Histogram::new_with_bounds(1, max_us, SIGFIGS).expect("valid histogram bounds") }
    }

    pub fn record(&mut self, d: Duration) {
        self.hist.saturating_record((d.as_micros() as u64).max(1));
    }

    pub fn len(&self) -> u64 {
        self.hist.len()
    }

    pub fn reset(&mut self) {
        self.hist.reset();
    }

    pub fn percentile_ms(&self, p: f64) -> f64 {
        us_to_ms(self.hist.value_at_quantile(p / 100.0))
    }

    pub fn summary(&self) -> Option<LatencySummary> {
        let h = &self.hist;
        if h.is_empty() {
            return None;
        }
        Some(LatencySummary {
            count: h.len(),
            min_ms: us_to_ms(h.min()),
            mean_ms: h.mean() / 1000.0,
            p50_ms: self.percentile_ms(50.0),
            p90_ms: self.percentile_ms(90.0),
            p99_ms: self.percentile_ms(99.0),
            max_ms: us_to_ms(h.max()),
            stddev_ms: h.stdev() / 1000.0,
        })
    }

    /// Equal-width bins from min to max, for a distribution chart.
    pub fn bins(&self) -> Vec<HistogramBin> {
        let h = &self.hist;
        if h.is_empty() {
            return vec![];
        }
        let (lo, hi) = (h.min() as f64, h.max() as f64);
        let width = ((hi - lo) / HISTOGRAM_BINS as f64).max(1.0);
        let mut counts = vec![0u64; HISTOGRAM_BINS];
        for v in h.iter_recorded() {
            let idx = (((v.value_iterated_to() as f64 - lo) / width) as usize).min(HISTOGRAM_BINS - 1);
            counts[idx] += v.count_at_value();
        }
        counts
            .into_iter()
            .enumerate()
            .map(|(i, count)| HistogramBin {
                lo_ms: (lo + width * i as f64) / 1000.0,
                hi_ms: (lo + width * (i + 1) as f64) / 1000.0,
                count,
            })
            .collect()
    }
}

fn us_to_ms(us: u64) -> f64 {
    us as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarises_and_bins() {
        let mut r = Recorder::new(Duration::from_secs(1));
        for ms in 1..=100 {
            r.record(Duration::from_millis(ms));
        }
        let s = r.summary().unwrap();
        assert_eq!(s.count, 100);
        assert!((s.p50_ms - 50.0).abs() < 0.5, "{}", s.p50_ms);
        assert!((s.p99_ms - 99.0).abs() < 0.5, "{}", s.p99_ms);
        assert!((s.max_ms - 100.0).abs() < 0.2);
        let bins = r.bins();
        assert_eq!(bins.len(), HISTOGRAM_BINS);
        assert_eq!(bins.iter().map(|b| b.count).sum::<u64>(), 100);
    }

    #[test]
    fn timeouts_beyond_max_are_clamped_not_lost() {
        let mut r = Recorder::new(Duration::from_millis(100));
        r.record(Duration::from_secs(60));
        assert_eq!(r.len(), 1);
    }
}
