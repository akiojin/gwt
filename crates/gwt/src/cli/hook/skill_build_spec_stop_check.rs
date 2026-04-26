//! `gwtd hook skill-build-spec-stop-check` — Stop-block handler for the
//! `gwt-build-spec` skill (SPEC-1935 Phase 10, FR-014r).
//!
//! Mirrors the `skill_plan_spec_stop_check` structure but targets the
//! `build-spec` state file. The shared decision body lives in
//! [`super::skill_plan_spec_stop_check::decide`] so both handlers stay
//! in lock-step regarding `stop_hook_active`, session isolation, and
//! fail-open policy.

use std::{
    io::{self, Read},
    path::Path,
};

use gwt_agent::GWT_SESSION_ID_ENV;

use super::{HookError, HookOutput};

pub const SKILL_NAME: &str = "build-spec";
pub const SKILL_DISPLAY: &str = "gwt-build-spec";

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
    super::skill_plan_spec_stop_check::decide(
        worktree,
        input,
        current_session_id,
        SKILL_NAME,
        SKILL_DISPLAY,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use gwt_core::skill_state::{save, SkillState};

    fn active_state(session: &str, phase: &str) -> SkillState {
        SkillState {
            active: true,
            owner_spec: Some(1935),
            started_at: Utc.with_ymd_and_hms(2026, 4, 21, 9, 0, 0).unwrap(),
            phase: Some(phase.to_string()),
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
    fn blocks_when_build_state_active_and_includes_phase_in_reason() {
        let dir = tempfile::tempdir().unwrap();
        save(
            dir.path(),
            SKILL_NAME,
            &active_state("sess-1", "red-green-refactor"),
        )
        .unwrap();
        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        assert_block_with(
            output,
            &[
                "gwt-build-spec for SPEC-1935",
                "phase: red-green-refactor",
                "gwtd build complete",
            ],
        );
    }

    #[test]
    fn silent_when_state_file_absent_even_with_active_session() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
        );
    }

    #[test]
    fn silent_when_session_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        save(
            dir.path(),
            SKILL_NAME,
            &active_state("sess-other", "verify"),
        )
        .unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
        );
    }

    #[test]
    fn silent_when_stop_hook_active_is_true() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), SKILL_NAME, &active_state("sess-1", "red")).unwrap();
        assert_eq!(
            handle_with_input(dir.path(), r#"{"stop_hook_active":true}"#, Some("sess-1")),
            HookOutput::Silent,
        );
    }
}
