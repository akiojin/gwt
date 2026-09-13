//! Budget-violation smoothing (SPEC #3700 FR-006).
//!
//! A single slow sample is noise: a GC pause, a cold page cache, another agent
//! saturating the host. A violation is only worth recording when the overage
//! persists, so this tracks per-target runs of over-budget samples and emits at
//! most one violation per run.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use super::record::PerfViolationDetails;

/// Consecutive over-budget samples that constitute a sustained violation.
pub const DEFAULT_VIOLATION_CONSECUTIVE_THRESHOLD: u32 = 3;

/// Wall-clock seconds of continuous overage that constitute a sustained
/// violation even when fewer samples arrived.
pub const DEFAULT_VIOLATION_SUSTAINED_SECONDS: f64 = 2.0;

/// Upper bound on tracked targets, so a pathological target cardinality cannot
/// grow the smoother without limit.
const MAX_TRACKED_TARGETS: usize = 512;

#[derive(Debug, Clone, Copy)]
struct RunState {
    consecutive: u32,
    first_over: DateTime<Utc>,
}

/// Per-target state machine turning raw over-budget samples into violations.
#[derive(Debug, Clone)]
pub struct ViolationSmoother {
    consecutive_threshold: u32,
    sustained_seconds: f64,
    runs: HashMap<String, RunState>,
}

impl Default for ViolationSmoother {
    fn default() -> Self {
        Self::new()
    }
}

impl ViolationSmoother {
    /// Build a smoother with the FR-006 defaults.
    pub fn new() -> Self {
        Self::with_thresholds(
            DEFAULT_VIOLATION_CONSECUTIVE_THRESHOLD,
            DEFAULT_VIOLATION_SUSTAINED_SECONDS,
        )
    }

    /// Build a smoother with explicit thresholds.
    ///
    /// A zero or non-finite threshold falls back to the default, so a smoother
    /// can never be configured into emitting a violation per spike.
    pub fn with_thresholds(consecutive_threshold: u32, sustained_seconds: f64) -> Self {
        Self {
            consecutive_threshold: if consecutive_threshold == 0 {
                DEFAULT_VIOLATION_CONSECUTIVE_THRESHOLD
            } else {
                consecutive_threshold
            },
            sustained_seconds: if sustained_seconds.is_finite() && sustained_seconds > 0.0 {
                sustained_seconds
            } else {
                DEFAULT_VIOLATION_SUSTAINED_SECONDS
            },
            runs: HashMap::new(),
        }
    }

    /// Feed one sample in.
    ///
    /// Returns the violation evidence when this sample completes a sustained
    /// run, and `None` otherwise. The run is reset on emission, so one
    /// continuous overage produces exactly one violation record.
    pub fn observe(
        &mut self,
        target: &str,
        value: f64,
        budget: f64,
        at: DateTime<Utc>,
    ) -> Option<PerfViolationDetails> {
        if !value.is_finite() || !budget.is_finite() || value <= budget {
            self.runs.remove(target);
            return None;
        }

        let run = match self.runs.get_mut(target) {
            Some(existing) => {
                existing.consecutive = existing.consecutive.saturating_add(1);
                *existing
            }
            None => {
                if self.runs.len() >= MAX_TRACKED_TARGETS {
                    self.runs.clear();
                }
                let fresh = RunState {
                    consecutive: 1,
                    first_over: at,
                };
                self.runs.insert(target.to_string(), fresh);
                fresh
            }
        };

        let duration_seconds = (at - run.first_over)
            .to_std()
            .map(|elapsed| elapsed.as_secs_f64())
            .unwrap_or(0.0);

        if run.consecutive >= self.consecutive_threshold
            || duration_seconds >= self.sustained_seconds
        {
            self.runs.remove(target);
            return Some(PerfViolationDetails::new(
                budget,
                run.consecutive,
                duration_seconds,
            ));
        }

        None
    }

