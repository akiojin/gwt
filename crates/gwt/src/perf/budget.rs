//! Normative performance budgets (SPEC #3700 FR-005).
//!
//! The budget values live here, in the performance domain, and are the single
//! source of truth. [`gwt_config::PerfBudgetOverrides`] keeps every field
//! `Option` precisely so that configuration never carries a second copy of the
//! defaults: `None` resolves to the constant below.

use gwt_config::PerfBudgetOverrides;

/// RAIL "response" budget for a single UI interaction, in milliseconds.
pub const DEFAULT_UI_RESPONSE_BUDGET_MS: f64 = 100.0;

/// 60fps frame budget, in milliseconds.
pub const DEFAULT_FRAME_BUDGET_MS: f64 = 16.0;

/// p95 budget for read-only gwtd operations, in milliseconds.
pub const DEFAULT_GWTD_READ_P95_BUDGET_MS: f64 = 100.0;

/// p95 budget for mutating gwtd operations, in milliseconds.
pub const DEFAULT_GWTD_MUTATION_P95_BUDGET_MS: f64 = 500.0;

/// The resolved budget set for one process.
///
/// Every field is a finite, strictly positive millisecond ceiling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerfBudgets {
    /// Maximum UI interaction response time.
    pub ui_response_ms: f64,
    /// Maximum rendered-frame duration.
    pub frame_ms: f64,
    /// Maximum p95 duration for read-only gwtd operations.
    pub gwtd_read_p95_ms: f64,
    /// Maximum p95 duration for mutating gwtd operations.
    pub gwtd_mutation_p95_ms: f64,
}

impl Default for PerfBudgets {
    fn default() -> Self {
        Self {
            ui_response_ms: DEFAULT_UI_RESPONSE_BUDGET_MS,
            frame_ms: DEFAULT_FRAME_BUDGET_MS,
            gwtd_read_p95_ms: DEFAULT_GWTD_READ_P95_BUDGET_MS,
            gwtd_mutation_p95_ms: DEFAULT_GWTD_MUTATION_P95_BUDGET_MS,
        }
    }
}

impl PerfBudgets {
    /// Resolve the effective budgets from configuration.
    ///
    /// An override is honoured only when it is a finite, strictly positive
    /// number of milliseconds; anything else keeps the normative default so a
    /// malformed `config.toml` cannot silently disable budget enforcement.
    pub fn resolve(overrides: &PerfBudgetOverrides) -> Self {
        let defaults = Self::default();
        Self {
            ui_response_ms: sanitize(overrides.ui_response_ms, defaults.ui_response_ms),
            frame_ms: sanitize(overrides.frame_ms, defaults.frame_ms),
            gwtd_read_p95_ms: sanitize(overrides.gwtd_read_p95_ms, defaults.gwtd_read_p95_ms),
            gwtd_mutation_p95_ms: sanitize(
                overrides.gwtd_mutation_p95_ms,
                defaults.gwtd_mutation_p95_ms,
            ),
        }
    }

    /// Budget for a gwtd operation, chosen by its read-only classification.
    pub fn for_operation(&self, read_only: bool) -> f64 {
        if read_only {
            self.gwtd_read_p95_ms
        } else {
            self.gwtd_mutation_p95_ms
        }
    }
}

fn sanitize(candidate: Option<f64>, fallback: f64) -> f64 {
    candidate
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_overrides_resolve_to_the_normative_fr_005_defaults() {
        let budgets = PerfBudgets::resolve(&PerfBudgetOverrides::default());

        assert_eq!(budgets.ui_response_ms, 100.0);
        assert_eq!(budgets.frame_ms, 16.0);
        assert_eq!(budgets.gwtd_read_p95_ms, 100.0);
        assert_eq!(budgets.gwtd_mutation_p95_ms, 500.0);
        assert_eq!(budgets, PerfBudgets::default());
    }

    #[test]
    fn present_overrides_replace_only_the_fields_they_set() {
        let budgets = PerfBudgets::resolve(&PerfBudgetOverrides {
            ui_response_ms: Some(50.0),
            gwtd_mutation_p95_ms: Some(750.0),
            ..PerfBudgetOverrides::default()
        });

        assert_eq!(budgets.ui_response_ms, 50.0);
        assert_eq!(budgets.gwtd_mutation_p95_ms, 750.0);
        assert_eq!(budgets.frame_ms, DEFAULT_FRAME_BUDGET_MS);
        assert_eq!(budgets.gwtd_read_p95_ms, DEFAULT_GWTD_READ_P95_BUDGET_MS);
    }

    #[test]
    fn malformed_overrides_fall_back_to_the_normative_default() {
        for malformed in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let budgets = PerfBudgets::resolve(&PerfBudgetOverrides {
                ui_response_ms: Some(malformed),
                ..PerfBudgetOverrides::default()
            });
            assert_eq!(
                budgets.ui_response_ms, DEFAULT_UI_RESPONSE_BUDGET_MS,
                "override {malformed} must not disable the budget"
            );
        }
    }

    #[test]
    fn operation_budget_follows_the_read_only_classification() {
        let budgets = PerfBudgets::default();

        assert_eq!(budgets.for_operation(true), DEFAULT_GWTD_READ_P95_BUDGET_MS);
        assert_eq!(
            budgets.for_operation(false),
            DEFAULT_GWTD_MUTATION_P95_BUDGET_MS
        );
    }
}
