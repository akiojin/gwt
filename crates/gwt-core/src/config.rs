//! Configuration management module
//!
//! Handles TOML configuration files with automatic migration from JSON.

mod migration;
mod profile;
mod session;
mod settings;
mod ts_session;

pub use migration::migrate_json_to_toml;
pub use profile::{Profile, ProfilesConfig};
pub use session::{get_session_for_branch, load_sessions_from_worktrees, Session};
pub use settings::Settings;
pub use ts_session::{
    get_branch_tool_history, get_last_tool_usage_map, get_ts_session_path, load_ts_session,
    save_session_entry, ToolSessionEntry, TsSessionData,
};
