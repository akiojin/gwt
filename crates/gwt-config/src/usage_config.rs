//! Provider usage display configuration (SPEC-2970 FR-008/FR-009/FR-013).
//!
//! Controls which provider usage axes gwt collects.
//!
//! - **Codex** usage reads local rollout files only, so it defaults **on**.
//! - **Claude account** usage reads OAuth credentials (Keychain) and contacts
//!   Anthropic, so it is **opt-in** and defaults **off**. Enable it in
//!   Settings → Usage & Limits. Until then the Claude account row shows
//!   `Enable in Settings` and no Keychain read / external request happens.
//! - Claude per-session usage (local transcript) is not gated here.

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
    /// Collect Claude account usage (Keychain + Anthropic request). Opt-in;
    /// defaults off and is enabled from Settings.
    #[serde(default)]
    pub claude_account_enabled: bool,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            codex_enabled: true,
            claude_account_enabled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_defaults_on_claude_defaults_off() {
        // Codex is local-only (auto on); Claude account is opt-in (off).
        let c = UsageConfig::default();
        assert!(c.codex_enabled);
        assert!(!c.claude_account_enabled);
    }

    #[test]
    fn missing_table_uses_defaults() {
        // No [usage] table at all → Codex on, Claude account opt-in (off).
        let c: UsageConfig = toml::from_str("").unwrap();
        assert!(c.codex_enabled);
        assert!(!c.claude_account_enabled);
    }

    #[test]
    fn claude_opt_in_is_honored_when_set() {
        // Explicit opt-in flips Claude on while Codex stays defaulted on.
        let c: UsageConfig = toml::from_str("claude_account_enabled = true\n").unwrap();
        assert!(c.codex_enabled);
        assert!(c.claude_account_enabled);
    }

    #[test]
    fn codex_can_be_disabled_without_enabling_claude() {
        // Disabling Codex must not silently enable the opt-in Claude path.
        let c: UsageConfig = toml::from_str("codex_enabled = false\n").unwrap();
        assert!(!c.codex_enabled);
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
