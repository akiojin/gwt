//! Launch defaults configuration
//!
//! Persisted default values for the agent launch wizard.

use std::collections::HashMap;

/// Default terminal rows for new PTY sessions.
pub const DEFAULT_PTY_ROWS: u16 = 24;
/// Default terminal cols for new PTY sessions.
pub const DEFAULT_PTY_COLS: u16 = 80;

/// Persisted launch dialog defaults.
#[derive(Debug, Clone, Default)]
pub struct LaunchDefaults {
    pub selected_agent: String,
    pub session_mode: String,
    pub model_by_agent: HashMap<String, String>,
    pub version_by_agent: HashMap<String, String>,
    pub skip_permissions: bool,
    pub reasoning_level: String,
    pub fast_mode: bool,
    pub extra_args: String,
    pub env_overrides: String,
}
