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

use super::{envelope::stop_hook_active_from, HookOutput};
use crate::cli::action_obligation;

/// UserPromptSubmit entry: arm typed obligations for producing prompts. A
/// missing or unparsable prompt arms nothing — unclassifiable input must not
/// over-block (conservative bias).
pub fn handle_user_prompt_submit(worktree: &Path, input: &str) {
    // SPEC-3431 FR-064: the resident PM cannot settle a producing obligation.
    // Every settlement path (all-passing `verify.run`, `pr.*`) requires
    // production artifacts the PM's contract forbids it from creating, so
    // arming one leaves it blocked at Stop with no exit but a false
    // `execution.blocked`. Never arm rather than block-then-excuse.
    if super::is_resident_pm_worktree(worktree) {
        return;
    }
    let resolved = gwt_core::paths::resolve_current_worktree_root(worktree);
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
    HookOutput::system_message(format!(
        "Warning: producing obligations from this session's prompts are still open: [{kinds}] (prompt-to-action bookkeeping, SPEC-3248 P11).\n\
         Settle them with the canonical operations before stopping:\n\
         - issue_update: JSON operations `issue.comment` / `issue.spec.edit`\n\
         - implementation / verification: an all-passing JSON operation `verify.run` (register the matrix with `verify.plan` first)\n\
         - pr: JSON operations `pr.create` / `pr.edit` / `pr.ready`\n\
         Blocked instead? Run JSON operation `execution.blocked` with a non-empty `params.reason` — it defers the open obligations with the blocker on record.\n\
         Prose, Board posts, and PR body text do not settle obligations. Stop is not blocked by this bookkeeping state."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_worktree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gwt")).unwrap();
        dir
    }

    #[test]
    fn open_obligations_warn_until_settled() {
        let dir = mk_worktree();
        action_obligation::mark_from_prompt(dir.path(), "sess-1", "バグを修正して").unwrap();

        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        let HookOutput::SystemMessage(reason) = output else {
            panic!("expected SystemMessage, got {output:?}");
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

    /// SPEC-3431 FR-064: the resident PM never arms a producing obligation.
    ///
    /// The settlement paths are an all-passing `verify.run`, `issue.comment` /
    /// `issue.spec.edit`, or `pr.*`. The PM's contract forbids it from touching
    /// production code or PRs at all — implementation is always performed by
    /// agents the Issue Monitor launches — so an implementation obligation is
    /// **structurally unsettleable** for the PM and its only exit is filing a
    /// false `execution.blocked` every turn. Observed live: the gate was
    /// already arming against the running PM session.
    #[test]
    fn the_resident_pm_never_arms_a_producing_obligation() {
        // GWT_SESSION_ID is process-global; without this these two tests race
        // each other and whichever loses reads the other's session id.
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let pm_worktree = crate::pm_registry::pm_worktree_path_for_repo_path(&repo);
        std::fs::create_dir_all(pm_worktree.join(".gwt")).unwrap();
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");

        handle_user_prompt_submit(
            &pm_worktree,
            &serde_json::json!({ "prompt": "#3457 を修正して" }).to_string(),
        );

        assert_eq!(
            handle_with_input(&pm_worktree, "{}", Some("pm-session")),
            HookOutput::Silent,
            "the PM must not be blocked by an obligation it cannot settle"
        );
    }

    /// The exemption is keyed on the PM worktree alone: an ordinary agent in
    /// any other worktree keeps the gate exactly as it was.
    #[test]
    fn an_ordinary_worktree_still_arms_obligations() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = mk_worktree();
        let _session = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "sess-ordinary",
        );

        handle_user_prompt_submit(
            dir.path(),
            &serde_json::json!({ "prompt": "バグを修正して" }).to_string(),
        );

        assert!(
            matches!(
                handle_with_input(dir.path(), "{}", Some("sess-ordinary")),
                HookOutput::SystemMessage(_)
            ),
            "non-PM sessions must keep prompt-to-action bookkeeping without blocking Stop"
        );
    }

    // SPEC-3393 P4: assistant prose is not gate state. Historical summaries
    // and legitimate completion reports pass when no structured obligation is
    // open, even when they contain completion keywords.
    #[test]
    fn completion_prose_does_not_create_an_obligation() {
        let dir = mk_worktree();
        action_obligation::mark_from_prompt(dir.path(), "sess-1", "バグを修正して").unwrap();
        action_obligation::settle_kinds_best_effort(
            dir.path(),
            "sess-1",
            &[action_obligation::ObligationKind::Implementation],
            "verify.run vr-x",
        );

        let transcript = dir.path().join("transcript.jsonl");
        std::fs::write(
            &transcript,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"検証しました。全テスト成功です。\"}]}}\n",
        )
        .unwrap();
        let input =
            serde_json::json!({ "transcript_path": transcript.to_string_lossy() }).to_string();

        assert_eq!(
            handle_with_input(dir.path(), &input, Some("sess-1")),
            HookOutput::Silent
        );
    }

    // Fail-open contracts: no state, cross-session, no session id,
    // stop_hook_active, and intake lanes stay silent.
    #[test]
    fn fail_open_contracts_hold() {
        let dir = mk_worktree();
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

        // SPEC #3245 FR-007: the former intake-lane exemption is gone — an
        // armed obligation warns in every worktree the same way.
        let former_intake = mk_worktree();
        action_obligation::mark_from_prompt(former_intake.path(), "sess-1", "実装して").unwrap();
        assert!(
            matches!(
                handle_with_input(former_intake.path(), "{}", Some("sess-1")),
                HookOutput::SystemMessage(_)
            ),
            "obligations must warn without blocking Stop"
        );
    }
}
