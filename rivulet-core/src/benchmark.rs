//! G5 – Performance verification: frame-time benchmarking and overhead budget checking.
//!
//! This module provides the statistical framework for measuring capture overhead:
//! frame-time percentiles (p50/p95/p99), A/B delta calculation (capture off vs. on),
//! and a hard budget gate that fails CI when overhead exceeds the threshold.
//!
//! **Budget:** <1% of frame time, measured as the p99 frame-time delta.
//!
//! | Refresh | Budget (p99 delta) |
//! |---------|-------------------|
//! | 60 Hz   | < 0.17 ms         |
//! | 120 Hz  | < 0.08 ms         |
//! | 144 Hz  | < 0.07 ms         |

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Overhead budget per refresh rate (p99 delta in milliseconds).
///
/// These thresholds come from `docs/game-capture-strategy.md` §5.
pub struct OverheadBudget;

impl OverheadBudget {
    /// Maximum allowed p99 frame-time delta (ms) for a given refresh rate.
    pub fn max_delta_ms(refresh_hz: u32) -> f64 {
        match refresh_hz {
            0..=60 => 0.17,
            61..=120 => 0.08,
            121.. => 0.07,
        }
    }

    /// Maximum allowed p99 frame-time delta as a `Duration`.
    pub fn max_delta(refresh_hz: u32) -> Duration {
        Duration::from_secs_f64(Self::max_delta_ms(refresh_hz) / 1000.0)
    }
}

/// A single frame-time sample (in milliseconds).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FrameSample {
    /// Wall-clock frame time in milliseconds.
    pub delta_ms: f64,
}

/// Collected frame-time samples for one measurement run (either baseline or capture-on).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleSet {
    /// Human-readable label (e.g. "baseline", "dxgi_capture", "vulkan_capture").
    pub label: String,
    /// Target refresh rate used for this run.
    pub refresh_hz: u32,
    /// Frame-time samples in milliseconds.
    pub samples: Vec<f64>,
}

impl SampleSet {
    pub fn new(label: impl Into<String>, refresh_hz: u32) -> Self {
        Self {
            label: label.into(),
            refresh_hz,
            samples: Vec::new(),
        }
    }

    pub fn push(&mut self, delta_ms: f64) {
        self.samples.push(delta_ms);
    }

    /// Compute sorted percentile value using linear interpolation.
    fn percentile(sorted: &[f64], p: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        if sorted.len() == 1 {
            return sorted[0];
        }
        let rank = (p / 100.0) * (sorted.len() as f64 - 1.0);
        let lo = rank.floor() as usize;
        let hi = (lo + 1).min(sorted.len() - 1);
        let frac = rank - rank.floor();
        sorted[lo] + frac * (sorted[hi] - sorted[lo])
    }

    /// Calculate frame-time percentiles from the samples.
    pub fn percentiles(&self) -> FramePercentiles {
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        FramePercentiles {
            p50: Self::percentile(&sorted, 50.0),
            p95: Self::percentile(&sorted, 95.0),
            p99: Self::percentile(&sorted, 99.0),
            min: sorted.first().copied().unwrap_or(0.0),
            max: sorted.last().copied().unwrap_or(0.0),
            count: sorted.len(),
        }
    }
}

/// Frame-time percentile summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FramePercentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub min: f64,
    pub max: f64,
    pub count: usize,
}

/// Result of comparing baseline vs. capture-on measurements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverheadResult {
    /// Backend label (e.g. "dxgi", "vulkan", "opengl").
    pub backend: String,
    /// Refresh rate used.
    pub refresh_hz: u32,
    /// Budget threshold in ms.
    pub budget_ms: f64,
    /// Baseline p99 frame time (ms).
    pub baseline_p99: f64,
    /// Capture-on p99 frame time (ms).
    pub capture_p99: f64,
    /// p99 delta (capture - baseline) in ms.
    pub delta_p99: f64,
    /// Whether the delta is within budget.
    pub within_budget: bool,
    /// Baseline percentiles.
    pub baseline: FramePercentiles,
    /// Capture-on percentiles.
    pub capture: FramePercentiles,
}

