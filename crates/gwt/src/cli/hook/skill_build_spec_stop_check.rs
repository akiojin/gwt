//! `gwtd hook skill-build-spec-stop-check` — Stop-block handler for the
//! `gwt-build-spec` skill (SPEC-1935 Phase 10, FR-014r).
//!
//! Uses the shared state-file gate for `stop_hook_active`, session isolation,
//! and fail-open policy. A stranded Blocked build whose abort is unreachable
//! releases this gate with a durable diagnostic, retaining its active marker.

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
    let output = super::state_file_stop_check::decide(
        worktree,
        input,
        current_session_id,
        SKILL_NAME,
        SKILL_DISPLAY,
    );
    if matches!(output, HookOutput::StopBlock { .. })
        && release_unabortable_build(worktree, current_session_id)
    {
        return HookOutput::Silent;
    }
    output
}

/// A Blocked execution can retain an active compatibility build marker when
/// canonical Work authority makes abort unreachable. Preserve that marker;
/// releasing this gate neither terminalizes Work nor claims successful delivery.
fn release_unabortable_build(worktree: &Path, current_session_id: Option<&str>) -> bool {
    use crate::cli::{
        execution_state::{self, ExecutionControlStatus},
        governance::RecoveryProbeState,
    };

    let Some(session) = current_session_id.filter(|session| !session.trim().is_empty()) else {
        return false;
    };
    let Ok(Some(state)) = gwt_core::skill_state::load(worktree, SKILL_NAME) else {
        return false;
    };
    let resolved = gwt_core::paths::resolve_current_worktree_root(worktree);
    let Ok(Some(record)) = execution_state::load(&resolved) else {
        return false;
    };
    // The recovery probe describes the Blocked-only abort path. In Active
    // executions it also returns unavailable/retryable:false (requires_blocked),
    // which must never exempt an ordinary unfinished build from Stop.
    if !state.active
        || state.session_id != session
        || record.primary_session_id != session
        || state.owner_spec != Some(record.owner_number)
        || record.status != ExecutionControlStatus::Blocked
        || !execution_state::integrity_ok(&record)
    {
        return false;
    }
    let diagnosis = execution_state::diagnose(&resolved, Some(session));
    let Some(probe) = diagnosis.recovery_probes.iter().find(|probe| {
        probe.operation == "build.abort"
            && probe.state == RecoveryProbeState::Unavailable
            && probe.governance.retryable == Some(false)
    }) else {
        return false;
    };
    super::diagnostics::record_stop_gate_decision(
        &resolved,
        serde_json::json!({
            "message": "Build Stop gate released with lifecycle still active: build.abort is unavailable and non-retryable",
            "gate": "skill-build-spec-stop-check",
            "issue": 4623,
            "session_id": session,
            "owner_number": record.owner_number,
            "owner_kind": record.owner_kind.as_str(),
            "build_active": state.active,
            "build_started_at": state.started_at,
            "phase": state.phase,
            "execution_status": record.status,
            "execution_generation": diagnosis.generation_id,
            "abort_probe": probe,
        }),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use gwt_core::skill_state::{save, SkillState};

    fn active_state(session: &str, phase: &str) -> SkillState {
        SkillState {
            start_evidence: None,
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
                "build.complete",
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

    #[test]
    fn active_build_state_gates_uniformly_after_lane_removal() {
        // SPEC #3245 FR-007: the former intake-lane exemption is gone — an
        // active build-spec state blocks Stop in every worktree the same way.
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), SKILL_NAME, &active_state("sess-1", "red")).unwrap();

        assert!(
            matches!(
                handle_with_input(dir.path(), "{}", Some("sess-1")),
                HookOutput::StopBlock { .. }
            ),
            "the build-spec gate fires uniformly after the lane removal"
        );
    }
}
