//! `gwtd hook runtime-state <event>` — write a tiny JSON state file that
//! tells the Branches tab whether the agent session is currently running
//! or waiting for user input.
//!
//! Ported from the retired external runtime hook and now used as the
//! managed runtime hook implementation wired from settings.

use std::{
    io,
    io::Read,
    path::{Path, PathBuf},
};

use chrono::{SecondsFormat, Utc};
use gwt_agent::{
    persist_agent_session_id, persist_session_status, AgentStatus, PendingDiscussionResume, Session,
};
use serde::Serialize;

use super::{
    resolve_hook_agent_session_id, GwtSessionId, HookAgentSessionId, HookError, HookSessionId,
    RawHookEvent,
};
use crate::discussion_resume::load_pending_resume;
use crate::window_state::window_state_for_hook_event;

/// The JSON shape the Branches tab polls from `$GWT_SESSION_RUNTIME_PATH`.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct RuntimeState {
    pub status: String,
    pub updated_at: String,
    pub last_activity_at: String,
    pub source_event: String,
    #[serde(default)]
    pub pending_discussion: Option<PendingDiscussionResume>,
}

/// Map a hook event name to the runtime status it should produce.
///
/// Returns `None` for event names that settings_local.rs should never
/// forward to this handler. Callers translate `None` into a
/// [`HookError::InvalidEvent`].
pub fn status_for_event(event: &str) -> Option<&'static str> {
    match window_state_for_hook_event(event)? {
        crate::persistence::WindowState::Running => Some("Running"),
        crate::persistence::WindowState::Starting => Some("Starting"),
        crate::persistence::WindowState::Idle => Some("Idle"),
        crate::persistence::WindowState::Waiting => Some("Waiting"),
        crate::persistence::WindowState::Stopped => Some("Stopped"),
        crate::persistence::WindowState::Error => Some("Error"),
    }
}

/// Serialize a [`RuntimeState`] for the given event and write it atomically
/// to `path`. On success, no `.tmp-*` siblings remain.
pub fn write_for_event(path: &Path, event: &str) -> Result<(), HookError> {
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let session = current_session_from_env(&sessions_dir);
    let pending_discussion = session.as_ref().and_then(|session| {
        pending_discussion_for_session(&sessions_dir, &session.id)
            .ok()
            .flatten()
    });

    write_for_event_with_pending_discussion(path, event, pending_discussion)?;

    Ok(())
}

fn write_for_event_with_pending_discussion(
    path: &Path,
    event: &str,
    pending_discussion: Option<PendingDiscussionResume>,
) -> Result<(), HookError> {
    let status =
        status_for_event(event).ok_or_else(|| HookError::InvalidEvent(event.to_string()))?;
    write_state_with_status(path, event, status, pending_discussion)
}

fn write_state_with_status(
    path: &Path,
    event: &str,
    status: &str,
    pending_discussion: Option<PendingDiscussionResume>,
) -> Result<(), HookError> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let state = RuntimeState {
        status: status.to_string(),
        updated_at: now.clone(),
        last_activity_at: now,
        source_event: event.to_string(),
        pending_discussion,
    };

    let bytes = serde_json::to_vec_pretty(&state)?;
    gwt_github::cache::write_atomic(path, &bytes)?;
    Ok(())
}

fn pending_discussion_for_session(
    sessions_dir: &Path,
    session_id: &str,
) -> io::Result<Option<PendingDiscussionResume>> {
    let session = Session::load_and_migrate(&sessions_dir.join(format!("{session_id}.toml")))?;
    load_pending_resume(&session.worktree_path)
}

fn current_session_from_env(sessions_dir: &Path) -> Option<Session> {
    let session_id = GwtSessionId::from_env()?;
    current_session_for_id(sessions_dir, &session_id)
}

