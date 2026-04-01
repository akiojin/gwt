//! Session management

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json;

use crate::error::{GwtError, Result};

/// Agent status for state visualization (gwt-spec issue FR-100a)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Status unknown (default for backward compatibility)
    #[default]
    Unknown,
    /// Agent is actively processing
    Running,
    /// Agent is waiting for user input (permission prompt, etc.)
    WaitingInput,
    /// Agent has stopped or is idle
    Stopped,
}

impl AgentStatus {
    /// Check if status indicates the agent needs attention
    pub fn needs_attention(&self) -> bool {
        matches!(self, AgentStatus::WaitingInput | AgentStatus::Stopped)
    }

    /// Check if status indicates the agent is active
    pub fn is_active(&self) -> bool {
        matches!(self, AgentStatus::Running)
    }
}

/// Session information (FR-069: Store version info in session history)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Session ID
    pub id: String,
    /// Worktree path
    pub worktree_path: PathBuf,
    /// Branch name
    pub branch: String,
    /// Agent ID (e.g., "claude-code", "codex-cli")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Agent display label (e.g., "Claude Code", "Codex")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_label: Option<String>,
    /// Agent session ID (for resume)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    /// Tool version (e.g., "1.0.3", "latest", "installed")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    /// Model used (e.g., "opus", "sonnet")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
    /// Agent status (gwt-spec issue FR-100a)
    #[serde(default)]
    pub status: AgentStatus,
    /// Last activity timestamp (gwt-spec issue FR-100b)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<DateTime<Utc>>,
    /// User-set display name for this worktree (gwt-spec issue FR-04)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl Session {
    /// Legacy JSON session file name
    const LEGACY_JSON_NAME: &'static str = ".gwt-session.json";

    /// Local TOML session file name (legacy, now migrated to global)
    const LOCAL_SESSION_NAME: &'static str = ".gwt-session.toml";

    /// Create a new session
    pub fn new(worktree_path: impl Into<PathBuf>, branch: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: worktree_path.into(),
            branch: branch.into(),
            agent: None,
            agent_label: None,
            agent_session_id: None,
            tool_version: None,
            model: None,
            created_at: now,
            updated_at: now,
            status: AgentStatus::Unknown,
            last_activity_at: None,
            display_name: None,
        }
    }

    /// Idle timeout in seconds (gwt-spec issue FR-100c)
    const IDLE_TIMEOUT_SECS: i64 = 60;

    /// Check if session should be marked as stopped due to inactivity
    /// Returns true if last_activity_at is more than 60 seconds ago
    pub fn should_mark_stopped(&self) -> bool {
        if self.status == AgentStatus::Stopped {
            return false; // Already stopped
        }
        if let Some(last_activity) = self.last_activity_at {
            let elapsed = Utc::now() - last_activity;
            elapsed > Duration::seconds(Self::IDLE_TIMEOUT_SECS)
        } else {
            false
        }
    }

    /// Update status and last_activity_at timestamp
    pub fn update_status(&mut self, status: AgentStatus) {
        self.status = status;
        self.last_activity_at = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    /// Check and update stopped status if idle timeout exceeded
    pub fn check_idle_timeout(&mut self) -> bool {
        if self.should_mark_stopped() {
            self.status = AgentStatus::Stopped;
            self.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Format tool usage string for display (FR-070)
    /// Returns format: "ToolName@X.Y.Z"
    pub fn format_tool_usage(&self) -> Option<String> {
        let label = self.agent_label.as_ref().or(self.agent.as_ref())?;
        let short_label = short_tool_label(self.agent.as_deref(), label);
        let version = self.tool_version.as_deref().unwrap_or("latest");
        Some(format!("{}@{}", short_label, version))
    }

    /// Save session to file
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| GwtError::ConfigWriteError {
            reason: e.to_string(),
        })?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(path, content)?;
        Ok(())
    }

    /// Load session from file
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content).map_err(|e| GwtError::ConfigParseError {
            reason: e.to_string(),
        })
    }

    /// Get the global sessions directory (gwt-spec issue FR-010)
    /// Falls back to config_dir or temp_dir if home_dir is unavailable
    /// Can be overridden via GWT_SESSIONS_DIR environment variable (for testing)
    pub fn sessions_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("GWT_SESSIONS_DIR") {
            return PathBuf::from(dir);
        }
        dirs::home_dir()
            .or_else(dirs::config_dir)
            .unwrap_or_else(std::env::temp_dir)
            .join(".gwt")
            .join("sessions")
    }

    /// Get the session file path for a worktree (global storage)
    /// Uses Base64 encoding of worktree path as filename (gwt-spec issue FR-010)
    pub fn session_path(worktree_path: &Path) -> PathBuf {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

        let sessions_dir = Self::sessions_dir();
        let path_str = worktree_path.to_string_lossy();
        let hash = URL_SAFE_NO_PAD.encode(path_str.as_bytes());

        sessions_dir.join(format!("{}.toml", hash))
    }

    /// Get local session file path (legacy, for migration)
    fn local_session_path(worktree_path: &Path) -> PathBuf {
        worktree_path.join(Self::LOCAL_SESSION_NAME)
    }

    /// Get legacy JSON session file path
    fn legacy_session_path(worktree_path: &Path) -> PathBuf {
        worktree_path.join(Self::LEGACY_JSON_NAME)
    }

    /// Migrate legacy JSON session file to global TOML if needed
    fn migrate_legacy_json_session(worktree_path: &Path) -> Result<()> {
        let json_path = Self::legacy_session_path(worktree_path);
        let toml_path = Self::session_path(worktree_path);

        if !json_path.exists() || toml_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&json_path)?;
        let session: Session =
            serde_json::from_str(&content).map_err(|e| GwtError::MigrationFailed {
                reason: format!("Failed to parse session JSON: {}", e),
            })?;
        session.save(&toml_path)?;
        // Delete legacy JSON file after successful migration
        let _ = std::fs::remove_file(&json_path);
        Ok(())
    }

    /// Migrate local TOML session file to global storage and delete local
    /// (gwt-spec issue FR-010)
    pub fn migrate_local_to_global(worktree_path: &Path) -> Result<()> {
        let local_path = Self::local_session_path(worktree_path);
        let global_path = Self::session_path(worktree_path);

        if !local_path.exists() {
            return Ok(());
        }

        // If global already exists, just delete local
        if global_path.exists() {
            let _ = std::fs::remove_file(&local_path);
            return Ok(());
        }

        // Migrate local to global
        let session = Self::load(&local_path)?;
        session.save(&global_path)?;

        // Delete local file after successful migration
        let _ = std::fs::remove_file(&local_path);
        Ok(())
    }

    /// Load session for a worktree if exists
    /// Migration order:
    /// 1. Legacy JSON (.gwt-session.json) -> Global TOML
    /// 2. Local TOML (.gwt-session.toml) -> Global TOML
    pub fn load_for_worktree(worktree_path: &Path) -> Option<Self> {
        // Run migrations
        let _ = Self::migrate_legacy_json_session(worktree_path);
        let _ = Self::migrate_local_to_global(worktree_path);

        let session_path = Self::session_path(worktree_path);
        if session_path.exists() {
            Self::load(&session_path).ok()
        } else {
            None
        }
    }
}

