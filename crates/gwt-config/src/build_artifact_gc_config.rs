//! Automatic build-artifact reclaim knobs (Issue #4391), persisted under
//! `[build_artifact_gc]` in `~/.gwt/config.toml`.
//!
//! The same thresholds drive the `disk_space` warning in
//! `issue.monitor.status` and the automatic `worktree.gc_build_artifacts`
//! sweep, so the warning and the reclaim can never disagree about what "low"
//! means. They are read at each decision, so an edit takes effect on the next
//! Issue Monitor scan without a restart.

use serde::{Deserialize, Serialize};

/// 20 GiB: about one cold workspace rebuild, the amount a single blocked
/// agent needs to recover on its own (Issue #4009 AC-4).
pub const DEFAULT_BELOW_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// 5%: catches a large volume that still clears the byte floor but is close
/// enough to full that one more fleet of builds fills it (Issue #4009 AC-4).
pub const DEFAULT_BELOW_PERCENT: u64 = 5;

fn default_auto() -> bool {
    // On by default. The sweep only removes `target/` from worktrees that are
    // merged and idle — a rebuild cost, never lost work — while the failure it
    // prevents (a full disk failing every `verify.run` on the host) stops the
    // whole fleet. Off by default is how 749 GB accumulated in the first place.
    true
}

fn default_below_bytes() -> u64 {
    DEFAULT_BELOW_BYTES
}

fn default_below_percent() -> u64 {
    DEFAULT_BELOW_PERCENT
}

/// `[build_artifact_gc]` settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildArtifactGcConfig {
    /// Run the reclaim automatically while free space is below a threshold.
    #[serde(default = "default_auto")]
    pub auto: bool,
    /// A volume with fewer free bytes than this is low.
    #[serde(default = "default_below_bytes")]
    pub below_bytes: u64,
    /// A volume with a smaller free percentage than this is low.
    #[serde(default = "default_below_percent")]
    pub below_percent: u64,
}

impl Default for BuildArtifactGcConfig {
    fn default() -> Self {
        Self {
            auto: default_auto(),
            below_bytes: default_below_bytes(),
            below_percent: default_below_percent(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #4391 AC-4 / AC-5: a missing table keeps the documented
    /// defaults, and each knob can be set on its own.
    #[test]
    fn missing_table_and_partial_table_fall_back_to_defaults() {
        let settings: crate::Settings = toml::from_str("").expect("empty settings");
        assert_eq!(settings.build_artifact_gc, BuildArtifactGcConfig::default());
        assert!(settings.build_artifact_gc.auto);
        assert_eq!(settings.build_artifact_gc.below_bytes, DEFAULT_BELOW_BYTES);
        assert_eq!(
            settings.build_artifact_gc.below_percent,
            DEFAULT_BELOW_PERCENT
        );

        let settings: crate::Settings =
            toml::from_str("[build_artifact_gc]\nauto = false\nbelow_percent = 10\n")
                .expect("partial");
        assert!(!settings.build_artifact_gc.auto);
        assert_eq!(settings.build_artifact_gc.below_percent, 10);
        assert_eq!(settings.build_artifact_gc.below_bytes, DEFAULT_BELOW_BYTES);
    }
}
