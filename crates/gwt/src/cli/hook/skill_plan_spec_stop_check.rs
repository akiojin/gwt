//! `gwtd hook skill-plan-spec-stop-check` — Stop-block handler for the
//! `gwt-plan-spec` skill (SPEC-1935 Phase 10, FR-014q).
//!
//! Reads `.gwt/skill-state/plan-spec.json` in the current worktree. When
//! the state is `active: true` and the recorded `session_id` matches the
//! current agent session, returns `HookOutput::StopBlock` so the skill
//! keeps planning instead of stopping. `stop_hook_active: true`
//! short-circuits to `Silent` (FR-014o).
//!
//! Fail-open policy (FR-014u): any I/O or JSON failure resolves to
//! `Silent`.
//!
//! Session isolation (FR-014t): if `state.session_id` differs from the
//! current `GWT_SESSION_ID` env value, the handler returns `Silent` so
//! other agent sessions are not interrupted.

use std::{
    io::{self, Read},
    path::Path,
};

use gwt_agent::GWT_SESSION_ID_ENV;
use gwt_core::skill_state;

use super::{envelope::stop_hook_active_from, HookError, HookOutput};

pub const SKILL_NAME: &str = "plan-spec";
pub const SKILL_DISPLAY: &str = "gwt-plan-spec";

pub fn handle() -> Result<HookOutput, HookError> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let cwd = std::env::current_dir()?;
    let current_session = std::env::var(GWT_SESSION_ID_ENV).ok();
    Ok(handle_with_input(&cwd, &input, current_session.as_deref()))
}

pub fn handle_with_input(
    worktree: &Path,
    input: &str,
    current_session_id: Option<&str>,
) -> HookOutput {
    decide(
        worktree,
        input,
        current_session_id,
        SKILL_NAME,
        SKILL_DISPLAY,
    )
}

/// Shared decision logic between plan-spec and build-spec handlers.
pub(crate) fn decide(
    worktree: &Path,
    input: &str,
    current_session_id: Option<&str>,
    skill_name: &str,
    skill_display: &str,
) -> HookOutput {
    if stop_hook_active_from(input) {
        return HookOutput::Silent;
    }
    let Ok(Some(state)) = skill_state::load(worktree, skill_name) else {
        return HookOutput::Silent;
    };
    if !state.active {
        return HookOutput::Silent;
    }
    if let Some(current) = current_session_id {
        if current != state.session_id {
            return HookOutput::Silent;
        }
    }

    let phase_clause = state
        .phase
        .as_deref()
        .map(|p| format!(" (phase: {p})"))
        .unwrap_or_default();
    let spec_clause = state
        .owner_spec
        .map(|n| format!("SPEC-{n}"))
        .unwrap_or_else(|| "the current owner".to_string());
    let short_name = short_verb_for(skill_name);

    HookOutput::stop_block(format!(
        "{skill_display} for {spec_clause} is still active{phase_clause}.\n\
         Continue the {skill_display} workflow, or call `gwtd {short_name} complete --spec <n>` when the artifacts are ready, \
         or `gwtd {short_name} abort --spec <n> --reason '<text>'` to abandon.",
    ))
}

fn short_verb_for(skill_name: &str) -> &'static str {
    match skill_name {
        "plan-spec" => "plan",
        "build-spec" => "build",
        _ => "<skill>",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use gwt_core::skill_state::{save, SkillState};

    fn active_state(session: &str) -> SkillState {
        SkillState {
            active: true,
            owner_spec: Some(1935),
            started_at: Utc.with_ymd_and_hms(2026, 4, 21, 9, 0, 0).unwrap(),
            phase: Some("plan-draft".to_string()),
            session_id: session.to_string(),
        }
    }

    fn assert_block_with(output: HookOutput, contains: &[&str]) {
        match output {
            HookOutput::StopBlock { reason } => {
                for needle in contains {
                    assert!(
                        reason.contains(needle),
                        "reason {reason:?} missing {needle:?}"
                    );
                }
            }
            other => panic!("expected StopBlock, got {other:?}"),
        }
    }

    #[test]
    fn blocks_when_state_is_active_and_session_matches() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), SKILL_NAME, &active_state("sess-1")).unwrap();
        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        assert_block_with(
            output,
            &[
                "gwt-plan-spec for SPEC-1935",
                "phase: plan-draft",
                "gwtd plan complete",
            ],
        );
    }

    #[test]
    fn silent_when_stop_hook_active_is_true() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), SKILL_NAME, &active_state("sess-1")).unwrap();
        assert_eq!(
            handle_with_input(dir.path(), r#"{"stop_hook_active":true}"#, Some("sess-1")),
            HookOutput::Silent,
        );
    }

    #[test]
    fn silent_when_state_file_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
        );
    }

    #[test]
    fn silent_when_state_is_inactive() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = active_state("sess-1");
        state.active = false;
        save(dir.path(), SKILL_NAME, &state).unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
        );
    }

    #[test]
    fn silent_when_session_id_does_not_match() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), SKILL_NAME, &active_state("sess-other")).unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
        );
    }

    #[test]
    fn blocks_when_current_session_is_unknown_and_state_is_active() {
        // No GWT_SESSION_ID set: we trust that the state file was written
        // for the current agent and still block to avoid missing work.
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), SKILL_NAME, &active_state("sess-1")).unwrap();
        let output = handle_with_input(dir.path(), "{}", None);
        assert!(matches!(output, HookOutput::StopBlock { .. }));
    }

    #[test]
    fn silent_when_state_file_json_is_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let path = skill_state::state_path(dir.path(), SKILL_NAME);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
        );
    }
}
