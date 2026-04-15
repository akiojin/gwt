//! `gwt hook runtime-state <event>` — write a tiny JSON state file that
//! tells the Branches tab whether the agent session is currently running
//! or waiting for user input.
//!
//! Ported from the retired external runtime hook and now used as the
//! managed runtime hook implementation wired from settings.

use std::{
    io,
    path::{Path, PathBuf},
};

use chrono::{SecondsFormat, Utc};
use gwt_agent::{PendingDiscussionResume, Session, GWT_SESSION_ID_ENV};
use serde::Serialize;

use super::HookError;
use crate::discussion_resume::load_pending_resume;

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
    match event {
        "SessionStart" | "Stop" => Some("WaitingInput"),
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => Some("Running"),
        _ => None,
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
    let session = Session::load(&sessions_dir.join(format!("{session_id}.toml")))?;
    load_pending_resume(&session.worktree_path)
}

fn current_session_from_env(sessions_dir: &Path) -> io::Result<Option<Session>> {
    let Some(session_id) = std::env::var_os(GWT_SESSION_ID_ENV) else {
        return Ok(None);
    };
    let path = sessions_dir.join(format!("{}.toml", session_id.to_string_lossy()));
    if !path.exists() {
        return Ok(None);
    }
    Session::load(&path).map(Some)
}

#[cfg(test)]
fn sync_coordination_for_session(_session: &Session, _event: &str) {}

/// Production entry point. Reads `$GWT_SESSION_RUNTIME_PATH` and delegates
/// to [`write_for_event`]. An unset env var is a silent no-op so that
/// sessions launched outside of gwt (e.g. a raw `claude` invocation) are
/// not broken by a hook we shipped.
pub fn handle(event: &str) -> Result<(), HookError> {
    let Some(path) = std::env::var_os("GWT_SESSION_RUNTIME_PATH") else {
        return Ok(());
    };
    let path = PathBuf::from(path);
    write_for_event(&path, event)
}

#[cfg(test)]
mod tests {
    use gwt_agent::{AgentId, Session};
    use gwt_core::coordination::load_snapshot;

    use super::*;

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
        assert_eq!(state.status, "WaitingInput");
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
    fn sync_coordination_for_session_running_event_does_not_append_message() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "PreToolUse");

        let snapshot = load_snapshot(dir.path()).unwrap();
        assert!(snapshot.board.entries.is_empty());
        let events =
            std::fs::read_to_string(dir.path().join(".gwt/coordination/events.jsonl")).unwrap();
        assert_eq!(events.lines().count(), 0);
    }

    #[test]
    fn sync_coordination_for_session_session_start_does_not_append_board_status_entry() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "SessionStart");

        let snapshot = load_snapshot(dir.path()).unwrap();
        assert!(snapshot.board.entries.is_empty());

        let events =
            std::fs::read_to_string(dir.path().join(".gwt/coordination/events.jsonl")).unwrap();
        assert_eq!(events.lines().count(), 0);
    }

    #[test]
    fn sync_coordination_for_session_stop_does_not_append_board_status_entry() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "Stop");

        let snapshot = load_snapshot(dir.path()).unwrap();
        assert!(snapshot.board.entries.is_empty());

        let events =
            std::fs::read_to_string(dir.path().join(".gwt/coordination/events.jsonl")).unwrap();
        assert_eq!(events.lines().count(), 0);
    }

    #[test]
    fn sync_coordination_for_session_skips_noop_status_updates() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "PreToolUse");
        sync_coordination_for_session(&session, "PostToolUse");

        let snapshot = load_snapshot(dir.path()).unwrap();
        assert!(snapshot.board.entries.is_empty());

        let events =
            std::fs::read_to_string(dir.path().join(".gwt/coordination/events.jsonl")).unwrap();
        assert_eq!(events.lines().count(), 0);
    }
}