impl OverheadResult {
    /// Evaluate overhead from baseline and capture sample sets.
    pub fn evaluate(baseline: &SampleSet, capture: &SampleSet, backend: impl Into<String>) -> Self {
        let b = baseline.percentiles();
        let c = capture.percentiles();
        let budget_ms = OverheadBudget::max_delta_ms(capture.refresh_hz);
        let delta = c.p99 - b.p99;
        Self {
            backend: backend.into(),
            refresh_hz: capture.refresh_hz,
            budget_ms,
            baseline_p99: b.p99,
            capture_p99: c.p99,
            delta_p99: delta,
            within_budget: delta <= budget_ms,
            baseline: b,
            capture: c,
        }
    }
}

/// Full benchmark report for all backends and refresh rates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// Timestamp (ISO 8601).
    pub timestamp: String,
    /// Hostname / runner ID.
    pub host: String,
    /// Results per backend per refresh rate.
    pub results: Vec<OverheadResult>,
}

impl BenchmarkReport {
    /// Check whether all results pass the budget.
    pub fn all_within_budget(&self) -> bool {
        self.results.iter().all(|r| r.within_budget)
    }

    /// Return a list of failing backends (if any).
    pub fn failures(&self) -> Vec<&OverheadResult> {
        self.results.iter().filter(|r| !r.within_budget).collect()
    }

