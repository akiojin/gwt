//! Launch defaults configuration
//!
<<<<<<< HEAD
//! Persisted defaults for the agent launch wizard.
=======
//! Persisted default values for the agent launch wizard.

use std::collections::HashMap;
>>>>>>> origin/feature/feature-1776

use std::collections::HashMap;

use std::collections::HashMap;

/// Default terminal rows for new PTY sessions.
pub const DEFAULT_PTY_ROWS: u16 = 24;
/// Default terminal cols for new PTY sessions.
pub const DEFAULT_PTY_COLS: u16 = 80;

<<<<<<< HEAD
/// Persisted launch dialog defaults.
=======
<<<<<<< HEAD
/// Persisted launch dialog defaults
#[derive(Debug, Clone, Default)]
pub struct LaunchDefaults {
    /// Selected agent ID (e.g. "claude", "codex")
    pub selected_agent: String,
    /// Session mode (e.g. "plan", "normal")
    pub session_mode: String,
    /// Model per agent (agent_id -> model name)
    pub model_by_agent: HashMap<String, String>,
    /// Version per agent (agent_id -> version string)
    pub version_by_agent: HashMap<String, String>,
    /// Skip permissions flag
    pub skip_permissions: bool,
    /// Reasoning level (e.g. "" for default, "low", "medium", "high")
    pub reasoning_level: String,
    /// Fast mode flag
    pub fast_mode: bool,
    /// Extra CLI arguments
    pub extra_args: String,
    /// Environment variable overrides
=======
<<<<<<< HEAD
/// Persisted launch defaults.
=======
/// Persisted launch dialog defaults.
>>>>>>> origin/feature/feature-1776
>>>>>>> origin/feature/feature-1776
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
<<<<<<< HEAD
=======
>>>>>>> origin/feature/feature-1776
>>>>>>> origin/feature/feature-1776
    pub env_overrides: String,
}
