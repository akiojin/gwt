//! Claude Code Plugin marketplace registration (SPEC-f8dab6e2)
//!
//! This module provides functionality to register gwt-plugins marketplace and
//! enable worktree-protection-hooks plugin in Claude Code settings.
//!
//! Marketplace registration format in `~/.claude/plugins/known_marketplaces.json`:
//! ```json
//! {
//!   "gwt-plugins": {
//!     "source": {
//!       "source": "github",
//!       "repo": "akiojin/gwt"
//!     },
//!     "installLocation": "/path/to/.claude/plugins/marketplaces/gwt-plugins",
//!     "lastUpdated": "2025-01-01T00:00:00.000Z"
//!   }
//! }
//! ```

use crate::error::GwtError;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{hash_map::Entry, HashMap};
use std::path::{Path, PathBuf};

/// Marketplace source information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketplaceSource {
    pub source: String,
    pub repo: String,
}

/// Marketplace entry in known_marketplaces.json
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketplaceEntry {
    pub source: MarketplaceSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "installLocation")]
    pub install_location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
}

/// Known marketplaces JSON structure
pub type KnownMarketplaces = HashMap<String, MarketplaceEntry>;

/// Constants for gwt-plugins marketplace
pub const GWT_MARKETPLACE_NAME: &str = "gwt-plugins";
pub const GWT_MARKETPLACE_SOURCE: &str = "github";
pub const GWT_MARKETPLACE_REPO: &str = "akiojin/gwt";
pub const GWT_PLUGIN_NAME: &str = "worktree-protection-hooks";
pub const GWT_PLUGIN_FULL_NAME: &str = "worktree-protection-hooks@gwt-plugins";

/// Get the path to known_marketplaces.json
pub fn get_known_marketplaces_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join(".claude")
            .join("plugins")
            .join("known_marketplaces.json")
    })
}

/// Check if gwt-plugins marketplace is registered (FR-001)
pub fn is_gwt_marketplace_registered() -> bool {
    let Some(path) = get_known_marketplaces_path() else {
        return false;
    };
    is_gwt_marketplace_registered_at(&path)
}

/// Check if gwt-plugins marketplace is registered at a specific path
pub fn is_gwt_marketplace_registered_at(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };

    let Ok(marketplaces) = serde_json::from_str::<KnownMarketplaces>(&content) else {
        return false;
    };

    let Some(entry) = marketplaces.get(GWT_MARKETPLACE_NAME) else {
        return false;
    };

    is_valid_marketplace_entry(entry)
}

/// Marketplace entry helpers
fn marketplace_install_location(path: &Path) -> String {
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    base.join("marketplaces")
        .join(GWT_MARKETPLACE_NAME)
        .to_string_lossy()
        .into_owned()
}

fn now_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn is_non_empty_string(value: &Option<String>) -> bool {
    value
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

fn is_valid_marketplace_entry(entry: &MarketplaceEntry) -> bool {
    is_non_empty_string(&entry.install_location) && is_non_empty_string(&entry.last_updated)
}

fn ensure_marketplace_entry(entry: &mut MarketplaceEntry, path: &Path) -> bool {
    let mut changed = false;

    if !is_non_empty_string(&entry.install_location) {
        entry.install_location = Some(marketplace_install_location(path));
        changed = true;
    }

    if !is_non_empty_string(&entry.last_updated) {
        entry.last_updated = Some(now_timestamp());
        changed = true;
    }

    changed
}

fn create_gwt_marketplace_entry(path: &Path) -> MarketplaceEntry {
    MarketplaceEntry {
        source: MarketplaceSource {
            source: GWT_MARKETPLACE_SOURCE.to_string(),
            repo: GWT_MARKETPLACE_REPO.to_string(),
        },
        install_location: Some(marketplace_install_location(path)),
        last_updated: Some(now_timestamp()),
    }
}

/// Register gwt-plugins marketplace (FR-003, FR-006)
pub fn register_gwt_marketplace() -> Result<(), GwtError> {
    let Some(path) = get_known_marketplaces_path() else {
        return Ok(()); // Silent continue (FR-009)
    };
    register_gwt_marketplace_at(&path)
}

/// Register gwt-plugins marketplace at a specific path
pub fn register_gwt_marketplace_at(path: &Path) -> Result<(), GwtError> {
    // Create parent directory if needed (FR-006)
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return Ok(()); // Silent continue (FR-009)
        }
    }

    // Load existing marketplaces or create new
    let mut marketplaces: KnownMarketplaces = if path.exists() {
        let content = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        KnownMarketplaces::new()
    };

    let mut changed = false;

    match marketplaces.entry(GWT_MARKETPLACE_NAME.to_string()) {
        Entry::Vacant(entry) => {
            entry.insert(create_gwt_marketplace_entry(path));
            changed = true;
        }
        Entry::Occupied(mut entry) => {
            if ensure_marketplace_entry(entry.get_mut(), path) {
                changed = true;
            }
        }
    }

    if changed {
        // Write back
        let content = serde_json::to_string_pretty(&marketplaces).map_err(|e| {
            GwtError::ConfigWriteError {
                reason: e.to_string(),
            }
        })?;

        if std::fs::write(path, content).is_err() {
            return Ok(()); // Silent continue (FR-009)
        }
    }

    Ok(())
}

