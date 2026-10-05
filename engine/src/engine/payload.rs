//! Payload scaling: how latency and throughput change with the bytes sent and received.
//! HANDOFF → Test catalogue → v1.x → Payload scaling.
//!
//! The sweep is Big-O's (`complexity.rs`), with `n` driving the payload size (`{{n:string}}` in a
//! body, or a size parameter that grows the response). This reads its per-size medians as bytes:
//! a straight-line fit of latency against total bytes splits each request into a fixed cost and a
//! cost per MB (the effective throughput), and each size gets its own MB/s. Big-O's log-log slope
//! says whether time grows faster than the payload.

use super::types::{ComplexityPoint, Finding, FindingLevel, PayloadPoint, PayloadResult};
use crate::stats::fit::Analysis;

const MB: f64 = 1_000_000.0;

/// Bytes must span at least this factor for a fit to mean anything.
const MIN_BYTE_RANGE: f64 = 2.0;

pub fn analyse(points: &[ComplexityPoint], analysis: &Analysis, loopback: bool) -> PayloadResult {
    let points: Vec<PayloadPoint> = points
        .iter()
        .filter(|p| p.samples > 0)
        .map(|p| {
            let bytes = p.request_bytes + p.response_bytes;
            PayloadPoint {
                n: p.n,
                bytes,
                median_ms: p.median_ms,
                mb_per_s: if p.median_ms > 0.0 { bytes as f64 / MB / (p.median_ms / 1000.0) } else { 0.0 },
            }
        })
        .collect();
    let peak_mb_per_s =
        points.iter().map(|p| p.mb_per_s).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
    let mut findings = Vec::new();
    let mut add = |level, message: String| findings.push(Finding { level, message });

    let (lo, hi) =
        (points.iter().map(|p| p.bytes).min().unwrap_or(0), points.iter().map(|p| p.bytes).max().unwrap_or(0));
    let mut fixed_ms = None;
    let mut mb_per_s = None;
    if points.len() < 3 || hi == 0 || (hi as f64) < (lo.max(1) as f64) * MIN_BYTE_RANGE {
        add(
            FindingLevel::Info,
            format!(
                "The payload barely changed ({} to {}), so there's nothing to scale against. Put {{{{n:string}}}} \
                 in the body, or use {{{{n}}}} in a parameter that grows the response.",
                human(lo),
                human(hi)
            ),
        );
    } else {
        let (a, b) = line(&points);
        fixed_ms = Some(a.max(0.0));
        if b > 0.0 {
            // b is ms per byte, so 1/b bytes per ms: × 1000 for per second, ÷ 1e6 for MB.
            let rate = 1.0 / (b * 1000.0);
            mb_per_s = Some(rate);
            add(
                FindingLevel::Info,
                format!(
                    "Each request costs about {:.1} ms whatever its size, plus about {:.1} ms per MB: an effective \
                     {} for the payload itself.",
                    a.max(0.0),
                    b * MB,
                    rate_text(rate)
                ),
            );
        } else {
            add(
                FindingLevel::Good,
                format!(
                    "Size made no measurable difference from {} to {}: transfer and parsing are cheap at these sizes.",
                    human(lo),
                    human(hi)
                ),
            );
        }
        if let Some(slope) = analysis.slope.filter(|s| *s > 1.3) {
            add(
                FindingLevel::Warn,
                format!(
                    "Latency grows faster than the payload (log-log slope {slope:.2}; 1 would be in step): doubling \
                     the size more than doubles the time. Look for parsing, validation or serialisation that isn't \
                     linear, or buffers being copied repeatedly."
                ),
            );
        }
        if let (Some(peak), Some(best)) =
            (peak_mb_per_s, points.iter().max_by(|x, y| x.mb_per_s.total_cmp(&y.mb_per_s)))
            && best.bytes < hi
            && peak > 0.0
        {
            add(
                FindingLevel::Info,
                format!(
                    "Throughput peaked at {} around {} and fell for larger payloads.",
                    rate_text(peak),
                    human(best.bytes)
                ),
            );
        }
    }
    if loopback {
        add(
            FindingLevel::Info,
            "The target is on this machine, so MB/s here measures CPU and memory copies, not a network.".into(),
        );
    }
    findings.sort_by_key(|f| f.level);
    PayloadResult { points, fixed_ms, mb_per_s, peak_mb_per_s, findings }
}

