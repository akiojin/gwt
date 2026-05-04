//! Pure planning core for `board-reminder`.
//!
//! [`plan_reminder`] decides what reminder text and reminders-sidecar
//! transition to emit for the three intent-boundary events. It is pure
//! (no IO) and takes preloaded Board entries plus the current reminders
//! sidecar through [`ReminderInputs`]. The IO wrapper lives in `mod.rs`.

use chrono::{DateTime, Utc};
use gwt_core::coordination::{BoardEntry, RemindersState};

use super::format::{filter_and_cap_latest, injection_text, session_start_text};
use super::texts::{
    SESSION_START_CAP, STOP_REMINDER, STOP_REMINDER_SHORT, USER_PROMPT_DIFF_CAP,
    USER_PROMPT_REMINDER, USER_PROMPT_REMINDER_SHORT,
};
use super::{HookOutput, IntentBoundaryEvent};

/// Pure inputs for [`plan_reminder`]. The caller is responsible for loading
/// Board entries (already filtered to the event's time window) and the
/// reminders sidecar; this function performs no IO.
#[derive(Debug, Clone)]
pub struct ReminderInputs {
    pub event: IntentBoundaryEvent,
    pub now: DateTime<Utc>,
    pub self_session_id: String,
    pub display_name: String,
    /// Identifiers used to detect self-targeted entries (SPEC-1974 FR-041).
    /// Typically the session id, the active branch, and the agent display
    /// name; an entry whose `target_owners` contains any of these values is
    /// rendered with the structured marker (see `texts::FOR_YOU_MARKER`).
    /// Empty disables highlighting.
    pub self_match_keys: Vec<String>,
    /// Board entries preloaded for the event's window:
    /// - `SessionStart`: entries whose `updated_at` is within the last 24h.
    /// - `UserPromptSubmit`: entries whose `updated_at > last_injected_at`.
    /// - `Stop`: ignored (pass empty).
    pub recent_entries: Vec<BoardEntry>,
    pub reminders: RemindersState,
    /// Whether the current agent has posted a status entry within the
    /// configured redundancy window. Used to pick between the full and
    /// short reminder text.
    pub has_recent_own_status: bool,
}

#[derive(Debug, Clone)]
pub struct ReminderPlan {
    pub output: HookOutput,
    pub next_reminders: RemindersState,
}

/// Pure core: decide what to emit and how the reminders sidecar should
/// transition for the three intent-boundary events.
pub fn plan_reminder(inputs: ReminderInputs) -> ReminderPlan {
    match inputs.event {
        IntentBoundaryEvent::SessionStart => plan_session_start(inputs),
        IntentBoundaryEvent::UserPromptSubmit => plan_user_prompt_submit(inputs),
        IntentBoundaryEvent::Stop => plan_stop(inputs),
    }
}

fn plan_session_start(inputs: ReminderInputs) -> ReminderPlan {
    let entries = filter_and_cap_latest(
        inputs.recent_entries,
        &inputs.self_session_id,
        SESSION_START_CAP,
    );
    let text = session_start_text(&entries, &inputs.self_match_keys);
    let mut next = inputs.reminders;
    next.last_injected_at = Some(inputs.now);
    ReminderPlan {
        output: HookOutput::hook_specific_additional_context(
            IntentBoundaryEvent::SessionStart,
            text,
        ),
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
        format!(
            "{}\n\n{}",
            injection_text(&entries, &inputs.self_match_keys),
            reminder
        )
    };

    let mut next = inputs.reminders;
    next.last_injected_at = Some(inputs.now);
    ReminderPlan {
        output: HookOutput::hook_specific_additional_context(
            IntentBoundaryEvent::UserPromptSubmit,
            context,
        ),
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
        output: HookOutput::system_message(text),
        next_reminders: inputs.reminders,
    }
}

