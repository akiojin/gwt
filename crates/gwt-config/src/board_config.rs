//! Board provider configuration (SPEC-2959).
//!
//! Selects which backend serves the coordination Board. `local` is the
//! default and the only implemented provider; `slack` / `teams` are reserved
//! for a future adapter (Issue #2960) and currently fall back to `local`.

use std::collections::BTreeMap;

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

/// Non-secret Slack provider settings (SPEC-2963). OAuth tokens and any
/// client secret live in the token store, never in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct SlackConfig {
    /// OAuth client id (non-secret).
    pub client_id: Option<String>,
    /// Channel id used when a Work has no explicit mapping.
    pub default_channel: Option<String>,
    /// `workspace_id` → Slack channel id mapping (FR-007).
    pub channel_map: BTreeMap<String, String>,
}

/// Non-secret Microsoft Teams provider settings (SPEC-2963).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct TeamsConfig {
    /// OAuth client (application) id (non-secret).
    pub client_id: Option<String>,
    /// Entra tenant id (or `organizations` / `common`).
    pub tenant_id: Option<String>,
    /// Default `team_id/channel_id` target when a Work has no mapping.
    pub default_channel: Option<String>,
    /// `workspace_id` → `team_id/channel_id` mapping (FR-007).
    pub channel_map: BTreeMap<String, String>,
}

/// Board configuration block, persisted under `[board]` in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BoardConfig {
    /// Selected Board provider. Defaults to `local`.
    pub provider: BoardProviderKind,
    /// Non-secret Slack settings (SPEC-2963).
    pub slack: SlackConfig,
    /// Non-secret Teams settings (SPEC-2963).
    pub teams: TeamsConfig,
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
            let cfg = BoardConfig {
                provider: kind,
                ..Default::default()
            };
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

    #[test]
    fn board_config_defaults_have_empty_remote_settings() {
        let cfg = BoardConfig::default();
        assert!(cfg.slack.client_id.is_none());
        assert!(cfg.slack.channel_map.is_empty());
        assert!(cfg.teams.client_id.is_none());
        assert!(cfg.teams.channel_map.is_empty());
    }

    #[test]
    fn board_config_carries_slack_teams_non_secret_settings() {
        let toml_src = r#"
provider = "slack"

[slack]
client_id = "C123"
default_channel = "CHGEN"
channel_map = { "ws-a" = "CH-A" }

[teams]
client_id = "T123"
tenant_id = "tenant-1"
"#;
        let cfg: BoardConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.provider, BoardProviderKind::Slack);
        assert_eq!(cfg.slack.client_id.as_deref(), Some("C123"));
        assert_eq!(cfg.slack.default_channel.as_deref(), Some("CHGEN"));
        assert_eq!(
            cfg.slack.channel_map.get("ws-a").map(String::as_str),
            Some("CH-A"),
        );
        assert_eq!(cfg.teams.client_id.as_deref(), Some("T123"));
        assert_eq!(cfg.teams.tenant_id.as_deref(), Some("tenant-1"));

        // Round-trip preserves the non-secret settings.
        let serialized = toml::to_string(&cfg).unwrap();
        let restored: BoardConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(restored.slack, cfg.slack);
        assert_eq!(restored.teams, cfg.teams);
    }
}
