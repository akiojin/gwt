//! Self-budget governor for the collector itself (SPEC #3700 FR-009).
//!
//! Issue #3264 is the reason this exists: monitoring that saturated the GUI
//! event loop became the perf bug it was meant to find. The collector therefore
//! measures its own cost and thins its own sampling rate when it exceeds the
//! configured share of CPU.
//!
//! The governor is clock-free on purpose — the caller supplies both the cost it
//! spent collecting and the wall-clock time that elapsed — so the control law
//! is deterministic and directly testable.

use std::time::Duration;

/// Share of a single core the collector may consume before it thins sampling.
pub const DEFAULT_SELF_BUDGET_CPU_PERCENT: f64 = 1.0;

/// Longest interval the governor accumulates before it re-evaluates.
pub const DEFAULT_EVALUATION_WINDOW: Duration = Duration::from_secs(5);

/// Hardest thinning the governor will apply: one sample in 64.
pub const MAX_SAMPLING_DIVISOR: u32 = 64;

/// Sampling-rate controller keeping collection inside its own CPU budget.
#[derive(Debug, Clone)]
pub struct SelfBudgetGovernor {
    limit_percent: f64,
    window: Duration,
    divisor: u32,
    seen: u64,
    spent: Duration,
    elapsed: Duration,
}

impl Default for SelfBudgetGovernor {
    fn default() -> Self {
        Self::new(DEFAULT_SELF_BUDGET_CPU_PERCENT)
    }
}

impl SelfBudgetGovernor {
    /// Build a governor for the configured CPU share.
    ///
    /// A non-finite or non-positive limit falls back to the default, so a
    /// malformed `config.toml` cannot pin sampling at its hardest thinning.
    pub fn new(limit_percent: f64) -> Self {
        Self::with_window(limit_percent, DEFAULT_EVALUATION_WINDOW)
    }

    /// Build a governor with an explicit evaluation window.
    pub fn with_window(limit_percent: f64, window: Duration) -> Self {
        Self {
            limit_percent: if limit_percent.is_finite() && limit_percent > 0.0 {
                limit_percent
            } else {
                DEFAULT_SELF_BUDGET_CPU_PERCENT
            },
            window: if window.is_zero() {
                DEFAULT_EVALUATION_WINDOW
            } else {
                window
            },
            divisor: 1,
            seen: 0,
            spent: Duration::ZERO,
            elapsed: Duration::ZERO,
        }
    }

    /// Current thinning factor: one sample in `sampling_divisor()` is kept.
    pub fn sampling_divisor(&self) -> u32 {
        self.divisor
    }

    /// Decide whether the next measurement should be persisted.
    ///
    /// The first candidate of every thinned group is kept, so a divisor of 1
    /// admits everything and a raised divisor still yields a representative
    /// series rather than a gap.
    pub fn should_sample(&mut self) -> bool {
        let admitted = self.seen.is_multiple_of(u64::from(self.divisor));
        self.seen = self.seen.wrapping_add(1);
        admitted
    }

    /// Report the cost of one collection and the wall time it covered.
    ///
    /// Once the accumulated wall time reaches the evaluation window, the
    /// collector's CPU share is compared against the limit: over budget doubles
    /// the divisor, comfortably under budget halves it back toward 1.
    pub fn observe_cost(&mut self, cost: Duration, elapsed: Duration) {
        self.spent = self.spent.saturating_add(cost);
        self.elapsed = self.elapsed.saturating_add(elapsed);
        if self.elapsed < self.window {
            return;
        }

        let percent = self.spent.as_secs_f64() / self.elapsed.as_secs_f64() * 100.0;
        if percent > self.limit_percent {
            self.divisor = (self.divisor.saturating_mul(2)).min(MAX_SAMPLING_DIVISOR);
        } else if percent < self.limit_percent / 2.0 {
            self.divisor = (self.divisor / 2).max(1);
        }

        self.spent = Duration::ZERO;
        self.elapsed = Duration::ZERO;
    }

    /// Observed CPU share of the window in progress, as a percentage.
    pub fn current_share_percent(&self) -> f64 {
        if self.elapsed.is_zero() {
            return 0.0;
        }
        self.spent.as_secs_f64() / self.elapsed.as_secs_f64() * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Duration {
        Duration::from_secs(1)
    }

    #[test]
    fn a_fresh_governor_admits_every_sample() {
        let mut governor = SelfBudgetGovernor::default();

        assert_eq!(governor.sampling_divisor(), 1);
        for _ in 0..10 {
            assert!(governor.should_sample());
        }
    }

    #[test]
    fn exceeding_the_cpu_share_thins_sampling() {
        let mut governor = SelfBudgetGovernor::with_window(1.0, window());

        // 5% of the window spent collecting: five times the 1% budget.
        governor.observe_cost(Duration::from_millis(50), window());

        assert_eq!(governor.sampling_divisor(), 2);
        assert!(governor.should_sample());
        assert!(!governor.should_sample());
        assert!(governor.should_sample());
    }

    #[test]
    fn sustained_overage_thins_further_but_never_past_the_cap() {
        let mut governor = SelfBudgetGovernor::with_window(1.0, window());

        for _ in 0..16 {
            governor.observe_cost(Duration::from_millis(500), window());
        }

        assert_eq!(governor.sampling_divisor(), MAX_SAMPLING_DIVISOR);
    }

    #[test]
    fn returning_under_budget_restores_the_full_sampling_rate() {
        let mut governor = SelfBudgetGovernor::with_window(1.0, window());
        governor.observe_cost(Duration::from_millis(50), window());
        governor.observe_cost(Duration::from_millis(50), window());
        assert_eq!(governor.sampling_divisor(), 4);

        for _ in 0..2 {
            governor.observe_cost(Duration::from_micros(10), window());
        }

        assert_eq!(governor.sampling_divisor(), 1);
    }

    #[test]
    fn the_window_must_fill_before_the_rate_changes() {
        let mut governor = SelfBudgetGovernor::with_window(1.0, window());

        governor.observe_cost(Duration::from_millis(400), Duration::from_millis(500));

        assert_eq!(governor.sampling_divisor(), 1);
        assert!((governor.current_share_percent() - 80.0).abs() < 1e-9);
    }

    #[test]
    fn a_malformed_limit_falls_back_to_the_fr_009_default() {
        for malformed in [0.0, -5.0, f64::NAN] {
            let mut governor = SelfBudgetGovernor::with_window(malformed, window());
            // 0.4% of the window is inside the 1% default and outside a 0% limit.
            governor.observe_cost(Duration::from_micros(4_000), window());
            assert_eq!(
                governor.sampling_divisor(),
                1,
                "limit {malformed} must resolve to the 1% default"
            );
        }
    }
}
