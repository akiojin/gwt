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
use crate::cli::{action_obligation, execution_state};

/// Name the Session holding this worktree's execution control record when the
/// caller is not that Session — an orphan window (Issue #4454).
///
/// An orphan holds no authority over the Work, so every settlement path this
/// gate names (`verify.run`, `pr.*`, `issue.*`) refuses it, and so does the
/// deferral path: `execution.blocked` points at `execution.adopt`, which
/// refuses for want of the same authority. The obligation then has no exit at
/// all and the session cannot stop — measured live on 2026-09-16 (#4234),
/// five refusals in one cycle. Never arm one, and release one already on
/// record at Stop.
///
/// An unreadable record yields `None`, so the gate is unchanged whenever
/// orphanhood cannot be proven.
fn foreign_record_holder(
    worktree: &Path,
    session: &str,
) -> Option<execution_state::ForeignExecutionRecordHolder> {
    execution_state::foreign_record_holder(worktree, session)
        .ok()
        .flatten()
}

/// UserPromptSubmit entry: arm typed obligations for producing prompts. A
/// missing or unparsable prompt arms nothing — unclassifiable input must not
/// over-block (conservative bias).
pub fn handle_user_prompt_submit(worktree: &Path, input: &str) {
    handle_user_prompt_submit_with_context(
        worktree,
        input,
        crate::issue_monitor_review::review_dispatch_session_active(),
    );
}

pub(crate) fn handle_user_prompt_submit_with_context(
    worktree: &Path,
    input: &str,
    review_dispatch_session: bool,
) {
    // Issue #3984 (AC-3): an independent-review dispatch session is subject to
    // the same structural trap as the resident PM below. Its review prompt
    // reads as a producing request ("verify AC-3 against the PR"), but every
    // settlement path — `verify.run`, `pr.*`, `issue.spec.edit` — belongs to
    // the implementing session and is refused in a review window, so an armed
    // obligation could only ever be discharged by a false `execution.blocked`
    // and would strand the finished verdict at Stop.
    if review_dispatch_session {
        return;
    }
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
    // Issue #4454: an orphan window can settle nothing, so it must arm
    // nothing. Not arming is the primary fix; the Stop-side release below
    // covers obligations already on record.
    if foreign_record_holder(&resolved, &session_id).is_some() {
        return;
    }
    if let Err(error) = action_obligation::mark_from_prompt(&resolved, &session_id, &prompt) {
        tracing::warn!(?error, "action obligation arming failed");
    }
}

pub fn handle_with_input(
    worktree: &Path,
    input: &str,
    current_session: Option<&str>,
) -> HookOutput {
    handle_with_input_with_context(
        worktree,
        input,
        current_session,
        crate::issue_monitor_review::review_dispatch_session_active(),
    )
}

