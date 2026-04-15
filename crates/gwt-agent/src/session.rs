//! Agent session persistence: save/load sessions as TOML files.

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::launch::normalize_launch_args;
use crate::types::{
    AgentId, AgentStatus, DockerLifecycleIntent, LaunchRuntimeTarget, WorkflowBypass,
};

/// Idle duration (in seconds) after which a session is considered stopped.
const IDLE_TIMEOUT_SECS: i64 = 60;

/// Environment variable injected into agent PTYs so hooks can identify the
/// backing gwt session.
pub const GWT_SESSION_ID_ENV: &str = "GWT_SESSION_ID";
/// Environment variable injected into agent PTYs so hooks can write the
/// matching runtime sidecar without discovering gwt paths on their own.
pub const GWT_SESSION_RUNTIME_PATH_ENV: &str = "GWT_SESSION_RUNTIME_PATH";
/// Environment variable injected into agent PTYs so skills can locate the
/// gwt binary for calling gwt CLI (GitHub operations, etc.).
pub const GWT_BIN_PATH_ENV: &str = "GWT_BIN_PATH";

/// Represents a single agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub worktree_path: PathBuf,
    pub branch: String,
    pub agent_id: AgentId,
    pub agent_session_id: Option<String>,
    pub status: AgentStatus,
    pub tool_version: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_level: Option<String>,
    #[serde(default)]
    pub skip_permissions: bool,
    #[serde(default)]
    pub codex_fast_mode: bool,
    #[serde(default)]
    pub runtime_target: LaunchRuntimeTarget,
    #[serde(default)]
    pub docker_service: Option<String>,
    #[serde(default)]
    pub docker_lifecycle_intent: DockerLifecycleIntent,
    #[serde(default)]
    pub linked_issue_number: Option<u64>,
    #[serde(default)]
    pub workflow_bypass: Option<WorkflowBypass>,
    #[serde(default)]
    pub launch_command: String,
    #[serde(default)]
    pub launch_args: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub display_name: String,
}

/// Lightweight runtime state updated by hook events while the PTY is alive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingDiscussionResume {
    pub proposal_label: String,
    pub proposal_title: String,
    #[serde(default)]
    pub next_question: Option<String>,
}

/// Lightweight runtime state updated by hook events while the PTY is alive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRuntimeState {
    pub status: AgentStatus,
    pub updated_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    #[serde(default)]
    pub source_event: Option<String>,
    #[serde(default)]
    pub pending_discussion: Option<PendingDiscussionResume>,
}

impl Session {
    /// Create a new session with a generated UUID.
    pub fn new(
        worktree_path: impl Into<PathBuf>,
        branch: impl Into<String>,
        agent_id: AgentId,
    ) -> Self {
        let now = Utc::now();
        let display_name = agent_id.display_name().to_string();
        Self {
            id: Uuid::new_v4().to_string(),
            worktree_path: worktree_path.into(),
            branch: branch.into(),
            agent_id,
            agent_session_id: None,
            status: AgentStatus::Unknown,
            tool_version: None,
            model: None,
            reasoning_level: None,
            skip_permissions: false,
            codex_fast_mode: false,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: DockerLifecycleIntent::Connect,
            linked_issue_number: None,
            workflow_bypass: None,
            launch_command: String::new(),
            launch_args: Vec::new(),
            created_at: now,
            updated_at: now,
            last_activity_at: now,
            display_name,
        }
    }

    /// Update the session status and touch timestamps.
    pub fn update_status(&mut self, status: AgentStatus) {
        self.status = status;
        let now = Utc::now();
        self.updated_at = now;
        if status == AgentStatus::Running || status == AgentStatus::WaitingInput {
            self.last_activity_at = now;
        }
    }

    /// Check if the session should be marked as stopped due to idle timeout.
    pub fn should_mark_stopped(&self) -> bool {
        if self.status == AgentStatus::Stopped {
            return false;
        }
        let elapsed = Utc::now()
            .signed_duration_since(self.last_activity_at)
            .num_seconds();
        elapsed >= IDLE_TIMEOUT_SECS
    }

