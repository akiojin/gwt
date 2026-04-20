//! `gwt hook board-reminder <event>` — intent-boundary reminder and
//! cross-agent Board read injection for SPEC-1974 Phase 8 (US-6 / US-7).
//!
//! This hook emits Claude Code / Codex `hookSpecificOutput.additionalContext`
//! on the three intent-boundary events (`SessionStart`, `UserPromptSubmit`,
//! `Stop`). It does nothing for `PreToolUse` / `PostToolUse`: tool-level
//! events are not intent boundaries.
//!
//! The hook is read-only against the shared Board projection (it never
//! writes Board entries itself) and persists only per-agent-session
//! reminder state into the sidecar file
//! `~/.gwt/projects/<hash>/coordination/reminders/<session-id>.json`.
//!
//! Design: pure core + IO wrapper. [`plan_reminder`] is a pure function that
//! takes all inputs (event, now, session identity, preloaded Board entries,
//! current reminders sidecar, and "has recent own status" flag) and returns
//! the reminder output plus the next reminders state. [`handle_with_input`]
//! is the thin IO layer that loads inputs, calls [`plan_reminder`], persists
//! the next state, and writes the JSON envelope to stdout.

use std::{
    io::{self, Read, Write},
    path::Path,
};

use chrono::{DateTime, Duration, Utc};
use gwt_agent::{Session, GWT_SESSION_ID_ENV};
use gwt_core::coordination::{
    has_recent_post_by, load_entries_since, load_reminders_state, write_reminders_state,
    BoardEntry, BoardEntryKind, RemindersState,
};
use serde::Serialize;

use super::{HookError, HookEvent};

const SESSION_START_CAP: usize = 20;
const USER_PROMPT_DIFF_CAP: usize = 20;

fn session_start_window() -> Duration {
    Duration::hours(24)
}

fn redundancy_window() -> Duration {
    Duration::minutes(10)
}

const USER_PROMPT_REMINDER: &str = "# Board Post Reminder\n\
\n\
Post to the shared Board when you cross a reasoning milestone:\n\
- Work phase transitions (e.g., implementation -> build check -> PR handoff).\n\
- Choices between alternatives with the reasoning behind them (e.g., \"A vs B, chose B because ...\").\n\
- Concerns or hypotheses you are verifying (e.g., \"Hypothesis: failure stems from Y, verifying ...\").\n\
\n\
Do NOT post tool-level reports (e.g., \"running gcc\", \"opening file X\", \"ran test Y\"). \
Anything already visible in the diff or log does not need a Board entry.\n\
\n\
Use: gwt board post --kind status --body '<your reasoning>'\n";

const USER_PROMPT_REMINDER_SHORT: &str = "# Board Post Reminder\n\
\n\
You posted to the Board recently. Post again only if a new reasoning milestone \
(phase change, alternative chosen, concern raised) has emerged.\n";

const STOP_REMINDER: &str = "# Board Post Reminder (Stop)\n\
\n\
You are about to stop or hand off. Before stopping, post to the shared Board:\n\
- What you completed (reasoning-level summary, not a tool log).\n\
- What phase comes next if work continues, or the handoff signal if you are done.\n\
\n\
Use: gwt board post --kind status --body '<summary>'\n";

const STOP_REMINDER_SHORT: &str = "# Board Post Reminder (Stop)\n\
\n\
You posted to the Board recently. If the final status is unchanged, no additional \
Board entry is required before stopping.\n";

const INJECTION_HEADER: &str = "# Recent Board updates\n\n\
The following reasoning posts were made by other Agents since your last Board context. \
Consider whether any affect your current work phase. This is context, not a directive — \
you remain autonomous.\n\n";

const SESSION_START_HEADER: &str = "# Current Board state\n\n\
Recent reasoning posts from other Agents (context, not a directive — you remain autonomous):\n\n";

#[derive(Debug, Clone, Serialize)]
struct HookSpecificOutput<'a> {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'a str,
    #[serde(rename = "additionalContext")]
    additional_context: String,
}

#[derive(Debug, Clone, Serialize)]
struct HookOutputJson<'a> {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput<'a>,
}