/// Check if an agent supports Hook-based status reporting (e.g., Claude Code).
/// Agents without hook support need pane output analysis for status inference.
pub fn agent_has_hook_support(agent_id: Option<&str>) -> bool {
    match agent_id {
        Some(id) => {
            let lower = id.to_lowercase();
            lower.contains("claude") || lower.contains("codex")
        }
        None => false,
    }
}

/// Infer agent status from pane output tail and process liveness.
///
/// Used for agents that lack Hook API support (Gemini, OpenCode).
/// Heuristics (FR-831):
/// 1. Process dead → Stopped
/// 2. Prompt pattern at end of output → WaitingInput
/// 3. Process alive & recent output → Running
pub fn infer_agent_status(scrollback_tail: &str, process_alive: bool) -> AgentStatus {
    if !process_alive {
        return AgentStatus::Stopped;
    }

    if looks_like_prompt(scrollback_tail) {
        return AgentStatus::WaitingInput;
    }

    AgentStatus::Running
}

/// Check if the tail of scrollback output ends with a prompt-like pattern.
fn looks_like_prompt(text: &str) -> bool {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return false;
    }

    // Get the last non-empty line
    let last_line = trimmed.lines().next_back().unwrap_or("").trim();
    if last_line.is_empty() {
        return false;
    }

    // Common prompt patterns
    let prompt_suffixes = ["> ", "→ ", "$ ", ">>> ", "# "];
    for suffix in &prompt_suffixes {
        if last_line.ends_with(suffix.trim_end()) {
            return true;
        }
    }

    // Input prompt patterns (case-insensitive)
    let last_lower = last_line.to_lowercase();
    let input_patterns = [
        "input:",
        "prompt:",
        "(y/n)",
        "[y/n]",
        "continue?",
        "proceed?",
    ];
    for pattern in &input_patterns {
        if last_lower.contains(pattern) {
            return true;
        }
    }

    false
}