    /// Save the session to a TOML file under the given directory.
    /// File is written to `<dir>/<session_id>.toml`.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.toml", self.id));
        let content = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(path, content)
    }

    /// Load a session from a TOML file.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut session: Self = toml::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        normalize_launch_args(
            &session.agent_id,
            &session.launch_command,
            &mut session.launch_args,
        );
        Ok(session)
    }
}

impl SessionRuntimeState {
    /// Create a new runtime state snapshot for the given status.
    pub fn new(status: AgentStatus) -> Self {
        let now = Utc::now();
        Self {
            status,
            updated_at: now,
            last_activity_at: now,
            source_event: None,
            pending_discussion: None,
        }
    }

    /// Create a runtime state snapshot from a supported hook event.
    pub fn from_hook_event(event: &str) -> Option<Self> {
        let status = hook_event_status(event)?;
        Some(Self {
            source_event: Some(event.to_string()),
            ..Self::new(status)
        })
    }

    /// Save the runtime state to a JSON sidecar file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let tmp_path = dir.join(format!(
            ".{}.tmp-{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("runtime.json"),
            std::process::id()
        ));

        {
            let mut tmp = std::fs::File::create(&tmp_path)?;
            tmp.write_all(content.as_bytes())?;
            tmp.write_all(b"\n")?;
            tmp.sync_all()?;
        }

        if cfg!(windows) && path.exists() {
            std::fs::remove_file(path)?;
        }
        std::fs::rename(tmp_path, path)
    }

    /// Load the runtime state from a JSON sidecar file.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
}

/// Return the JSON sidecar path for a session runtime state record.
pub fn runtime_state_path(sessions_dir: &Path, session_id: &str) -> PathBuf {
    runtime_state_path_for_pid(sessions_dir, std::process::id(), session_id)
}

/// Return the runtime namespace directory for a specific gwt process id.
pub fn runtime_state_dir_for_pid(sessions_dir: &Path, pid: u32) -> PathBuf {
    sessions_dir.join("runtime").join(pid.to_string())
}

/// Return the JSON sidecar path for a session runtime state record scoped to a
/// specific gwt process id.
pub fn runtime_state_path_for_pid(sessions_dir: &Path, pid: u32, session_id: &str) -> PathBuf {
    runtime_state_dir_for_pid(sessions_dir, pid).join(format!("{session_id}.json"))
}

/// Reset the runtime namespace for the current gwt process.
pub fn reset_runtime_state_dir(sessions_dir: &Path) -> std::io::Result<()> {
    reset_runtime_state_dir_for_pid(sessions_dir, std::process::id())
}

/// Reset the runtime namespace for the provided gwt process id without
/// touching sibling PID namespaces.
pub fn reset_runtime_state_dir_for_pid(sessions_dir: &Path, pid: u32) -> std::io::Result<()> {
    let runtime_dir = runtime_state_dir_for_pid(sessions_dir, pid);
    if runtime_dir.exists() {
        std::fs::remove_dir_all(&runtime_dir)?;
    }
    std::fs::create_dir_all(&runtime_dir)
}

/// Persist a final session status into both the TOML metadata and the runtime
/// sidecar so future renders do not keep stale active states around.
pub fn persist_session_status(
    sessions_dir: &Path,
    session_id: &str,
    status: AgentStatus,
) -> std::io::Result<()> {
    let session_path = sessions_dir.join(format!("{session_id}.toml"));
    let mut session = Session::load(&session_path)?;
    session.update_status(status);
    session.save(sessions_dir)?;
    SessionRuntimeState::new(status).save(&runtime_state_path(sessions_dir, session_id))
}