/// Check if worktree-protection-hooks plugin is enabled in settings
pub fn is_plugin_enabled_in_settings(settings_path: &Path) -> bool {
    if !settings_path.exists() {
        return false;
    }

    let Ok(content) = std::fs::read_to_string(settings_path) else {
        return false;
    };

    let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };

    settings
        .get("enabledPlugins")
        .and_then(|p| p.get(GWT_PLUGIN_FULL_NAME))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Check if plugin was explicitly disabled by user (FR-010)
pub fn is_plugin_explicitly_disabled(settings_path: &Path) -> bool {
    if !settings_path.exists() {
        return false;
    }

    let Ok(content) = std::fs::read_to_string(settings_path) else {
        return false;
    };

    let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };

    // If the key exists and is false, it was explicitly disabled
    settings
        .get("enabledPlugins")
        .and_then(|p| p.get(GWT_PLUGIN_FULL_NAME))
        .map(|v| v.as_bool() == Some(false))
        .unwrap_or(false)
}

/// Enable worktree-protection-hooks plugin in settings (FR-004)
pub fn enable_worktree_protection_plugin(settings_path: &Path) -> Result<(), GwtError> {
    // Don't re-enable if explicitly disabled (FR-010)
    if is_plugin_explicitly_disabled(settings_path) {
        return Ok(());
    }

    // Create parent directory if needed (FR-005)
    if let Some(parent) = settings_path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return Ok(()); // Silent continue (FR-009)
        }
    }

    // Load existing settings or create new
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(settings_path).unwrap_or_else(|_| "{}".to_string());
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Ensure enabledPlugins object exists
    if settings.get("enabledPlugins").is_none() {
        settings["enabledPlugins"] = serde_json::json!({});
    }

    // Add plugin if not exists
    if settings["enabledPlugins"]
        .get(GWT_PLUGIN_FULL_NAME)
        .is_none()
    {
        settings["enabledPlugins"][GWT_PLUGIN_FULL_NAME] = serde_json::json!(true);

        // Write back
        let content =
            serde_json::to_string_pretty(&settings).map_err(|e| GwtError::ConfigWriteError {
                reason: e.to_string(),
            })?;

        if std::fs::write(settings_path, content).is_err() {
            return Ok(()); // Silent continue (FR-009)
        }
    }

    Ok(())
}

/// Get the path to global Claude settings
pub fn get_global_claude_settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".claude").join("settings.json"))
}

/// Get the path to local Claude settings
pub fn get_local_claude_settings_path() -> PathBuf {
    PathBuf::from(".claude").join("settings.json")
}