fn short_tool_label(tool_id: Option<&str>, tool_label: &str) -> String {
    let id = tool_id.unwrap_or("");
    let id_lower = id.to_lowercase();
    if id_lower.contains("claude") {
        return "Claude".to_string();
    }
    if id_lower.contains("codex") {
        return "Codex".to_string();
    }
    if id_lower.contains("gemini") {
        return "Gemini".to_string();
    }
    if id_lower.contains("opencode") || id_lower.contains("open-code") {
        return "OpenCode".to_string();
    }
    if id_lower.contains("copilot") {
        return "GitHub Copilot".to_string();
    }

    let label_lower = tool_label.to_lowercase();
    if label_lower.contains("claude") {
        return "Claude".to_string();
    }
    if label_lower.contains("codex") {
        return "Codex".to_string();
    }
    if label_lower.contains("gemini") {
        return "Gemini".to_string();
    }
    if label_lower.contains("opencode") || label_lower.contains("open-code") {
        return "OpenCode".to_string();
    }
    if label_lower.contains("copilot") {
        return "GitHub Copilot".to_string();
    }

    tool_label.to_string()
}

/// Load all sessions from worktrees
pub fn load_sessions_from_worktrees(worktrees: &[crate::worktree::Worktree]) -> Vec<Session> {
    worktrees
        .iter()
        .filter_map(|wt| Session::load_for_worktree(&wt.path))
        .collect()
}

