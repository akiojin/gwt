//! `gwtd hook board-reminder <event>` — intent-boundary reminder and
//! cross-agent Board read injection for SPEC-1974 Phase 8 (US-6 / US-7).
//!
//! SessionStart and UserPromptSubmit emit Claude Code / Codex
//! `hookSpecificOutput.additionalContext`, which is injected into the
//! agent's context. `Stop` emits top-level `systemMessage` because
//! Claude Code rejects `hookSpecificOutput` on Stop, so that reminder is
//! user-facing rather than agent-injected. PreToolUse / PostToolUse
//! remain silent: tool-level events are not intent boundaries.
//!
//! Module layout (SPEC-1974 Phase 10 module restructure):
//!
//! - `texts`: reminder string constants, caps, windows, and the
//!   `FOR_YOU_MARKER` (FR-043 single source of truth)
//! - `plan`: pure planning core ([`ReminderInputs`], [`ReminderPlan`],
//!   [`plan_reminder`]) — no IO
//! - `format`: entry-line formatting and self-target detection
//! - this module (`mod.rs`): IO wrapper that loads disk state, calls the
//!   pure planner, and writes the JSON envelope to stdout

mod format;
mod plan;
mod texts;

use std::{
    io::{self, Read},
    path::Path,
};

use chrono::{DateTime, Utc};
use gwt_agent::{Session, GWT_SESSION_ID_ENV};
use gwt_core::coordination::{
    has_recent_post_by, load_entries_since, load_reminders_state, write_reminders_state,
    BoardEntryKind,
};

use super::{HookError, HookEvent, HookOutput, IntentBoundaryEvent};

pub use plan::{plan_reminder, ReminderInputs, ReminderPlan};

use plan::plan_event;
use texts::{redundancy_window, session_start_window};

pub fn handle(event: &str) -> Result<(), HookError> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let output = handle_with_input(event, &input)?;
    output.serialize_to(&mut io::stdout())
}

pub fn handle_with_input(event: &str, input: &str) -> Result<HookOutput, HookError> {
    let _ = HookEvent::read_from_str(input)?;
    let Some(intent_event) = IntentBoundaryEvent::from_name(event) else {
        return Ok(HookOutput::Silent);
    };
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let Some(session) = current_session_from_env(&sessions_dir)? else {
        return Ok(HookOutput::Silent);
    };
    let Some(plan) = compute_plan(event, &session, Utc::now())? else {
        return Ok(HookOutput::Silent);
    };
    write_reminders_state(&session.worktree_path, &session.id, &plan.next_reminders)?;
    debug_assert_eq!(intent_event, plan_event(&plan.output));
    Ok(plan.output)
}