fn current_session_for_id(sessions_dir: &Path, gwt_session_id: &GwtSessionId) -> Option<Session> {
    let path = sessions_dir.join(format!("{}.toml", gwt_session_id.as_str()));
    if !path.exists() {
        return None;
    }
    match Session::load_and_migrate(&path) {
        Ok(session) => Some(session),
        Err(error) => {
            eprintln!(
                "gwtd hook runtime-state: failed to load session metadata {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn sync_agent_session_id(
    sessions_dir: &Path,
    gwt_session_id: &GwtSessionId,
    agent_session_id: &HookSessionId,
) -> io::Result<()> {
    persist_agent_session_id(
        sessions_dir,
        gwt_session_id.as_str(),
        agent_session_id.as_str(),
    )
}

fn validated_hook_agent_session_id(
    event: &str,
    gwt_session_id: &GwtSessionId,
    session: Option<&Session>,
    hook_event: Option<&RawHookEvent>,
) -> Result<Option<HookSessionId>, HookError> {
    match resolve_hook_agent_session_id(session, hook_event) {
        HookAgentSessionId::Provided(session_id) => Ok(Some(session_id)),
        HookAgentSessionId::MissingRequiredForCodex => {
            // Fail open, mirroring daemon_runtime::live_event_agent_session_id.
            // Codex omits a usable session_id on tool-use events (PreToolUse /
            // PostToolUse), but the id was already captured at SessionStart and
            // persisted in the session .toml. Returning Ok(None) skips
            // sync_agent_session_id (so the persisted id is preserved and the
            // "agent-session" placeholder is never written) while the caller
            // still writes runtime state and records the hook event. Failing
            // closed here surfaces "hook exited with code 1" to the user on
            // every tool call for no functional gain. Only warn when there is
            // genuinely no persisted id to fall back to.
            if session.and_then(Session::exact_resume_session_id).is_none() {
                log_missing_codex_hook_session_id(event, gwt_session_id, session, hook_event);
            }
            Ok(None)
        }
        HookAgentSessionId::MissingOptional => Ok(None),
    }
}

fn log_missing_codex_hook_session_id(
    event: &str,
    gwt_session_id: &GwtSessionId,
    session: Option<&Session>,
    hook_event: Option<&RawHookEvent>,
) {
    let persisted_agent_session_id = session
        .and_then(gwt_agent::Session::exact_resume_session_id)
        .unwrap_or("-");
    let tool_name = hook_event.and_then(RawHookEvent::tool_name).unwrap_or("-");
    eprintln!(
        "gwtd hook runtime-state: missing Codex hook session_id event={event} gwt_session_id={} persisted_agent_session_id={persisted_agent_session_id} tool_name={tool_name}",
        gwt_session_id.as_str()
    );
}

fn log_session_metadata_error(action: &str, gwt_session_id: &GwtSessionId, error: &io::Error) {
    eprintln!(
        "gwtd hook runtime-state: failed to {action} session metadata gwt_session_id={}: {error}",
        gwt_session_id.as_str()
    );
}

#[cfg(test)]
fn sync_coordination_for_session(_session: &Session, _event: &str) {}

/// Production entry point. Reads `$GWT_SESSION_RUNTIME_PATH` and delegates
/// to [`write_for_event`]. An unset env var is a silent no-op so that
/// sessions launched outside of gwt (e.g. a raw `claude` invocation) are
/// not broken by a hook we shipped.
pub fn handle(event: &str) -> Result<(), HookError> {
    if std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV).is_none() {
        return Ok(());
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    handle_with_input(event, &input)
}

pub fn handle_with_input(event: &str, input: &str) -> Result<(), HookError> {
    let Some(runtime_path) = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV) else {
        return Ok(());
    };
    let runtime_path = PathBuf::from(runtime_path);
    let hook_event = if input.trim().is_empty() {
        None
    } else {
        RawHookEvent::read_from_str(input)?
    };
    let sessions_dir = sessions_dir_for_runtime_path(&runtime_path);
    let gwt_session_id = GwtSessionId::required_from_env(event)?;
    let session = current_session_for_id(&sessions_dir, &gwt_session_id);
    let agent_session_id = validated_hook_agent_session_id(
        event,
        &gwt_session_id,
        session.as_ref(),
        hook_event.as_ref(),
    )?;
    if let Some(agent_session_id) = agent_session_id.as_ref() {
        if let Err(error) = sync_agent_session_id(&sessions_dir, &gwt_session_id, agent_session_id)
        {
            log_session_metadata_error("sync agent_session_id for", &gwt_session_id, &error);
        }
    }
    if session.is_some() {
        if let Err(error) =
            gwt_agent::persist_session_hook_event(&sessions_dir, gwt_session_id.as_str(), event)
        {
            log_session_metadata_error("record hook event for", &gwt_session_id, &error);
        }
    }

    let pending_discussion = session.as_ref().and_then(|session| {
        pending_discussion_for_session(&sessions_dir, &session.id)
            .ok()
            .flatten()
    });
    write_for_event_with_pending_discussion(&runtime_path, event, pending_discussion)
}

pub fn record_completed_stop_from_env() -> Result<(), HookError> {
    let Some(gwt_session_id) = GwtSessionId::from_env() else {
        return Ok(());
    };
    let sessions_dir = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .map(PathBuf::from)
        .map(|path| sessions_dir_for_runtime_path(&path))
        .unwrap_or_else(gwt_core::paths::gwt_sessions_dir);
    if let Err(error) =
        gwt_agent::persist_session_completed_stop(&sessions_dir, gwt_session_id.as_str())
    {
        log_session_metadata_error("record completed Stop for", &gwt_session_id, &error);
    }
    Ok(())
}

pub fn record_blocked_stop_from_env() -> Result<(), HookError> {
    let Some(gwt_session_id) = GwtSessionId::from_env() else {
        return Ok(());
    };
    let Some(runtime_path) = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV) else {
        return Ok(());
    };
    let runtime_path = PathBuf::from(runtime_path);
    let sessions_dir = sessions_dir_for_runtime_path(&runtime_path);
    let session = current_session_for_id(&sessions_dir, &gwt_session_id);
    if session.is_some() {
        if let Err(error) =
            persist_session_status(&sessions_dir, gwt_session_id.as_str(), AgentStatus::Running)
        {
            log_session_metadata_error("restore blocked Stop status for", &gwt_session_id, &error);
        }
    }
    let pending_discussion = session.as_ref().and_then(|session| {
        pending_discussion_for_session(&sessions_dir, &session.id)
            .ok()
            .flatten()
    });
    write_state_with_status(&runtime_path, "Stop", "Running", pending_discussion)
}

fn sessions_dir_for_runtime_path(runtime_path: &Path) -> PathBuf {
    gwt_agent::sessions_dir_from_runtime_path(runtime_path)
        .unwrap_or_else(gwt_core::paths::gwt_sessions_dir)
}

#[cfg(test)]
mod tests {
    use gwt_agent::{AgentId, Session, GWT_SESSION_ID_ENV};
    use gwt_core::coordination::{coordination_events_segments_dir, load_snapshot};
    use std::ffi::OsString;
    use std::time::Duration;

    use super::*;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn runtime_state_env_lock_shares_crate_wide_env_lock() {
        let global = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (tx, rx) = std::sync::mpsc::channel();

        let handle = std::thread::spawn(move || {
            started_tx.send(()).expect("send lock attempt started");
            let _lock = env_lock();
            tx.send(()).expect("send lock acquired");
        });

        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("env lock probe thread started");
        assert!(
            rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "runtime_state tests must wait on the crate-wide env lock"
        );
        drop(global);
        handle.join().expect("env lock probe thread");
    }

    struct EnvGuard {
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        fn new() -> Self {
            Self { saved: Vec::new() }
        }

        fn set(&mut self, key: &'static str, value: impl Into<OsString>) {
            if !self.saved.iter().any(|(saved, _)| *saved == key) {
                self.saved.push((key, std::env::var_os(key)));
            }
            std::env::set_var(key, value.into());
        }

        fn unset(&mut self, key: &'static str) {
            if !self.saved.iter().any(|(saved, _)| *saved == key) {
                self.saved.push((key, std::env::var_os(key)));
            }
            std::env::remove_var(key);
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            while let Some((key, value)) = self.saved.pop() {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn assert_no_board_entries_or_events(root: &std::path::Path) {
        let snapshot = load_snapshot(root).unwrap();
        assert!(snapshot.board.entries.is_empty());
        let event_lines = std::fs::read_dir(coordination_events_segments_dir(root))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
            .map(|entry| {
                std::fs::read_to_string(entry.path())
                    .unwrap()
                    .lines()
                    .count()
            })
            .sum::<usize>();
        assert_eq!(event_lines, 0);
    }

    #[test]
    fn pending_discussion_for_session_reads_active_discussion_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("wt-feature");
        let discussion_path = gwt_core::paths::gwt_repo_local_discussions_path(&worktree);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(
            discussion_path,
            r#"## Discussion TODO

### Proposal A - Hook-driven resume [active]
- Summary:
- Open Questions:
- Dependency Checks:
- Deferred Decisions:
- Next Question: Should SessionStart surface the proposal?
- Promotable Changes:
"#,
        )
        .unwrap();

        let session = Session::new(&worktree, "feature", AgentId::Codex);
        session.save(&sessions_dir).unwrap();

        let pending = pending_discussion_for_session(&sessions_dir, &session.id).unwrap();

        assert_eq!(
            pending,
            Some(PendingDiscussionResume {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Hook-driven resume".to_string(),
                next_question: Some("Should SessionStart surface the proposal?".to_string()),
            })
        );
    }

    #[test]
    fn write_for_event_with_pending_discussion_persists_resume_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime-state.json");

        write_for_event_with_pending_discussion(
            &path,
            "Stop",
            Some(PendingDiscussionResume {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Hook-driven resume".to_string(),
                next_question: Some("Should SessionStart surface the proposal?".to_string()),
            }),
        )
        .unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Idle");
        assert_eq!(state.source_event, "Stop");
        assert_eq!(
            state.pending_discussion,
            Some(PendingDiscussionResume {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Hook-driven resume".to_string(),
                next_question: Some("Should SessionStart surface the proposal?".to_string()),
            })
        );
    }

    #[test]
    fn status_for_event_maps_idle_and_running_lifecycle_events() {
        assert_eq!(status_for_event("SessionStart"), Some("Idle"));
        assert_eq!(status_for_event("UserPromptSubmit"), Some("Running"));
        assert_eq!(status_for_event("PreToolUse"), Some("Running"));
        assert_eq!(status_for_event("PostToolUse"), Some("Running"));
        assert_eq!(status_for_event("Stop"), Some("Idle"));
    }

    #[test]
    fn handle_with_input_ignores_corrupt_session_metadata() {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(sessions_dir.join("session-corrupt.toml"), "odex\"").unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "session-corrupt");
        let mut env = EnvGuard::new();
        env.set(GWT_SESSION_ID_ENV, "session-corrupt");
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str(),
        );

        handle_with_input("PostToolUse", "{}").expect("corrupt metadata must not fail the hook");

        let raw = std::fs::read_to_string(&runtime_path).unwrap();
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Running");
        assert_eq!(state.source_event, "PostToolUse");
    }

    #[test]
    fn handle_with_input_fails_open_when_corrupt_metadata_has_hook_session_id() {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(
            sessions_dir.join("session-corrupt.toml"),
            "started_at = \"2026-06-16T04:30:00.\n320310Z\"",
        )
        .unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "session-corrupt");
        let mut env = EnvGuard::new();
        env.set(GWT_SESSION_ID_ENV, "session-corrupt");
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str(),
        );

        handle_with_input("SessionStart", r#"{"session_id":"agent-123"}"#)
            .expect("corrupt metadata with a hook session_id must fail open");

        let raw = std::fs::read_to_string(&runtime_path).unwrap();
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Idle");
        assert_eq!(state.source_event, "SessionStart");
    }

    #[test]
    fn record_completed_stop_fails_open_when_session_metadata_is_corrupt() {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(sessions_dir.join("session-corrupt.toml"), "169630Z\"").unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "session-corrupt");
        let mut env = EnvGuard::new();
        env.set(GWT_SESSION_ID_ENV, "session-corrupt");
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str(),
        );

        handle_with_input("Stop", "{}").expect("Stop sidecar write must succeed");
        record_completed_stop_from_env()
            .expect("completed-stop metadata update must fail open on corrupt TOML");

        let raw = std::fs::read_to_string(&runtime_path).unwrap();
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Idle");
        assert_eq!(state.source_event, "Stop");
    }

    #[test]
    fn sync_coordination_for_session_running_event_does_not_append_message() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "PreToolUse");

        assert_no_board_entries_or_events(dir.path());
    }

    #[test]
    fn sync_coordination_for_session_session_start_does_not_append_board_status_entry() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "SessionStart");

        assert_no_board_entries_or_events(dir.path());
    }

    #[test]
    fn sync_coordination_for_session_stop_does_not_append_board_status_entry() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "Stop");

        assert_no_board_entries_or_events(dir.path());
    }

    #[test]
    fn sync_coordination_for_session_skips_noop_status_updates() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "PreToolUse");
        sync_coordination_for_session(&session, "PostToolUse");

        assert_no_board_entries_or_events(dir.path());
    }

    #[test]
    fn sync_agent_session_id_persists_value_into_session_toml() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        let gwt_session_id = GwtSessionId::from_env().unwrap();

        let agent_session_id = RawHookEvent::read_from_str(r#"{"session_id":"agent-123"}"#)
            .unwrap()
            .unwrap()
            .session_id()
            .unwrap();
        sync_agent_session_id(&sessions_dir, &gwt_session_id, &agent_session_id).unwrap();

        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("agent-123"));
    }

    #[test]
    fn sync_agent_session_id_does_not_clear_existing_value_when_hook_session_id_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let mut session = Session::new(dir.path(), "feature/demo", AgentId::Codex);
        session.agent_session_id = Some("agent-existing".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();

        let parsed = RawHookEvent::read_from_str(r#"{"tool_name":"Bash"}"#)
            .unwrap()
            .unwrap()
            .session_id();
        assert!(parsed.is_none());

        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("agent-existing"));
    }

    #[test]
    fn codex_runtime_state_fails_open_when_hook_session_id_missing() {
        // A Codex tool-use payload without a session_id (the shape Codex
        // actually sends on PreToolUse/PostToolUse) must NOT break the tool
        // call. The agent_session_id was already captured at SessionStart, so
        // the hook fails open: it preserves the persisted id (never writing the
        // "agent-session" placeholder) and still records runtime state.
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let mut session = Session::new(&worktree, "feature/demo", AgentId::Codex);
        session.agent_session_id = Some("agent-existing".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );
        env.unset("CODEX_THREAD_ID");

        handle_with_input("PreToolUse", r#"{"tool_name":"Bash"}"#)
            .expect("missing Codex hook session_id must fail open, not break the tool call");

        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("agent-existing"));
        assert_eq!(loaded.last_hook_event.as_deref(), Some("PreToolUse"));

        let raw = std::fs::read_to_string(&runtime_path).expect("runtime state written");
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Running");
        assert_eq!(state.source_event, "PreToolUse");
    }

    #[test]
    fn codex_runtime_state_fails_open_when_hook_session_id_blank() {
        // A blank session_id resolves to MissingRequiredForCodex with no
        // persisted id to fall back to. The hook must still fail open: write
        // runtime state and leave agent_session_id unset (never the placeholder
        // or the blank value).
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let session = Session::new(&worktree, "feature/demo", AgentId::Codex);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );
        env.unset("CODEX_THREAD_ID");

        handle_with_input("PostToolUse", r#"{"session_id":"   "}"#)
            .expect("blank Codex session_id must fail open");

        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id, None);

        let raw = std::fs::read_to_string(&runtime_path).expect("runtime state written");
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Running");
        assert_eq!(state.source_event, "PostToolUse");
    }

    #[test]
    fn managed_runtime_state_rejects_missing_gwt_session_id() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "missing-gwt-session");
        env.unset(GWT_SESSION_ID_ENV);
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        let err = handle_with_input("PreToolUse", r#"{"session_id":"agent-123"}"#)
            .expect_err("managed hooks must include GWT_SESSION_ID");

        match err {
            HookError::InvalidEvent(message) => {
                assert!(message.contains("missing GWT_SESSION_ID"), "{message}");
            }
            other => panic!("expected InvalidEvent, got {other:?}"),
        }
        assert!(
            !runtime_path.exists(),
            "invalid managed identity must not write state"
        );
    }

    #[test]
    fn non_codex_runtime_state_allows_missing_hook_session_id_for_compatibility() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let session = Session::new(&worktree, "feature/demo", AgentId::ClaudeCode);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id);
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        handle_with_input("PreToolUse", r#"{"tool_name":"Bash"}"#).unwrap();

        assert!(runtime_path.exists());
    }

    #[test]
    fn runtime_state_persists_hook_lifecycle_to_session_toml() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let session = Session::new(&worktree, "feature/demo", AgentId::ClaudeCode);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        handle_with_input("PreToolUse", r#"{"tool_name":"Bash"}"#).unwrap();
        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();

        assert_eq!(loaded.last_hook_event.as_deref(), Some("PreToolUse"));
        assert!(loaded.last_hook_event_at.is_some());
        assert!(loaded.last_completed_stop_at.is_none());
        assert_eq!(loaded.status, gwt_agent::AgentStatus::Running);
        assert!(loaded.should_mark_interrupted_from_lifecycle());
    }

    #[test]
    fn runtime_state_records_completed_stop_as_clean_boundary() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let session = Session::new(&worktree, "feature/demo", AgentId::ClaudeCode);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        handle_with_input("Stop", r#"{}"#).unwrap();
        let stopped_before_completion =
            Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert!(stopped_before_completion.should_mark_interrupted_from_lifecycle());

        record_completed_stop_from_env().unwrap();
        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();

        assert_eq!(loaded.last_hook_event.as_deref(), Some("Stop"));
        assert!(loaded.last_completed_stop_at.is_some());
        assert_eq!(serde_json::to_string(&loaded.status).unwrap(), "\"Idle\"");
        assert!(!loaded.should_mark_interrupted_from_lifecycle());
    }
}