/// Least-squares line through (bytes, median ms): (intercept ms, slope ms per byte).
fn line(points: &[PayloadPoint]) -> (f64, f64) {
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.bytes as f64).sum::<f64>() / n;
    let my = points.iter().map(|p| p.median_ms).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|p| (p.bytes as f64 - mx).powi(2)).sum();
    let sxy: f64 = points.iter().map(|p| (p.bytes as f64 - mx) * (p.median_ms - my)).sum();
    let b = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    (my - b * mx, b)
}

fn human(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= 1000.0 {
        format!("{:.1} kB", b / 1000.0)
    } else {
        format!("{bytes} B")
    }
}

fn rate_text(mb_per_s: f64) -> String {
    if mb_per_s >= 1000.0 { format!("{:.1} GB/s", mb_per_s / 1000.0) } else { format!("{mb_per_s:.1} MB/s") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::fit::{self, SizeStats};

    /// Points at sizes `n` (request bytes = n) with latency `ms(n)`.
    fn sweep(ms: impl Fn(f64) -> f64) -> (Vec<ComplexityPoint>, Analysis) {
        let points: Vec<ComplexityPoint> = [1_000u64, 4_000, 16_000, 64_000, 256_000, 1_000_000]
            .iter()
            .map(|&n| {
                let t = ms(n as f64);
                ComplexityPoint {
                    n,
                    median_ms: t,
                    p25_ms: t * 0.98,
                    p75_ms: t * 1.02,
                    samples: 15,
                    errors: 0,
                    request_bytes: n,
                    response_bytes: 0,
                    baseline_median_ms: None,
                }
            })
            .collect();
        let stats: Vec<SizeStats> = points
            .iter()
            .map(|p| SizeStats { n: p.n as f64, median: p.median_ms, p25: p.p25_ms, p75: p.p75_ms })
            .collect();
        let analysis = fit::analyse(&stats);
        (points, analysis)
    }

    #[test]
    fn a_linear_cost_gives_the_fixed_part_and_the_throughput() {
        // 2 ms fixed, then 10 ms per MB: 100 MB/s.
        let (points, analysis) = sweep(|n| 2.0 + n / MB * 10.0);
        let r = analyse(&points, &analysis, false);
        assert!((r.fixed_ms.unwrap() - 2.0).abs() < 0.05, "{r:?}");
        assert!((r.mb_per_s.unwrap() - 100.0).abs() < 1.0, "{r:?}");
        assert!(r.findings.iter().all(|f| f.level != FindingLevel::Warn), "{:?}", r.findings);
        assert!(r.findings.iter().any(|f| f.message.contains("100.0 MB/s")), "{:?}", r.findings);
        // Bigger payloads amortise the fixed cost, so MB/s rises with size.
        assert!(r.points.last().unwrap().mb_per_s > r.points[0].mb_per_s * 10.0, "{r:?}");
    }

    #[test]
    fn faster_than_linear_growth_is_flagged() {
        let (points, analysis) = sweep(|n| 1.0 + (n / 10_000.0).powi(2));
        let r = analyse(&points, &analysis, false);
        assert_eq!(r.findings[0].level, FindingLevel::Warn, "{:?}", r.findings);
        assert!(r.findings[0].message.contains("faster than the payload"), "{:?}", r.findings);
    }

    #[test]
    fn a_payload_that_does_not_grow_says_so() {
        let (mut points, analysis) = sweep(|_| 5.0);
        for p in &mut points {
            p.request_bytes = 100;
        }
        let r = analyse(&points, &analysis, true);
        assert!(r.findings.iter().any(|f| f.message.contains("barely changed")), "{:?}", r.findings);
        assert!(r.findings.iter().any(|f| f.message.contains("CPU and memory")), "{:?}", r.findings);
        assert_eq!(r.mb_per_s, None);
    }
}
