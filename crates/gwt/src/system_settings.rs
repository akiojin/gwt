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

use gwt_config::Settings;

use crate::protocol::BackendEvent;

/// Service-layer error for System Settings operations. Mapped to
/// [`BackendEvent::SystemSettingsError`] in the dispatch layer.
#[derive(Debug, thiserror::Error)]
pub enum SystemSettingsError {
    #[error("invalid language `{0}`: expected `auto`, `en`, or `ja`")]
    InvalidLanguage(String),
    #[error("config storage error: {0}")]
    Storage(String),
}

/// Whitelist of language values the System tab can persist. Kept as a
/// constant so the dispatch validator and (future) tests share it.
pub const ALLOWED_LANGUAGES: &[&str] = &["auto", "en", "ja"];

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
    let settings = if path.exists() {
        Settings::load_from_path(path)
            .map_err(|err| SystemSettingsError::Storage(err.to_string()))?
    } else {
        Settings::default()
    };
    Ok(settings
        .ai
        .language
        .clone()
        .unwrap_or_else(|| "auto".to_string()))
}

/// Persist `language` into `path` under `[ai].language`. Returns the
/// canonical value that was written so the dispatch layer can echo it
/// back to the frontend.
pub fn write_language(path: &Path, language: &str) -> Result<String, SystemSettingsError> {
    let canonical = validate_language(language)?;
    let mut settings = if path.exists() {
        Settings::load_from_path(path)
            .map_err(|err| SystemSettingsError::Storage(err.to_string()))?
    } else {
        Settings::default()
    };
    settings.ai.language = Some(canonical.clone());
    settings
        .save(path)
        .map_err(|err| SystemSettingsError::Storage(err.to_string()))?;
    Ok(canonical)
}

/// Build the `BackendEvent` reply for `FrontendEvent::GetSystemSettings`.
pub fn get_event(path: &Path) -> BackendEvent {
    match read_language(path) {
        Ok(language) => BackendEvent::SystemSettings { language },
        Err(err) => BackendEvent::SystemSettingsError {
            message: err.to_string(),
        },
    }
}

/// Build the `BackendEvent` reply for `FrontendEvent::UpdateSystemSettings`.
pub fn update_event(path: &Path, language: String) -> BackendEvent {
    match write_language(path, &language) {
        Ok(canonical) => BackendEvent::SystemSettingsUpdated {
            language: canonical,
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
    fn update_event_returns_updated_on_success() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let event = update_event(&path, "ja".to_string());
        match event {
            BackendEvent::SystemSettingsUpdated { language } => assert_eq!(language, "ja"),
            other => panic!("expected SystemSettingsUpdated, got {other:?}"),
        }
    }

    #[test]
    fn update_event_returns_error_for_invalid_language() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let event = update_event(&path, "zh".to_string());
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
            BackendEvent::SystemSettings { language } => assert_eq!(language, "ja"),
            other => panic!("expected SystemSettings, got {other:?}"),
        }
    }
}
