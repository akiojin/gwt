//! Profiles (env + AI settings) management commands

use gwt_core::config::ProfilesConfig;

/// Get current profiles config (global: ~/.gwt/profiles.{toml,yaml})
#[tauri::command]
pub fn get_profiles() -> Result<ProfilesConfig, String> {
    ProfilesConfig::load().map_err(|e| e.to_string())
}

/// Save profiles config (always writes TOML: ~/.gwt/profiles.toml)
#[tauri::command]
pub fn save_profiles(config: ProfilesConfig) -> Result<(), String> {
    config.save().map_err(|e| e.to_string())
}