pub(crate) fn handle_with_input_with_context(
    worktree: &Path,
    input: &str,
    current_session: Option<&str>,
    review_dispatch_session: bool,
) -> HookOutput {
    if stop_hook_active_from(input) {
        return HookOutput::Silent;
    }
    // Issue #3984 (AC-3): not arming is the primary fix, but an obligation
    // already on record for this session id (state written before this gate
    // existed, or a session id reused after a crash) must not strand a
    // finished verdict at Stop either — the review window has no settlement
    // path other than a false `execution.blocked`.
    if review_dispatch_session {
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
    // Issue #4454: an obligation armed before this window lost (or never
    // held) the Work's authority must not strand it either. Release Stop and
    // record why, naming the Session that does hold the record (AC-2).
    if let Some(holder) = foreign_record_holder(&resolved, session.trim()) {
        super::diagnostics::record_stop_gate_decision(
            &resolved,
            serde_json::json!({
                "message": "Stop released without settling producing obligations: this session holds no execution authority for the Work",
                "gate": "action-obligation-stop-check",
                "issue": 4454,
                "session_id": session.trim(),
                "record_holder_session_id": holder.holder_session_id,
                "owner": format!(
                    "{kind} #{number}",
                    kind = holder.owner_kind.as_str(),
                    number = holder.owner_number
                ),
                "open_obligations": open
                    .iter()
                    .map(|kind| kind.as_str())
                    .collect::<Vec<_>>(),
            }),
        );
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
         - pr: JSON operation `pr.edit` (any PR state, including MERGED), `pr.ready` (open draft only), or `pr.create` (only while no PR exists); a PR readied or merged on your behalf is settled by `pr.edit` on it\n\
         Genuinely blocked? JSON operation `execution.blocked` with a non-empty `params.reason` defers the open obligations with the blocker on record — but it is terminal, and recovery costs `execution.reopen` plus a fresh derived-plan `verify.run`, so it is never the easy way out.\n\
         Prose, Board posts, and PR body text do not settle obligations."
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

    // T-242 core: open producing obligations block Stop; settlement passes;
    // the block routes to canonical operations and the deferral path.
    #[test]
    fn open_obligations_block_until_settled() {
        let dir = mk_worktree();
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

    /// Issue #3984 (AC-3): an independent-review dispatch session never arms a
    /// producing obligation, and Stop stays open even when one is already on
    /// record for it. Every settlement path (`verify.run`, `pr.*`,
    /// `issue.spec.edit`) belongs to the implementing session and is refused in
    /// a review window, so an armed obligation would strand the finished
    /// verdict behind a false `execution.blocked`.
    #[test]
    fn a_review_dispatch_session_never_blocks_on_producing_obligations() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = mk_worktree();
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "review-sess");
        // A review request reads as a producing `pr` prompt to the classifier —
        // which is exactly the trap: the review window cannot run `pr.*`.
        let prompt = serde_json::json!({ "prompt": "PR #4000 の AC を検証して verdict を返して" })
            .to_string();

        handle_user_prompt_submit_with_context(dir.path(), &prompt, true);
        assert_eq!(
            handle_with_input_with_context(dir.path(), "{}", Some("review-sess"), true),
            HookOutput::Silent,
            "the review window must not arm an obligation it cannot settle"
        );

        // An obligation already persisted for this session id — state written
        // before this gate existed, or a reused id — must not strand the
        // verdict at Stop either.
        action_obligation::mark_from_prompt(dir.path(), "review-sess", "実装して").unwrap();
        assert_eq!(
            handle_with_input_with_context(dir.path(), "{}", Some("review-sess"), true),
            HookOutput::Silent,
            "a persisted obligation must not gate a review window's Stop"
        );
        assert!(
            matches!(
                handle_with_input_with_context(dir.path(), "{}", Some("review-sess"), false),
                HookOutput::StopBlock { .. }
            ),
            "the same persisted obligation still gates a producing session"
        );

        // Non-regression: the exemption is keyed on the review marker alone —
        // the same prompt in an implementing session still arms and blocks.
        handle_user_prompt_submit_with_context(dir.path(), &prompt, false);
        assert!(
            matches!(
                handle_with_input_with_context(dir.path(), "{}", Some("review-sess"), false),
                HookOutput::StopBlock { .. }
            ),
            "a producing session keeps the prompt-to-action gate"
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
                HookOutput::StopBlock { .. }
            ),
            "non-PM sessions must keep the prompt-to-action gate"
        );
    }

    /// Issue #4454 AC-1 / AC-3: an orphan window — one whose worktree's
    /// execution control record belongs to another session — is never asked to
    /// settle a producing obligation.
    ///
    /// Observed live on 2026-09-16 (#4234): every settlement path this gate
    /// names (`verify.run`, `pr.*`, `issue.*`) and the deferral path
    /// (`execution.blocked`) refused the window for want of authority, and
    /// `execution.adopt` — the operation `execution.blocked` pointed at —
    /// refused too. Five refusals, no exit, and the session could not stop at
    /// all.
    #[test]
    fn an_orphan_window_stops_without_settling_obligations() {
        let dir = mk_worktree();
        crate::cli::execution_state::materialize_at_launch(
            dir.path(),
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            4234,
            "owner-sess",
            "$gwt-execute",
            false,
        )
        .unwrap();
        action_obligation::mark_from_prompt(dir.path(), "orphan-sess", "バグを修正して").unwrap();

        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("orphan-sess")),
            HookOutput::Silent,
            "a window holding no execution authority must not be asked to settle"
        );
    }

    /// Issue #4454 AC-4: the exemption is keyed on a record that positively
    /// names another session. The session the record belongs to keeps the gate
    /// exactly as it was, and so does a worktree with no record at all (covered
    /// by `open_obligations_block_until_settled`).
    #[test]
    fn the_record_holding_session_keeps_the_obligation_gate() {
        let dir = mk_worktree();
        crate::cli::execution_state::materialize_at_launch(
            dir.path(),
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            4234,
            "sess-1",
            "$gwt-execute",
            false,
        )
        .unwrap();
        action_obligation::mark_from_prompt(dir.path(), "sess-1", "バグを修正して").unwrap();

        assert!(
            matches!(
                handle_with_input(dir.path(), "{}", Some("sess-1")),
                HookOutput::StopBlock { .. }
            ),
            "the authority-holding session must keep the unchanged settlement contract"
        );
    }

    /// Issue #4454 AC-2: the grounds for the exemption survive the session in
    /// the project log — including which session actually holds the record.
    /// A refusal the agent never sees is not evidence.
    #[test]
    fn the_orphan_exemption_is_recorded_in_the_project_log() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = mk_worktree();
        let resolved = gwt_core::paths::resolve_current_worktree_root(dir.path());
        crate::cli::execution_state::materialize_at_launch(
            dir.path(),
            crate::cli::execution_state::ExecutionOwnerKind::Issue,
            4234,
            "owner-sess",
            "$gwt-execute",
            false,
        )
        .unwrap();
        action_obligation::mark_from_prompt(dir.path(), "orphan-sess", "実装して").unwrap();

        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("orphan-sess")),
            HookOutput::Silent
        );

        let log_dir = gwt_core::paths::gwt_project_logs_dir_for_project_path(&resolved);
        let log = std::fs::read_to_string(gwt_core::logging::current_log_file(&log_dir))
            .expect("project log written at Stop");
        assert!(log.contains("owner-sess"), "{log}");
        assert!(log.contains("orphan-sess"), "{log}");
        assert!(log.contains("issue #4234"), "{log}");
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
        // armed obligation blocks Stop in every worktree the same way.
        let former_intake = mk_worktree();
        action_obligation::mark_from_prompt(former_intake.path(), "sess-1", "実装して").unwrap();
        assert!(
            matches!(
                handle_with_input(former_intake.path(), "{}", Some("sess-1")),
                HookOutput::StopBlock { .. }
            ),
            "obligations gate uniformly after the lane removal"
        );
    }
}