/// Setup gwt plugin (marketplace registration + plugin enable) (FR-003, FR-004)
pub fn setup_gwt_plugin() -> Result<(), GwtError> {
    // Register marketplace
    register_gwt_marketplace()?;

    // Enable plugin in global settings
    if let Some(global_path) = get_global_claude_settings_path() {
        enable_worktree_protection_plugin(&global_path)?;
    }

    // Enable plugin in local settings
    let local_path = get_local_claude_settings_path();
    enable_worktree_protection_plugin(&local_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // FR-001: Check marketplace registration status
    #[test]
    fn test_is_gwt_marketplace_registered_when_not_exists() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("known_marketplaces.json");

        assert!(!is_gwt_marketplace_registered_at(&path));
    }

    #[test]
    fn test_is_gwt_marketplace_registered_when_exists() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("known_marketplaces.json");

        let content = r#"{"gwt-plugins": {"source": {"source": "github", "repo": "akiojin/gwt"}, "installLocation": "/tmp/marketplaces/gwt-plugins", "lastUpdated": "2025-01-01T00:00:00.000Z"}}"#;
        std::fs::write(&path, content).unwrap();

        assert!(is_gwt_marketplace_registered_at(&path));
    }

    #[test]
    fn test_is_gwt_marketplace_registered_when_missing_required_fields() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("known_marketplaces.json");

        let content = r#"{"gwt-plugins": {"source": {"source": "github", "repo": "akiojin/gwt"}}}"#;
        std::fs::write(&path, content).unwrap();

        assert!(!is_gwt_marketplace_registered_at(&path));
    }

    #[test]
    fn test_is_gwt_marketplace_registered_when_other_marketplace_exists() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("known_marketplaces.json");

        let content =
            r#"{"other-marketplace": {"source": {"source": "github", "repo": "other/repo"}}}"#;
        std::fs::write(&path, content).unwrap();

        assert!(!is_gwt_marketplace_registered_at(&path));
    }

    // FR-003: Marketplace registration
    #[test]
    fn test_register_gwt_marketplace_creates_correct_entry() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("known_marketplaces.json");

        register_gwt_marketplace_at(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let marketplaces: KnownMarketplaces = serde_json::from_str(&content).unwrap();

        assert!(marketplaces.contains_key(GWT_MARKETPLACE_NAME));
        let entry = marketplaces.get(GWT_MARKETPLACE_NAME).unwrap();
        assert_eq!(entry.source.source, GWT_MARKETPLACE_SOURCE);
        assert_eq!(entry.source.repo, GWT_MARKETPLACE_REPO);
        let expected_install_location = path
            .parent()
            .unwrap()
            .join("marketplaces")
            .join(GWT_MARKETPLACE_NAME)
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            entry.install_location.as_deref(),
            Some(expected_install_location.as_str())
        );
        assert!(matches!(
            entry.last_updated.as_deref(),
            Some(value) if !value.is_empty()
        ));
    }

    #[test]
    fn test_register_gwt_marketplace_preserves_existing() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("known_marketplaces.json");

        // Create existing marketplace
        let content =
            r#"{"other-marketplace": {"source": {"source": "github", "repo": "other/repo"}}}"#;
        std::fs::write(&path, content).unwrap();

        register_gwt_marketplace_at(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let marketplaces: KnownMarketplaces = serde_json::from_str(&content).unwrap();

        assert!(marketplaces.contains_key("other-marketplace"));
        assert!(marketplaces.contains_key(GWT_MARKETPLACE_NAME));
    }

    #[test]
    fn test_register_gwt_marketplace_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("known_marketplaces.json");

        register_gwt_marketplace_at(&path).unwrap();
        register_gwt_marketplace_at(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let marketplaces: KnownMarketplaces = serde_json::from_str(&content).unwrap();

        assert_eq!(marketplaces.len(), 1);
    }

    #[test]
    fn test_register_gwt_marketplace_repairs_missing_fields() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("known_marketplaces.json");

        let content = r#"{"gwt-plugins": {"source": {"source": "github", "repo": "akiojin/gwt"}}}"#;
        std::fs::write(&path, content).unwrap();

        register_gwt_marketplace_at(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let marketplaces: KnownMarketplaces = serde_json::from_str(&content).unwrap();
        let entry = marketplaces.get(GWT_MARKETPLACE_NAME).unwrap();

        assert!(matches!(
            entry.install_location.as_deref(),
            Some(value) if !value.is_empty()
        ));
        assert!(matches!(
            entry.last_updated.as_deref(),
            Some(value) if !value.is_empty()
        ));
    }

    // FR-004: Plugin enable
    #[test]
    fn test_enable_worktree_protection_plugin_adds_to_enabled_plugins() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("settings.json");

        enable_worktree_protection_plugin(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(
            settings["enabledPlugins"][GWT_PLUGIN_FULL_NAME],
            serde_json::json!(true)
        );
    }

    #[test]
    fn test_enable_worktree_protection_plugin_preserves_existing() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("settings.json");

        let content = r#"{"enabledPlugins": {"other-plugin@other": true}}"#;
        std::fs::write(&path, content).unwrap();

        enable_worktree_protection_plugin(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(
            settings["enabledPlugins"]["other-plugin@other"],
            serde_json::json!(true)
        );
        assert_eq!(
            settings["enabledPlugins"][GWT_PLUGIN_FULL_NAME],
            serde_json::json!(true)
        );
    }

    // FR-006: Directory auto-creation
    #[test]
    fn test_register_creates_plugins_directory_if_not_exists() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir
            .path()
            .join(".claude")
            .join("plugins")
            .join("known_marketplaces.json");

        assert!(!path.parent().unwrap().exists());

        register_gwt_marketplace_at(&path).unwrap();

        assert!(path.exists());
    }

    // FR-009: Silent error handling
    #[test]
    fn test_register_silently_continues_on_invalid_json() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("known_marketplaces.json");

        // Write invalid JSON
        std::fs::write(&path, "invalid json {{{").unwrap();

        // Should not panic, should create new valid JSON
        let result = register_gwt_marketplace_at(&path);
        assert!(result.is_ok());

        // File should now be valid
        let content = std::fs::read_to_string(&path).unwrap();
        let marketplaces: Result<KnownMarketplaces, _> = serde_json::from_str(&content);
        assert!(marketplaces.is_ok());

        let marketplaces = marketplaces.unwrap();
        let entry = marketplaces.get(GWT_MARKETPLACE_NAME).unwrap();
        assert!(matches!(
            entry.install_location.as_deref(),
            Some(value) if !value.is_empty()
        ));
        assert!(matches!(
            entry.last_updated.as_deref(),
            Some(value) if !value.is_empty()
        ));
    }

    // FR-010: Don't re-enable disabled plugin
    #[test]
    fn test_does_not_reenable_disabled_plugin() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("settings.json");

        // User explicitly disabled the plugin
        let content = format!(
            r#"{{"enabledPlugins": {{"{}": false}}}}"#,
            GWT_PLUGIN_FULL_NAME
        );
        std::fs::write(&path, content).unwrap();

        enable_worktree_protection_plugin(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Should still be false
        assert_eq!(
            settings["enabledPlugins"][GWT_PLUGIN_FULL_NAME],
            serde_json::json!(false)
        );
    }

    #[test]
    fn test_is_plugin_explicitly_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("settings.json");

        // Not disabled (doesn't exist)
        assert!(!is_plugin_explicitly_disabled(&path));

        // Enabled
        let content = format!(
            r#"{{"enabledPlugins": {{"{}": true}}}}"#,
            GWT_PLUGIN_FULL_NAME
        );
        std::fs::write(&path, content).unwrap();
        assert!(!is_plugin_explicitly_disabled(&path));

        // Explicitly disabled
        let content = format!(
            r#"{{"enabledPlugins": {{"{}": false}}}}"#,
            GWT_PLUGIN_FULL_NAME
        );
        std::fs::write(&path, content).unwrap();
        assert!(is_plugin_explicitly_disabled(&path));
    }

    #[test]
    fn test_is_plugin_enabled_in_settings() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("settings.json");

        // Not enabled (doesn't exist)
        assert!(!is_plugin_enabled_in_settings(&path));

        // Enabled
        let content = format!(
            r#"{{"enabledPlugins": {{"{}": true}}}}"#,
            GWT_PLUGIN_FULL_NAME
        );
        std::fs::write(&path, content).unwrap();
        assert!(is_plugin_enabled_in_settings(&path));

        // Disabled
        let content = format!(
            r#"{{"enabledPlugins": {{"{}": false}}}}"#,
            GWT_PLUGIN_FULL_NAME
        );
        std::fs::write(&path, content).unwrap();
        assert!(!is_plugin_enabled_in_settings(&path));
    }

    // T504: Edge case tests

    #[test]
    fn test_register_marketplace_with_empty_json_object() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("known_marketplaces.json");

        // Write empty JSON object
        std::fs::write(&path, "{}").unwrap();

        register_gwt_marketplace_at(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let marketplaces: KnownMarketplaces = serde_json::from_str(&content).unwrap();

        assert!(marketplaces.contains_key(GWT_MARKETPLACE_NAME));
        let entry = marketplaces.get(GWT_MARKETPLACE_NAME).unwrap();
        assert!(matches!(
            entry.install_location.as_deref(),
            Some(value) if !value.is_empty()
        ));
        assert!(matches!(
            entry.last_updated.as_deref(),
            Some(value) if !value.is_empty()
        ));
    }

    #[test]
    fn test_enable_plugin_with_settings_without_enabled_plugins() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("settings.json");

        // Write settings without enabledPlugins
        std::fs::write(&path, r#"{"mcpServers": {}}"#).unwrap();

        enable_worktree_protection_plugin(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Should have both mcpServers and enabledPlugins
        assert!(settings["mcpServers"].is_object());
        assert_eq!(
            settings["enabledPlugins"][GWT_PLUGIN_FULL_NAME],
            serde_json::json!(true)
        );
    }

    #[test]
    fn test_enable_plugin_with_empty_settings_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("settings.json");

        // Write empty JSON object
        std::fs::write(&path, "{}").unwrap();

        enable_worktree_protection_plugin(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(
            settings["enabledPlugins"][GWT_PLUGIN_FULL_NAME],
            serde_json::json!(true)
        );
    }

    #[test]
    fn test_enable_plugin_with_invalid_settings_json() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("settings.json");

        // Write invalid JSON
        std::fs::write(&path, "not valid json").unwrap();

        // Should handle gracefully
        let result = enable_worktree_protection_plugin(&path);
        assert!(result.is_ok());

        // File should now be valid
        let content = std::fs::read_to_string(&path).unwrap();
        let settings: Result<serde_json::Value, _> = serde_json::from_str(&content);
        assert!(settings.is_ok());
    }
}