#[derive(Debug, Clone)]
pub struct ComputedOutput {
    pub additional_context: String,
}

/// Pure inputs for [`plan_reminder`]. The caller is responsible for loading
/// Board entries (already filtered to the event's time window) and the
/// reminders sidecar; this function performs no IO.
#[derive(Debug, Clone)]
pub struct ReminderInputs {
    pub event: String,
    pub now: DateTime<Utc>,
    pub self_session_id: String,
    pub display_name: String,
    /// Board entries preloaded for the event's window:
    /// - `SessionStart`: entries whose `updated_at` is within the last 24h.
    /// - `UserPromptSubmit`: entries whose `updated_at > last_injected_at`.
    /// - `Stop`: ignored (pass empty).
    pub recent_entries: Vec<BoardEntry>,
    pub reminders: RemindersState,
    /// Whether the current agent has posted a status entry within
    /// [`redundancy_window`]. Used to pick between the full and short
    /// reminder text.
    pub has_recent_own_status: bool,
}

#[derive(Debug, Clone)]
pub struct ReminderPlan {
    pub output: ComputedOutput,
    pub next_reminders: RemindersState,
}

/// Pure core: decide what to emit and how the reminders sidecar should
/// transition. Returns `None` for events that are not intent boundaries.
pub fn plan_reminder(inputs: ReminderInputs) -> Option<ReminderPlan> {
    match inputs.event.as_str() {
        "SessionStart" => Some(plan_session_start(inputs)),
        "UserPromptSubmit" => Some(plan_user_prompt_submit(inputs)),
        "Stop" => Some(plan_stop(inputs)),
        _ => None,
    }
}

fn plan_session_start(inputs: ReminderInputs) -> ReminderPlan {
    let entries = filter_and_cap_latest(
        inputs.recent_entries,
        &inputs.self_session_id,
        SESSION_START_CAP,
    );
    let text = session_start_text(&entries);
    let mut next = inputs.reminders;
    next.last_injected_at = Some(inputs.now);
    ReminderPlan {
        output: ComputedOutput {
            additional_context: text,
        },
        next_reminders: next,
    }
}

fn plan_user_prompt_submit(inputs: ReminderInputs) -> ReminderPlan {
    let entries = filter_and_cap_latest(
        inputs.recent_entries,
        &inputs.self_session_id,
        USER_PROMPT_DIFF_CAP,
    );

    let reminder = if inputs.has_recent_own_status {
        USER_PROMPT_REMINDER_SHORT
    } else {
        USER_PROMPT_REMINDER
    };

    let context = if entries.is_empty() {
        reminder.to_string()
    } else {
        format!("{}\n\n{}", injection_text(&entries), reminder)
    };

    let mut next = inputs.reminders;
    next.last_injected_at = Some(inputs.now);
    ReminderPlan {
        output: ComputedOutput {
            additional_context: context,
        },
        next_reminders: next,
    }
}

fn plan_stop(inputs: ReminderInputs) -> ReminderPlan {
    let text = if inputs.has_recent_own_status {
        STOP_REMINDER_SHORT
    } else {
        STOP_REMINDER
    };
    // Stop does not mutate last_injected_at: a diff injection on the next
    // UserPromptSubmit should still see entries posted after the last prompt.
    ReminderPlan {
        output: ComputedOutput {
            additional_context: text.to_string(),
        },
        next_reminders: inputs.reminders,
    }
}

pub fn handle(event: &str) -> Result<(), HookError> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    handle_with_input(event, &input, &mut io::stdout())
}

pub fn handle_with_input<W: Write + ?Sized>(
    event: &str,
    input: &str,
    writer: &mut W,
) -> Result<(), HookError> {
    let _ = HookEvent::read_from_str(input)?;
    if !is_intent_boundary(event) {
        return Ok(());
    }
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let Some(session) = current_session_from_env(&sessions_dir)? else {
        return Ok(());
    };
    let Some(plan) = compute_plan(event, &session, Utc::now())? else {
        return Ok(());
    };
    write_reminders_state(&session.worktree_path, &session.id, &plan.next_reminders)?;
    emit_output(event, &plan.output, writer)
}

