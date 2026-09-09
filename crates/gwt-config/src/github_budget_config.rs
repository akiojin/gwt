//! GitHub API budget throttle knobs (SPEC #4093 FR-007), persisted under
//! `[github_budget]` in `~/.gwt/config.toml`.
//!
//! The three values shape every non-essential GitHub read gwt makes:
//! how much of a window must stay in reserve, how many local spawns per
//! minute count as a burst, and how old a `rate_limit` probe may be before it
//! says nothing. They are read at each throttle decision, so an edit takes
//! effect on the next spawn without a restart.

use serde::{Deserialize, Serialize};

fn default_reserve_fraction() -> f64 {
    0.2
}

fn default_burst_calls_per_minute() -> u64 {
    60
}

fn default_probe_max_age_secs() -> i64 {
    15 * 60
}

/// `[github_budget]` settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GitHubBudgetConfig {
    /// Fraction of a resource's hourly limit kept for essential calls;
    /// non-essential reads are skipped below it. Clamped to `0.0..=1.0`.
    #[serde(default = "default_reserve_fraction")]
    pub reserve_fraction: f64,
    /// Local budget-spending spawns per minute treated as a secondary-limit
    /// burst. At least 1.
    #[serde(default = "default_burst_calls_per_minute")]
    pub burst_calls_per_minute: u64,
    /// A `rate_limit` probe older than this is re-taken before deciding.
    /// At least 0.
    #[serde(default = "default_probe_max_age_secs")]
    pub probe_max_age_secs: i64,
}

impl Default for GitHubBudgetConfig {
    fn default() -> Self {
        Self {
            reserve_fraction: default_reserve_fraction(),
            burst_calls_per_minute: default_burst_calls_per_minute(),
            probe_max_age_secs: default_probe_max_age_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_table_and_partial_table_fall_back_to_defaults() {
        let settings: crate::Settings = toml::from_str("").expect("empty settings");
        assert_eq!(settings.github_budget, GitHubBudgetConfig::default());

        let settings: crate::Settings =
            toml::from_str("[github_budget]\nburst_calls_per_minute = 30\n").expect("partial");
        assert_eq!(settings.github_budget.burst_calls_per_minute, 30);
        assert_eq!(settings.github_budget.reserve_fraction, 0.2);
        assert_eq!(settings.github_budget.probe_max_age_secs, 900);
    }

    #[test]
    fn settings_roundtrip_keeps_the_three_knobs() {
        let settings = crate::Settings {
            github_budget: GitHubBudgetConfig {
                reserve_fraction: 0.35,
                burst_calls_per_minute: 20,
                probe_max_age_secs: 120,
            },
            ..crate::Settings::default()
        };
        let text = toml::to_string(&settings).expect("serialize");
        let parsed: crate::Settings = toml::from_str(&text).expect("parse");
        assert_eq!(parsed.github_budget, settings.github_budget);
    }
}
