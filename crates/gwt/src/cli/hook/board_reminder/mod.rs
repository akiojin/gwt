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
    path::{Path, PathBuf},
};

use crate::board_provider::{has_recent_post_by, load_entries_since_for_scope};
use chrono::{DateTime, Utc};
use gwt_agent::{Session, GWT_SESSION_ID_ENV};
use gwt_core::coordination::{
    load_reminders_state, write_reminders_state, BoardEntryKind, RemindersState,
};
use gwt_core::workspace_projection::WorkspaceProjection;

/// SPEC-2359 Phase U-9 (FR-178): minimum number of consecutive
/// UserPromptSubmit turns with an unchanged `title_summary` before the
/// stale reminder fires. Tuned conservatively to avoid reminder fatigue.
const TITLE_SUMMARY_STALE_TURN_THRESHOLD: u32 = 8;

/// Minimum unchanged-progress turns before the cumulative progress summary is
/// considered stale. Lower than title-summary because progress summary is
/// expected to evolve during implementation/verification, not only on scope
/// changes.
const PROGRESS_SUMMARY_STALE_TURN_THRESHOLD: u32 = 4;

/// Issue #2987: throttle window for the memory-update reminder. After the
/// reminder fires it stays silent for this many hours, then fires again, so a
/// long session does not pay the reminder on every UserPromptSubmit turn.
const MEMORY_REMINDER_THROTTLE_WINDOW_HOURS: i64 = 6;

use super::{HookError, HookEvent, HookOutput, IntentBoundaryEvent};
use crate::board_audience::current_session_board_scope;

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
    let hook_event = HookEvent::read_from_str(input)?;
    let Some(intent_event) = IntentBoundaryEvent::from_name(event) else {
        return Ok(HookOutput::Silent);
    };
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let Some(session) = current_session_from_env(&sessions_dir) else {
        return Ok(HookOutput::Silent);
    };
    let session = session_scoped_to_hook_cwd(session, hook_event.as_ref());
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

fn current_session_from_env(sessions_dir: &Path) -> Option<Session> {
    let session_id = std::env::var_os(GWT_SESSION_ID_ENV)?;
    let path = sessions_dir.join(format!("{}.toml", session_id.to_string_lossy()));
    if !path.exists() {
        return None;
    }
    match Session::load(&path) {
        Ok(session) => Some(session),
        Err(error) => {
            eprintln!(
                "gwtd hook board-reminder: failed to load session metadata {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn session_scoped_to_hook_cwd(mut session: Session, hook_event: Option<&HookEvent>) -> Session {
    let Some(cwd) = hook_event.and_then(|event| hook_cwd_path(event.cwd.as_deref())) else {
        return session;
    };
    session.worktree_path = cwd;
    session.repo_hash =
        gwt_core::repo_hash::detect_repo_hash(&session.worktree_path).map(|hash| hash.to_string());
    session
}

fn hook_cwd_path(cwd: Option<&str>) -> Option<PathBuf> {
    let value = cwd.map(str::trim).filter(|value| !value.is_empty())?;
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    if !path.exists() {
        return None;
    }
    Some(dunce::canonicalize(&path).unwrap_or(path))
}

/// Build the OR-match key list for self-targeted entry detection
/// (SPEC-1974 FR-041). Includes legacy raw target keys and typed
/// mention keys so `target_owners` and `mentions` stay backward-compatible.
fn build_self_match_keys(session: &Session) -> Vec<String> {
    let mut keys = Vec::with_capacity(6);
    if !session.id.trim().is_empty() {
        keys.push(session.id.clone());
        keys.push(format!("session:{}", session.id));
    }
    if !session.branch.trim().is_empty() {
        keys.push(session.branch.clone());
        keys.push(format!("branch:{}", session.branch));
    }
    if !session.display_name.trim().is_empty() {
        keys.push(session.display_name.clone());
    }
    let agent_command = session.agent_id.command();
    if !agent_command.trim().is_empty() {
        keys.push(format!("agent:{agent_command}"));
    }
    keys
}

fn agent_title_summary_missing(session: &Session) -> Result<bool, HookError> {
    let project_state_root = crate::agent_project_state::canonical_project_state_root_for_session(
        session,
        &session.worktree_path,
    );
    let projection =
        gwt_core::workspace_projection::load_workspace_projection(&project_state_root)?;
    Ok(title_summary_missing_in_projection(
        projection.as_ref(),
        &session.id,
    ))
}

/// Pure decision for whether `session_id`'s agent still needs a `title_summary`.
///
/// SPEC-2359 Phase W-11 (US-58 / US-46 / FR-179): the title reminder must also
/// fire for Unassigned agents. Start Work / standalone agents are Unassigned
/// yet still need a purpose title; with the prompt-derivation path removed
/// (W-11), an `is_unassigned()` early-return would leave them with neither a
/// derived title nor a reminder. (The derivation path dropped the same guard
/// under US-46/FR-179 for the same reason.) A missing projection or an
/// unregistered session is treated as "not missing" so the reminder only
/// fires once the agent is actually present in the projection.
fn title_summary_missing_in_projection(
    projection: Option<&gwt_core::workspace_projection::WorkspaceProjection>,
    session_id: &str,
) -> bool {
    let Some(projection) = projection else {
        return false;
    };
    let Some(agent) = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session_id)
    else {
        return false;
    };
    agent
        .title_summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
}

fn append_title_summary_required_context(
    output: HookOutput,
    event: IntentBoundaryEvent,
    missing: bool,
    language: &str,
) -> HookOutput {
    if !missing || event == IntentBoundaryEvent::Stop {
        return output;
    }
    let required = texts::title_summary_required_reminder(language);
    match output {
        // SPEC-2359 Phase W-11 (US-58 / FR-347): prepend so the
        // "set a provisional purpose before responding" instruction is the
        // first thing the agent sees, ahead of the board reminder.
        HookOutput::HookSpecificAdditionalContext { event, text } => {
            HookOutput::hook_specific_additional_context(event, format!("{required}\n\n{text}"))
        }
        // Inject even when there is no board reminder this turn, so a fresh
        // agent's first UserPromptSubmit always carries the title instruction.
        HookOutput::Silent => {
            HookOutput::hook_specific_additional_context(event, required.to_string())
        }
        other => other,
    }
}

/// SPEC-2359 Phase U-9 (FR-178): observe the current agent's
/// `title_summary` and `current_focus` against the previously persisted
/// reminder state, returning the updated state plus a `stale` flag that
/// is true only on UserPromptSubmit when the title has stayed unchanged
/// for at least [`TITLE_SUMMARY_STALE_TURN_THRESHOLD`] turns AND
/// `current_focus` drifted within that window. Empty / unset title is
/// owned by the empty-trigger reminder path and never triggers stale.
fn compute_title_summary_stale_state(
    event: IntentBoundaryEvent,
    projection: Option<&WorkspaceProjection>,
    session_id: &str,
    current_state: &RemindersState,
) -> (bool, RemindersState) {
    let mut new_state = current_state.clone();
    if event != IntentBoundaryEvent::UserPromptSubmit {
        return (false, new_state);
    }
    let Some(projection) = projection else {
        return (false, new_state);
    };
    let Some(agent) = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session_id)
    else {
        return (false, new_state);
    };
    let current_title = agent
        .title_summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let current_focus = agent
        .current_focus
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let Some(title) = current_title else {
        // Empty title is owned by the empty-trigger required reminder.
        // Reset stale-detection state so the counter does not bleed
        // across an empty → re-set cycle.
        new_state.last_title_summary_seen = None;
        new_state.unchanged_turn_count = 0;
        new_state.phase_changed_in_window = false;
        new_state.last_current_focus_seen = current_focus;
        return (false, new_state);
    };

    if new_state.last_title_summary_seen.as_ref() == Some(&title) {
        new_state.unchanged_turn_count = new_state.unchanged_turn_count.saturating_add(1);
        if new_state.last_current_focus_seen != current_focus {
            new_state.phase_changed_in_window = true;
        }
    } else {
        new_state.last_title_summary_seen = Some(title);
        new_state.unchanged_turn_count = 0;
        new_state.phase_changed_in_window = false;
    }
    new_state.last_current_focus_seen = current_focus;

    let stale = new_state.unchanged_turn_count >= TITLE_SUMMARY_STALE_TURN_THRESHOLD
        && new_state.phase_changed_in_window;
    (stale, new_state)
}

fn append_title_summary_stale_context(
    output: HookOutput,
    event: IntentBoundaryEvent,
    stale: bool,
    language: &str,
) -> HookOutput {
    if !stale || event != IntentBoundaryEvent::UserPromptSubmit {
        return output;
    }
    let stale_text = texts::title_summary_stale_reminder(language);
    match output {
        HookOutput::HookSpecificAdditionalContext { event, text } => {
            HookOutput::hook_specific_additional_context(event, format!("{text}\n\n{stale_text}"))
        }
        HookOutput::Silent => {
            HookOutput::hook_specific_additional_context(event, stale_text.to_string())
        }
        other => other,
    }
}

fn progress_summary_focus_signal(
    projection: &WorkspaceProjection,
    session_id: &str,
) -> Option<String> {
    let agent_focus = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session_id)
        .and_then(|agent| agent.current_focus.as_deref());
    let parts = [
        agent_focus,
        projection.summary.as_deref(),
        projection.next_action.as_deref(),
        Some(projection.status_text.as_str()),
    ];
    let joined = parts
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!joined.is_empty()).then_some(joined)
}