    /// Format a compact CI summary line.
    pub fn ci_summary(&self) -> String {
        let total = self.results.len();
        let passed = self.results.iter().filter(|r| r.within_budget).count();
        if self.all_within_budget() {
            format!("G5: {passed}/{total} backends within budget OK")
        } else {
            let failing: Vec<String> = self
                .failures()
                .iter()
                .map(|r| {
                    format!(
                        "{}@{}Hz: Δp99={:.3}ms (budget {:.3}ms)",
                        r.backend, r.refresh_hz, r.delta_p99, r.budget_ms
                    )
                })
                .collect();
            format!(
                "G5: {passed}/{total} within budget FAIL — {}",
                failing.join(", ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_samples(label: &str, hz: u32, deltas: Vec<f64>) -> SampleSet {
        let mut s = SampleSet::new(label, hz);
        for d in deltas {
            s.push(d);
        }
        s
    }

    #[test]
    fn percentile_empty_set() {
        let s = SampleSet::new("empty", 60);
        let p = s.percentiles();
        assert_eq!(p.p50, 0.0);
        assert_eq!(p.p99, 0.0);
        assert_eq!(p.count, 0);
    }

    #[test]
    fn percentile_single_value() {
        let s = make_samples("single", 60, vec![1.5]);
        let p = s.percentiles();
        assert!((p.p50 - 1.5).abs() < 1e-9);
        assert!((p.p99 - 1.5).abs() < 1e-9);
        assert_eq!(p.count, 1);
    }

    #[test]
    fn percentile_uniform_distribution() {
        // 100 values: 1.0, 1.1, 1.2, ..., 10.9
        let vals: Vec<f64> = (0..100).map(|i| 1.0 + i as f64 * 0.1).collect();
        let s = make_samples("uniform", 60, vals);
        let p = s.percentiles();
        // Linear interpolation: p50 at rank 49.5 → 5.9 + 0.5*0.1 = 5.95
        assert!((p.p50 - 5.95).abs() < 0.01, "p50 = {}", p.p50);
        // p99 at rank 98.01 → ~10.801
        assert!((p.p99 - 10.801).abs() < 0.02, "p99 = {}", p.p99);
        assert!((p.min - 1.0).abs() < 1e-9);
        assert!((p.max - 10.9).abs() < 1e-9);
    }

    #[test]
    fn budget_60hz() {
        assert!((OverheadBudget::max_delta_ms(60) - 0.17).abs() < 1e-9);
    }

    #[test]
    fn budget_120hz() {
        assert!((OverheadBudget::max_delta_ms(120) - 0.08).abs() < 1e-9);
    }

    #[test]
    fn budget_144hz() {
        assert!((OverheadBudget::max_delta_ms(144) - 0.07).abs() < 1e-9);
    }

    #[test]
    fn budget_interpolation() {
        // 90 Hz should use the 61-120 bucket
        assert!((OverheadBudget::max_delta_ms(90) - 0.08).abs() < 1e-9);
    }

    #[test]
    fn overhead_within_budget() {
        // Baseline ~16.67ms (60 Hz), capture adds 0.10ms — within 0.17ms budget
        let baseline = make_samples("baseline", 60, vec![16.67; 200]);
        let capture = make_samples("dxgi", 60, vec![16.77; 200]);
        let result = OverheadResult::evaluate(&baseline, &capture, "dxgi");
        assert!(result.within_budget, "delta={:.4}ms", result.delta_p99);
    }

    #[test]
    fn overhead_exceeds_budget() {
        // Baseline ~16.67ms (60 Hz), capture adds 0.25ms — exceeds 0.17ms budget
        let baseline = make_samples("baseline", 60, vec![16.67; 200]);
        let capture = make_samples("dxgi", 60, vec![16.92; 200]);
        let result = OverheadResult::evaluate(&baseline, &capture, "dxgi");
        assert!(!result.within_budget, "delta={:.4}ms", result.delta_p99);
    }

    #[test]
    fn report_all_pass() {
        let report = BenchmarkReport {
            timestamp: "2025-01-01T00:00:00Z".into(),
            host: "ci-runner".into(),
            results: vec![OverheadResult {
                backend: "dxgi".into(),
                refresh_hz: 60,
                budget_ms: 0.17,
                baseline_p99: 16.67,
                capture_p99: 16.75,
                delta_p99: 0.08,
                within_budget: true,
                baseline: FramePercentiles {
                    p50: 16.6,
                    p95: 16.65,
                    p99: 16.67,
                    min: 16.5,
                    max: 16.8,
                    count: 200,
                },
                capture: FramePercentiles {
                    p50: 16.7,
                    p95: 16.73,
                    p99: 16.75,
                    min: 16.6,
                    max: 16.9,
                    count: 200,
                },
            }],
        };
        assert!(report.all_within_budget());
        assert!(report.failures().is_empty());
        assert!(report.ci_summary().contains("OK"));
    }

    #[test]
    fn report_one_failure() {
        let report = BenchmarkReport {
            timestamp: "2025-01-01T00:00:00Z".into(),
            host: "ci-runner".into(),
            results: vec![
                OverheadResult {
                    backend: "dxgi".into(),
                    refresh_hz: 60,
                    budget_ms: 0.17,
                    baseline_p99: 16.67,
                    capture_p99: 16.75,
                    delta_p99: 0.08,
                    within_budget: true,
                    baseline: FramePercentiles {
                        p50: 16.6,
                        p95: 16.65,
                        p99: 16.67,
                        min: 16.5,
                        max: 16.8,
                        count: 200,
                    },
                    capture: FramePercentiles {
                        p50: 16.7,
                        p95: 16.73,
                        p99: 16.75,
                        min: 16.6,
                        max: 16.9,
                        count: 200,
                    },
                },
                OverheadResult {
                    backend: "vulkan".into(),
                    refresh_hz: 60,
                    budget_ms: 0.17,
                    baseline_p99: 16.67,
                    capture_p99: 16.95,
                    delta_p99: 0.28,
                    within_budget: false,
                    baseline: FramePercentiles {
                        p50: 16.6,
                        p95: 16.65,
                        p99: 16.67,
                        min: 16.5,
                        max: 16.8,
                        count: 200,
                    },
                    capture: FramePercentiles {
                        p50: 16.8,
                        p95: 16.9,
                        p99: 16.95,
                        min: 16.6,
                        max: 17.1,
                        count: 200,
                    },
                },
            ],
        };
        assert!(!report.all_within_budget());
        assert_eq!(report.failures().len(), 1);
        assert!(report.failures()[0].backend == "vulkan");
        assert!(report.ci_summary().contains("FAIL"));
        assert!(report.ci_summary().contains("vulkan@60Hz"));
    }

    #[test]
    fn ci_summary_format() {
        let report = BenchmarkReport {
            timestamp: "2025-01-01T00:00:00Z".into(),
            host: "ci-runner".into(),
            results: vec![],
        };
        let summary = report.ci_summary();
        assert!(summary.contains("3/3") || summary.contains("0/0"));
    }
}
