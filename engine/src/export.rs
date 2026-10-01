//! Report export (HANDOFF → M5). JSON is the report itself, already redacted (secrets are scrubbed
//! before a report is stored). CSV is the one table worth charting in a spreadsheet: the per-window
//! timeline for latency and load runs, or the per-size points for Big-O runs.

use std::fmt::Write;

use crate::engine::types::RunReport;

/// The report as CSV (RFC 4180: comma-separated, CRLF line endings, a header row).
pub fn csv(report: &RunReport) -> String {
    let mut out = String::new();
    match &report.complexity {
        Some(result) => {
            row(
                &mut out,
                &["n", "median_ms", "p25_ms", "p75_ms", "samples", "errors", "request_bytes", "response_bytes"],
            );
            for p in &result.points {
                row(
                    &mut out,
                    &[
                        &p.n.to_string(),
                        &num(p.median_ms),
                        &num(p.p25_ms),
                        &num(p.p75_ms),
                        &p.samples.to_string(),
                        &p.errors.to_string(),
                        &p.request_bytes.to_string(),
                        &p.response_bytes.to_string(),
                    ],
                );
            }
        }
        None => {
            row(&mut out, &["t_ms", "requests", "errors", "rps", "p50_ms", "p99_ms", "dropped", "in_flight", "lag_ms"]);
            for b in &report.timeline {
                row(
                    &mut out,
                    &[
                        &b.t_ms.to_string(),
                        &b.requests.to_string(),
                        &b.errors.to_string(),
                        &num(b.rps),
                        &num(b.p50_ms),
                        &num(b.p99_ms),
                        &b.dropped.to_string(),
                        &b.in_flight.to_string(),
                        &num(b.lag_ms),
                    ],
                );
            }
        }
    }
    out
}

/// A file name for the download, e.g. `kestrel-load-1f2e3d4c.csv`.
pub fn file_name(report: &RunReport, extension: &str) -> String {
    let kind = serde_json::to_value(report.config.kind()).ok().and_then(|v| v.as_str().map(str::to_owned));
    let id = report.run_id.simple().to_string();
    format!("kestrel-{}-{}.{extension}", kind.as_deref().unwrap_or("run"), &id[..8])
}

/// Three decimals is well below measurement noise and keeps the file readable. Non-finite values
/// (no data in a window) are left empty rather than written as `NaN`.
fn num(v: f64) -> String {
    if v.is_finite() { format!("{v:.3}") } else { String::new() }
}

fn row(out: &mut String, fields: &[&str]) {
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        if field.contains([',', '"', '\n', '\r']) {
            let _ = write!(out, "\"{}\"", field.replace('"', "\"\""));
        } else {
            out.push_str(field);
        }
    }
    out.push_str("\r\n");
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::engine::types::{Bucket, FakeConfig, RunConfig, RunStatus};

    fn report() -> RunReport {
        let mut r = RunReport::base(
            Uuid::parse_str("1f2e3d4c-0000-4000-8000-000000000000").unwrap(),
            RunConfig::Fake(FakeConfig { duration_ms: 500 }),
            RunStatus::Completed,
            0,
            500,
        );
        r.timeline = vec![
            Bucket {
                t_ms: 250,
                requests: 10,
                errors: 1,
                rps: 40.0,
                p50_ms: 1.23456,
                p99_ms: 9.0,
                dropped: 0,
                in_flight: 2,
                lag_ms: 0.0,
            },
            Bucket {
                t_ms: 500,
                requests: 0,
                errors: 0,
                rps: 0.0,
                p50_ms: f64::NAN,
                p99_ms: f64::NAN,
                dropped: 0,
                in_flight: 0,
                lag_ms: 0.0,
            },
        ];
        r
    }

    #[test]
    fn writes_the_timeline_with_a_header() {
        let csv = csv(&report());
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(lines[0], "t_ms,requests,errors,rps,p50_ms,p99_ms,dropped,in_flight,lag_ms");
        assert_eq!(lines[1], "250,10,1,40.000,1.235,9.000,0,2,0.000");
        assert_eq!(lines[2], "500,0,0,0.000,,,0,0,0.000", "no NaN in the file");
        assert_eq!(lines.len(), 4, "CRLF after every row");
        assert_eq!(file_name(&report(), "csv"), "kestrel-fake-1f2e3d4c.csv");
    }

    #[test]
    fn quotes_fields_that_need_it() {
        let mut out = String::new();
        row(&mut out, &["plain", "a,b", "say \"hi\""]);
        assert_eq!(out, "plain,\"a,b\",\"say \"\"hi\"\"\"\r\n");
    }
}
