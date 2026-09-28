//! Empirical complexity: fit `t(n) = a + b·f(n)` to median latency per input size.
//! HANDOFF → Test catalogue → Big-O.
//!
//! Models are ranked by AICc, not R²: a two-parameter model always fits at least as well as a
//! constant, so R² alone could never answer "O(1)". The intercept absorbs the network floor, so no
//! baseline is subtracted before fitting, and `b` is constrained to be ≥ 0.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Model {
    Constant,
    Log,
    Linear,
    Linearithmic,
    Quadratic,
    Cubic,
}

impl Model {
    /// 2ⁿ isn't a candidate: it overflows and means nothing at these n.
    pub const ALL: [Model; 6] =
        [Model::Constant, Model::Log, Model::Linear, Model::Linearithmic, Model::Quadratic, Model::Cubic];

    pub fn f(self, n: f64) -> f64 {
        match self {
            Model::Constant => 0.0,
            Model::Log => n.ln(),
            Model::Linear => n,
            Model::Linearithmic => n * n.ln(),
            Model::Quadratic => n * n,
            Model::Cubic => n * n * n,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Model::Constant => "O(1)",
            Model::Log => "O(log n)",
            Model::Linear => "O(n)",
            Model::Linearithmic => "O(n log n)",
            Model::Quadratic => "O(n²)",
            Model::Cubic => "O(n³)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelFit {
    pub model: Model,
    /// Intercept (ms): the fixed cost, network included.
    pub a: f64,
    /// Coefficient on f(n), ≥ 0.
    pub b: f64,
    pub r2: f64,
    /// Lower is better. Differences under ~4 mean "hard to tell apart".
    pub aicc: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Confidence {
    High,
    Medium,
    Low,
    Inconclusive,
}

/// Median and spread of latency at one size.
#[derive(Debug, Clone, Copy)]
pub struct SizeStats {
    pub n: f64,
    pub median: f64,
    pub p25: f64,
    pub p75: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Analysis {
    /// Best first.
    pub fits: Vec<ModelFit>,
    /// None when inconclusive.
    pub best: Option<Model>,
    pub verdict: String,
    pub confidence: Confidence,
    /// Slope of log(t − a) vs log n over the upper half of the range: ≈0 constant, ≈1 linear, ≈2
    /// quadratic. A second opinion that doesn't depend on the candidate list.
    pub slope: Option<f64>,
    /// A runner-up that fits nearly as well.
    pub also_plausible: Option<Model>,
    pub explanation: String,
}

/// AICc gap below which two models count as a close call.
const CLOSE_CALL_AICC: f64 = 4.0;
/// Spread (IQR) relative to the fixed cost below which an O(1) verdict is trusted.
const QUIET_NOISE: f64 = 0.25;
/// Jitter below this (ms) is timer and loopback noise, not signal: a 40 µs endpoint with 40 µs of
/// jitter is still flat.
const NOISE_FLOOR_MS: f64 = 0.1;

pub fn fit(points: &[(f64, f64)]) -> Vec<ModelFit> {
    let n = points.len() as f64;
    let y_mean = points.iter().map(|p| p.1).sum::<f64>() / n;
    let ss_tot: f64 = points.iter().map(|p| (p.1 - y_mean).powi(2)).sum();

    let mut fits: Vec<ModelFit> = Model::ALL
        .iter()
        .filter_map(|&model| {
            let (a, b, k) = if model == Model::Constant {
                (y_mean, 0.0, 1.0)
            } else {
                let xs: Vec<f64> = points.iter().map(|p| model.f(p.0)).collect();
                let x_mean = xs.iter().sum::<f64>() / n;
                let var_x: f64 = xs.iter().map(|x| (x - x_mean).powi(2)).sum();
                if var_x == 0.0 || !var_x.is_finite() {
                    return None;
                }
                let cov: f64 = xs.iter().zip(points).map(|(x, p)| (x - x_mean) * (p.1 - y_mean)).sum();
                let b = (cov / var_x).max(0.0);
                let a = if b == 0.0 { y_mean } else { y_mean - b * x_mean };
                (a, b, 2.0)
            };
            let ss_res: f64 = points.iter().map(|p| (p.1 - (a + b * model.f(p.0))).powi(2)).sum();
            let r2 = if ss_tot > 0.0 {
                1.0 - ss_res / ss_tot
            } else if ss_res == 0.0 {
                1.0
            } else {
                0.0
            };
            // Floor the residual so a perfect fit doesn't give -inf.
            let mut aicc = n * (ss_res.max(1e-12) / n).ln() + 2.0 * k;
            if n - k - 1.0 > 0.0 {
                aicc += 2.0 * k * (k + 1.0) / (n - k - 1.0);
            }
            Some(ModelFit { model, a, b, r2, aicc })
        })
        .collect();
    fits.sort_by(|x, y| x.aicc.total_cmp(&y.aicc));
    fits
}

pub fn analyse(points: &[SizeStats]) -> Analysis {
    let inconclusive = |fits, explanation: String| Analysis {
        fits,
        best: None,
        verdict: "Inconclusive".into(),
        confidence: Confidence::Inconclusive,
        slope: None,
        also_plausible: None,
        explanation,
    };
    if points.len() < 3 {
        return inconclusive(vec![], format!("Need at least 3 sizes with results; got {}.", points.len()));
    }

    let fits = fit(&points.iter().map(|p| (p.n, p.median)).collect::<Vec<_>>());
    let best = fits[0].clone();
    let (first, last) = (points.first().unwrap(), points.last().unwrap());
    let growth = last.median - first.median;
    let noise = points.iter().map(|p| p.p75 - p.p25).fold(0.0, f64::max);
    let level = points.iter().map(|p| p.median).fold(f64::INFINITY, f64::min).max(1e-9);
    let slope = log_log_slope(points, best.a);

    if best.model == Model::Constant {
        // Spread below the floor is measurement noise, however small the latency itself is.
        let rel_noise = noise / level;
        let quiet = rel_noise <= QUIET_NOISE || noise <= NOISE_FLOOR_MS;
        if !quiet {
            return inconclusive(
                fits,
                format!(
                    "No clear growth, but samples are too noisy to call it constant (spread {noise:.2} ms at \
                     ~{level:.2} ms)."
                ),
            );
        }
        return Analysis {
            confidence: if rel_noise <= 0.1 || noise <= NOISE_FLOOR_MS / 2.0 {
                Confidence::High
            } else {
                Confidence::Medium
            },
            verdict: Model::Constant.label().into(),
            best: Some(Model::Constant),
            also_plausible: None,
            slope,
            explanation: format!(
                "Latency stays flat ({:.2} → {:.2} ms from n = {} to {}).",
                first.median, last.median, first.n, last.n
            ),
            fits,
        };
    }

    let ratio = growth / noise.max(1e-9);
    if ratio < 1.0 {
        return inconclusive(
            fits,
            format!(
                "Latency grew by {growth:.2} ms, less than the spread of samples at one size ({noise:.2} ms). \
                 Try larger n or more samples."
            ),
        );
    }
    let mut confidence = match ratio {
        r if r >= 10.0 => Confidence::High,
        r if r >= 3.0 => Confidence::Medium,
        _ => Confidence::Low,
    };

    let runner_up = fits
        .iter()
        .skip(1)
        .find(|f| f.model != Model::Constant && f.aicc - best.aicc < CLOSE_CALL_AICC)
        .map(|f| f.model);
    let n_vs_nlogn = matches!(
        (best.model, runner_up),
        (Model::Linear, Some(Model::Linearithmic)) | (Model::Linearithmic, Some(Model::Linear))
    );
    if runner_up.is_some() && confidence == Confidence::High {
        confidence = Confidence::Medium;
    }
    let verdict = if n_vs_nlogn {
        "O(n) or O(n log n): can't separate at this range".to_owned()
    } else {
        best.model.label().to_owned()
    };
    let mut explanation = format!(
        "Latency grew {:.2} → {:.2} ms from n = {} to {}; {} fits best (R² {:.3}).",
        first.median,
        last.median,
        first.n,
        last.n,
        best.model.label(),
        best.r2
    );
    if let Some(s) = slope {
        explanation.push_str(&format!(" Log-log slope over the upper half: {s:.2}."));
    }
    if n_vs_nlogn {
        explanation.push_str(" n and n log n differ by only a log factor; a wider n range would help.");
    }

    Analysis { fits, best: Some(best.model), verdict, confidence, slope, also_plausible: runner_up, explanation }
}

/// Least-squares slope of ln(t − â) against ln n over the upper half of the sizes.
fn log_log_slope(points: &[SizeStats], intercept: f64) -> Option<f64> {
    let upper = &points[points.len() / 2..];
    // Keep t − â positive over the points used. Clamping against the whole range instead would pull â
    // down to the small-n medians and bend the curve.
    let min_upper = upper.iter().map(|p| p.median).fold(f64::INFINITY, f64::min);
    let a_hat = intercept.clamp(0.0, min_upper * 0.99);
    // Where t − â is a sliver of the total, noise dominates its log; use the points where growth shows.
    let max_growth = upper.iter().map(|p| p.median - a_hat).fold(0.0, f64::max);
    let upper: Vec<(f64, f64)> = upper
        .iter()
        .filter(|p| p.median - a_hat >= 0.05 * max_growth && p.median - a_hat > 0.0 && p.n > 0.0)
        .map(|p| (p.n.ln(), (p.median - a_hat).ln()))
        .collect();
    if upper.len() < 2 {
        return None;
    }
    let k = upper.len() as f64;
    let (mx, my) = (upper.iter().map(|p| p.0).sum::<f64>() / k, upper.iter().map(|p| p.1).sum::<f64>() / k);
    let var: f64 = upper.iter().map(|p| (p.0 - mx).powi(2)).sum();
    (var > 0.0).then(|| upper.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>() / var)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes() -> Vec<f64> {
        (0..13).map(|i| 2f64.powi(i)).collect() // 1 … 4096
    }

    /// Deterministic ±`jitter` noise and a matching IQR.
    fn points(t: impl Fn(f64) -> f64, jitter: f64) -> Vec<SizeStats> {
        sizes()
            .into_iter()
            .enumerate()
            .map(|(i, n)| {
                let wobble = if i % 2 == 0 { jitter } else { -jitter };
                let median = t(n) + wobble;
                SizeStats { n, median, p25: median - jitter, p75: median + jitter }
            })
            .collect()
    }

    #[test]
    fn classifies_clean_curves() {
        type Curve = fn(f64) -> f64;
        let cases: [(&str, Curve, Model); 4] = [
            ("linear", |n| 2.0 + 0.01 * n, Model::Linear),
            ("quadratic", |n| 2.0 + 2e-6 * n * n, Model::Quadratic),
            ("cubic", |n| 2.0 + 1e-9 * n * n * n, Model::Cubic),
            ("log", |n| 2.0 + 0.5 * n.ln(), Model::Log),
        ];
        for (name, t, expected) in cases {
            let a = analyse(&points(t, 0.01));
            assert_eq!(a.best, Some(expected), "{name}: {a:#?}");
            assert_ne!(a.confidence, Confidence::Inconclusive, "{name}");
        }
    }

    #[test]
    fn slope_tracks_the_exponent() {
        let linear = analyse(&points(|n| 2.0 + 0.01 * n, 0.001)).slope.unwrap();
        let quadratic = analyse(&points(|n| 2.0 + 2e-6 * n * n, 0.001)).slope.unwrap();
        assert!((linear - 1.0).abs() < 0.15, "{linear}");
        assert!((quadratic - 2.0).abs() < 0.15, "{quadratic}");
    }

    #[test]
    fn flat_and_quiet_is_constant() {
        let a = analyse(&points(|_| 5.0, 0.05));
        assert_eq!((a.best, a.verdict.as_str()), (Some(Model::Constant), "O(1)"));
        assert_eq!(a.confidence, Confidence::High);
    }

    #[test]
    fn microsecond_jitter_does_not_hide_a_flat_endpoint() {
        // 50 µs endpoint with 40 µs of loopback jitter.
        let a = analyse(&points(|_| 0.05, 0.02));
        assert_eq!(a.best, Some(Model::Constant), "{a:#?}");
        assert_eq!(a.confidence, Confidence::High);
    }

    #[test]
    fn growth_smaller_than_noise_is_inconclusive() {
        // Grows by ~1 ms but samples at each size spread over 4 ms.
        let a = analyse(&points(|n| 5.0 + n / 4096.0, 2.0));
        assert_eq!(a.confidence, Confidence::Inconclusive, "{a:#?}");
        assert!(a.best.is_none());
    }

    #[test]
    fn n_log_n_is_reported_honestly() {
        let a = analyse(&points(|n| 2.0 + 0.002 * n * n.ln(), 0.02));
        let ok = a.best == Some(Model::Linearithmic) || a.verdict.contains("can't separate");
        assert!(ok, "must be n log n or flagged as a close call, never confidently wrong: {a:#?}");
    }

    #[test]
    fn needs_three_sizes() {
        let a = analyse(&points(|n| n, 0.0)[..2]);
        assert_eq!(a.confidence, Confidence::Inconclusive);
    }

    #[test]
    fn negative_slopes_are_clamped() {
        let fits = fit(&[(1.0, 10.0), (10.0, 5.0), (100.0, 1.0)]);
        assert!(fits.iter().all(|f| f.b >= 0.0));
    }
}