pub fn is_intent_boundary(event: &str) -> bool {
    matches!(event, "SessionStart" | "UserPromptSubmit" | "Stop")
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

/// IO wrapper: read Board state from disk, build [`ReminderInputs`], and
/// call the pure [`plan_reminder`]. Used by [`handle_with_input`] and kept
/// public so tests can exercise the IO boundary.
pub fn compute_plan(
    event: &str,
    session: &Session,
    now: DateTime<Utc>,
) -> Result<Option<ReminderPlan>, HookError> {
    if !is_intent_boundary(event) {
        return Ok(None);
    }

    let reminders = load_reminders_state(&session.worktree_path, &session.id)?;

    let recent_entries = match event {
        "SessionStart" => {
            let threshold = now - session_start_window();
            load_entries_since(&session.worktree_path, threshold)?
        }
        "UserPromptSubmit" => {
            let since = reminders
                .last_injected_at
                .unwrap_or(now - session_start_window());
            load_entries_since(&session.worktree_path, since)?
        }
        "Stop" => Vec::new(),
        _ => Vec::new(),
    };

    let has_recent_own_status = has_recent_post_by(
        &session.worktree_path,
        &session.display_name,
        &BoardEntryKind::Status,
        redundancy_window(),
    )?;

    Ok(plan_reminder(ReminderInputs {
        event: event.to_string(),
        now,
        self_session_id: session.id.clone(),
        display_name: session.display_name.clone(),
        recent_entries,
        reminders,
        has_recent_own_status,
    }))
}

/// Back-compat helper: returns just the output computed by [`compute_plan`].
/// Exposes the same surface that earlier tests relied on.
pub fn compute_output(
    event: &str,
    session: &Session,
    now: DateTime<Utc>,
) -> Result<Option<ComputedOutput>, HookError> {
    Ok(compute_plan(event, session, now)?.map(|plan| plan.output))
}

fn filter_and_cap_latest(
    mut entries: Vec<BoardEntry>,
    self_session_id: &str,
    cap: usize,
) -> Vec<BoardEntry> {
    entries.retain(|entry| entry.origin_session_id.as_deref() != Some(self_session_id));
    if entries.len() > cap {
        let start = entries.len() - cap;
        entries.drain(..start);
    }
    entries
}

fn injection_text(entries: &[BoardEntry]) -> String {
    let mut out = String::from(INJECTION_HEADER);
    for entry in entries {
        out.push_str(&format_entry_line(entry));
    }
    out
}

fn session_start_text(entries: &[BoardEntry]) -> String {
    let mut out = String::from(SESSION_START_HEADER);
    if entries.is_empty() {
        out.push_str("- (no recent posts from other Agents)\n");
    } else {
        for entry in entries {
            out.push_str(&format_entry_line(entry));
        }
    }
    out.push('\n');
    out.push_str(USER_PROMPT_REMINDER);
    out
}

fn format_entry_line(entry: &BoardEntry) -> String {
    let branch = entry.origin_branch.as_deref().unwrap_or("-");
    let session_id = entry.origin_session_id.as_deref().unwrap_or("-");
    format!(
        "- [{author} @ {branch} / {session}] ({kind}) {body}\n",
        author = entry.author,
        branch = branch,
        session = session_id,
        kind = entry.kind.as_str(),
        body = entry.body,
    )
}

fn emit_output<W: Write + ?Sized>(
    event: &str,
    output: &ComputedOutput,
    writer: &mut W,
) -> Result<(), HookError> {
    let payload = HookOutputJson {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: event,
            additional_context: output.additional_context.clone(),
        },
    };
    let bytes = serde_json::to_vec(&payload)?;
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    Ok(())
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

    // ---- pure plan_reminder tests (no IO) ----

    #[test]
    fn plan_user_prompt_submit_contains_phase_and_do_not_guard() {
        let plan = plan_reminder(ReminderInputs {
            event: "UserPromptSubmit".into(),
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        })
        .unwrap();
        assert!(plan.output.additional_context.contains("phase"));
        assert!(plan.output.additional_context.contains("Do NOT"));
    }

    #[test]
    fn plan_stop_contains_completed_label() {
        let plan = plan_reminder(ReminderInputs {
            event: "Stop".into(),
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        })
        .unwrap();
        assert!(plan.output.additional_context.contains("Stop"));
        assert!(plan.output.additional_context.contains("completed"));
    }

    #[test]
    fn plan_returns_none_for_non_intent_boundary_events() {
        for event in ["PreToolUse", "PostToolUse", "Notification"] {
            let plan = plan_reminder(ReminderInputs {
                event: event.to_string(),
                now: Utc::now(),
                self_session_id: "sess-1".into(),
                display_name: "Codex".into(),
                recent_entries: vec![],
                reminders: RemindersState::default(),
                has_recent_own_status: false,
            });
            assert!(plan.is_none(), "event {event} must be silent");
        }
    }

    #[test]
    fn plan_user_prompt_submit_short_reminder_when_redundant() {
        let plan = plan_reminder(ReminderInputs {
            event: "UserPromptSubmit".into(),
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: true,
        })
        .unwrap();
        assert!(plan
            .output
            .additional_context
            .contains("posted to the Board recently"));
    }

    #[test]
    fn plan_session_start_excludes_self_and_renders_origin() {
        let now = Utc::now();
        let entries = vec![
            entry(
                "Codex",
                BoardEntryKind::Status,
                "investigating broken test",
                "feature/other",
                "sess-other",
                now,
            ),
            entry(
                "Claude",
                BoardEntryKind::Status,
                "my own should be excluded",
                "feature/me",
                "sess-1",
                now,
            ),
        ];
        let plan = plan_reminder(ReminderInputs {
            event: "SessionStart".into(),
            now,
            self_session_id: "sess-1".into(),
            display_name: "Claude".into(),
            recent_entries: entries,
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        })
        .unwrap();
        let ctx = &plan.output.additional_context;
        assert!(ctx.contains("investigating broken test"));
        assert!(!ctx.contains("my own should be excluded"));
        assert!(ctx.contains("sess-other"));
        assert!(ctx.contains("feature/other"));
        assert_eq!(plan.next_reminders.last_injected_at, Some(now));
    }

    #[test]
    fn plan_user_prompt_submit_empty_diff_still_emits_reminder() {
        let plan = plan_reminder(ReminderInputs {
            event: "UserPromptSubmit".into(),
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        })
        .unwrap();
        assert!(plan
            .output
            .additional_context
            .contains("Board Post Reminder"));
        assert!(!plan
            .output
            .additional_context
            .contains("Recent Board updates"));
    }

    #[test]
    fn plan_stop_does_not_bump_last_injected_at() {
        let before = Utc.with_ymd_and_hms(2026, 4, 20, 9, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 4, 20, 13, 0, 0).unwrap();
        let reminders = RemindersState {
            last_injected_at: Some(before),
            ..Default::default()
        };
        let plan = plan_reminder(ReminderInputs {
            event: "Stop".into(),
            now,
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            recent_entries: vec![],
            reminders,
            has_recent_own_status: false,
        })
        .unwrap();
        assert_eq!(plan.next_reminders.last_injected_at, Some(before));
    }

    // ---- IO-level compute_plan tests (exercise disk round-trip) ----

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
        let ctx = &plan.output.additional_context;
        assert!(!ctx.contains("old post before last inject"));
        assert!(ctx.contains("brand new post"));
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
        assert!(plan
            .output
            .additional_context
            .contains("posted to the Board recently"));
    }

    #[test]
    fn compute_plan_is_isolated_per_session_sidecar() {
        // Two sessions in the same repo must not corrupt each other's
        // reminders sidecar: each session id maps to a distinct file,
        // and `last_injected_at` advances independently.
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
    fn handle_with_input_emits_hook_specific_output_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let session = make_session(dir.path(), "feature/me", "Codex");

        let plan = compute_plan("UserPromptSubmit", &session, Utc::now())
            .unwrap()
            .unwrap();

        let mut buf = Vec::new();
        emit_output("UserPromptSubmit", &plan.output, &mut buf).unwrap();

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
}