    /// Number of targets currently inside an over-budget run.
    pub fn tracked_targets(&self) -> usize {
        self.runs.len()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone as _};

    use super::*;

    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 8, 10, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    #[test]
    fn a_single_spike_is_not_a_violation() {
        let mut smoother = ViolationSmoother::new();

        assert!(smoother
            .observe("route:project.switch", 250.0, 100.0, base())
            .is_none());
        assert_eq!(smoother.tracked_targets(), 1);
    }

    #[test]
    fn three_consecutive_overages_emit_one_violation_and_reset_the_run() {
        let mut smoother = ViolationSmoother::new();
        let at = base();

        assert!(smoother
            .observe("route:project.switch", 250.0, 100.0, at)
            .is_none());
        assert!(smoother
            .observe(
                "route:project.switch",
                260.0,
                100.0,
                at + Duration::milliseconds(10)
            )
            .is_none());
        let details = smoother
            .observe(
                "route:project.switch",
                270.0,
                100.0,
                at + Duration::milliseconds(20),
            )
            .expect("third consecutive overage is a violation");

        assert_eq!(details.budget(), 100.0);
        assert_eq!(details.consecutive_count(), 3);
        assert!(details.duration_seconds() >= 0.02);
        assert_eq!(
            smoother.tracked_targets(),
            0,
            "the run must reset so one overage emits one violation"
        );
    }

    #[test]
    fn a_sustained_overage_emits_before_the_consecutive_threshold_is_reached() {
        let mut smoother = ViolationSmoother::new();
        let at = base();

        assert!(smoother
            .observe("route:search", 5_000.0, 2_000.0, at)
            .is_none());
        let details = smoother
            .observe("route:search", 5_100.0, 2_000.0, at + Duration::seconds(3))
            .expect("three seconds of continuous overage is a violation");

        assert_eq!(details.consecutive_count(), 2);
        assert!((details.duration_seconds() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn an_in_budget_sample_clears_the_run() {
        let mut smoother = ViolationSmoother::new();
        let at = base();

        smoother.observe("route:pane.close", 250.0, 100.0, at);
        smoother.observe(
            "route:pane.close",
            260.0,
            100.0,
            at + Duration::milliseconds(10),
        );
        assert!(smoother
            .observe(
                "route:pane.close",
                10.0,
                100.0,
                at + Duration::milliseconds(20)
            )
            .is_none());
        assert_eq!(smoother.tracked_targets(), 0);

        assert!(smoother
            .observe(
                "route:pane.close",
                250.0,
                100.0,
                at + Duration::milliseconds(30)
            )
            .is_none());
        assert!(smoother
            .observe(
                "route:pane.close",
                250.0,
                100.0,
                at + Duration::milliseconds(40)
            )
            .is_none());
    }

    #[test]
    fn runs_are_tracked_per_target() {
        let mut smoother = ViolationSmoother::new();
        let at = base();

        for step in 0..2 {
            let at = at + Duration::milliseconds(step * 10);
            assert!(smoother.observe("route:a", 250.0, 100.0, at).is_none());
            assert!(smoother.observe("route:b", 250.0, 100.0, at).is_none());
        }

        assert!(smoother
            .observe("route:a", 250.0, 100.0, at + Duration::milliseconds(20))
            .is_some());
        assert_eq!(
            smoother.tracked_targets(),
            1,
            "route:b must keep its own independent run"
        );
    }

    #[test]
    fn degenerate_thresholds_fall_back_to_the_fr_006_defaults() {
        let smoother = ViolationSmoother::with_thresholds(0, f64::NAN);
        assert_eq!(
            smoother.consecutive_threshold,
            DEFAULT_VIOLATION_CONSECUTIVE_THRESHOLD
        );
        assert_eq!(
            smoother.sustained_seconds,
            DEFAULT_VIOLATION_SUSTAINED_SECONDS
        );
    }
}