/// Get session for a specific branch
pub fn get_session_for_branch<'a>(sessions: &'a [Session], branch: &str) -> Option<&'a Session> {
    sessions.iter().find(|s| s.branch == branch)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tempfile::TempDir;

    use super::*;

    // Mutex to serialize tests that use GWT_SESSIONS_DIR environment variable
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }

        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn test_session_save_load() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("session.toml");

        let session = Session::new("/repo/.worktrees/feature", "feature/test");

        session.save(&session_path).unwrap();

        let loaded = Session::load(&session_path).unwrap();
        assert_eq!(loaded.branch, "feature/test");
    }

    #[test]
    fn test_session_path_global() {
        // Lock mutex to prevent concurrent env var access
        let _guard = ENV_MUTEX.lock().unwrap();
        let _home_lock = crate::config::HOME_LOCK.lock().unwrap();
        let _env_guard = EnvVarGuard::unset("GWT_SESSIONS_DIR");

        // Global session path should be under ~/.gwt/sessions/
        let worktree_path = PathBuf::from("/repo/.worktrees/feature");
        let session_path = Session::session_path(&worktree_path);

        // Should be under sessions directory
        let sessions_dir = Session::sessions_dir();
        assert!(session_path.starts_with(&sessions_dir));

        // Should have .toml extension
        assert_eq!(session_path.extension().unwrap(), "toml");

        // Different worktree paths should produce different session paths
        let other_worktree = PathBuf::from("/other/repo/.worktrees/main");
        let other_session_path = Session::session_path(&other_worktree);
        assert_ne!(session_path, other_session_path);
    }

    #[test]
    fn test_session_path_hash_consistency() {
        // Lock mutex to prevent concurrent env var access
        let _guard = ENV_MUTEX.lock().unwrap();
        let _home_lock = crate::config::HOME_LOCK.lock().unwrap();
        let _env_guard = EnvVarGuard::unset("GWT_SESSIONS_DIR");

        // Same worktree path should always produce same session path
        let worktree_path = PathBuf::from("/repo/.worktrees/feature");
        let path1 = Session::session_path(&worktree_path);
        let path2 = Session::session_path(&worktree_path);
        assert_eq!(path1, path2);
    }

    #[test]
    fn test_migrate_local_to_global() {
        // Lock mutex to prevent concurrent env var access
        let _guard = ENV_MUTEX.lock().unwrap();
        let _home_lock = crate::config::HOME_LOCK.lock().unwrap();

        let temp = TempDir::new().unwrap();
        let worktree_path = temp.path();

        // Use temp directory as sessions dir to avoid writing to real home
        let sessions_dir = temp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let _env_guard = EnvVarGuard::set("GWT_SESSIONS_DIR", sessions_dir.to_str().unwrap());

        // Create a local session file
        let local_path = worktree_path.join(Session::LOCAL_SESSION_NAME);
        let session = Session::new(worktree_path, "feature/migrate");
        session.save(&local_path).unwrap();

        // Migration should move local to global
        let result = Session::migrate_local_to_global(worktree_path);
        assert!(result.is_ok());

        // Local file should be deleted after migration
        assert!(!local_path.exists());

        // Global file should exist
        let global_path = Session::session_path(worktree_path);
        assert!(global_path.exists());

        // Cleanup
        std::env::remove_var("GWT_SESSIONS_DIR");
    }

    #[test]
    fn test_legacy_session_migration() {
        // Lock mutex to prevent concurrent env var access
        let _guard = ENV_MUTEX.lock().unwrap();
        let _home_lock = crate::config::HOME_LOCK.lock().unwrap();

        let temp = TempDir::new().unwrap();
        let legacy_path = temp.path().join(Session::LEGACY_JSON_NAME);

        // Use temp directory as sessions dir to avoid writing to real home
        let sessions_dir = temp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::env::set_var("GWT_SESSIONS_DIR", sessions_dir.to_str().unwrap());

        let session = Session::new(temp.path(), "feature/legacy");
        std::fs::write(
            &legacy_path,
            serde_json::to_string_pretty(&session).unwrap(),
        )
        .unwrap();

        let global_path = Session::session_path(temp.path());

        let loaded = Session::load_for_worktree(temp.path()).unwrap();
        assert_eq!(loaded.branch, "feature/legacy");

        // Global session file should be created
        assert!(global_path.exists());

        // Legacy JSON should be deleted after migration
        assert!(!legacy_path.exists());

        // Cleanup
        std::env::remove_var("GWT_SESSIONS_DIR");
    }

    #[test]
    fn test_format_tool_usage_short_label() {
        let mut session = Session::new("/repo/.worktrees/feature", "feature/test");
        session.agent_label = Some("Claude Code".to_string());
        session.tool_version = Some("1.0.3".to_string());
        assert_eq!(
            session.format_tool_usage(),
            Some("Claude@1.0.3".to_string())
        );

        let mut session = Session::new("/repo/.worktrees/feature", "feature/test");
        session.agent = Some("codex-cli".to_string());
        session.tool_version = None;
        assert_eq!(
            session.format_tool_usage(),
            Some("Codex@latest".to_string())
        );
    }

    // gwt-spec issue T-100 tests

    #[test]
    fn test_agent_status_default() {
        let status = AgentStatus::default();
        assert_eq!(status, AgentStatus::Unknown);
    }

    #[test]
    fn test_agent_status_serialize_deserialize() {
        let status = AgentStatus::Running;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"running\"");

        let deserialized: AgentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, AgentStatus::Running);
    }

    #[test]
    fn test_agent_status_all_variants() {
        let variants = [
            (AgentStatus::Unknown, "\"unknown\""),
            (AgentStatus::Running, "\"running\""),
            (AgentStatus::WaitingInput, "\"waiting_input\""),
            (AgentStatus::Stopped, "\"stopped\""),
        ];

        for (status, expected_json) in variants {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected_json);
        }
    }

    #[test]
    fn test_session_with_status_field() {
        let session = Session::new("/test/path", "test-branch");
        assert_eq!(session.status, AgentStatus::Unknown);
        assert!(session.last_activity_at.is_none());
    }

    #[test]
    fn test_session_status_update() {
        let mut session = Session::new("/test/path", "test-branch");
        session.update_status(AgentStatus::Running);

        assert_eq!(session.status, AgentStatus::Running);
        assert!(session.last_activity_at.is_some());
    }

    #[test]
    fn test_session_load_without_status_field() {
        // Old format TOML (without status field)
        let toml_content = r#"
id = "test-id"
worktree_path = "/test/path"
branch = "test-branch"
created_at = "2026-01-20T00:00:00Z"
updated_at = "2026-01-20T00:00:00Z"
"#;

        let session: Session = toml::from_str(toml_content).unwrap();
        assert_eq!(session.status, AgentStatus::Unknown);
    }

    #[test]
    fn test_session_auto_stopped_after_60_seconds() {
        let mut session = Session::new("/test/path", "test-branch");
        session.status = AgentStatus::Running;
        session.last_activity_at = Some(Utc::now() - Duration::seconds(61));

        assert!(session.should_mark_stopped());
    }

    #[test]
    fn test_session_not_stopped_within_60_seconds() {
        let mut session = Session::new("/test/path", "test-branch");
        session.status = AgentStatus::Running;
        session.last_activity_at = Some(Utc::now() - Duration::seconds(30));

        assert!(!session.should_mark_stopped());
    }

    #[test]
    fn test_session_check_idle_timeout() {
        let mut session = Session::new("/test/path", "test-branch");
        session.status = AgentStatus::Running;
        session.last_activity_at = Some(Utc::now() - Duration::seconds(61));

        let changed = session.check_idle_timeout();
        assert!(changed);
        assert_eq!(session.status, AgentStatus::Stopped);
    }

    #[test]
    fn test_agent_status_needs_attention() {
        assert!(!AgentStatus::Unknown.needs_attention());
        assert!(!AgentStatus::Running.needs_attention());
        assert!(AgentStatus::WaitingInput.needs_attention());
        assert!(AgentStatus::Stopped.needs_attention());
    }

    #[test]
    fn test_agent_status_is_active() {
        assert!(!AgentStatus::Unknown.is_active());
        assert!(AgentStatus::Running.is_active());
        assert!(!AgentStatus::WaitingInput.is_active());
        assert!(!AgentStatus::Stopped.is_active());
    }

    #[test]
    fn test_check_idle_timeout_transitions_to_stopped() {
        let mut session = Session::new("/test/path", "test-branch");
        session.status = AgentStatus::Running;
        session.last_activity_at = Some(Utc::now() - Duration::seconds(61));

        let changed = session.check_idle_timeout();
        assert!(
            changed,
            "check_idle_timeout should return true when > 60s elapsed"
        );
        assert_eq!(session.status, AgentStatus::Stopped);
    }

    #[test]
    fn test_check_idle_timeout_no_change_within_60s() {
        let mut session = Session::new("/test/path", "test-branch");
        session.status = AgentStatus::Running;
        session.last_activity_at = Some(Utc::now() - Duration::seconds(30));

        let changed = session.check_idle_timeout();
        assert!(
            !changed,
            "check_idle_timeout should return false when < 60s"
        );
        assert_eq!(
            session.status,
            AgentStatus::Running,
            "status should remain Running"
        );
    }

    #[test]
    fn test_check_idle_timeout_already_stopped() {
        let mut session = Session::new("/test/path", "test-branch");
        session.status = AgentStatus::Stopped;
        session.last_activity_at = Some(Utc::now() - Duration::seconds(120));

        let changed = session.check_idle_timeout();
        assert!(!changed, "already Stopped should not change");
        assert_eq!(session.status, AgentStatus::Stopped);
    }

    #[test]
    fn test_check_idle_timeout_no_last_activity() {
        let mut session = Session::new("/test/path", "test-branch");
        session.status = AgentStatus::Running;
        session.last_activity_at = None;

        let changed = session.check_idle_timeout();
        assert!(!changed, "no last_activity_at should not trigger timeout");
        assert_eq!(session.status, AgentStatus::Running);
    }

    #[test]
    fn test_agent_status_serde_roundtrip_all_variants() {
        let variants = [
            AgentStatus::Unknown,
            AgentStatus::Running,
            AgentStatus::WaitingInput,
            AgentStatus::Stopped,
        ];

        for status in variants {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: AgentStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, status, "roundtrip failed for {:?}", status);
        }
    }

    #[test]
    fn test_agent_status_toml_roundtrip() {
        // AgentStatus uses rename_all = "snake_case", verify TOML compat
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Wrapper {
            status: AgentStatus,
        }

        let variants = [
            AgentStatus::Unknown,
            AgentStatus::Running,
            AgentStatus::WaitingInput,
            AgentStatus::Stopped,
        ];

        for status in variants {
            let wrapper = Wrapper { status };
            let toml_str = toml::to_string(&wrapper).unwrap();
            let deserialized: Wrapper = toml::from_str(&toml_str).unwrap();
            assert_eq!(
                deserialized, wrapper,
                "TOML roundtrip failed for {:?}",
                status
            );
        }
    }

    #[test]
    fn test_session_save_load_with_status() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("session.toml");

        let mut session = Session::new("/repo/.worktrees/feature", "feature/test");
        session.update_status(AgentStatus::WaitingInput);

        session.save(&session_path).unwrap();

        let loaded = Session::load(&session_path).unwrap();
        assert_eq!(loaded.status, AgentStatus::WaitingInput);
        assert!(loaded.last_activity_at.is_some());
    }

    // -- infer_agent_status tests (gwt-spec issue T-014) --

    #[test]
    fn infer_stopped_when_process_dead() {
        let status = infer_agent_status("some output\n$", false);
        assert_eq!(status, AgentStatus::Stopped);
    }

    #[test]
    fn infer_waiting_input_with_dollar_prompt() {
        let status = infer_agent_status("output line\n$ ", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn infer_waiting_input_with_arrow_prompt() {
        let status = infer_agent_status("output line\n→ ", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn infer_waiting_input_with_chevron_prompt() {
        let status = infer_agent_status("output line\n> ", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn infer_waiting_input_with_triple_chevron() {
        let status = infer_agent_status("output line\n>>> ", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn infer_waiting_input_with_hash_prompt() {
        let status = infer_agent_status("output\n# ", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn infer_waiting_input_with_input_colon() {
        let status = infer_agent_status("Please provide:\nInput: ", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn infer_waiting_input_with_yn_prompt() {
        let status = infer_agent_status("Continue? (y/n)", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn infer_waiting_input_with_yn_bracket() {
        let status = infer_agent_status("Proceed [Y/n]", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn infer_waiting_input_with_continue_question() {
        let status = infer_agent_status("Do you want to continue?", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn infer_waiting_input_with_proceed_question() {
        let status = infer_agent_status("proceed?", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn infer_running_when_no_prompt() {
        let status = infer_agent_status("Compiling project...\nBuilding module foo", true);
        assert_eq!(status, AgentStatus::Running);
    }

    #[test]
    fn infer_running_on_empty_output() {
        let status = infer_agent_status("", true);
        assert_eq!(status, AgentStatus::Running);
    }

    #[test]
    fn infer_stopped_on_empty_output_dead_process() {
        let status = infer_agent_status("", false);
        assert_eq!(status, AgentStatus::Stopped);
    }

    #[test]
    fn infer_prompt_with_trailing_whitespace() {
        let status = infer_agent_status("line\n$   \n  ", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    // -- agent_has_hook_support tests --

    #[test]
    fn claude_has_hook_support() {
        assert!(agent_has_hook_support(Some("claude-code")));
        assert!(agent_has_hook_support(Some("Claude Code")));
    }

    #[test]
    fn codex_has_hook_support() {
        assert!(agent_has_hook_support(Some("codex-cli")));
        assert!(agent_has_hook_support(Some("Codex")));
    }

    #[test]
    fn non_hook_agents_no_support() {
        assert!(!agent_has_hook_support(Some("gemini-cli")));
        assert!(!agent_has_hook_support(Some("opencode")));
        assert!(!agent_has_hook_support(None));
    }

    // --- short_tool_label ---

    #[test]
    fn short_tool_label_claude_from_id() {
        assert_eq!(
            short_tool_label(Some("claude-code"), "Some Label"),
            "Claude"
        );
    }

    #[test]
    fn short_tool_label_codex_from_id() {
        assert_eq!(short_tool_label(Some("codex-cli"), "Some Label"), "Codex");
    }

    #[test]
    fn short_tool_label_gemini_from_id() {
        assert_eq!(short_tool_label(Some("gemini-cli"), "Gemini CLI"), "Gemini");
    }

    #[test]
    fn short_tool_label_opencode_from_id() {
        assert_eq!(short_tool_label(Some("opencode"), "OpenCode"), "OpenCode");
    }

    #[test]
    fn short_tool_label_open_code_hyphen_from_id() {
        assert_eq!(short_tool_label(Some("open-code"), "OpenCode"), "OpenCode");
    }

    #[test]
    fn short_tool_label_copilot_from_id() {
        assert_eq!(
            short_tool_label(Some("github-copilot"), "Some Label"),
            "GitHub Copilot"
        );
    }

    #[test]
    fn short_tool_label_claude_from_label_when_id_unknown() {
        assert_eq!(short_tool_label(Some("unknown"), "Claude Code"), "Claude");
    }

    #[test]
    fn short_tool_label_codex_from_label() {
        assert_eq!(short_tool_label(Some("custom"), "Codex CLI"), "Codex");
    }

    #[test]
    fn short_tool_label_copilot_from_label() {
        assert_eq!(
            short_tool_label(Some("custom"), "GitHub Copilot"),
            "GitHub Copilot"
        );
    }

    #[test]
    fn short_tool_label_fallback_to_label() {
        assert_eq!(
            short_tool_label(Some("custom"), "Custom Agent"),
            "Custom Agent"
        );
    }

    #[test]
    fn short_tool_label_none_id_uses_label() {
        assert_eq!(short_tool_label(None, "Claude Code"), "Claude");
    }

    #[test]
    fn short_tool_label_none_id_unknown_label() {
        assert_eq!(short_tool_label(None, "My Custom Tool"), "My Custom Tool");
    }

    // --- get_session_for_branch ---

    #[test]
    fn get_session_for_branch_found() {
        let sessions = vec![
            Session::new("/repo/.worktrees/a", "feature/a"),
            Session::new("/repo/.worktrees/b", "feature/b"),
        ];
        let found = get_session_for_branch(&sessions, "feature/b");
        assert!(found.is_some());
        assert_eq!(found.unwrap().branch, "feature/b");
    }

    #[test]
    fn get_session_for_branch_not_found() {
        let sessions = vec![Session::new("/repo/.worktrees/a", "feature/a")];
        let found = get_session_for_branch(&sessions, "nonexistent");
        assert!(found.is_none());
    }

    #[test]
    fn get_session_for_branch_empty_list() {
        let sessions: Vec<Session> = vec![];
        let found = get_session_for_branch(&sessions, "feature/a");
        assert!(found.is_none());
    }

    // --- Session::format_tool_usage ---

    #[test]
    fn format_tool_usage_with_agent_label() {
        let mut session = Session::new("/repo", "main");
        session.agent_label = Some("Gemini CLI".to_string());
        session.tool_version = Some("0.2.1".to_string());
        assert_eq!(
            session.format_tool_usage(),
            Some("Gemini@0.2.1".to_string())
        );
    }

    #[test]
    fn format_tool_usage_falls_back_to_agent_id() {
        let mut session = Session::new("/repo", "main");
        session.agent = Some("opencode".to_string());
        session.agent_label = None;
        session.tool_version = Some("1.0.0".to_string());
        assert_eq!(
            session.format_tool_usage(),
            Some("OpenCode@1.0.0".to_string())
        );
    }

    #[test]
    fn format_tool_usage_none_when_no_agent() {
        let session = Session::new("/repo", "main");
        assert_eq!(session.format_tool_usage(), None);
    }

    #[test]
    fn format_tool_usage_defaults_to_latest_version() {
        let mut session = Session::new("/repo", "main");
        session.agent_label = Some("Claude Code".to_string());
        session.tool_version = None;
        assert_eq!(
            session.format_tool_usage(),
            Some("Claude@latest".to_string())
        );
    }

    // --- Session save/load with corrupt data ---

    #[test]
    fn session_load_corrupt_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("corrupt.toml");
        std::fs::write(&path, "this is not valid toml!!!").unwrap();
        let result = Session::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn session_load_nonexistent() {
        let result = Session::load(std::path::Path::new("/nonexistent/session.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn session_save_creates_parent_dirs() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("deep").join("nested").join("session.toml");
        let session = Session::new("/repo", "main");
        session.save(&path).unwrap();
        assert!(path.exists());
    }

    // --- Session sessions_dir with env var ---

    #[test]
    fn sessions_dir_uses_env_var() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let _home_lock = crate::config::HOME_LOCK.lock().unwrap();

        let temp = TempDir::new().unwrap();
        let custom_dir = temp.path().join("custom-sessions");
        std::fs::create_dir_all(&custom_dir).unwrap();
        let _env_guard = EnvVarGuard::set("GWT_SESSIONS_DIR", custom_dir.to_str().unwrap());

        let dir = Session::sessions_dir();
        assert_eq!(dir, custom_dir);
    }

    // --- should_mark_stopped edge cases ---

    #[test]
    fn should_mark_stopped_false_for_waiting_input_within_timeout() {
        let mut session = Session::new("/test", "branch");
        session.status = AgentStatus::WaitingInput;
        session.last_activity_at = Some(Utc::now() - Duration::seconds(30));
        assert!(!session.should_mark_stopped());
    }

    #[test]
    fn should_mark_stopped_true_for_waiting_input_past_timeout() {
        let mut session = Session::new("/test", "branch");
        session.status = AgentStatus::WaitingInput;
        session.last_activity_at = Some(Utc::now() - Duration::seconds(61));
        assert!(session.should_mark_stopped());
    }

    #[test]
    fn should_mark_stopped_false_for_unknown_status() {
        let mut session = Session::new("/test", "branch");
        session.status = AgentStatus::Unknown;
        session.last_activity_at = Some(Utc::now() - Duration::seconds(120));
        assert!(session.should_mark_stopped());
    }

    // --- looks_like_prompt additional tests ---

    #[test]
    fn infer_prompt_with_prompt_colon() {
        let status = infer_agent_status("Enter password:\nPrompt: ", true);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    // --- display_name tests ---

    #[test]
    fn test_session_new_has_no_display_name() {
        let session = Session::new("/repo/.worktrees/feature", "feature/test");
        assert_eq!(session.display_name, None);
    }

    #[test]
    fn test_session_save_load_with_display_name() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("session.toml");

        let mut session = Session::new("/repo/.worktrees/feature", "feature/auth");
        session.display_name = Some("Add auth feature".to_string());

        session.save(&session_path).unwrap();

        let loaded = Session::load(&session_path).unwrap();
        assert_eq!(loaded.display_name, Some("Add auth feature".to_string()));
    }

    #[test]
    fn test_session_save_load_without_display_name() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("session.toml");

        let session = Session::new("/repo/.worktrees/feature", "feature/test");
        assert_eq!(session.display_name, None);

        session.save(&session_path).unwrap();

        let loaded = Session::load(&session_path).unwrap();
        assert_eq!(loaded.display_name, None);

        // Verify skip_serializing_if works: TOML should not contain "display_name"
        let content = std::fs::read_to_string(&session_path).unwrap();
        assert!(
            !content.contains("display_name"),
            "TOML should not contain display_name when None (skip_serializing_if)"
        );
    }

    #[test]
    fn test_session_load_old_format_without_display_name() {
        // Backward compatibility: old TOML without display_name field
        let toml_content = r#"
id = "old-session-id"
worktree_path = "/repo/.worktrees/feature"
branch = "feature/old"
created_at = "2026-01-20T00:00:00Z"
updated_at = "2026-01-20T00:00:00Z"
"#;

        let session: Session = toml::from_str(toml_content).unwrap();
        assert_eq!(session.display_name, None);
        assert_eq!(session.branch, "feature/old");
    }

    #[test]
    fn test_session_display_name_roundtrip_empty_after_clear() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("session.toml");

        let mut session = Session::new("/repo/.worktrees/feature", "feature/test");
        session.display_name = Some("test".to_string());

        // Clear the display_name
        session.display_name = None;

        session.save(&session_path).unwrap();

        let loaded = Session::load(&session_path).unwrap();
        assert_eq!(loaded.display_name, None);
    }
}