pub(super) fn plan_event(output: &HookOutput) -> IntentBoundaryEvent {
    match output {
        HookOutput::HookSpecificAdditionalContext { event, .. } => *event,
        HookOutput::SystemMessage(_) => IntentBoundaryEvent::Stop,
        HookOutput::PreToolUsePermission { .. }
        | HookOutput::Silent
        | HookOutput::StopBlock { .. } => {
            panic!("board reminder plans must emit intent-boundary output")
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind};

    use super::*;

    fn entry_with_target(
        author: &str,
        kind: BoardEntryKind,
        body: &str,
        origin_branch: &str,
        origin_session: &str,
        timestamp: DateTime<Utc>,
        target_owners: Vec<String>,
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
        .with_origin_session_id(origin_session)
        .with_target_owners(target_owners);
        e.created_at = timestamp;
        e.updated_at = timestamp;
        e
    }

    fn entry(
        author: &str,
        kind: BoardEntryKind,
        body: &str,
        origin_branch: &str,
        origin_session: &str,
        timestamp: DateTime<Utc>,
    ) -> BoardEntry {
        entry_with_target(
            author,
            kind,
            body,
            origin_branch,
            origin_session,
            timestamp,
            vec![],
        )
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
    fn plan_user_prompt_submit_contains_phase_and_do_not_guard() {
        let plan = plan_reminder(ReminderInputs {
            event: IntentBoundaryEvent::UserPromptSubmit,
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec![],
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        });
        let text = additional_context(&plan.output);
        assert!(text.contains("phase"));
        assert!(text.contains("Do NOT"));
    }

    #[test]
    fn plan_stop_reminder_is_user_facing() {
        let plan = plan_reminder(ReminderInputs {
            event: IntentBoundaryEvent::Stop,
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec![],
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        });
        let text = system_message(&plan.output);
        assert!(text.contains("Stop"));
        assert!(text.contains("completed"));
        assert!(text.contains("Board"));
        assert!(text.contains("gwtd board post"));
    }

    #[test]
    fn plan_user_prompt_submit_includes_coordination_axes() {
        let plan = plan_reminder(ReminderInputs {
            event: IntentBoundaryEvent::UserPromptSubmit,
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec![],
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        });
        let text = additional_context(&plan.output);

        for axis in ["claim", "next", "blocked", "handoff", "decision"] {
            assert!(
                text.contains(axis),
                "USER_PROMPT_REMINDER should promote coordination axis '{axis}', got:\n{text}"
            );
        }
        assert!(text.contains("--kind claim"));
        assert!(text.contains("--kind handoff"));
        assert!(text.contains("--target"));
    }

    #[test]
    fn plan_user_prompt_submit_includes_workspace_policy_guidance() {
        let plan = plan_reminder(ReminderInputs {
            event: IntentBoundaryEvent::UserPromptSubmit,
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec![],
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        });
        let text = additional_context(&plan.output);

        assert!(text.contains("AGENTS.md is project-local"));
        assert!(text.contains("Do NOT create, switch, or delete branches"));
        assert!(text.contains("git worktree add"));
        assert!(text.contains("Start Work"));
    }

    #[test]
    fn plan_user_prompt_submit_short_reminder_mentions_coordination() {
        let plan = plan_reminder(ReminderInputs {
            event: IntentBoundaryEvent::UserPromptSubmit,
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec![],
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: true,
        });
        let text = additional_context(&plan.output);
        assert!(text.contains("coordination"));
        assert!(text.contains("AGENTS.md is project-local"));
        assert!(text.contains("Do NOT create, switch, or delete branches"));
    }

    #[test]
    fn plan_session_start_marks_for_you_when_target_matches_session_id() {
        let now = Utc::now();
        let other = entry_with_target(
            "OtherAgent",
            BoardEntryKind::Claim,
            "claim feature/foo migration",
            "feature/other",
            "sess-other",
            now,
            vec!["sess-1".into()],
        );

        let plan = plan_reminder(ReminderInputs {
            event: IntentBoundaryEvent::SessionStart,
            now,
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec!["sess-1".into(), "feature/me".into(), "Codex".into()],
            recent_entries: vec![other],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        });
        let text = additional_context(&plan.output);
        let entry_line = text
            .lines()
            .find(|line| line.contains("claim feature/foo migration"))
            .expect("entry line missing");
        assert!(
            entry_line.contains(">>"),
            "for-you marker missing on entry targeted at self session id: {entry_line}"
        );
    }

    #[test]
    fn plan_session_start_marks_for_you_when_target_matches_branch() {
        let now = Utc::now();
        let other = entry_with_target(
            "OtherAgent",
            BoardEntryKind::Handoff,
            "handing off to feature/me",
            "feature/other",
            "sess-other",
            now,
            vec!["feature/me".into()],
        );

        let plan = plan_reminder(ReminderInputs {
            event: IntentBoundaryEvent::SessionStart,
            now,
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec!["sess-1".into(), "feature/me".into(), "Codex".into()],
            recent_entries: vec![other],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        });
        let text = additional_context(&plan.output);
        let entry_line = text
            .lines()
            .find(|line| line.contains("handing off to feature/me"))
            .expect("entry line missing");
        assert!(
            entry_line.contains(">>"),
            "for-you marker missing on entry targeted at self branch: {entry_line}"
        );
    }

    #[test]
    fn plan_session_start_no_for_you_marker_when_target_owners_empty() {
        let now = Utc::now();
        let other = entry(
            "OtherAgent",
            BoardEntryKind::Status,
            "broadcast status",
            "feature/other",
            "sess-other",
            now,
        );

        let plan = plan_reminder(ReminderInputs {
            event: IntentBoundaryEvent::SessionStart,
            now,
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec!["sess-1".into(), "feature/me".into(), "Codex".into()],
            recent_entries: vec![other],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        });
        let text = additional_context(&plan.output);
        let entry_line = text
            .lines()
            .find(|line| line.contains("broadcast status"))
            .expect("entry line missing");
        assert!(
            !entry_line.contains(">>"),
            "broadcast entry must not be highlighted: {entry_line}"
        );
    }

    #[test]
    fn plan_user_prompt_submit_short_reminder_when_redundant() {
        let plan = plan_reminder(ReminderInputs {
            event: IntentBoundaryEvent::UserPromptSubmit,
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec![],
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: true,
        });
        assert!(additional_context(&plan.output).contains("posted to the Board recently"));
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
            event: IntentBoundaryEvent::SessionStart,
            now,
            self_session_id: "sess-1".into(),
            display_name: "Claude".into(),
            self_match_keys: vec![],
            recent_entries: entries,
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        });
        let text = additional_context(&plan.output);
        assert!(text.contains("investigating broken test"));
        assert!(!text.contains("my own should be excluded"));
        assert!(text.contains("sess-other"));
        assert!(text.contains("feature/other"));
        assert_eq!(plan.next_reminders.last_injected_at, Some(now));
    }

    #[test]
    fn plan_user_prompt_submit_empty_diff_still_emits_reminder() {
        let plan = plan_reminder(ReminderInputs {
            event: IntentBoundaryEvent::UserPromptSubmit,
            now: Utc::now(),
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec![],
            recent_entries: vec![],
            reminders: RemindersState::default(),
            has_recent_own_status: false,
        });
        let text = additional_context(&plan.output);
        assert!(text.contains("Board Post Reminder"));
        assert!(!text.contains("Recent Board updates"));
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
            event: IntentBoundaryEvent::Stop,
            now,
            self_session_id: "sess-1".into(),
            display_name: "Codex".into(),
            self_match_keys: vec![],
            recent_entries: vec![],
            reminders,
            has_recent_own_status: false,
        });
        assert_eq!(plan.next_reminders.last_injected_at, Some(before));
    }
}