fn compute_progress_summary_state(
    event: IntentBoundaryEvent,
    projection: Option<&WorkspaceProjection>,
    session_id: &str,
    current_state: &RemindersState,
) -> (bool, bool, RemindersState) {
    let mut new_state = current_state.clone();
    let Some(projection) = projection else {
        return (false, false, new_state);
    };
    let current_progress = projection
        .progress_summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let current_signal = progress_summary_focus_signal(projection, session_id);

    if current_progress.is_none() {
        new_state.last_progress_summary_seen = None;
        new_state.progress_summary_unchanged_turn_count = 0;
        new_state.progress_focus_changed_in_window = false;
        new_state.last_progress_focus_seen = current_signal;
        return (
            matches!(
                event,
                IntentBoundaryEvent::UserPromptSubmit | IntentBoundaryEvent::Stop
            ),
            false,
            new_state,
        );
    }

    if event != IntentBoundaryEvent::UserPromptSubmit {
        new_state.last_progress_summary_seen = current_progress;
        new_state.last_progress_focus_seen = current_signal;
        return (false, false, new_state);
    }

    let progress = current_progress.expect("checked above");
    if new_state.last_progress_summary_seen.as_ref() == Some(&progress) {
        new_state.progress_summary_unchanged_turn_count = new_state
            .progress_summary_unchanged_turn_count
            .saturating_add(1);
        if new_state.last_progress_focus_seen != current_signal {
            new_state.progress_focus_changed_in_window = true;
        }
    } else {
        new_state.last_progress_summary_seen = Some(progress);
        new_state.progress_summary_unchanged_turn_count = 0;
        new_state.progress_focus_changed_in_window = false;
    }
    new_state.last_progress_focus_seen = current_signal;
    let stale = new_state.progress_summary_unchanged_turn_count
        >= PROGRESS_SUMMARY_STALE_TURN_THRESHOLD
        && new_state.progress_focus_changed_in_window;
    (false, stale, new_state)
}

fn append_progress_summary_context(
    output: HookOutput,
    event: IntentBoundaryEvent,
    missing: bool,
    stale: bool,
    language: &str,
) -> HookOutput {
    if !missing && !stale {
        return output;
    }
    let reminder =
        texts::progress_summary_reminder(language, stale, event == IntentBoundaryEvent::Stop);
    match output {
        HookOutput::HookSpecificAdditionalContext { event, text } => {
            HookOutput::hook_specific_additional_context(event, format!("{text}\n\n{reminder}"))
        }
        HookOutput::SystemMessage(text) => {
            HookOutput::system_message(format!("{text}\n\n{reminder}"))
        }
        HookOutput::Silent if event == IntentBoundaryEvent::Stop => {
            HookOutput::system_message(reminder.to_string())
        }
        HookOutput::Silent => {
            HookOutput::hook_specific_additional_context(event, reminder.to_string())
        }
        other => other,
    }
}

fn memory_source_present(worktree_path: &Path) -> bool {
    worktree_path.join(".gwt/work/memory.md").is_file()
        || worktree_path.join("tasks/memory.md").is_file()
        || worktree_path.join("tasks/lessons.md").is_file()
}

/// Issue #2987: throttle the memory-update reminder so it does not inject on
/// every UserPromptSubmit turn. Fires on the first encounter (no prior
/// timestamp) and again only after [`MEMORY_REMINDER_THROTTLE_WINDOW_HOURS`]
/// has elapsed, stamping the fire time into the reminder sidecar. SessionStart
/// never fires (initial onboarding guidance is owned by the SessionStart board
/// path). Stop shares the same throttle so it does not immediately re-nag right
/// after a UserPromptSubmit reminder. Throttling on our own persisted timestamp
/// (not `memory.md` mtime) keeps first-encounter and post-window turns firing
/// even across git operations that would reset file mtime.
fn compute_memory_reminder_state(
    event: IntentBoundaryEvent,
    present: bool,
    current_state: &RemindersState,
    now: DateTime<Utc>,
) -> (bool, RemindersState) {
    let mut new_state = current_state.clone();
    if event == IntentBoundaryEvent::SessionStart {
        return (true, new_state);
    }
    if !present {
        return (true, new_state);
    }
    let fire = match new_state.last_memory_reminded_at {
        None => true,
        Some(last) => now - last >= chrono::Duration::hours(MEMORY_REMINDER_THROTTLE_WINDOW_HOURS),
    };
    if fire {
        new_state.last_memory_reminded_at = Some(now);
        (false, new_state)
    } else {
        (true, new_state)
    }
}

