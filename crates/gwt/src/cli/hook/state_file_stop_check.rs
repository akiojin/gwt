//! Generic Stop-block decision for skills backed by a
//! `.gwt/skill-state/<skill>.json` file (SPEC-3248 hooks v2 P2).
//!
//! `gwt-plan-spec`, `gwt-build-spec`, and `gwt-register-spec` all share the
//! same Stop-block semantics: while the skill state is `active: true` for the
//! current agent session, block Stop so the skill keeps running; honour
//! `stop_hook_active: true` to cap forced continuation at one per cycle
//! (FR-014o); fail open on any I/O or JSON error (FR-014u); and stay silent
//! when the recorded `session_id` differs from the current session (FR-014t).
//!
//! Before P2 this body lived inside `skill_plan_spec_stop_check` and the other
//! two handlers reached into it, so they stayed in lock-step by accident rather
//! than design. This module makes the shared decision a first-class, per-skill
//! parameterized function; each `skill_*_stop_check` handler is now a thin
//! wrapper that supplies only its skill name / display / action prefix.

use std::path::Path;

use gwt_core::skill_state;

use super::{envelope::stop_hook_active_from, HookOutput};

/// Shared Stop-block decision for a skill-state-backed skill.
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
    let action_prefix = action_prefix_for(skill_name);

    HookOutput::stop_block(format!(
        "{skill_display} for {spec_clause} is still active{phase_clause}.\n\
         Continue the {skill_display} workflow, or run JSON operation `{action_prefix}.complete` with `params.spec:<n>` when the artifacts are ready, \
         or JSON operation `{action_prefix}.abort` with `params.spec:<n>` and `params.reason` to abandon.",
    ))
}

/// Map a skill name to its `gwtd` JSON operation prefix (`<prefix>.complete` /
/// `<prefix>.abort`).
fn action_prefix_for(skill_name: &str) -> &'static str {
    match skill_name {
        "plan-spec" => "plan",
        "build-spec" => "build",
        "register-spec" => "register",
        _ => "<skill>",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use gwt_core::skill_state::{save, SkillState};

    fn active_state(session: &str, phase: &str) -> SkillState {
        SkillState {
            active: true,
            owner_spec: Some(3248),
            started_at: Utc.with_ymd_and_hms(2026, 7, 5, 9, 0, 0).unwrap(),
            phase: Some(phase.to_string()),
            session_id: session.to_string(),
        }
    }

    #[test]
    fn decide_is_generic_across_skills_with_correct_action_prefix() {
        for (skill, display, prefix) in [
            ("plan-spec", "gwt-plan-spec", "plan.complete"),
            ("build-spec", "gwt-build-spec", "build.complete"),
            ("register-spec", "gwt-register-spec", "register.complete"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            save(dir.path(), skill, &active_state("sess-1", "phase-x")).unwrap();
            let HookOutput::StopBlock { reason } =
                decide(dir.path(), "{}", Some("sess-1"), skill, display)
            else {
                panic!("expected StopBlock for {skill}");
            };
            assert!(reason.contains(display), "{reason}");
            assert!(
                reason.contains(prefix),
                "{skill} must use {prefix}: {reason}"
            );
        }
    }

    #[test]
    fn decide_honours_stop_hook_active_session_isolation_and_fail_open() {
        let dir = tempfile::tempdir().unwrap();
        // Fail-open: no state file → Silent.
        assert_eq!(
            decide(
                dir.path(),
                "{}",
                Some("sess-1"),
                "plan-spec",
                "gwt-plan-spec"
            ),
            HookOutput::Silent
        );
        save(dir.path(), "plan-spec", &active_state("sess-1", "p")).unwrap();
        // stop_hook_active → Silent.
        assert_eq!(
            decide(
                dir.path(),
                r#"{"stop_hook_active":true}"#,
                Some("sess-1"),
                "plan-spec",
                "gwt-plan-spec"
            ),
            HookOutput::Silent
        );
        // Session mismatch → Silent.
        assert_eq!(
            decide(
                dir.path(),
                "{}",
                Some("other"),
                "plan-spec",
                "gwt-plan-spec"
            ),
            HookOutput::Silent
        );
    }
}
