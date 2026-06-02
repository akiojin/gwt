//! Provider usage display configuration (SPEC-2970 FR-008/FR-009/FR-013).
//!
//! Controls which provider usage axes gwt collects. Both providers default on.
//! Claude **account** usage reads OAuth credentials (Keychain) and contacts
//! Anthropic; it can be turned off in Settings. Claude per-session usage
//! (local transcript) is not gated here.

use serde::{Deserialize, Serialize};

fn default_enabled() -> bool {
    true
}

/// Usage display configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageConfig {
    /// Collect Codex usage (local rollout files). Defaults on.
    #[serde(default = "default_enabled")]
    pub codex_enabled: bool,
    /// Collect Claude account usage (Keychain + Anthropic request). Defaults
    /// on; can be disabled in Settings.
    #[serde(default = "default_enabled")]
    pub claude_account_enabled: bool,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            codex_enabled: true,
            claude_account_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_both_on() {
        let c = UsageConfig::default();
        assert!(c.codex_enabled);
        assert!(c.claude_account_enabled);
    }

    #[test]
    fn missing_table_uses_defaults() {
        // No [usage] table at all → both default on.
        let c: UsageConfig = toml::from_str("").unwrap();
        assert!(c.codex_enabled);
        assert!(c.claude_account_enabled);
    }

    #[test]
    fn partial_table_keeps_other_default_on() {
        // Only one flag is set; the missing one must stay defaulted on.
        let c: UsageConfig = toml::from_str("claude_account_enabled = false\n").unwrap();
        assert!(c.codex_enabled);
        assert!(!c.claude_account_enabled);
    }

    #[test]
    fn roundtrip() {
        let c = UsageConfig {
            codex_enabled: false,
            claude_account_enabled: true,
        };
        let s = toml::to_string(&c).unwrap();
        let back: UsageConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.codex_enabled, c.codex_enabled);
        assert_eq!(back.claude_account_enabled, c.claude_account_enabled);
    }
}
