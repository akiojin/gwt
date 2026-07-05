//! `gwtd hook skill-register-spec-stop-check` — Stop-block handler for the
//! `gwt-register-spec` skill (SPEC-2784).
//!
//! Identical decision body to `skill-plan-spec-stop-check` and
//! `skill-build-spec-stop-check`: read `.gwt/skill-state/register-spec.json`,
//! return `HookOutput::StopBlock` while the state is `active: true` for the
//! current session, honour `stop_hook_active` to cap forced continuation at
//! one per Stop cycle, and stay silent on every other path.

use std::{
    io::{self, Read},
    path::Path,
};

use gwt_agent::GWT_SESSION_ID_ENV;

use super::{HookError, HookOutput};

pub const SKILL_NAME: &str = "register-spec";
pub const SKILL_DISPLAY: &str = "gwt-register-spec";

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
    super::state_file_stop_check::decide(
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
            owner_spec: Some(2784),
            started_at: Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).unwrap(),
            phase: Some(phase.to_string()),
            session_id: session.to_string(),
        }
    }

    #[test]
    fn blocks_while_register_state_is_active() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), SKILL_NAME, &active_state("sess-1", "create")).unwrap();
        match handle_with_input(dir.path(), "{}", Some("sess-1")) {
            HookOutput::StopBlock { reason } => {
                assert!(reason.contains("gwt-register-spec"));
                assert!(reason.contains("create"));
            }
            other => panic!("expected StopBlock, got {other:?}"),
        }
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
    fn silent_when_session_id_does_not_match() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), SKILL_NAME, &active_state("sess-other", "edit")).unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent,
        );
    }

    #[test]
    fn silent_when_stop_hook_active_is_true() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), SKILL_NAME, &active_state("sess-1", "create")).unwrap();
        assert_eq!(
            handle_with_input(dir.path(), r#"{"stop_hook_active":true}"#, Some("sess-1"),),
            HookOutput::Silent,
        );
    }
}
