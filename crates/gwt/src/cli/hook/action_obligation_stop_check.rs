//! Prompt-to-action Stop gate (SPEC-3248 P11 core, T-242 core).
//!
//! Execution-lane sessions with OPEN producing obligations (armed by
//! request-form prompts, `action_obligation::mark_from_prompt`) cannot
//! stop: settle them with the canonical operations — `issue.comment` /
//! `issue.spec.edit` for issue updates, an all-passing `verify.run` for
//! implementation/verification, `pr.create` / `pr.edit` / `pr.ready` for
//! PR work — or defer them with `execution.blocked` and a real reason.
//! Prose, Board posts, and PR body text never settle anything.
//!
//! Standard Stop contracts hold: `stop_hook_active` short-circuits,
//! missing/malformed/cross-session state fails open (FR-014t/u), and
//! intake lanes are excluded (their artifact gate owns them).

use std::path::Path;

use super::{context::HookContext, envelope::stop_hook_active_from, HookOutput};
use crate::cli::action_obligation;

/// UserPromptSubmit entry: arm typed obligations for producing prompts in
/// execution lanes (intake lanes have their own artifact gate). A missing
/// or unparsable prompt arms nothing — unclassifiable input must not
/// over-block (conservative bias, opposite of the intake dirty marker).
pub fn handle_user_prompt_submit(worktree: &Path, input: &str) {
    let resolved = gwt_core::paths::resolve_current_worktree_root(worktree);
    let lane = HookContext::for_worktree(&resolved).lane;
    if lane.policy_flags.completion_gate {
        return;
    }
    let Some(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let Some(prompt) = serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|value| {
            value
                .get("prompt")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
    else {
        return;
    };
    if let Err(error) = action_obligation::mark_from_prompt(&resolved, &session_id, &prompt) {
        tracing::warn!(?error, "action obligation arming failed");
    }
}

pub fn handle_with_input(
    worktree: &Path,
    input: &str,
    current_session: Option<&str>,
) -> HookOutput {
    if stop_hook_active_from(input) {
        return HookOutput::Silent;
    }
    let resolved = gwt_core::paths::resolve_current_worktree_root(worktree);
    let lane = HookContext::for_worktree(&resolved).lane;
    if lane.policy_flags.completion_gate {
        return HookOutput::Silent;
    }
    let Some(session) = current_session else {
        return HookOutput::Silent;
    };
    let open = action_obligation::open_kinds(&resolved, session.trim());
    if open.is_empty() {
        return HookOutput::Silent;
    }
    let kinds = open
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    HookOutput::stop_block(format!(
        "Producing obligations from this session's prompts are still open: [{kinds}] (prompt-to-action gate, SPEC-3248 P11).\n\
         Settle them with the canonical operations before stopping:\n\
         - issue_update: JSON operations `issue.comment` / `issue.spec.edit`\n\
         - implementation / verification: an all-passing JSON operation `verify.run` (register the matrix with `verify.plan` first)\n\
         - pr: JSON operations `pr.create` / `pr.edit` / `pr.ready`\n\
         Blocked instead? Run JSON operation `execution.blocked` with a non-empty `params.reason` — it defers the open obligations with the blocker on record.\n\
         Prose, Board posts, and PR body text do not settle obligations."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_skills::{write_lane_file, EXECUTION_PROFILE, INTAKE_PROFILE};

    fn mk_worktree(profile: &gwt_skills::LaneProfile) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gwt")).unwrap();
        write_lane_file(dir.path(), profile).unwrap();
        dir
    }

    // T-242 core: open producing obligations block Stop; settlement passes;
    // the block routes to canonical operations and the deferral path.
    #[test]
    fn open_obligations_block_until_settled() {
        let dir = mk_worktree(&EXECUTION_PROFILE);
        action_obligation::mark_from_prompt(dir.path(), "sess-1", "バグを修正して").unwrap();

        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        let HookOutput::StopBlock { reason } = output else {
            panic!("expected StopBlock, got {output:?}");
        };
        assert!(reason.contains("implementation"), "{reason}");
        assert!(reason.contains("verify.run"), "{reason}");
        assert!(reason.contains("execution.blocked"), "{reason}");

        action_obligation::settle_kinds_best_effort(
            dir.path(),
            "sess-1",
            &[
                action_obligation::ObligationKind::Implementation,
                action_obligation::ObligationKind::Verification,
            ],
            "verify.run vr-test",
        );
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent
        );
    }

    // Fail-open contracts: no state, cross-session, no session id,
    // stop_hook_active, and intake lanes stay silent.
    #[test]
    fn fail_open_contracts_hold() {
        let dir = mk_worktree(&EXECUTION_PROFILE);
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-1")),
            HookOutput::Silent
        );
        action_obligation::mark_from_prompt(dir.path(), "sess-1", "実装して").unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("other")),
            HookOutput::Silent
        );
        assert_eq!(
            handle_with_input(dir.path(), "{}", None),
            HookOutput::Silent
        );
        assert_eq!(
            handle_with_input(dir.path(), r#"{"stop_hook_active":true}"#, Some("sess-1")),
            HookOutput::Silent
        );

        let intake = mk_worktree(&INTAKE_PROFILE);
        action_obligation::mark_from_prompt(intake.path(), "sess-1", "実装して").unwrap();
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _kind = gwt_core::test_support::ScopedEnvVar::unset(gwt_skills::GWT_SESSION_KIND_ENV);
        assert_eq!(
            handle_with_input(intake.path(), "{}", Some("sess-1")),
            HookOutput::Silent
        );
    }
}
