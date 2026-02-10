//! Configuration management module
//!
//! Handles TOML configuration files with automatic migration from JSON.

mod agent_config;
mod bare_project;
mod claude_hooks;
mod claude_plugins;
pub mod migration;
mod profile;
mod session;
mod settings;
pub mod tools;
mod ts_session;

pub use agent_config::{AgentConfig, ClaudeAgentConfig, ClaudeAgentProvider, ClaudeGlmConfig};
pub use bare_project::BareProjectConfig;
pub use claude_hooks::{
    all_hook_events, get_claude_settings_path, is_gwt_hooks_registered, is_temporary_execution,
    is_temporary_execution_path, register_gwt_hooks, reregister_gwt_hooks, unregister_gwt_hooks,
    HOOK_EVENTS_WITHOUT_MATCHER, HOOK_EVENTS_WITH_MATCHER,
};
pub use claude_plugins::{
    enable_worktree_protection_plugin, get_global_claude_settings_path,
    get_known_marketplaces_path, get_local_claude_settings_path, is_gwt_marketplace_registered,
    is_gwt_marketplace_registered_at, is_plugin_enabled_in_settings, is_plugin_explicitly_disabled,
    register_gwt_marketplace, register_gwt_marketplace_at, setup_gwt_plugin, GWT_MARKETPLACE_NAME,
    GWT_MARKETPLACE_REPO, GWT_MARKETPLACE_SOURCE, GWT_PLUGIN_FULL_NAME, GWT_PLUGIN_NAME,
};
pub use migration::{
    backup_broken_file, ensure_config_dir, get_cleanup_candidates, migrate_json_to_toml,
    migrate_yaml_to_toml, write_atomic, CleanupCandidate,
};
pub use profile::{
    AISettings, ActiveAISettingsResolution, ActiveAISettingsSource, Profile, ProfilesConfig,
    ResolvedAISettings,
};
pub use session::{get_session_for_branch, load_sessions_from_worktrees, AgentStatus, Session};
pub use settings::Settings;
pub use tools::{AgentType, CustomCodingAgent, ModeArgs, ModelDef, ToolsConfig};
pub use ts_session::{
    get_branch_tool_history, get_last_tool_usage_map, get_ts_session_json_path,
    get_ts_session_path, get_ts_session_toml_path, load_ts_session, migrate_ts_session_if_needed,
    needs_ts_session_migration, save_session_entry, ToolSessionEntry, TsSessionData,
};

#[cfg(test)]
pub(crate) static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Helper for tests that need to manipulate HOME and XDG_CONFIG_HOME
#[cfg(test)]
pub(crate) struct TestEnvGuard {
    prev_home: Option<std::ffi::OsString>,
    prev_xdg_config: Option<std::ffi::OsString>,
}

#[cfg(test)]
impl TestEnvGuard {
    /// Create a new guard that sets HOME to the given path and forces XDG_CONFIG_HOME to {HOME}/.config.
    ///
    /// Note: `dirs::config_dir()` is platform-dependent (e.g., macOS uses
    /// ~/Library/Application Support). Most tests in this crate expect an XDG-style
    /// config layout, so we explicitly set XDG_CONFIG_HOME for determinism.
    pub fn new(home_path: &std::path::Path) -> Self {
        let prev_home = std::env::var_os("HOME");
        let prev_xdg_config = std::env::var_os("XDG_CONFIG_HOME");

        std::env::set_var("HOME", home_path);
        std::env::set_var("XDG_CONFIG_HOME", home_path.join(".config"));

        Self {
            prev_home,
            prev_xdg_config,
        }
    }

    /// Create a guard with explicit XDG_CONFIG_HOME setting
    pub fn with_xdg(home_path: &std::path::Path, xdg_config_home: &std::path::Path) -> Self {
        let prev_home = std::env::var_os("HOME");
        let prev_xdg_config = std::env::var_os("XDG_CONFIG_HOME");

        std::env::set_var("HOME", home_path);
        std::env::set_var("XDG_CONFIG_HOME", xdg_config_home);

        Self {
            prev_home,
            prev_xdg_config,
        }
    }
}

#[cfg(test)]
impl Drop for TestEnvGuard {
    fn drop(&mut self) {
        match &self.prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match &self.prev_xdg_config {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}
