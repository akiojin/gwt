//! Board provider configuration (SPEC-2959).
//!
//! Selects which backend serves the coordination Board. `local` is the
//! default and the only implemented provider; `slack` / `teams` are reserved
//! for a future adapter (Issue #2960) and currently fall back to `local`.

use std::collections::BTreeMap;
use std::path::Path;

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

/// File name of the per-project Board config under `<repo>/.gwt/work/`.
pub const PROJECT_BOARD_FILE: &str = "board.toml";

/// Per-project Board provider override (SPEC-2963 FR-025..FR-032).
///
/// Persisted as a git-tracked `<repo>/.gwt/work/board.toml`, so the channel
/// binding and provider choice travel with the repo and are shared by the whole
/// team / every machine / every agent. Absent fields inherit the global
/// `[board]` config (precedence: project board.toml → global `[board]` →
/// local). Tokens are **never** stored here — they live machine-local in the
/// token store, keyed by tenant (FR-029).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct ProjectBoardConfig {
    /// Per-project provider. `None` inherits the global provider (FR-028/FR-031).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<BoardProviderKind>,
    /// Channel this project's Board posts/reads use (Slack channel id, or Teams
    /// `team_id/channel_id`). `None` inherits the global default channel
    /// (FR-025/FR-027).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Tenant the channel belongs to (Slack `team_id` / Teams `tenant_id`), used
    /// to key the machine-local OAuth token so projects in different tenants
    /// authenticate independently (FR-029).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

impl ProjectBoardConfig {
    /// Load `<work_dir>/board.toml`. A missing or unparseable file yields the
    /// empty config (inherit global), mirroring `Settings::load`'s
    /// default-on-error behaviour so a stray file never breaks Board loading.
    pub fn load_from_work_dir(work_dir: &Path) -> Self {
        let path = work_dir.join(PROJECT_BOARD_FILE);
        match std::fs::read_to_string(&path) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Persist to `<work_dir>/board.toml`, creating the directory if needed.
    pub fn save_to_work_dir(&self, work_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(work_dir)?;
        let path = work_dir.join(PROJECT_BOARD_FILE);
        let body = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, body)
    }

    /// Whether the project sets nothing (so the global config fully applies).
    pub fn is_empty(&self) -> bool {
        self.provider.is_none() && self.channel.is_none() && self.tenant.is_none()
    }
}

/// Default fixed loopback port for the OAuth redirect callback (SPEC-2963).
/// OAuth providers require the redirect_uri to exactly match a pre-registered
/// URL, so the callback must use a stable port rather than the embedded
/// server's ephemeral port.
pub const DEFAULT_OAUTH_REDIRECT_PORT: u16 = 8765;

fn default_oauth_redirect_port() -> u16 {
    DEFAULT_OAUTH_REDIRECT_PORT
}

/// Board configuration block, persisted under `[board]` in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BoardConfig {
    /// Selected Board provider. Defaults to `local`.
    pub provider: BoardProviderKind,
    /// Non-secret Slack settings (SPEC-2963).
    pub slack: SlackConfig,
    /// Non-secret Teams settings (SPEC-2963).
    pub teams: TeamsConfig,
    /// Fixed loopback port for the OAuth redirect callback. The sign-in
    /// redirect_uri is `http://127.0.0.1:<port>/oauth/callback` and must match
    /// the redirect URL registered in the Slack/Teams app. Defaults to 8765;
    /// configurable from Settings so a user whose 8765 is busy can pick
    /// another port (and register the matching URL).
    #[serde(default = "default_oauth_redirect_port")]
    pub oauth_redirect_port: u16,
}

impl Default for BoardConfig {
    fn default() -> Self {
        Self {
            provider: BoardProviderKind::default(),
            slack: SlackConfig::default(),
            teams: TeamsConfig::default(),
            oauth_redirect_port: default_oauth_redirect_port(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_provider_is_local() {
        assert_eq!(BoardConfig::default().provider, BoardProviderKind::Local);
    }

    #[test]
    fn default_oauth_redirect_port_is_8765() {
        assert_eq!(BoardConfig::default().oauth_redirect_port, 8765);
        // Missing from a partial [board] table still defaults to 8765.
        let cfg: BoardConfig = toml::from_str("provider = \"slack\"").unwrap();
        assert_eq!(cfg.oauth_redirect_port, 8765);
    }

    #[test]
    fn oauth_redirect_port_roundtrips_when_customized() {
        let cfg = BoardConfig {
            oauth_redirect_port: 9123,
            ..Default::default()
        };
        let serialized = toml::to_string(&cfg).unwrap();
        let restored: BoardConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(restored.oauth_redirect_port, 9123);
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

    #[test]
    fn project_board_config_missing_file_is_empty_inherit_global() {
        // FR-031: a repo with no board.toml inherits the global config.
        let dir = tempfile::tempdir().unwrap();
        let cfg = ProjectBoardConfig::load_from_work_dir(dir.path());
        assert!(cfg.is_empty());
        assert!(cfg.provider.is_none());
        assert!(cfg.channel.is_none());
        assert!(cfg.tenant.is_none());
    }

    #[test]
    fn project_board_config_roundtrips_through_work_dir() {
        // FR-025: provider + channel + tenant persist to <repo>/.gwt/work/board.toml.
        let dir = tempfile::tempdir().unwrap();
        let cfg = ProjectBoardConfig {
            provider: Some(BoardProviderKind::Slack),
            channel: Some("C-PROJ-A".to_string()),
            tenant: Some("T-ACME".to_string()),
        };
        cfg.save_to_work_dir(dir.path()).unwrap();
        assert!(dir.path().join(PROJECT_BOARD_FILE).exists());
        let restored = ProjectBoardConfig::load_from_work_dir(dir.path());
        assert_eq!(restored, cfg);
        assert!(!restored.is_empty());
    }

    #[test]
    fn project_board_config_partial_only_channel() {
        // Absent provider/tenant stay None so they inherit global.
        let toml_src = "channel = \"C-ONLY\"\n";
        let cfg: ProjectBoardConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.channel.as_deref(), Some("C-ONLY"));
        assert!(cfg.provider.is_none());
        assert!(cfg.tenant.is_none());
    }

    #[test]
    fn project_board_config_per_project_provider_kinds() {
        // FR-028: each project can pick a different provider kind.
        for kind in [
            BoardProviderKind::Local,
            BoardProviderKind::Slack,
            BoardProviderKind::Teams,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let cfg = ProjectBoardConfig {
                provider: Some(kind),
                ..Default::default()
            };
            cfg.save_to_work_dir(dir.path()).unwrap();
            let restored = ProjectBoardConfig::load_from_work_dir(dir.path());
            assert_eq!(
                restored.provider,
                Some(kind),
                "round-trip failed for {kind:?}"
            );
        }
    }

    #[test]
    fn project_board_config_unparseable_falls_back_to_empty() {
        // A stray/corrupt board.toml never breaks Board loading (inherit global).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(PROJECT_BOARD_FILE), "this is = not [valid").unwrap();
        let cfg = ProjectBoardConfig::load_from_work_dir(dir.path());
        assert!(cfg.is_empty());
    }
}
