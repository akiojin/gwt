//! System Settings service + dispatch helpers (SPEC-1933 US-4 / FR-007).
//!
//! Exposes the global `[ai].language` field of `~/.gwt/config.toml` to the
//! frontend's Settings > System tab via two WebSocket events:
//!
//! - [`crate::protocol::FrontendEvent::GetSystemSettings`] →
//!   [`crate::protocol::BackendEvent::SystemSettings`]
//! - [`crate::protocol::FrontendEvent::UpdateSystemSettings`] →
//!   [`crate::protocol::BackendEvent::SystemSettingsUpdated`] /
//!   [`crate::protocol::BackendEvent::SystemSettingsError`]
//!
//! Validation: the only currently-accepted values for `language` are `auto`,
//! `en`, and `ja`. Anything else is rejected with
//! [`SystemSettingsError::InvalidLanguage`] so the frontend dropdown stays
//! the source of truth for allowed values.

use std::path::Path;

use gwt_config::{BoardProviderKind, Settings};

use crate::protocol::BackendEvent;

/// Service-layer error for System Settings operations. Mapped to
/// [`BackendEvent::SystemSettingsError`] in the dispatch layer.
#[derive(Debug, thiserror::Error)]
pub enum SystemSettingsError {
    #[error("invalid language `{0}`: expected `auto`, `en`, or `ja`")]
    InvalidLanguage(String),
    #[error("invalid board provider `{0}`: expected `local`, `slack`, or `teams`")]
    InvalidBoardProvider(String),
    #[error("config storage error: {0}")]
    Storage(String),
}

/// Whitelist of language values the System tab can persist. Kept as a
/// constant so the dispatch validator and (future) tests share it.
pub const ALLOWED_LANGUAGES: &[&str] = &["auto", "en", "ja"];

/// Whitelist of Board provider values the System tab can persist (SPEC-2959).
pub const ALLOWED_BOARD_PROVIDERS: &[&str] = &["local", "slack", "teams"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemSettingsSnapshot {
    pub language: String,
    pub codex_trust_managed_hooks: Option<bool>,
    pub board_provider: String,
}

/// Validate that `value` is one of [`ALLOWED_BOARD_PROVIDERS`] (case-insensitive,
/// trimmed) and return the canonical lowercase form plus its [`BoardProviderKind`].
pub fn validate_board_provider(
    value: &str,
) -> Result<(String, BoardProviderKind), SystemSettingsError> {
    let trimmed = value.trim().to_lowercase();
    match trimmed.as_str() {
        "local" => Ok((trimmed, BoardProviderKind::Local)),
        "slack" => Ok((trimmed, BoardProviderKind::Slack)),
        "teams" => Ok((trimmed, BoardProviderKind::Teams)),
        _ => Err(SystemSettingsError::InvalidBoardProvider(value.to_string())),
    }
}

/// Validate that `value` is one of [`ALLOWED_LANGUAGES`] (case-insensitive,
/// trimmed). Returns the canonical lowercase value on success.
pub fn validate_language(value: &str) -> Result<String, SystemSettingsError> {
    let trimmed = value.trim().to_lowercase();
    if ALLOWED_LANGUAGES.iter().any(|allowed| *allowed == trimmed) {
        Ok(trimmed)
    } else {
        Err(SystemSettingsError::InvalidLanguage(value.to_string()))
    }
}

/// Read the current global language from `path`. Returns `auto` when the
/// config file does not exist (matching [`Settings::default`]).
pub fn read_language(path: &Path) -> Result<String, SystemSettingsError> {
    Ok(read_settings(path)?.language)
}

pub fn read_settings(path: &Path) -> Result<SystemSettingsSnapshot, SystemSettingsError> {
    let settings = if path.exists() {
        Settings::load_from_path(path)
            .map_err(|err| SystemSettingsError::Storage(err.to_string()))?
    } else {
        Settings::default()
    };
    Ok(SystemSettingsSnapshot {
        language: settings
            .ai
            .language
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
        codex_trust_managed_hooks: Some(codex_trust_managed_hooks_enabled(&settings)),
        board_provider: settings.board.provider.as_str().to_string(),
    })
}

/// Persist `language` into `path` under `[ai].language`. Returns the
/// canonical value that was written so the dispatch layer can echo it
/// back to the frontend.
pub fn write_language(path: &Path, language: &str) -> Result<String, SystemSettingsError> {
    Ok(write_settings(path, language, None, None)?.language)
}

pub fn write_settings(
    path: &Path,
    language: &str,
    codex_trust_managed_hooks: Option<bool>,
    board_provider: Option<&str>,
) -> Result<SystemSettingsSnapshot, SystemSettingsError> {
    let canonical = validate_language(language)?;
    // Validate the provider (if supplied) before touching disk so an invalid
    // value never half-writes config.
    let provider = board_provider.map(validate_board_provider).transpose()?;
    let mut settings = if path.exists() {
        Settings::load_from_path(path)
            .map_err(|err| SystemSettingsError::Storage(err.to_string()))?
    } else {
        Settings::default()
    };
    settings.ai.language = Some(canonical.clone());
    if let Some(value) = codex_trust_managed_hooks {
        settings.agent.codex_trust_managed_hooks = Some(value);
    }
    if let Some((_, kind)) = provider {
        settings.board.provider = kind;
        // No in-memory cache to update: `board_provider::provider()` reads the
        // selection fresh from config on each call, so the persisted value
        // takes effect immediately (FR-008).
    }
    settings
        .save(path)
        .map_err(|err| SystemSettingsError::Storage(err.to_string()))?;
    Ok(SystemSettingsSnapshot {
        language: canonical,
        codex_trust_managed_hooks: Some(codex_trust_managed_hooks_enabled(&settings)),
        board_provider: settings.board.provider.as_str().to_string(),
    })
}

fn codex_trust_managed_hooks_enabled(settings: &Settings) -> bool {
    settings.agent.codex_trust_managed_hooks != Some(false)
}

// --- Board provider configuration (SPEC-2963 FR-006) ------------------------
// Non-secret provider fields (client id, default channel, tenant id) live in
// `[board.slack]` / `[board.teams]` of `config.toml`; the OAuth client secret
// is captured here too but routed to the secure credential store, never to
// `config.toml`. The settings UI edits these so the user does not hand-edit
// `config.toml`.

use crate::board_remote::token_store;

/// Snapshot of the editable provider configuration surfaced to the settings UI.
/// Secrets are never returned — only a `*_has_secret` flag so the UI can show
/// "configured" without echoing the value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BoardProviderConfigSnapshot {
    pub slack_client_id: Option<String>,
    pub slack_default_channel: Option<String>,
    pub slack_has_secret: bool,
    pub teams_client_id: Option<String>,
    pub teams_tenant_id: Option<String>,
    pub teams_default_channel: Option<String>,
}

