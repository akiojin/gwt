//! Configuration management module
//!
//! Handles TOML configuration files with automatic migration from JSON.

mod claude_hooks;
mod migration;
mod profile;
mod session;
mod settings;
mod ts_session;

pub use claude_hooks::{
    all_hook_events, get_claude_settings_path, is_gwt_hooks_registered, register_gwt_hooks,
    reregister_gwt_hooks, unregister_gwt_hooks, HOOK_EVENTS_WITHOUT_MATCHER,
    HOOK_EVENTS_WITH_MATCHER,
};
pub use migration::migrate_json_to_toml;
pub use profile::{AISettings, Profile, ProfilesConfig, ResolvedAISettings};
pub use session::{get_session_for_branch, load_sessions_from_worktrees, AgentStatus, Session};
pub use settings::Settings;
pub use ts_session::{
    get_branch_tool_history, get_last_tool_usage_map, get_ts_session_path, load_ts_session,
    save_session_entry, ToolSessionEntry, TsSessionData,
};

#[cfg(test)]
pub(crate) static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