fn append_memory_update_context(
    output: HookOutput,
    event: IntentBoundaryEvent,
    present: bool,
    suppress: bool,
    language: &str,
) -> HookOutput {
    if !present || event == IntentBoundaryEvent::SessionStart || suppress {
        return output;
    }
    let reminder = texts::memory_update_reminder(language, event == IntentBoundaryEvent::Stop);
    match output {
        HookOutput::HookSpecificAdditionalContext { event, text } => {
            HookOutput::hook_specific_additional_context(event, format!("{text}\n\n{reminder}"))
        }
        HookOutput::SystemMessage(text) => {
            HookOutput::system_message(format!("{text}\n\n{reminder}"))
        }
        other => other,
    }
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
    let audience_scope = current_session_board_scope(&session.worktree_path, Some(&session.id))?;
    let self_workspace_id = match &audience_scope {
        gwt_core::coordination::BoardAudienceScope::Workspace(workspace_id) => {
            Some(workspace_id.clone())
        }
        _ => None,
    };

    let recent_entries = match intent_event {
        IntentBoundaryEvent::SessionStart => {
            let threshold = now - session_start_window();
            load_entries_since_for_scope(&session.worktree_path, threshold, &audience_scope)?
        }
        IntentBoundaryEvent::UserPromptSubmit => {
            let since = reminders
                .last_injected_at
                .unwrap_or(now - session_start_window());
            load_entries_since_for_scope(&session.worktree_path, since, &audience_scope)?
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
    let language = resolve_narrative_language();
    // SPEC-3247 FR-003 / AS-4: intake (Curate) sessions own no Work, so the
    // Work-state reminders (title purpose, progress summary) must not fire.
    // Board-read injection and the memory reminder still apply — an intake
    // session still coordinates and records lessons. Absent/unknown signal
    // defaults to Execution, preserving the current behavior (FR-004).
    let session_is_intake = gwt_skills::SessionKind::from_env().is_intake();

    let mut plan = plan_reminder(ReminderInputs {
        event: intent_event,
        now,
        self_session_id: session.id.clone(),
        display_name: session.display_name.clone(),
        self_match_keys,
        recent_entries,
        reminders,
        has_recent_own_status,
        language: language.clone(),
        self_workspace_id,
    });

    if !session_is_intake {
        plan.output = append_title_summary_required_context(
            plan.output,
            intent_event,
            agent_title_summary_missing(session)?,
            &language,
        );
    }

    // The stale/progress reminder state is still advanced for intake to keep a
    // single, uniform compute path (no divergent intake state machine); only
    // the Work-state *text* injection is suppressed by the guards below.
    let project_state_root = crate::agent_project_state::canonical_project_state_root_for_session(
        session,
        &session.worktree_path,
    );
    let projection_for_stale =
        gwt_core::workspace_projection::load_workspace_projection(&project_state_root)?;
    let (stale, updated_state) = compute_title_summary_stale_state(
        intent_event,
        projection_for_stale.as_ref(),
        &session.id,
        &plan.next_reminders,
    );
    plan.next_reminders = updated_state;
    if !session_is_intake {
        plan.output =
            append_title_summary_stale_context(plan.output, intent_event, stale, &language);
    }
    let (progress_missing, progress_stale, progress_state) = compute_progress_summary_state(
        intent_event,
        projection_for_stale.as_ref(),
        &session.id,
        &plan.next_reminders,
    );
    plan.next_reminders = progress_state;
    if !session_is_intake {
        plan.output = append_progress_summary_context(
            plan.output,
            intent_event,
            progress_missing,
            progress_stale,
            &language,
        );
    }
    let memory_present = memory_source_present(&session.worktree_path);
    let (memory_suppress, memory_state) =
        compute_memory_reminder_state(intent_event, memory_present, &plan.next_reminders, now);
    plan.next_reminders = memory_state;
    plan.output = append_memory_update_context(
        plan.output,
        intent_event,
        memory_present,
        memory_suppress,
        &language,
    );

    Ok(Some(plan))
}

/// Resolve the narrative-output language from the global gwt config
/// (SPEC-1933 FR-009 / FR-010). Falls back to `"en"` when settings
/// cannot be loaded.
fn resolve_narrative_language() -> String {
    gwt_config::Settings::load()
        .map(|settings| settings.ai.effective_language().to_string())
        .unwrap_or_else(|_| "en".to_string())
}

#[cfg(test)]
mod tests {
    use crate::cli::test_support::ScopedEnvVar;
    use chrono::TimeZone;
    use gwt_agent::AgentId;
    use gwt_core::{
        coordination::{post_entry, AuthorKind, BoardEntry, BoardEntryKind},
        workspace_projection::{
            save_workspace_projection, WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary,
            WorkspaceProjection, WorkspaceStatusCategory,
        },
    };

    use super::*;

    fn make_session(dir: &Path, branch: &str, display_name: &str) -> Session {
        let mut session = Session::new(dir, branch, AgentId::Codex);
        session.display_name = display_name.to_string();
        session
    }

    fn workspace_agent(
        session_id: &str,
        workspace_id: Option<&str>,
        affiliation_status: WorkspaceAgentAffiliationStatus,
    ) -> WorkspaceAgentSummary {
        WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("Board audience".to_string()),
            title_summary: Some("Board audience".to_string()),
            worktree_path: None,
            branch: Some("work/board-audience".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status,
            workspace_id: workspace_id.map(str::to_string),
            updated_at: Utc::now(),
        }
    }

    fn save_projection(repo: &Path, agents: Vec<WorkspaceAgentSummary>) {
        let mut projection = WorkspaceProjection::default_for_project(repo);
        projection.id = "workspace-current".to_string();
        projection.agents = agents;
        save_workspace_projection(repo, &projection).expect("save workspace projection");
    }

    fn init_repo(path: &Path, origin: &str) {
        std::fs::create_dir_all(path).expect("repo dir");
        let init = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(path)
            .status()
            .expect("git init");
        assert!(init.success(), "git init failed for {}", path.display());
        let remote = std::process::Command::new("git")
            .args(["remote", "add", "origin", origin])
            .current_dir(path)
            .status()
            .expect("git remote add");
        assert!(
            remote.success(),
            "git remote add failed for {}",
            path.display()
        );
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

    #[test]
    fn title_summary_guard_injects_japanese_required_update_when_missing() {
        let output = HookOutput::hook_specific_additional_context(
            IntentBoundaryEvent::UserPromptSubmit,
            "existing reminder",
        );

        let guarded = append_title_summary_required_context(
            output,
            IntentBoundaryEvent::UserPromptSubmit,
            true,
            "ja",
        );

        let HookOutput::HookSpecificAdditionalContext { text, .. } = guarded else {
            panic!("expected additional context");
        };
        assert!(text.contains("existing reminder"));
        assert!(text.contains("title-summary"));
        assert!(text.contains("gwtd <<'JSON'"));
        assert!(text.contains(r#""operation":"workspace.update""#));
        assert!(text.contains(r#""purpose""#));
        assert!(!text.contains("--title-summary"));
        assert!(text.contains("作業名"));
        assert!(text.contains("完了"));
        assert!(text.contains("Use language: ja"));
    }

    /// SPEC-2359 Phase W-11 (US-58 / US-59 / SC-229): the required reminder
    /// must instruct the agent to author the work purpose (not the raw
    /// prompt), set a provisional purpose when it is not settled, and update
    /// it once confirmed — in both Japanese and English.
    #[test]
    fn title_summary_required_reminder_instructs_provisional_purpose() {
        let ja_text = texts::title_summary_required_reminder("ja");
        assert!(ja_text.contains("目的"), "{ja_text}");
        assert!(ja_text.contains("暫定"), "{ja_text}");
        assert!(ja_text.contains("生プロンプト"), "{ja_text}");
        // Imperative: must instruct setting the title before responding.
        assert!(ja_text.contains("応答する前に"), "{ja_text}");
        assert!(ja_text.contains("最初のアクション"), "{ja_text}");
        assert!(
            ja_text.contains(r#""operation":"workspace.update""#),
            "{ja_text}"
        );
        assert!(ja_text.contains(r#""purpose""#), "{ja_text}");
        assert!(!ja_text.contains("--title-summary"), "{ja_text}");

        let en_text = texts::title_summary_required_reminder("en");
        assert!(en_text.contains("purpose"), "{en_text}");
        assert!(en_text.contains("provisional"), "{en_text}");
        assert!(en_text.to_lowercase().contains("raw prompt"), "{en_text}");
        // Imperative: must instruct setting the title before responding.
        assert!(en_text.contains("before you respond"), "{en_text}");
        assert!(en_text.contains("first action"), "{en_text}");
        assert!(
            en_text.contains(r#""operation":"workspace.update""#),
            "{en_text}"
        );
        assert!(en_text.contains(r#""purpose""#), "{en_text}");
        assert!(!en_text.contains("--title-summary"), "{en_text}");
    }

    /// Issue #3184: the required reminder is what pushes agents to write a
    /// title every turn until one is set; it must explicitly forbid transient
    /// activity phases (browser check etc.) as the purpose, in both languages.
    #[test]
    fn title_summary_required_reminder_forbids_transient_activity_labels() {
        let ja_text = texts::title_summary_required_reminder("ja");
        assert!(ja_text.contains("browser check"), "{ja_text}");
        assert!(ja_text.contains("current_focus"), "{ja_text}");

        let en_text = texts::title_summary_required_reminder("en");
        assert!(en_text.contains("browser check"), "{en_text}");
        assert!(en_text.contains("transient activity"), "{en_text}");
        assert!(en_text.contains("current_focus"), "{en_text}");
    }

    /// SPEC-2359 Phase W-11 (US-58 / FR-347): the title-required reminder must
    /// fire even when there is no board reminder this turn (Silent), so a fresh
    /// agent's first UserPromptSubmit always carries the instruction.
    #[test]
    fn title_summary_required_context_injects_even_when_board_is_silent() {
        let guarded = append_title_summary_required_context(
            HookOutput::Silent,
            IntentBoundaryEvent::UserPromptSubmit,
            true,
            "en",
        );
        let HookOutput::HookSpecificAdditionalContext { text, .. } = guarded else {
            panic!("title reminder must inject even when board output is Silent");
        };
        assert!(text.contains("before you respond"), "{text}");
    }

    #[test]
    fn title_summary_guard_is_silent_when_agent_title_is_set() {
        let output = HookOutput::hook_specific_additional_context(
            IntentBoundaryEvent::SessionStart,
            "existing reminder",
        );

        let guarded = append_title_summary_required_context(
            output,
            IntentBoundaryEvent::SessionStart,
            false,
            "en",
        );

        let HookOutput::HookSpecificAdditionalContext { text, .. } = guarded else {
            panic!("expected additional context");
        };
        assert_eq!(text, "existing reminder");
    }

    #[test]
    fn user_prompt_submit_includes_memory_reminder_when_memory_file_exists() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("work dir");
        std::fs::write(repo.join(".gwt/work/memory.md"), "# Memory\n").expect("memory");
        let session = make_session(&repo, "work/memory", "Codex");

        let plan = compute_plan("UserPromptSubmit", &session, Utc::now())
            .expect("compute plan")
            .expect("plan");

        let HookOutput::HookSpecificAdditionalContext { text, .. } = plan.output else {
            panic!("expected additional context");
        };
        assert!(text.contains("Memory Reminder"));
        assert!(text.contains("memory.add"));
        assert!(text.contains(".gwt/work/memory.md"));
        assert!(text.contains("Future Action"));
    }

    #[test]
    fn stop_includes_memory_reminder_without_stop_block() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("work dir");
        std::fs::write(repo.join(".gwt/work/memory.md"), "# Memory\n").expect("memory");
        let session = make_session(&repo, "work/memory", "Codex");

        let plan = compute_plan("Stop", &session, Utc::now())
            .expect("compute plan")
            .expect("plan");

        let HookOutput::SystemMessage(text) = plan.output else {
            panic!("expected non-blocking system message");
        };
        assert!(text.contains("Memory Reminder"));
        assert!(text.contains("memory.add"));
        assert!(text.contains(".gwt/work/memory.md"));
    }

    #[test]
    fn compute_plan_throttles_memory_reminder_after_first_fire() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("tasks")).expect("tasks");
        std::fs::write(repo.join("tasks/memory.md"), "# Memory\n").expect("memory");
        let session = make_session(&repo, "work/memory", "Codex");

        let t0 = "2026-06-04T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let t1 = "2026-06-04T13:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let t2 = "2026-06-04T18:01:00Z".parse::<DateTime<Utc>>().unwrap();

        let plan = compute_plan("UserPromptSubmit", &session, t0)
            .expect("compute plan")
            .expect("plan");
        assert!(additional_context(&plan.output).contains("Memory Reminder"));
        write_reminders_state(&session.worktree_path, &session.id, &plan.next_reminders).unwrap();
        assert_eq!(
            load_reminders_state(&session.worktree_path, &session.id)
                .unwrap()
                .last_memory_reminded_at,
            Some(t0)
        );

        let plan = compute_plan("UserPromptSubmit", &session, t1)
            .expect("compute plan")
            .expect("plan");
        assert!(!additional_context(&plan.output).contains("Memory Reminder"));
        write_reminders_state(&session.worktree_path, &session.id, &plan.next_reminders).unwrap();
        assert_eq!(
            load_reminders_state(&session.worktree_path, &session.id)
                .unwrap()
                .last_memory_reminded_at,
            Some(t0)
        );

        let plan = compute_plan("UserPromptSubmit", &session, t2)
            .expect("compute plan")
            .expect("plan");
        assert!(additional_context(&plan.output).contains("Memory Reminder"));
        write_reminders_state(&session.worktree_path, &session.id, &plan.next_reminders).unwrap();
        assert_eq!(
            load_reminders_state(&session.worktree_path, &session.id)
                .unwrap()
                .last_memory_reminded_at,
            Some(t2)
        );
    }

    #[test]
    fn compute_plan_never_fires_memory_reminder_on_session_start() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("tasks")).expect("tasks");
        std::fs::write(repo.join("tasks/memory.md"), "# Memory\n").expect("memory");
        let session = make_session(&repo, "work/memory", "Codex");

        let plan = compute_plan("SessionStart", &session, Utc::now())
            .expect("compute plan")
            .expect("plan");
        match &plan.output {
            HookOutput::HookSpecificAdditionalContext { text, .. } => {
                assert!(!text.contains("Memory Reminder"));
            }
            HookOutput::Silent => {}
            other => panic!("unexpected SessionStart output: {other:?}"),
        }
    }

    #[test]
    fn compute_plan_throttles_stop_memory_reminder_after_user_prompt() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("tasks")).expect("tasks");
        std::fs::write(repo.join("tasks/memory.md"), "# Memory\n").expect("memory");
        let session = make_session(&repo, "work/memory", "Codex");

        let t0 = "2026-06-04T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let t_near = "2026-06-04T12:30:00Z".parse::<DateTime<Utc>>().unwrap();
        let t_far = "2026-06-04T18:01:00Z".parse::<DateTime<Utc>>().unwrap();

        let plan = compute_plan("UserPromptSubmit", &session, t0)
            .expect("compute plan")
            .expect("plan");
        assert!(additional_context(&plan.output).contains("Memory Reminder"));
        write_reminders_state(&session.worktree_path, &session.id, &plan.next_reminders).unwrap();

        let plan = compute_plan("Stop", &session, t_near)
            .expect("compute plan")
            .expect("plan");
        assert!(!system_message(&plan.output).contains("Memory Reminder"));
        write_reminders_state(&session.worktree_path, &session.id, &plan.next_reminders).unwrap();

        let plan = compute_plan("Stop", &session, t_far)
            .expect("compute plan")
            .expect("plan");
        assert!(system_message(&plan.output).contains("Memory Reminder"));
    }

    #[test]
    fn compute_plan_fires_memory_reminder_on_first_mid_session_encounter() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let session = make_session(&repo, "work/memory", "Codex");

        let plan = compute_plan("SessionStart", &session, Utc::now())
            .expect("compute plan")
            .expect("plan");
        match &plan.output {
            HookOutput::HookSpecificAdditionalContext { text, .. } => {
                assert!(!text.contains("Memory Reminder"));
            }
            HookOutput::Silent => {}
            other => panic!("unexpected SessionStart output: {other:?}"),
        }
        write_reminders_state(&session.worktree_path, &session.id, &plan.next_reminders).unwrap();

        std::fs::create_dir_all(repo.join("tasks")).expect("tasks");
        std::fs::write(repo.join("tasks/memory.md"), "# Memory\n").expect("memory");
        let plan = compute_plan("UserPromptSubmit", &session, Utc::now())
            .expect("compute plan")
            .expect("plan");
        assert!(additional_context(&plan.output).contains("Memory Reminder"));
    }

    #[test]
    fn reminders_state_persists_memory_reminded_timestamp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let session = make_session(&repo, "work/memory", "Codex");
        let now = "2026-06-04T12:00:00Z".parse::<DateTime<Utc>>().unwrap();

        let state = RemindersState {
            last_memory_reminded_at: Some(now),
            ..RemindersState::default()
        };
        write_reminders_state(&session.worktree_path, &session.id, &state).unwrap();
        let loaded = load_reminders_state(&session.worktree_path, &session.id).unwrap();
        assert_eq!(loaded.last_memory_reminded_at, Some(now));
    }

    #[test]
    fn compute_memory_reminder_state_returns_correct_suppress_flag() {
        let now = "2026-06-04T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let base = RemindersState::default();

        let (suppress, next) =
            compute_memory_reminder_state(IntentBoundaryEvent::UserPromptSubmit, true, &base, now);
        assert!(!suppress);
        assert_eq!(next.last_memory_reminded_at, Some(now));

        let recent = RemindersState {
            last_memory_reminded_at: Some(now - chrono::Duration::hours(2)),
            ..RemindersState::default()
        };
        let (suppress, next) = compute_memory_reminder_state(
            IntentBoundaryEvent::UserPromptSubmit,
            true,
            &recent,
            now,
        );
        assert!(suppress);
        assert_eq!(next.last_memory_reminded_at, recent.last_memory_reminded_at);

        let old = RemindersState {
            last_memory_reminded_at: Some(now - chrono::Duration::hours(7)),
            ..RemindersState::default()
        };
        let (suppress, next) =
            compute_memory_reminder_state(IntentBoundaryEvent::UserPromptSubmit, true, &old, now);
        assert!(!suppress);
        assert_eq!(next.last_memory_reminded_at, Some(now));

        let (suppress, next) =
            compute_memory_reminder_state(IntentBoundaryEvent::SessionStart, true, &base, now);
        assert!(suppress);
        assert_eq!(next.last_memory_reminded_at, None);

        let (suppress, _) =
            compute_memory_reminder_state(IntentBoundaryEvent::UserPromptSubmit, false, &base, now);
        assert!(suppress);

        let (suppress, _) =
            compute_memory_reminder_state(IntentBoundaryEvent::Stop, true, &recent, now);
        assert!(suppress);
        let (suppress, _) =
            compute_memory_reminder_state(IntentBoundaryEvent::Stop, true, &old, now);
        assert!(!suppress);
    }

    #[test]
    fn agent_title_summary_missing_reads_workspace_projection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let session = make_session(&repo, "work/title", "Codex");

        assert!(
            !agent_title_summary_missing(&session).expect("missing title check"),
            "sessions without a Workspace projection are Unassigned and must not require a title update"
        );

        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: session.id.clone(),
                window_id: None,
                agent_id: "codex".to_string(),
                display_name: "Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                current_focus: Some("Implement title-summary guard".to_string()),
                title_summary: Some("Title summary guard".to_string()),
                worktree_path: Some(repo.clone()),
                branch: Some("work/title".to_string()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                workspace_id: None,
                updated_at: Utc::now(),
            });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        assert!(
            !agent_title_summary_missing(&session).expect("title check"),
            "saved non-empty title_summary must satisfy the guard"
        );
    }

    #[test]
    fn agent_title_summary_missing_reads_canonical_project_state_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("workspace-home");
        let worktree = project_root.join("work").join("20260601-0934");
        std::fs::create_dir_all(&worktree).expect("worktree");
        let mut session = make_session(&worktree, "work/title", "Codex");
        session.project_state_root = Some(project_root.clone());

        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&project_root);
        projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: session.id.clone(),
                window_id: Some("project::agent-1".to_string()),
                agent_id: "codex".to_string(),
                display_name: "Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                current_focus: Some("Implement canonical title guard".to_string()),
                title_summary: Some("Canonical title guard".to_string()),
                worktree_path: Some(worktree.clone()),
                branch: Some("work/title".to_string()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                workspace_id: None,
                updated_at: Utc::now(),
            });
        gwt_core::workspace_projection::save_workspace_projection(&project_root, &projection)
            .expect("save projection");

        assert!(
            !agent_title_summary_missing(&session).expect("title check"),
            "title guard must read the canonical Project State root, not the worktree root"
        );
    }

    /// SPEC-2359 Phase W-11 (US-58 / US-46 / FR-179): the title-missing
    /// decision must return `true` for an Unassigned agent with no title, so
    /// the reminder fires. Start Work / standalone agents are Unassigned; the
    /// old `is_unassigned()` early-return left them with neither a derived
    /// title nor a reminder once the prompt-derivation path was removed.
    /// Pure-logic test (no global store / env) to stay deterministic.
    #[test]
    fn title_summary_missing_in_projection_covers_affiliation_and_presence() {
        use gwt_core::workspace_projection::{
            WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary, WorkspaceProjection,
            WorkspaceStatusCategory,
        };

        let mut agent = WorkspaceAgentSummary {
            session_id: "sess-1".to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
            workspace_id: None,
            updated_at: Utc::now(),
        };
        let mut projection = WorkspaceProjection::default_for_project("/repo");
        projection.agents.push(agent.clone());

        // Unassigned + empty title -> missing (the fix).
        assert!(title_summary_missing_in_projection(
            Some(&projection),
            "sess-1"
        ));

        // Assigned + empty title -> still missing.
        projection.agents[0].affiliation_status = WorkspaceAgentAffiliationStatus::Assigned;
        assert!(title_summary_missing_in_projection(
            Some(&projection),
            "sess-1"
        ));

        // Any affiliation + non-empty title -> not missing.
        projection.agents[0].title_summary = Some("Agent title purpose".to_string());
        assert!(!title_summary_missing_in_projection(
            Some(&projection),
            "sess-1"
        ));

        // No projection / unknown session -> not missing.
        assert!(!title_summary_missing_in_projection(None, "sess-1"));
        agent.title_summary = None;
        let mut other = WorkspaceProjection::default_for_project("/repo");
        other.agents.push(agent);
        assert!(!title_summary_missing_in_projection(
            Some(&other),
            "sess-unknown"
        ));
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
        assert!(
            keys.iter().any(|k| k == &format!("session:{}", session.id)),
            "typed session mention key missing"
        );
        assert!(
            keys.iter().any(|k| k == "branch:feature/me"),
            "typed branch mention key missing"
        );
        assert!(
            keys.iter().any(|k| k == "agent:codex"),
            "typed agent mention key missing"
        );
    }

    #[test]
    fn build_self_match_keys_skips_empty_fields() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new(dir.path(), "", AgentId::Codex);
        session.display_name = String::new();
        let keys = build_self_match_keys(&session);
        // Raw and typed session id survive; typed agent identity survives even
        // when optional branch/display fields are empty.
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[0], session.id);
        assert_eq!(keys[1], format!("session:{}", session.id));
        assert_eq!(keys[2], "agent:codex");
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

    /// SPEC-3247 FR-003 / AS-4: an intake (Curate) session must not receive the
    /// producing-work Work reminders (title-summary AND progress-summary) — it
    /// owns no Work. The same setup in an execution session (default signal)
    /// still injects both. The shared Board-coordination reminder and the
    /// memory reminder survive in intake, so intake is not silenced wholesale.
    #[test]
    fn intake_session_suppresses_title_summary_work_reminder() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(repo.join(".gwt/work")).expect("repo work dir");
        // Memory source present so the memory reminder is in play for intake too.
        std::fs::write(repo.join(".gwt/work/memory.md"), "# Memory\n").expect("memory");
        let session = make_session(&repo, "work/intake", "Codex");
        // Agent present with an empty title and no progress summary -> execution
        // would inject BOTH the title reminder and the progress-missing reminder.
        save_projection(
            &repo,
            vec![WorkspaceAgentSummary {
                title_summary: None,
                current_focus: Some("Curate the backlog".to_string()),
                ..workspace_agent(
                    &session.id,
                    Some("workspace-current"),
                    WorkspaceAgentAffiliationStatus::Assigned,
                )
            }],
        );

        // Language-independent expected markers: the exact reminder texts that
        // the append functions inject.
        let language = resolve_narrative_language();
        let title_reminder = texts::title_summary_required_reminder(&language);
        let progress_reminder = texts::progress_summary_reminder(&language, false, false);

        // Execution (signal unset -> default Execution): both Work reminders fire.
        let _clear = ScopedEnvVar::unset(gwt_skills::GWT_SESSION_KIND_ENV);
        let exec = compute_plan("UserPromptSubmit", &session, Utc::now())
            .expect("compute plan")
            .expect("plan");
        let exec_text = additional_context(&exec.output);
        assert!(
            exec_text.contains(title_reminder),
            "execution session must still receive the title-summary Work reminder"
        );
        assert!(
            exec_text.contains(progress_reminder),
            "execution session must still receive the progress-summary Work reminder"
        );

        // Intake: both producing-work reminders are suppressed, but the shared
        // Board reminder and the memory reminder survive.
        let _intake = ScopedEnvVar::set(gwt_skills::GWT_SESSION_KIND_ENV, "intake");
        let intake = compute_plan("UserPromptSubmit", &session, Utc::now())
            .expect("compute plan")
            .expect("plan");
        let intake_text = match &intake.output {
            HookOutput::HookSpecificAdditionalContext { text, .. } => text.as_str(),
            HookOutput::Silent => "",
            other => panic!("unexpected intake output: {other:?}"),
        };
        assert!(
            !intake_text.contains(title_reminder),
            "intake session must not receive the producing-work title reminder: {intake_text}"
        );
        assert!(
            !intake_text.contains(progress_reminder),
            "intake session must not receive the producing-work progress reminder: {intake_text}"
        );
        assert!(
            intake_text.contains("Board Post Reminder"),
            "intake session must still receive the shared Board coordination reminder: {intake_text}"
        );
        assert!(
            intake_text.contains("Memory Reminder"),
            "intake session must still receive the memory reminder: {intake_text}"
        );
    }

    #[test]
    fn handle_user_prompt_submit_uses_hook_cwd_for_board_scope() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo_a = home.path().join("repo-a");
        let repo_b = home.path().join("repo-b");
        init_repo(&repo_a, "https://github.com/example/repo-a.git");
        init_repo(&repo_b, "https://github.com/example/repo-b.git");

        let session = make_session(&repo_a, "work/repo-a", "Codex");
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save session");
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
        let now = Utc::now();

        post_entry(
            &repo_a,
            entry(
                "Other",
                BoardEntryKind::Status,
                "repo-a stale update",
                "work/repo-a",
                "session-repo-a",
                now - chrono::Duration::minutes(5),
            ),
        )
        .unwrap();
        post_entry(
            &repo_b,
            entry(
                "Other",
                BoardEntryKind::Status,
                "repo-b current update",
                "work/repo-b",
                "session-repo-b",
                now - chrono::Duration::minutes(4),
            ),
        )
        .unwrap();

        let input = serde_json::json!({ "cwd": repo_b }).to_string();
        let output = handle_with_input("UserPromptSubmit", &input).unwrap();
        let text = additional_context(&output);

        assert!(text.contains("repo-b current update"), "{text}");
        assert!(!text.contains("repo-a stale update"), "{text}");
    }

    #[test]
    fn compute_plan_session_start_filters_entries_to_current_workspace_audience() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", dir.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", dir.path());
        let session = make_session(dir.path(), "feature/me", "Codex");
        save_projection(
            dir.path(),
            vec![workspace_agent(
                &session.id,
                Some("workspace-current"),
                WorkspaceAgentAffiliationStatus::Assigned,
            )],
        );
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        let broadcast = entry(
            "Other",
            BoardEntryKind::Status,
            "broadcast update",
            "work/other",
            "session-other",
            now - chrono::Duration::minutes(5),
        );
        let current = entry(
            "Other",
            BoardEntryKind::Status,
            "current workspace update",
            "work/other",
            "session-other",
            now - chrono::Duration::minutes(4),
        )
        .with_audience(vec!["workspace-current"]);
        let other = entry(
            "Other",
            BoardEntryKind::Status,
            "other workspace update",
            "work/other",
            "session-other",
            now - chrono::Duration::minutes(3),
        )
        .with_audience(vec!["workspace-other"]);
        post_entry(dir.path(), broadcast).unwrap();
        post_entry(dir.path(), current).unwrap();
        post_entry(dir.path(), other).unwrap();

        let plan = compute_plan("SessionStart", &session, now)
            .unwrap()
            .unwrap();
        let text = additional_context(&plan.output);

        assert!(text.contains("broadcast update"), "{text}");
        assert!(text.contains("current workspace update"), "{text}");
        assert!(!text.contains("other workspace update"), "{text}");
    }

    #[test]
    fn compute_plan_unassigned_agent_only_reads_broadcast_board_entries() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", dir.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", dir.path());
        let session = make_session(dir.path(), "feature/me", "Codex");
        save_projection(
            dir.path(),
            vec![workspace_agent(
                &session.id,
                None,
                WorkspaceAgentAffiliationStatus::Unassigned,
            )],
        );
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        post_entry(
            dir.path(),
            entry(
                "Other",
                BoardEntryKind::Status,
                "broadcast update",
                "work/other",
                "session-other",
                now - chrono::Duration::minutes(5),
            ),
        )
        .unwrap();
        post_entry(
            dir.path(),
            entry(
                "Other",
                BoardEntryKind::Status,
                "workspace-only update",
                "work/other",
                "session-other",
                now - chrono::Duration::minutes(4),
            )
            .with_audience(vec!["workspace-current"]),
        )
        .unwrap();

        let plan = compute_plan("SessionStart", &session, now)
            .unwrap()
            .unwrap();
        let text = additional_context(&plan.output);

        assert!(text.contains("broadcast update"), "{text}");
        assert!(!text.contains("workspace-only update"), "{text}");
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
        let text = additional_context(&plan.output);
        assert!(
            text.contains("posted to the Board recently") || text.contains("最近 Board に投稿済み")
        );
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
        let text = system_message(&plan.output);
        assert!(
            text.contains("posted to the Board recently") || text.contains("最近 Board に投稿済み")
        );
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

    fn projection_with_agent_identity(
        repo: &Path,
        session_id: &str,
        title_summary: Option<&str>,
        current_focus: Option<&str>,
    ) -> WorkspaceProjection {
        let mut projection = WorkspaceProjection::default_for_project(repo);
        let mut agent = workspace_agent(
            session_id,
            Some("ws-stale"),
            WorkspaceAgentAffiliationStatus::Assigned,
        );
        agent.title_summary = title_summary.map(str::to_string);
        agent.current_focus = current_focus.map(str::to_string);
        projection.agents.push(agent);
        projection
    }

    #[test]
    fn compute_plan_injects_title_summary_stale_reminder_after_threshold_with_focus_drift() {
        let session_id = "session-stale";
        let mut state = RemindersState::default();
        // Prime the state: first turn observes the title, second turn introduces
        // a focus change, then 7 more identical turns push the counter past the
        // threshold (8) while the focus-drift flag stays sticky.
        let projection_initial = projection_with_agent_identity(
            Path::new("/repo"),
            session_id,
            Some("Workspace"),
            Some("phase 1"),
        );
        let (stale_initial, next) = compute_title_summary_stale_state(
            IntentBoundaryEvent::UserPromptSubmit,
            Some(&projection_initial),
            session_id,
            &state,
        );
        assert!(!stale_initial, "first observation cannot be stale");
        state = next;

        // Drift current_focus to phase 2 while keeping the title constant.
        let projection_drift = projection_with_agent_identity(
            Path::new("/repo"),
            session_id,
            Some("Workspace"),
            Some("phase 2"),
        );
        for _ in 0..8 {
            let (_, n) = compute_title_summary_stale_state(
                IntentBoundaryEvent::UserPromptSubmit,
                Some(&projection_drift),
                session_id,
                &state,
            );
            state = n;
        }

        assert!(
            state.unchanged_turn_count >= TITLE_SUMMARY_STALE_TURN_THRESHOLD,
            "counter must reach threshold; got {}",
            state.unchanged_turn_count
        );
        assert!(
            state.phase_changed_in_window,
            "focus drift must flip phase_changed_in_window"
        );

        // The next turn returns stale=true.
        let (stale_final, _) = compute_title_summary_stale_state(
            IntentBoundaryEvent::UserPromptSubmit,
            Some(&projection_drift),
            session_id,
            &state,
        );
        assert!(
            stale_final,
            "stale must fire once threshold + phase drift hold"
        );
    }

    #[test]
    fn compute_plan_resets_stale_counter_when_title_summary_changes() {
        let session_id = "session-reset";
        let mut state = RemindersState::default();

        let projection_a = projection_with_agent_identity(
            Path::new("/repo"),
            session_id,
            Some("Title A"),
            Some("phase 1"),
        );
        for _ in 0..5 {
            let (_, n) = compute_title_summary_stale_state(
                IntentBoundaryEvent::UserPromptSubmit,
                Some(&projection_a),
                session_id,
                &state,
            );
            state = n;
        }
        assert!(state.unchanged_turn_count >= 4);

        let projection_b = projection_with_agent_identity(
            Path::new("/repo"),
            session_id,
            Some("Title B"),
            Some("phase 1"),
        );
        let (stale, n) = compute_title_summary_stale_state(
            IntentBoundaryEvent::UserPromptSubmit,
            Some(&projection_b),
            session_id,
            &state,
        );
        state = n;
        assert!(
            !stale,
            "title change must not trigger stale on the same turn"
        );
        assert_eq!(
            state.unchanged_turn_count, 0,
            "title change must reset counter"
        );
        assert!(
            !state.phase_changed_in_window,
            "title change must reset phase_changed_in_window"
        );
        assert_eq!(
            state.last_title_summary_seen.as_deref(),
            Some("Title B"),
            "last_title_summary_seen tracks the new value"
        );
    }

    #[test]
    fn compute_plan_does_not_inject_stale_reminder_on_stop_event() {
        let session_id = "session-stop";
        let state = RemindersState {
            last_title_summary_seen: Some("Title".to_string()),
            unchanged_turn_count: TITLE_SUMMARY_STALE_TURN_THRESHOLD + 5,
            last_current_focus_seen: Some("focus".to_string()),
            phase_changed_in_window: true,
            ..RemindersState::default()
        };
        let projection = projection_with_agent_identity(
            Path::new("/repo"),
            session_id,
            Some("Title"),
            Some("focus"),
        );

        let (stale, next) = compute_title_summary_stale_state(
            IntentBoundaryEvent::Stop,
            Some(&projection),
            session_id,
            &state,
        );

        assert!(!stale, "Stop event must never trigger stale reminder");
        assert_eq!(
            next.unchanged_turn_count,
            TITLE_SUMMARY_STALE_TURN_THRESHOLD + 5,
            "Stop event must leave state untouched"
        );
    }

    #[test]
    fn compute_plan_does_not_inject_stale_reminder_when_focus_unchanged() {
        let session_id = "session-no-drift";
        let mut state = RemindersState::default();
        let projection = projection_with_agent_identity(
            Path::new("/repo"),
            session_id,
            Some("Title"),
            Some("focus"),
        );

        for _ in 0..(TITLE_SUMMARY_STALE_TURN_THRESHOLD + 3) {
            let (stale, n) = compute_title_summary_stale_state(
                IntentBoundaryEvent::UserPromptSubmit,
                Some(&projection),
                session_id,
                &state,
            );
            state = n;
            assert!(
                !stale,
                "no phase drift must keep stale=false even past the threshold"
            );
        }
        assert!(
            !state.phase_changed_in_window,
            "phase_changed_in_window must remain false without focus drift"
        );
    }

    #[test]
    fn compute_plan_skips_stale_when_title_is_empty() {
        let session_id = "session-empty";
        let mut state = RemindersState {
            last_title_summary_seen: Some("Stale".to_string()),
            unchanged_turn_count: TITLE_SUMMARY_STALE_TURN_THRESHOLD,
            phase_changed_in_window: true,
            ..RemindersState::default()
        };
        let projection_empty =
            projection_with_agent_identity(Path::new("/repo"), session_id, None, Some("focus"));

        let (stale, next) = compute_title_summary_stale_state(
            IntentBoundaryEvent::UserPromptSubmit,
            Some(&projection_empty),
            session_id,
            &state,
        );

        assert!(!stale, "empty title is owned by the required reminder path");
        assert_eq!(
            next.unchanged_turn_count, 0,
            "empty title must reset the counter"
        );
        assert_eq!(next.last_title_summary_seen, None);
        state = next;
        assert!(!state.phase_changed_in_window);
    }

    #[test]
    fn compute_progress_summary_state_marks_missing_on_user_prompt_and_stop() {
        let session_id = "session-progress-missing";
        let projection = projection_with_agent_identity(
            Path::new("/repo"),
            session_id,
            Some("Workspace detail"),
            Some("implementing detail summary"),
        );

        let (missing, stale, _) = compute_progress_summary_state(
            IntentBoundaryEvent::UserPromptSubmit,
            Some(&projection),
            session_id,
            &RemindersState::default(),
        );
        assert!(
            missing,
            "UserPromptSubmit should remind when progress_summary is missing"
        );
        assert!(!stale);

        let (missing, stale, _) = compute_progress_summary_state(
            IntentBoundaryEvent::Stop,
            Some(&projection),
            session_id,
            &RemindersState::default(),
        );
        assert!(
            missing,
            "Stop should remind when progress_summary is missing"
        );
        assert!(!stale);
    }

    #[test]
    fn compute_progress_summary_state_stales_when_focus_changes_but_summary_does_not() {
        let session_id = "session-progress-stale";
        let mut state = RemindersState::default();
        let mut projection = projection_with_agent_identity(
            Path::new("/repo"),
            session_id,
            Some("Workspace detail"),
            Some("phase 1"),
        );
        projection.progress_summary = Some("Initial cumulative summary".to_string());

        let (missing, stale, next) = compute_progress_summary_state(
            IntentBoundaryEvent::UserPromptSubmit,
            Some(&projection),
            session_id,
            &state,
        );
        assert!(!missing);
        assert!(!stale);
        state = next;

        let mut drifted = projection_with_agent_identity(
            Path::new("/repo"),
            session_id,
            Some("Workspace detail"),
            Some("phase 2"),
        );
        drifted.progress_summary = Some("Initial cumulative summary".to_string());
        for _ in 0..PROGRESS_SUMMARY_STALE_TURN_THRESHOLD {
            let (_, stale, next) = compute_progress_summary_state(
                IntentBoundaryEvent::UserPromptSubmit,
                Some(&drifted),
                session_id,
                &state,
            );
            state = next;
            if state.progress_summary_unchanged_turn_count < PROGRESS_SUMMARY_STALE_TURN_THRESHOLD {
                assert!(!stale);
            }
        }

        assert!(
            state.progress_focus_changed_in_window,
            "focus drift must be sticky while progress_summary is unchanged"
        );
        let (_, stale, _) = compute_progress_summary_state(
            IntentBoundaryEvent::UserPromptSubmit,
            Some(&drifted),
            session_id,
            &state,
        );
        assert!(
            stale,
            "stale should fire once threshold and focus drift hold"
        );
    }

    #[test]
    fn append_progress_summary_context_uses_system_message_on_stop() {
        let output = append_progress_summary_context(
            HookOutput::Silent,
            IntentBoundaryEvent::Stop,
            true,
            false,
            "en",
        );
        let text = system_message(&output);
        assert!(text.contains("Progress Summary Reminder"));
        assert!(text.contains("progress_summary"));
        assert!(!text.contains("StopBlock"));
    }

    #[test]
    fn reminders_state_round_trips_phase_u9_fields() {
        let original = RemindersState {
            last_injected_at: None,
            last_reminded_kind: Default::default(),
            last_title_summary_seen: Some("Title".to_string()),
            unchanged_turn_count: 12,
            last_current_focus_seen: Some("focus".to_string()),
            phase_changed_in_window: true,
            last_memory_reminded_at: Some("2026-06-04T12:00:00Z".parse::<DateTime<Utc>>().unwrap()),
            last_progress_summary_seen: Some("Progress".to_string()),
            progress_summary_unchanged_turn_count: 4,
            last_progress_focus_seen: Some("focus".to_string()),
            progress_focus_changed_in_window: true,
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: RemindersState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, original);

        // Legacy state without the new fields must round-trip via serde defaults.
        let legacy = r#"{"last_injected_at":null,"last_reminded_kind":{}}"#;
        let restored_legacy: RemindersState = serde_json::from_str(legacy).unwrap();
        assert_eq!(restored_legacy.last_title_summary_seen, None);
        assert_eq!(restored_legacy.unchanged_turn_count, 0);
        assert_eq!(restored_legacy.last_current_focus_seen, None);
        assert!(!restored_legacy.phase_changed_in_window);
        assert_eq!(restored_legacy.last_progress_summary_seen, None);
        assert_eq!(restored_legacy.progress_summary_unchanged_turn_count, 0);
        assert_eq!(restored_legacy.last_progress_focus_seen, None);
        assert!(!restored_legacy.progress_focus_changed_in_window);
    }
}