fn load_settings_or_default(path: &Path) -> Result<Settings, SystemSettingsError> {
    if path.exists() {
        Settings::load_from_path(path).map_err(|err| SystemSettingsError::Storage(err.to_string()))
    } else {
        Ok(Settings::default())
    }
}

fn normalize_field(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Read the current provider config from `path` and secret presence from
/// `credentials_dir`.
pub fn read_board_provider_config_in(
    path: &Path,
    credentials_dir: &Path,
) -> Result<BoardProviderConfigSnapshot, SystemSettingsError> {
    let settings = load_settings_or_default(path)?;
    let slack_has_secret = token_store::load_secret_in(credentials_dir, "slack")
        .ok()
        .flatten()
        .is_some();
    Ok(BoardProviderConfigSnapshot {
        slack_client_id: settings.board.slack.client_id.clone(),
        slack_default_channel: settings.board.slack.default_channel.clone(),
        slack_has_secret,
        teams_client_id: settings.board.teams.client_id.clone(),
        teams_tenant_id: settings.board.teams.tenant_id.clone(),
        teams_default_channel: settings.board.teams.default_channel.clone(),
    })
}

/// Read the current provider config using the default credentials directory.
pub fn read_board_provider_config(
    path: &Path,
) -> Result<BoardProviderConfigSnapshot, SystemSettingsError> {
    read_board_provider_config_in(path, &token_store::default_dir())
}

/// Persist provider config: non-secret fields to `path`, the client secret to
/// `credentials_dir`. A `Some("")` field clears that value; `None` leaves it
/// unchanged. The secret follows the same rule but is never written to
/// `config.toml`.
#[allow(clippy::too_many_arguments)]
pub fn write_board_provider_config_in(
    path: &Path,
    credentials_dir: &Path,
    provider: &str,
    client_id: Option<String>,
    default_channel: Option<String>,
    tenant_id: Option<String>,
    client_secret: Option<String>,
) -> Result<BoardProviderConfigSnapshot, SystemSettingsError> {
    let (_, kind) = validate_board_provider(provider)?;
    let provider_key = match kind {
        BoardProviderKind::Slack => "slack",
        BoardProviderKind::Teams => "teams",
        BoardProviderKind::Local => {
            return Err(SystemSettingsError::InvalidBoardProvider(
                "local has no provider configuration".to_string(),
            ))
        }
    };

    let mut settings = load_settings_or_default(path)?;
    match kind {
        BoardProviderKind::Slack => {
            if let Some(v) = &client_id {
                settings.board.slack.client_id = normalize_field(&Some(v.clone()));
            }
            if let Some(v) = &default_channel {
                settings.board.slack.default_channel = normalize_field(&Some(v.clone()));
            }
        }
        BoardProviderKind::Teams => {
            if let Some(v) = &client_id {
                settings.board.teams.client_id = normalize_field(&Some(v.clone()));
            }
            if let Some(v) = &tenant_id {
                settings.board.teams.tenant_id = normalize_field(&Some(v.clone()));
            }
            if let Some(v) = &default_channel {
                settings.board.teams.default_channel = normalize_field(&Some(v.clone()));
            }
        }
        BoardProviderKind::Local => unreachable!(),
    }
    settings
        .save(path)
        .map_err(|err| SystemSettingsError::Storage(err.to_string()))?;

    if let Some(secret) = client_secret {
        let trimmed = secret.trim();
        let result = if trimmed.is_empty() {
            token_store::clear_secret_in(credentials_dir, provider_key)
        } else {
            token_store::save_secret_in(credentials_dir, provider_key, trimmed)
        };
        result.map_err(|err| SystemSettingsError::Storage(err.to_string()))?;
    }

    read_board_provider_config_in(path, credentials_dir)
}

/// Persist provider config using the default credentials directory.
pub fn write_board_provider_config(
    path: &Path,
    provider: &str,
    client_id: Option<String>,
    default_channel: Option<String>,
    tenant_id: Option<String>,
    client_secret: Option<String>,
) -> Result<BoardProviderConfigSnapshot, SystemSettingsError> {
    write_board_provider_config_in(
        path,
        &token_store::default_dir(),
        provider,
        client_id,
        default_channel,
        tenant_id,
        client_secret,
    )
}

/// Build the [`BackendEvent::BoardAuthStatus`] event carrying remote provider
/// sign-in state plus the editable (non-secret) config view. Shared by the
/// `GetBoardAuthStatus` reply path and the OAuth `/oauth/callback` broadcast so
/// both surfaces report identical state (FR-012: the settings UI updates after
/// a browser sign-in without a manual Refresh).
pub fn board_auth_status_event(message: Option<String>) -> BackendEvent {
    let config = Settings::global_config_path()
        .and_then(|path| read_board_provider_config(&path).ok())
        .unwrap_or_default();
    BackendEvent::BoardAuthStatus {
        slack: crate::board_remote::signin::is_signed_in("slack"),
        teams: crate::board_remote::signin::is_signed_in("teams"),
        message,
        slack_client_id: config.slack_client_id,
        slack_default_channel: config.slack_default_channel,
        slack_has_secret: config.slack_has_secret,
        teams_client_id: config.teams_client_id,
        teams_tenant_id: config.teams_tenant_id,
        teams_default_channel: config.teams_default_channel,
    }
}

/// Build the `BackendEvent` reply for `FrontendEvent::GetSystemSettings`.
pub fn get_event(path: &Path) -> BackendEvent {
    match read_settings(path) {
        Ok(snapshot) => BackendEvent::SystemSettings {
            language: snapshot.language,
            codex_trust_managed_hooks: snapshot.codex_trust_managed_hooks,
            board_provider: Some(snapshot.board_provider),
        },
        Err(err) => BackendEvent::SystemSettingsError {
            message: err.to_string(),
        },
    }
}

/// Build the `BackendEvent` reply for `FrontendEvent::UpdateSystemSettings`.
pub fn update_event(
    path: &Path,
    language: String,
    codex_trust_managed_hooks: Option<bool>,
    board_provider: Option<String>,
) -> BackendEvent {
    match write_settings(
        path,
        &language,
        codex_trust_managed_hooks,
        board_provider.as_deref(),
    ) {
        Ok(snapshot) => BackendEvent::SystemSettingsUpdated {
            language: snapshot.language,
            codex_trust_managed_hooks: snapshot.codex_trust_managed_hooks,
            board_provider: Some(snapshot.board_provider),
        },
        Err(err) => BackendEvent::SystemSettingsError {
            message: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validate_accepts_canonical_values() {
        assert_eq!(validate_language("auto").unwrap(), "auto");
        assert_eq!(validate_language("en").unwrap(), "en");
        assert_eq!(validate_language("ja").unwrap(), "ja");
    }

    #[test]
    fn validate_normalizes_case_and_whitespace() {
        assert_eq!(validate_language(" JA ").unwrap(), "ja");
        assert_eq!(validate_language("Auto").unwrap(), "auto");
    }

    #[test]
    fn validate_rejects_unknown_values() {
        assert!(validate_language("zh").is_err());
        assert!(validate_language("").is_err());
        assert!(validate_language("english").is_err());
    }

    #[test]
    fn read_language_returns_auto_when_config_missing() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        assert_eq!(read_language(&path).unwrap(), "auto");
    }

    #[test]
    fn write_then_read_roundtrip() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let written = write_language(&path, "ja").unwrap();
        assert_eq!(written, "ja");
        assert_eq!(read_language(&path).unwrap(), "ja");

        let written = write_language(&path, "EN").unwrap();
        assert_eq!(written, "en");
        assert_eq!(read_language(&path).unwrap(), "en");
    }

    #[test]
    fn write_preserves_other_settings_fields() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let original = Settings {
            default_base_branch: "develop".to_string(),
            protected_branches: vec!["main".to_string(), "release".to_string()],
            ..Default::default()
        };
        original.save(&path).unwrap();

        write_language(&path, "ja").unwrap();

        let reloaded = Settings::load_from_path(&path).unwrap();
        assert_eq!(reloaded.default_base_branch, "develop");
        assert_eq!(reloaded.protected_branches, vec!["main", "release"]);
        assert_eq!(reloaded.ai.language.as_deref(), Some("ja"));
    }

    #[test]
    fn read_and_write_codex_hook_trust_false_only_opt_out() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let snapshot = read_settings(&path).unwrap();
        assert_eq!(
            snapshot.codex_trust_managed_hooks,
            Some(true),
            "missing config should render System Settings as enabled by default"
        );

        let snapshot = write_settings(&path, "en", Some(false), None).unwrap();
        assert_eq!(snapshot.language, "en");
        assert_eq!(snapshot.codex_trust_managed_hooks, Some(false));

        let reloaded = Settings::load_from_path(&path).unwrap();
        assert_eq!(reloaded.agent.codex_trust_managed_hooks, Some(false));
    }

    #[test]
    fn update_event_returns_updated_on_success() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let event = update_event(&path, "ja".to_string(), Some(true), None);
        match event {
            BackendEvent::SystemSettingsUpdated {
                language,
                codex_trust_managed_hooks,
                ..
            } => {
                assert_eq!(language, "ja");
                assert_eq!(codex_trust_managed_hooks, Some(true));
            }
            other => panic!("expected SystemSettingsUpdated, got {other:?}"),
        }
    }

    #[test]
    fn update_event_returns_error_for_invalid_language() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let event = update_event(&path, "zh".to_string(), None, None);
        match event {
            BackendEvent::SystemSettingsError { message } => {
                assert!(message.contains("invalid language"));
            }
            other => panic!("expected SystemSettingsError, got {other:?}"),
        }
    }

    #[test]
    fn get_event_returns_current_language() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        write_language(&path, "ja").unwrap();
        let event = get_event(&path);
        match event {
            BackendEvent::SystemSettings {
                language,
                codex_trust_managed_hooks,
                ..
            } => {
                assert_eq!(language, "ja");
                assert_eq!(codex_trust_managed_hooks, Some(true));
            }
            other => panic!("expected SystemSettings, got {other:?}"),
        }
    }

    #[test]
    fn board_provider_defaults_to_local_and_roundtrips() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        // Missing config → local default.
        assert_eq!(read_settings(&path).unwrap().board_provider, "local");

        // Persist slack and read it back; language is unchanged.
        let snapshot = write_settings(&path, "auto", None, Some("slack")).unwrap();
        assert_eq!(snapshot.board_provider, "slack");
        assert_eq!(read_settings(&path).unwrap().board_provider, "slack");

        // None leaves the persisted provider unchanged.
        let snapshot = write_settings(&path, "en", None, None).unwrap();
        assert_eq!(snapshot.board_provider, "slack");
    }

    #[test]
    fn validate_board_provider_accepts_canonical_and_rejects_unknown() {
        assert_eq!(validate_board_provider(" Local ").unwrap().0, "local");
        assert_eq!(validate_board_provider("SLACK").unwrap().0, "slack");
        assert_eq!(validate_board_provider("teams").unwrap().0, "teams");
        assert!(validate_board_provider("discord").is_err());
    }

    #[test]
    fn update_event_persists_board_provider() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let event = update_event(&path, "auto".to_string(), None, Some("teams".to_string()));
        match event {
            BackendEvent::SystemSettingsUpdated { board_provider, .. } => {
                assert_eq!(board_provider.as_deref(), Some("teams"))
            }
            other => panic!("expected SystemSettingsUpdated, got {other:?}"),
        }
        let reloaded = Settings::load_from_path(&path).unwrap();
        assert_eq!(reloaded.board.provider, BoardProviderKind::Teams);
    }

    #[test]
    fn write_slack_provider_config_routes_secret_to_store_not_config() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let creds = tempdir().unwrap();

        let snapshot = write_board_provider_config_in(
            &path,
            creds.path(),
            "slack",
            Some("2389371082.11261832736740".to_string()),
            Some("C0B74NMMALX".to_string()),
            None,
            Some("super-secret".to_string()),
        )
        .unwrap();

        assert_eq!(
            snapshot.slack_client_id.as_deref(),
            Some("2389371082.11261832736740")
        );
        assert_eq!(
            snapshot.slack_default_channel.as_deref(),
            Some("C0B74NMMALX")
        );
        assert!(snapshot.slack_has_secret);

        // Non-secret fields persisted to config.toml.
        let reloaded = Settings::load_from_path(&path).unwrap();
        assert_eq!(
            reloaded.board.slack.client_id.as_deref(),
            Some("2389371082.11261832736740")
        );
        // Secret never written to config.toml; lives only in the store.
        let toml_text = std::fs::read_to_string(&path).unwrap();
        assert!(!toml_text.contains("super-secret"));
        assert_eq!(
            token_store::load_secret_in(creds.path(), "slack")
                .unwrap()
                .as_deref(),
            Some("super-secret")
        );
    }

    #[test]
    fn write_provider_config_empty_secret_clears_store() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let creds = tempdir().unwrap();

        write_board_provider_config_in(
            &path,
            creds.path(),
            "slack",
            None,
            None,
            None,
            Some("seed".to_string()),
        )
        .unwrap();
        assert!(
            read_board_provider_config_in(&path, creds.path())
                .unwrap()
                .slack_has_secret
        );

        // Explicit empty secret clears it; None would leave it untouched.
        let snapshot = write_board_provider_config_in(
            &path,
            creds.path(),
            "slack",
            None,
            None,
            None,
            Some("   ".to_string()),
        )
        .unwrap();
        assert!(!snapshot.slack_has_secret);
    }

    #[test]
    fn write_teams_provider_config_persists_tenant_and_channel() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let creds = tempdir().unwrap();

        let snapshot = write_board_provider_config_in(
            &path,
            creds.path(),
            "teams",
            Some("app-123".to_string()),
            Some("team/chan".to_string()),
            Some("tenant-xyz".to_string()),
            None,
        )
        .unwrap();
        assert_eq!(snapshot.teams_client_id.as_deref(), Some("app-123"));
        assert_eq!(snapshot.teams_tenant_id.as_deref(), Some("tenant-xyz"));
        assert_eq!(snapshot.teams_default_channel.as_deref(), Some("team/chan"));

        let reloaded = Settings::load_from_path(&path).unwrap();
        assert_eq!(
            reloaded.board.teams.tenant_id.as_deref(),
            Some("tenant-xyz")
        );
    }

    #[test]
    fn write_provider_config_rejects_local() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let creds = tempdir().unwrap();
        assert!(write_board_provider_config_in(
            &path,
            creds.path(),
            "local",
            None,
            None,
            None,
            None
        )
        .is_err());
    }

    #[test]
    fn write_provider_config_some_empty_clears_field_none_keeps() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let creds = tempdir().unwrap();

        write_board_provider_config_in(
            &path,
            creds.path(),
            "slack",
            Some("C123".to_string()),
            Some("CHAN".to_string()),
            None,
            None,
        )
        .unwrap();

        // None leaves client_id intact; Some("") clears default_channel.
        let snapshot = write_board_provider_config_in(
            &path,
            creds.path(),
            "slack",
            None,
            Some("".to_string()),
            None,
            None,
        )
        .unwrap();
        assert_eq!(snapshot.slack_client_id.as_deref(), Some("C123"));
        assert_eq!(snapshot.slack_default_channel, None);
    }
}
