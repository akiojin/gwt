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
use gwt_agent::{persist_agent_session_id, PendingDiscussionResume, Session};
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
        crate::persistence::WindowState::Waiting => Some("Waiting"),
        crate::persistence::WindowState::Stopped => Some("Stopped"),
        crate::persistence::WindowState::Error => Some("Error"),
    }
}

/// Serialize a [`RuntimeState`] for the given event and write it atomically
/// to `path`. On success, no `.tmp-*` siblings remain.
pub fn write_for_event(path: &Path, event: &str) -> Result<(), HookError> {
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let session = current_session_from_env(&sessions_dir)?;
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

fn current_session_from_env(sessions_dir: &Path) -> io::Result<Option<Session>> {
    let Some(session_id) = GwtSessionId::from_env() else {
        return Ok(None);
    };
    current_session_for_id(sessions_dir, &session_id)
}

fn current_session_for_id(
    sessions_dir: &Path,
    gwt_session_id: &GwtSessionId,
) -> io::Result<Option<Session>> {
    let path = sessions_dir.join(format!("{}.toml", gwt_session_id.as_str()));
    if !path.exists() {
        return Ok(None);
    }
    Session::load_and_migrate(&path).map(Some)
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
            log_missing_codex_hook_session_id(event, gwt_session_id, session, hook_event);
            Err(HookError::InvalidEvent(format!(
                "missing session_id for Codex hook event {event}"
            )))
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
        .and_then(|session| session.agent_session_id.as_deref())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or("-");
    let tool_name = hook_event.and_then(RawHookEvent::tool_name).unwrap_or("-");
    eprintln!(
        "gwtd hook runtime-state: missing Codex hook session_id event={event} gwt_session_id={} persisted_agent_session_id={persisted_agent_session_id} tool_name={tool_name}",
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
    let session = current_session_for_id(&sessions_dir, &gwt_session_id)?;
    let agent_session_id = validated_hook_agent_session_id(
        event,
        &gwt_session_id,
        session.as_ref(),
        hook_event.as_ref(),
    )?;
    if let Some(agent_session_id) = agent_session_id.as_ref() {
        sync_agent_session_id(&sessions_dir, &gwt_session_id, agent_session_id)?;
    }

    let pending_discussion = session.as_ref().and_then(|session| {
        pending_discussion_for_session(&sessions_dir, &session.id)
            .ok()
            .flatten()
    });
    write_for_event_with_pending_discussion(&runtime_path, event, pending_discussion)
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
        std::fs::create_dir_all(worktree.join(".gwt")).unwrap();
        std::fs::write(
            worktree.join(".gwt/discussion.md"),
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
        assert_eq!(state.status, "Waiting");
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
    fn status_for_event_maps_stop_to_waiting_and_runtime_events_to_running() {
        assert_eq!(status_for_event("SessionStart"), Some("Running"));
        assert_eq!(status_for_event("UserPromptSubmit"), Some("Running"));
        assert_eq!(status_for_event("PreToolUse"), Some("Running"));
        assert_eq!(status_for_event("PostToolUse"), Some("Running"));
        assert_eq!(status_for_event("Stop"), Some("Waiting"));
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
    fn codex_runtime_state_rejects_missing_hook_session_id() {
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

        let err = handle_with_input("PreToolUse", r#"{"tool_name":"Bash"}"#)
            .expect_err("managed Codex hooks must include session_id");

        match err {
            HookError::InvalidEvent(message) => {
                assert!(message.contains("missing session_id"), "{message}");
            }
            other => panic!("expected InvalidEvent, got {other:?}"),
        }
        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("agent-existing"));
        assert!(
            !runtime_path.exists(),
            "invalid payload must not write state"
        );
    }

    #[test]
    fn codex_runtime_state_rejects_blank_hook_session_id() {
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
        env.set(GWT_SESSION_ID_ENV, session_id);
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        let err = handle_with_input("PreToolUse", r#"{"session_id":"   "}"#)
            .expect_err("blank Codex session_id must fail");

        match err {
            HookError::InvalidEvent(message) => {
                assert!(message.contains("missing session_id"), "{message}");
            }
            other => panic!("expected InvalidEvent, got {other:?}"),
        }
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
}