fn hook_event_status(event: &str) -> Option<AgentStatus> {
    match event {
        "SessionStart" | "Stop" => Some(AgentStatus::WaitingInput),
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => Some(AgentStatus::Running),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_has_uuid_id() {
        let session = Session::new("/tmp/wt", "feature/test", AgentId::ClaudeCode);
        assert!(!session.id.is_empty());
        // Verify it's a valid UUID
        assert!(Uuid::parse_str(&session.id).is_ok());
    }

    #[test]
    fn new_session_defaults() {
        let session = Session::new("/tmp/wt", "main", AgentId::Codex);
        assert_eq!(session.status, AgentStatus::Unknown);
        assert_eq!(session.branch, "main");
        assert_eq!(session.agent_id, AgentId::Codex);
        assert_eq!(session.display_name, "Codex");
        assert!(session.agent_session_id.is_none());
        assert!(session.tool_version.is_none());
        assert!(session.model.is_none());
        assert!(session.reasoning_level.is_none());
        assert!(!session.skip_permissions);
        assert!(!session.codex_fast_mode);
        assert_eq!(session.runtime_target, LaunchRuntimeTarget::Host);
        assert!(session.docker_service.is_none());
        assert_eq!(
            session.docker_lifecycle_intent,
            DockerLifecycleIntent::Connect
        );
        assert!(session.workflow_bypass.is_none());
    }

    #[test]
    fn update_status_touches_timestamps() {
        let mut session = Session::new("/tmp/wt", "main", AgentId::ClaudeCode);
        let before = session.updated_at;
        // Small sleep not needed; just verify the method works
        session.update_status(AgentStatus::Running);
        assert_eq!(session.status, AgentStatus::Running);
        assert!(session.updated_at >= before);
    }

    #[test]
    fn should_mark_stopped_returns_false_when_already_stopped() {
        let mut session = Session::new("/tmp/wt", "main", AgentId::ClaudeCode);
        session.status = AgentStatus::Stopped;
        assert!(!session.should_mark_stopped());
    }

    #[test]
    fn should_mark_stopped_recent_activity() {
        let session = Session::new("/tmp/wt", "main", AgentId::ClaudeCode);
        // Just created, so last_activity_at is now
        assert!(!session.should_mark_stopped());
    }

    #[test]
    fn should_mark_stopped_old_activity() {
        let mut session = Session::new("/tmp/wt", "main", AgentId::ClaudeCode);
        session.last_activity_at = Utc::now() - chrono::Duration::seconds(120);
        session.status = AgentStatus::Running;
        assert!(session.should_mark_stopped());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new("/tmp/wt", "feature/x", AgentId::Gemini);
        session.model = Some("gemini-2.5-pro".into());
        session.tool_version = Some("0.1.0".into());
        session.agent_session_id = Some("agent-abc".into());
        session.reasoning_level = Some("high".into());
        session.skip_permissions = true;
        session.codex_fast_mode = true;
        session.runtime_target = LaunchRuntimeTarget::Docker;
        session.docker_service = Some("web".into());
        session.docker_lifecycle_intent = DockerLifecycleIntent::Restart;
        session.workflow_bypass = Some(WorkflowBypass::Release);
        session.launch_command = "codex".into();
        session.launch_args = vec![
            "--no-alt-screen".into(),
            "--model=gpt-5.4".into(),
            "resume".into(),
            "--last".into(),
        ];

        session.save(dir.path()).unwrap();

        let path = dir.path().join(format!("{}.toml", session.id));
        assert!(path.exists());

        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.branch, "feature/x");
        assert_eq!(loaded.agent_id, AgentId::Gemini);
        assert_eq!(loaded.model, Some("gemini-2.5-pro".into()));
        assert_eq!(loaded.tool_version, Some("0.1.0".into()));
        assert_eq!(loaded.agent_session_id, Some("agent-abc".into()));
        assert_eq!(loaded.reasoning_level, Some("high".into()));
        assert!(loaded.skip_permissions);
        assert!(loaded.codex_fast_mode);
        assert_eq!(loaded.runtime_target, LaunchRuntimeTarget::Docker);
        assert_eq!(loaded.docker_service, Some("web".into()));
        assert_eq!(
            loaded.docker_lifecycle_intent,
            DockerLifecycleIntent::Restart
        );
        assert_eq!(loaded.launch_command, "codex");
        assert_eq!(
            loaded.launch_args,
            vec![
                "--no-alt-screen".to_string(),
                "--model=gpt-5.4".to_string(),
                "resume".to_string(),
                "--last".to_string()
            ]
        );
        assert_eq!(loaded.workflow_bypass, Some(WorkflowBypass::Release));
        assert_eq!(loaded.display_name, "Gemini CLI");
    }

    #[test]
    fn load_legacy_toml_without_runtime_fields_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.toml");
        let session = Session::new("/tmp/wt", "feature/x", AgentId::Gemini);
        let mut legacy = toml::map::Map::new();
        legacy.insert("id".into(), toml::Value::String(session.id.clone()));
        legacy.insert(
            "worktree_path".into(),
            toml::Value::String(session.worktree_path.display().to_string()),
        );
        legacy.insert("branch".into(), toml::Value::String(session.branch.clone()));
        legacy.insert(
            "agent_id".into(),
            toml::Value::try_from(session.agent_id.clone()).unwrap(),
        );
        legacy.insert(
            "agent_session_id".into(),
            toml::Value::String("agent-legacy".into()),
        );
        legacy.insert(
            "status".into(),
            toml::Value::try_from(session.status).unwrap(),
        );
        legacy.insert("tool_version".into(), toml::Value::String("1.2.3".into()));
        legacy.insert("model".into(), toml::Value::String("gemini-pro".into()));
        legacy.insert("reasoning_level".into(), toml::Value::String("high".into()));
        legacy.insert("skip_permissions".into(), toml::Value::Boolean(true));
        legacy.insert("codex_fast_mode".into(), toml::Value::Boolean(false));
        legacy.insert(
            "created_at".into(),
            toml::Value::try_from(session.created_at).unwrap(),
        );
        legacy.insert(
            "updated_at".into(),
            toml::Value::try_from(session.updated_at).unwrap(),
        );
        legacy.insert(
            "last_activity_at".into(),
            toml::Value::try_from(session.last_activity_at).unwrap(),
        );
        legacy.insert(
            "display_name".into(),
            toml::Value::String(session.display_name.clone()),
        );

        std::fs::write(&path, toml::to_string(&legacy).unwrap()).unwrap();

        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.runtime_target, LaunchRuntimeTarget::Host);
        assert!(loaded.docker_service.is_none());
        assert_eq!(
            loaded.docker_lifecycle_intent,
            DockerLifecycleIntent::Connect
        );
        assert!(loaded.launch_command.is_empty());
        assert!(loaded.launch_args.is_empty());
        assert!(loaded.workflow_bypass.is_none());
    }

    #[test]
    fn load_legacy_codex_toml_injects_no_alt_screen_into_launch_args() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-codex.toml");
        let session = Session::new("/tmp/wt", "feature/x", AgentId::Codex);
        let mut legacy = toml::map::Map::new();
        legacy.insert("id".into(), toml::Value::String(session.id.clone()));
        legacy.insert(
            "worktree_path".into(),
            toml::Value::String(session.worktree_path.display().to_string()),
        );
        legacy.insert("branch".into(), toml::Value::String(session.branch.clone()));
        legacy.insert(
            "agent_id".into(),
            toml::Value::try_from(session.agent_id.clone()).unwrap(),
        );
        legacy.insert(
            "status".into(),
            toml::Value::try_from(session.status).unwrap(),
        );
        legacy.insert(
            "launch_command".into(),
            toml::Value::String("codex".to_string()),
        );
        legacy.insert(
            "launch_args".into(),
            toml::Value::Array(vec![
                toml::Value::String("--model=gpt-5.4".to_string()),
                toml::Value::String("resume".to_string()),
                toml::Value::String("sess-legacy".to_string()),
            ]),
        );
        legacy.insert(
            "created_at".into(),
            toml::Value::try_from(session.created_at).unwrap(),
        );
        legacy.insert(
            "updated_at".into(),
            toml::Value::try_from(session.updated_at).unwrap(),
        );
        legacy.insert(
            "last_activity_at".into(),
            toml::Value::try_from(session.last_activity_at).unwrap(),
        );
        legacy.insert(
            "display_name".into(),
            toml::Value::String(session.display_name.clone()),
        );

        std::fs::write(&path, toml::to_string(&legacy).unwrap()).unwrap();

        let loaded = Session::load(&path).unwrap();
        assert!(
            loaded
                .launch_args
                .iter()
                .any(|arg| arg == "--no-alt-screen"),
            "legacy Codex sessions should be normalized to preserve inline scrollback"
        );
        assert_eq!(
            loaded.launch_args,
            vec![
                "--no-alt-screen".to_string(),
                "--model=gpt-5.4".to_string(),
                "resume".to_string(),
                "sess-legacy".to_string(),
            ]
        );
    }

    #[test]
    fn load_nonexistent_returns_error() {
        let result = Session::load(Path::new("/nonexistent/session.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn load_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "not valid toml {{{{").unwrap();
        let result = Session::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn hook_runtime_state_maps_running_and_waiting_events() {
        for event in ["UserPromptSubmit", "PreToolUse", "PostToolUse"] {
            let runtime = SessionRuntimeState::from_hook_event(event).expect("running event");
            assert_eq!(runtime.status, AgentStatus::Running, "{event}");
            assert_eq!(runtime.source_event.as_deref(), Some(event));
        }

        let session_start =
            SessionRuntimeState::from_hook_event("SessionStart").expect("session start event");
        assert_eq!(session_start.status, AgentStatus::WaitingInput);
        assert_eq!(session_start.source_event.as_deref(), Some("SessionStart"));

        let waiting = SessionRuntimeState::from_hook_event("Stop").expect("waiting event");
        assert_eq!(waiting.status, AgentStatus::WaitingInput);
        assert_eq!(waiting.source_event.as_deref(), Some("Stop"));

        assert!(SessionRuntimeState::from_hook_event("Notification").is_none());
    }

    #[test]
    fn runtime_state_save_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime").join("session-123.json");
        let first = SessionRuntimeState::new(AgentStatus::Running);
        first.save(&path).unwrap();

        let second = SessionRuntimeState::new(AgentStatus::WaitingInput);
        second.save(&path).unwrap();

        let loaded = SessionRuntimeState::load(&path).unwrap();
        assert_eq!(loaded.status, AgentStatus::WaitingInput);
    }

    #[test]
    fn runtime_state_path_scopes_sidecars_to_current_process_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = runtime_state_path(dir.path(), "session-123");

        assert_eq!(
            path,
            dir.path()
                .join("runtime")
                .join(std::process::id().to_string())
                .join("session-123.json")
        );
    }

    #[test]
    fn reset_runtime_state_dir_for_pid_clears_only_target_pid_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let current_pid = 4242_u32;
        let other_pid = 4343_u32;
        let current_dir = dir.path().join("runtime").join(current_pid.to_string());
        let other_dir = dir.path().join("runtime").join(other_pid.to_string());

        std::fs::create_dir_all(&current_dir).unwrap();
        std::fs::create_dir_all(&other_dir).unwrap();
        std::fs::write(current_dir.join("session-a.json"), "{}").unwrap();
        std::fs::write(other_dir.join("session-b.json"), "{}").unwrap();

        reset_runtime_state_dir_for_pid(dir.path(), current_pid).unwrap();

        assert!(current_dir.is_dir());
        assert_eq!(std::fs::read_dir(&current_dir).unwrap().count(), 0);
        assert!(other_dir.join("session-b.json").exists());
    }
}