pub fn is_intent_boundary(event: &str) -> bool {
    IntentBoundaryEvent::from_name(event).is_some()
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

/// Build the OR-match key list for self-targeted entry detection
/// (SPEC-1974 FR-041). Includes the session id, the active branch, and
/// the agent display name when each is non-empty.
fn build_self_match_keys(session: &Session) -> Vec<String> {
    let mut keys = Vec::with_capacity(3);
    if !session.id.trim().is_empty() {
        keys.push(session.id.clone());
    }
    if !session.branch.trim().is_empty() {
        keys.push(session.branch.clone());
    }
    if !session.display_name.trim().is_empty() {
        keys.push(session.display_name.clone());
    }
    keys
}

/// IO wrapper: read Board state from disk, build [`ReminderInputs`], and
/// call the pure [`plan_reminder`]. Used by [`handle_with_input`] and kept
/// public so tests can exercise the IO boundary.
pub fn compute_plan(
    event: &str,
    session: &Session,
    now: DateTime<Utc>,
) -> Result<Option<ReminderPlan>, HookError> {
    let Some(intent_event) = IntentBoundaryEvent::from_name(event) else {
        return Ok(None);
    };

    let reminders = load_reminders_state(&session.worktree_path, &session.id)?;

    let recent_entries = match intent_event {
        IntentBoundaryEvent::SessionStart => {
            let threshold = now - session_start_window();
            load_entries_since(&session.worktree_path, threshold)?
        }
        IntentBoundaryEvent::UserPromptSubmit => {
            let since = reminders
                .last_injected_at
                .unwrap_or(now - session_start_window());
            load_entries_since(&session.worktree_path, since)?
        }
        IntentBoundaryEvent::Stop => Vec::new(),
    };

    let has_recent_own_status = has_recent_post_by(
        &session.worktree_path,
        &session.display_name,
        &BoardEntryKind::Status,
        redundancy_window(),
    )?;

    let self_match_keys = build_self_match_keys(session);

    Ok(Some(plan_reminder(ReminderInputs {
        event: intent_event,
        now,
        self_session_id: session.id.clone(),
        display_name: session.display_name.clone(),
        self_match_keys,
        recent_entries,
        reminders,
        has_recent_own_status,
    })))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use gwt_agent::AgentId;
    use gwt_core::coordination::{post_entry, AuthorKind, BoardEntry, BoardEntryKind};

    use super::*;

    fn make_session(dir: &Path, branch: &str, display_name: &str) -> Session {
        let mut session = Session::new(dir, branch, AgentId::Codex);
        session.display_name = display_name.to_string();
        session
    }

    fn entry(
        author: &str,
        kind: BoardEntryKind,
        body: &str,
        origin_branch: &str,
        origin_session: &str,
        timestamp: DateTime<Utc>,
    ) -> BoardEntry {
        let mut e = BoardEntry::new(
            AuthorKind::Agent,
            author,
            kind,
            body,
            None,
            None,
            vec![],
            vec![],
        )
        .with_origin_branch(origin_branch)
        .with_origin_session_id(origin_session);
        e.created_at = timestamp;
        e.updated_at = timestamp;
        e
    }

    fn push_entry(
        root: &Path,
        author: &str,
        kind: BoardEntryKind,
        body: &str,
        origin_branch: &str,
        origin_session: &str,
    ) -> BoardEntry {
        let e = BoardEntry::new(
            AuthorKind::Agent,
            author,
            kind,
            body,
            None,
            None,
            vec![],
            vec![],
        )
        .with_origin_branch(origin_branch)
        .with_origin_session_id(origin_session);
        post_entry(root, e.clone()).unwrap();
        e
    }

    fn additional_context(output: &HookOutput) -> &str {
        match output {
            HookOutput::HookSpecificAdditionalContext { text, .. } => text,
            other => panic!("expected additional context output, got {other:?}"),
        }
    }

    fn system_message(output: &HookOutput) -> &str {
        match output {
            HookOutput::SystemMessage(text) => text,
            other => panic!("expected system message output, got {other:?}"),
        }
    }

    #[test]
    fn non_intent_boundary_events_are_filtered_before_planning() {
        assert!(!is_intent_boundary("PreToolUse"));
        assert!(!is_intent_boundary("PostToolUse"));
        assert!(!is_intent_boundary("Notification"));
    }

    #[test]
    fn build_self_match_keys_includes_id_branch_agent() {
        let dir = tempfile::tempdir().unwrap();
        let session = make_session(dir.path(), "feature/me", "Codex");
        let keys = build_self_match_keys(&session);
        assert!(keys.iter().any(|k| k == &session.id), "session id missing");
        assert!(keys.iter().any(|k| k == "feature/me"), "branch missing");
        assert!(keys.iter().any(|k| k == "Codex"), "display name missing");
    }

    #[test]
    fn build_self_match_keys_skips_empty_fields() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new(dir.path(), "", AgentId::Codex);
        session.display_name = String::new();
        let keys = build_self_match_keys(&session);
        // Only the session id (always non-empty after Session::new) survives.
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], session.id);
    }

    #[test]
    fn compute_plan_session_start_persists_last_injected_at_via_handle() {
        let dir = tempfile::tempdir().unwrap();
        let session = make_session(dir.path(), "feature/me", "Codex");
        let now = Utc::now();

        let plan = compute_plan("SessionStart", &session, now)
            .unwrap()
            .unwrap();
        write_reminders_state(&session.worktree_path, &session.id, &plan.next_reminders).unwrap();

        let state = load_reminders_state(&session.worktree_path, &session.id).unwrap();
        assert_eq!(state.last_injected_at, Some(now));
    }

    #[test]
    fn compute_plan_user_prompt_submit_uses_last_injected_at_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let session = make_session(dir.path(), "feature/me", "Claude");

        let before = Utc.with_ymd_and_hms(2026, 4, 20, 10, 0, 0).unwrap();
        let last_inject = Utc.with_ymd_and_hms(2026, 4, 20, 11, 0, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 4, 20, 12, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 4, 20, 13, 0, 0).unwrap();

        let old = entry(
            "Codex",
            BoardEntryKind::Status,
            "old post before last inject",
            "feature/codex",
            "sess-codex-old",
            before,
        );
        post_entry(dir.path(), old).unwrap();

        let new_e = entry(
            "Codex",
            BoardEntryKind::Status,
            "brand new post",
            "feature/codex",
            "sess-codex-new",
            after,
        );
        post_entry(dir.path(), new_e).unwrap();

        let mut state = load_reminders_state(&session.worktree_path, &session.id).unwrap();
        state.last_injected_at = Some(last_inject);
        write_reminders_state(&session.worktree_path, &session.id, &state).unwrap();

        let plan = compute_plan("UserPromptSubmit", &session, now)
            .unwrap()
            .unwrap();
        let text = additional_context(&plan.output);
        assert!(!text.contains("old post before last inject"));
        assert!(text.contains("brand new post"));
        assert_eq!(plan.next_reminders.last_injected_at, Some(now));
    }

    #[test]
    fn compute_plan_returns_none_for_pre_and_post_tool_use() {
        let dir = tempfile::tempdir().unwrap();
        let session = make_session(dir.path(), "feature/me", "Codex");
        for event in ["PreToolUse", "PostToolUse"] {
            assert!(compute_plan(event, &session, Utc::now()).unwrap().is_none());
        }
    }

    #[test]
    fn compute_plan_redundancy_shortens_user_prompt_reminder() {
        let dir = tempfile::tempdir().unwrap();
        let session = make_session(dir.path(), "feature/me", "Codex");
        push_entry(
            dir.path(),
            "Codex",
            BoardEntryKind::Status,
            "recent self status",
            "feature/me",
            &session.id,
        );

        let plan = compute_plan("UserPromptSubmit", &session, Utc::now())
            .unwrap()
            .unwrap();
        assert!(additional_context(&plan.output).contains("posted to the Board recently"));
    }

    #[test]
    fn compute_plan_redundancy_shortens_stop_reminder() {
        let dir = tempfile::tempdir().unwrap();
        let session = make_session(dir.path(), "feature/me", "Codex");
        push_entry(
            dir.path(),
            "Codex",
            BoardEntryKind::Status,
            "recent self status",
            "feature/me",
            &session.id,
        );

        let plan = compute_plan("Stop", &session, Utc::now()).unwrap().unwrap();
        assert!(system_message(&plan.output).contains("posted to the Board recently"));
    }

    #[test]
    fn compute_plan_is_isolated_per_session_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let session_a = make_session(dir.path(), "feature/a", "Codex");
        let session_b = make_session(dir.path(), "feature/b", "Codex");
        assert_ne!(session_a.id, session_b.id);

        let t_a = Utc.with_ymd_and_hms(2026, 4, 20, 10, 0, 0).unwrap();
        let t_b = Utc.with_ymd_and_hms(2026, 4, 20, 11, 0, 0).unwrap();

        let plan_a = compute_plan("SessionStart", &session_a, t_a)
            .unwrap()
            .unwrap();
        write_reminders_state(
            &session_a.worktree_path,
            &session_a.id,
            &plan_a.next_reminders,
        )
        .unwrap();
        let plan_b = compute_plan("SessionStart", &session_b, t_b)
            .unwrap()
            .unwrap();
        write_reminders_state(
            &session_b.worktree_path,
            &session_b.id,
            &plan_b.next_reminders,
        )
        .unwrap();

        let state_a = load_reminders_state(&session_a.worktree_path, &session_a.id).unwrap();
        let state_b = load_reminders_state(&session_b.worktree_path, &session_b.id).unwrap();
        assert_eq!(state_a.last_injected_at, Some(t_a));
        assert_eq!(state_b.last_injected_at, Some(t_b));
    }

    #[test]
    fn user_prompt_submit_serializes_additional_context_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let session = make_session(dir.path(), "feature/me", "Codex");

        let plan = compute_plan("UserPromptSubmit", &session, Utc::now())
            .unwrap()
            .unwrap();

        let mut buf = Vec::new();
        plan.output.serialize_to(&mut buf).unwrap();

        let text = String::from_utf8(buf).unwrap();
        let json: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(
            json["hookSpecificOutput"]["hookEventName"],
            serde_json::json!("UserPromptSubmit")
        );
        assert!(json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Board Post Reminder"));
    }

    #[test]
    fn stop_serializes_system_message_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let session = make_session(dir.path(), "feature/me", "Codex");

        let plan = compute_plan("Stop", &session, Utc::now()).unwrap().unwrap();

        let mut buf = Vec::new();
        plan.output.serialize_to(&mut buf).unwrap();

        let text = String::from_utf8(buf).unwrap();
        let json: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "systemMessage": system_message(&plan.output)
            })
        );
    }
}
