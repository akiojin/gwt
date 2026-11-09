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
//! a nonblocking warning. A missing state file stays `Silent`.
//!
//! Session isolation (FR-014t): if `state.session_id` differs from the
//! current `GWT_SESSION_ID` env value, the handler returns `Silent` so
//! other agent sessions are not interrupted.

use std::{
    io::{self, Read},
    path::Path,
};

use gwt_agent::GWT_SESSION_ID_ENV;

use super::{HookError, HookOutput};

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
    // SPEC-3248 hooks v2 P2: the shared decision now lives in the neutral
    // `state_file_stop_check` module (build-spec / register-spec delegate to
    // the same body by design, not by reaching into this module's internals).
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

    fn active_state(session: &str) -> SkillState {
        SkillState {
            start_evidence: None,
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
                "plan.complete",
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
    fn malformed_state_warns_without_blocking_public_stop() {
        use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};

        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session_env = [
            gwt_agent::GWT_SESSION_ID_ENV,
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "CODEX_THREAD_ID",
        ]
        .map(ScopedEnvVar::unset);
        let dir = tempfile::tempdir().unwrap();
        let path = gwt_core::skill_state::state_path(dir.path(), SKILL_NAME);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{broken").unwrap();
        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        let HookOutput::SystemMessage(message) = output else {
            panic!("malformed skill state must warn without blocking Stop: {output:?}");
        };
        assert!(message.contains(SKILL_DISPLAY), "{message}");
        assert!(message.contains("unreadable"), "{message}");
        assert!(message.contains("not blocked"), "{message}");

        let output = super::super::event_dispatcher::handle_with_input(
            "Stop",
            "{}",
            dir.path(),
            Some("sess-1"),
        )
        .expect("public Stop hook");
        let HookOutput::SystemMessage(public_message) = output else {
            panic!("public Stop must retain the skill state warning: {output:?}");
        };
        assert!(public_message.contains(&message), "{public_message}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
    }
}
