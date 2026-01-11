//! JSON to TOML migration

use crate::error::{GwtError, Result};
use std::path::Path;

/// Migrate JSON configuration to TOML
pub fn migrate_json_to_toml(json_path: &Path, toml_path: &Path) -> Result<()> {
    // Read JSON file
    let json_content = std::fs::read_to_string(json_path)?;

    // Parse JSON
    let json_value: serde_json::Value =
        serde_json::from_str(&json_content).map_err(|e| GwtError::MigrationFailed {
            reason: format!("Failed to parse JSON: {}", e),
        })?;

    // Convert to TOML
    let toml_content =
        toml::to_string_pretty(&json_value).map_err(|e| GwtError::MigrationFailed {
            reason: format!("Failed to convert to TOML: {}", e),
        })?;

    // Write TOML file
    std::fs::write(toml_path, toml_content)?;

    Ok(())
}

/// Check if migration is needed
pub fn needs_migration(repo_root: &Path) -> bool {
    let json_path = repo_root.join(".gwt.json");
    let toml_path = repo_root.join(".gwt.toml");

    json_path.exists() && !toml_path.exists()
}

/// Auto-migrate if needed
pub fn auto_migrate(repo_root: &Path) -> Result<bool> {
    if needs_migration(repo_root) {
        let json_path = repo_root.join(".gwt.json");
        let toml_path = repo_root.join(".gwt.toml");
        migrate_json_to_toml(&json_path, &toml_path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_migrate_json_to_toml() {
        let temp = TempDir::new().unwrap();
        let json_path = temp.path().join("config.json");
        let toml_path = temp.path().join("config.toml");

        std::fs::write(
            &json_path,
            r#"{"protected_branches": ["main"], "debug": true}"#,
        )
        .unwrap();

        migrate_json_to_toml(&json_path, &toml_path).unwrap();

        let content = std::fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("protected_branches"));
        assert!(content.contains("debug = true"));
    }

    #[test]
    fn test_needs_migration() {
        let temp = TempDir::new().unwrap();

        // No files - no migration needed
        assert!(!needs_migration(temp.path()));

        // JSON only - migration needed
        std::fs::write(temp.path().join(".gwt.json"), "{}").unwrap();
        assert!(needs_migration(temp.path()));

        // Both files - no migration needed
        std::fs::write(temp.path().join(".gwt.toml"), "").unwrap();
        assert!(!needs_migration(temp.path()));
    }
}
