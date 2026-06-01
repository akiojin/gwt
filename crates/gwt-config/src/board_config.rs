//! Board provider configuration (SPEC-2959).
//!
//! Selects which backend serves the coordination Board. `local` is the
//! default and the only implemented provider; `slack` / `teams` are reserved
//! for a future adapter (Issue #2960) and currently fall back to `local`.

use serde::{Deserialize, Deserializer, Serialize};

/// Which backend serves the Board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BoardProviderKind {
    /// Filesystem-backed local provider (offline, default).
    #[default]
    Local,
    /// Slack-backed provider (not yet implemented — falls back to local).
    Slack,
    /// Microsoft Teams-backed provider (not yet implemented — falls back to local).
    Teams,
}

impl BoardProviderKind {
    /// Canonical lowercase identifier used in config and protocol payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            BoardProviderKind::Local => "local",
            BoardProviderKind::Slack => "slack",
            BoardProviderKind::Teams => "teams",
        }
    }

    /// Whether a real adapter exists for this provider today. Only `local`
    /// is implemented; the others resolve to `local` at runtime (FR-004).
    pub fn is_implemented(self) -> bool {
        matches!(self, BoardProviderKind::Local)
    }
}

impl<'de> Deserialize<'de> for BoardProviderKind {
    /// Unknown or missing values fall back to `local` (FR-004) so a stray
    /// config value never breaks Board loading.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.trim().to_ascii_lowercase().as_str() {
            "slack" => BoardProviderKind::Slack,
            "teams" => BoardProviderKind::Teams,
            _ => BoardProviderKind::Local,
        })
    }
}

/// Board configuration block, persisted under `[board]` in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BoardConfig {
    /// Selected Board provider. Defaults to `local`.
    pub provider: BoardProviderKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_provider_is_local() {
        assert_eq!(BoardConfig::default().provider, BoardProviderKind::Local);
    }

    #[test]
    fn unknown_provider_falls_back_to_local() {
        let cfg: BoardConfig = toml::from_str("provider = \"discord\"").unwrap();
        assert_eq!(cfg.provider, BoardProviderKind::Local);
    }

    #[test]
    fn empty_block_defaults_to_local() {
        let cfg: BoardConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.provider, BoardProviderKind::Local);
    }

    #[test]
    fn provider_roundtrips_through_toml() {
        for kind in [
            BoardProviderKind::Local,
            BoardProviderKind::Slack,
            BoardProviderKind::Teams,
        ] {
            let cfg = BoardConfig { provider: kind };
            let serialized = toml::to_string(&cfg).unwrap();
            let restored: BoardConfig = toml::from_str(&serialized).unwrap();
            assert_eq!(restored.provider, kind, "round-trip failed for {kind:?}");
        }
    }

    #[test]
    fn only_local_is_implemented() {
        assert!(BoardProviderKind::Local.is_implemented());
        assert!(!BoardProviderKind::Slack.is_implemented());
        assert!(!BoardProviderKind::Teams.is_implemented());
    }
}
