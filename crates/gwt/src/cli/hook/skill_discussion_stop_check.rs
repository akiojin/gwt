//! `gwtd hook skill-discussion-stop-check` — Stop-block handler for the
//! `gwt-discussion` skill (SPEC-1935 Phase 10, FR-014p).
//!
//! Reads `.gwt/work/discussions.md` in the current worktree, falling back to
//! legacy `.gwt/discussion.md`, and returns `HookOutput::StopBlock` when an
//! active proposal still has a next question, evidence blocker, or depth
//! blocker. Claude Code's built-in `stop_hook_active` flag (FR-014o)
//! short-circuits this handler to prevent infinite loops.
//!
//! The legacy proposal parser remains fail-open. A managed root Intake is
//! stricter: a missing/stale structured checkpoint blocks one Stop cycle so
//! an incomplete crash turn is never guessed into Recovery or Board.

use std::{
    io::{self, Read},
    path::Path,
};

use super::{envelope::stop_hook_active_from, HookError, HookOutput};
use crate::discussion_resume::discussion_stop_blocker;

pub fn handle() -> Result<HookOutput, HookError> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let cwd = std::env::current_dir()?;
    Ok(handle_with_input(&cwd, &input))
}

/// Pure core decision. Always returns `Silent` on any parse/IO failure
/// so the Stop hook stays fail-open.
pub fn handle_with_input(worktree: &Path, input: &str) -> HookOutput {
    if stop_hook_active_from(input) || subagent_hook_input(input) {
        return HookOutput::Silent;
    }
    if let Ok(Some(pending)) = discussion_stop_blocker(worktree) {
        if let Some(question) = pending
            .next_question
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
        {
            return HookOutput::stop_block(format!(
                "Discussion is still [active] on proposal \"{title}\".\n\
                 Next question, evidence blocker, or depth blocker: {question}\n\
                 Continue the gwt-discussion workflow (investigate → ask the user → update Discussion TODO), \
                 or run JSON operation `discuss.resolve`, `discuss.park`, or `discuss.reject` with `params.proposal:\"{label}\"` to exit the discussion explicitly.",
                title = pending.proposal_title,
                question = question,
                label = pending.proposal_label,
            ));
        }
    }

    match crate::cli::discussion::current_intake_durability_blocker(worktree) {
        Ok(Some(reason)) => HookOutput::stop_block(reason),
        Ok(None) => HookOutput::Silent,
        Err(error) if managed_intake_environment_present() => HookOutput::stop_block(format!(
            "Current Intake durability cannot be verified: {error}. Resolve the candidate in Recovery Center or complete `discussion.update` before stopping; no private transcript or incomplete Board milestone was backfilled."
        )),
        Err(_) => HookOutput::Silent,
    }
}

fn managed_intake_environment_present() -> bool {
    [
        gwt_agent::GWT_SESSION_ID_ENV,
        gwt_agent::GWT_RECOVERY_ID_ENV,
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    })
}

fn subagent_hook_input(input: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(input) else {
        return false;
    };
    value
        .get("isSidechain")
        .or_else(|| value.get("is_sidechain"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        || ["agent_id", "agentId"].iter().any(|name| {
            value
                .get(*name)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use gwt_core::{
        recovery::{
            BindingQuality, BoardMilestoneIntent, CreateRecovery, ProviderRootBinding,
            RecoverySessionKind, RecoveryStore, SemanticCheckpoint,
        },
        test_support::ScopedEnvVar,
    };
    use sha2::{Digest, Sha256};
    use std::fs;

    const ACTIVE_WITH_QUESTION: &str = "## Discussion TODO\n\n\
### Proposal A - Hook-driven resume [active]\n\
- Summary: Keep unfinished discussion state in the local artifact.\n\
- Next Question: Should SessionStart or UserPromptSubmit surface the resume proposal?\n\
";

    const ACTIVE_WITHOUT_QUESTION: &str = "## Discussion TODO\n\n\
### Proposal A - Hook-driven resume [active]\n\
- Summary: Keep unfinished discussion state in the local artifact.\n\
- Implementation Proof: crates/gwt/src/discussion_resume.rs inspected.\n\
- SPEC/Issue Proof: SPEC-1935 checked.\n\
- Gap Check Proof: scope/integration/failure/migration/verification checked.\n\
- Official Docs Proof: Claude Code hooks docs checked.\n\
- External Research Proof: not-applicable: local-only behavior.\n\
- Exit Blockers: none\n\
- Depth Mode: normal\n\
- Question Ledger: scope boundary, integration, failure, migration, verification checked\n\
- Depth Gate: complete\n\
- Next Question:\n\
- Evidence Gate: complete\n\
";

    const ACTIVE_WITH_EXIT_BLOCKER_WITHOUT_QUESTION: &str = "## Discussion TODO\n\n\
### Proposal A - Evidence gate [active]\n\
- Summary: Implementation is not proven yet.\n\
- Implementation Proof: TODO\n\
- SPEC/Issue Proof: SPEC-1935 checked.\n\
- Gap Check Proof: scope/integration/failure/migration/verification checked.\n\
- Official Docs Proof: Claude Code hooks docs checked.\n\
- External Research Proof: not-applicable: local-only behavior.\n\
- Exit Blockers: implementation proof is missing\n\
- Depth Mode: normal\n\
- Question Ledger: scope boundary checked only\n\
- Depth Gate: open\n\
- Next Question:\n\
- Evidence Gate: open\n\
";

    const ACTIVE_WITH_DEPTH_BLOCKER_WITHOUT_QUESTION: &str = "## Discussion TODO\n\n\
### Proposal A - Depth gate [active]\n\
- Summary: Evidence is complete but depth coverage is shallow.\n\
- Implementation Proof: crates/gwt/src/discussion_resume.rs inspected.\n\
- SPEC/Issue Proof: SPEC-1935 checked.\n\
- Gap Check Proof: scope/integration/failure/migration/verification checked.\n\
- Official Docs Proof: not-applicable: local-only behavior.\n\
- External Research Proof: not-applicable: local-only behavior.\n\
- Exit Blockers: none\n\
- Depth Mode: normal\n\
- Question Ledger: scope boundary checked only\n\
- Depth Gate: open\n\
- Next Question:\n\
- Evidence Gate: complete\n\
";

    const ALL_RESOLVED: &str = "## Discussion TODO\n\n\
### Proposal A - Hook-driven resume [chosen]\n\
- Summary: Done.\n\
- Next Question: Should SessionStart surface the proposal?\n\
";

    fn write_discussion(dir: &Path, body: &str) {
        let path = dir.join(".gwt/discussion.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn write_canonical_discussion(dir: &Path, body: &str) {
        let path = gwt_core::paths::gwt_repo_local_discussions_path(dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn write_durable_checkpoint_memo(repo: &Path, recovery_id: &str, operation_id: &str) {
        let digest = Sha256::digest(recovery_id.as_bytes());
        let key = hex::encode(&digest[..12]);
        let path = gwt_core::paths::gwt_work_notes_discussions_path(repo);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                "# Discussions\n\n<!-- gwt-intake-current {key} {operation_id} -->\n\n## 2026-07-16 — Stop durability\n\nStatus: active\nCheckpoint Operation: {operation_id}\n\nSummary:\nDurable turn one\n\nNext:\nWait for the next root turn\n"
            ),
        )
        .unwrap();
    }

    fn assert_stop_block(output: HookOutput, contains: &[&str]) {
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

    fn standalone_environment() -> (
        std::sync::MutexGuard<'static, ()>,
        ScopedEnvVar,
        ScopedEnvVar,
        ScopedEnvVar,
    ) {
        let lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let session = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let recovery = ScopedEnvVar::unset(gwt_agent::GWT_RECOVERY_ID_ENV);
        let provider = ScopedEnvVar::unset("CODEX_THREAD_ID");
        (lock, session, recovery, provider)
    }

    #[test]
    fn blocks_when_active_proposal_has_non_empty_next_question() {
        let _env = standalone_environment();
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_QUESTION);
        let output = handle_with_input(dir.path(), "{}");
        assert_stop_block(
            output,
            &[
                "Hook-driven resume",
                "Should SessionStart or UserPromptSubmit surface the resume proposal?",
                "discuss.resolve",
                "Proposal A",
            ],
        );
    }

    #[test]
    fn blocks_when_active_proposal_is_in_canonical_discussions_md() {
        let _env = standalone_environment();
        let dir = tempfile::tempdir().unwrap();
        write_canonical_discussion(
            dir.path(),
            &format!(
                "# Discussions\n\n## 2026-06-17 — Managed Hooks\n\nStatus: active\n\n{}",
                ACTIVE_WITH_QUESTION
            ),
        );

        assert_stop_block(
            handle_with_input(dir.path(), "{}"),
            &[
                "Hook-driven resume",
                "Should SessionStart or UserPromptSubmit surface the resume proposal?",
                "Proposal A",
            ],
        );
    }

    #[test]
    fn block_reason_quotes_proposal_label_for_shell_safety() {
        let _env = standalone_environment();
        // Regression: default labels like `Proposal A` contain spaces.
        // The remediation command must quote the label so the LLM's
        // tool call does not split on whitespace.
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_QUESTION);
        match handle_with_input(dir.path(), "{}") {
            HookOutput::StopBlock { reason } => {
                assert!(
                    reason.contains("params.proposal:\"Proposal A\""),
                    "proposal label must be double-quoted in reason; got: {reason}"
                );
                assert!(
                    !reason.contains("params.proposal:Proposal A"),
                    "unquoted form would shell-split; got: {reason}"
                );
            }
            other => panic!("expected StopBlock, got {other:?}"),
        }
    }

    #[test]
    fn silent_when_stop_hook_active_flag_is_true() {
        let _env = standalone_environment();
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_QUESTION);
        assert_eq!(
            handle_with_input(dir.path(), r#"{"stop_hook_active":true}"#),
            HookOutput::Silent
        );
    }

    #[test]
    fn silent_when_discussion_file_is_absent() {
        let _env = standalone_environment();
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(handle_with_input(dir.path(), "{}"), HookOutput::Silent);
    }

    #[test]
    fn silent_when_active_proposal_has_empty_next_question() {
        let _env = standalone_environment();
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITHOUT_QUESTION);
        assert_eq!(handle_with_input(dir.path(), "{}"), HookOutput::Silent);
    }

    #[test]
    fn blocks_when_evidence_gate_is_incomplete_without_next_question() {
        let _env = standalone_environment();
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_EXIT_BLOCKER_WITHOUT_QUESTION);
        assert_stop_block(
            handle_with_input(dir.path(), "{}"),
            &[
                "Evidence gate",
                "Exit Blockers remain unresolved",
                "Next question, evidence blocker, or depth blocker",
            ],
        );
    }

    #[test]
    fn blocks_when_depth_gate_is_incomplete_without_next_question() {
        let _env = standalone_environment();
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_DEPTH_BLOCKER_WITHOUT_QUESTION);
        assert_stop_block(
            handle_with_input(dir.path(), "{}"),
            &[
                "Depth gate",
                "Depth Gate is not complete",
                "Next question, evidence blocker, or depth blocker",
            ],
        );
    }

    #[test]
    fn silent_when_no_active_proposals_remain() {
        let _env = standalone_environment();
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ALL_RESOLVED);
        assert_eq!(handle_with_input(dir.path(), "{}"), HookOutput::Silent);
    }

    #[test]
    fn silent_when_discussion_md_is_malformed() {
        let _env = standalone_environment();
        let dir = tempfile::tempdir().unwrap();
        // `parse_proposals` is tolerant, but this test future-proofs the
        // fail-open contract: any unparseable input must not block Stop.
        write_discussion(
            dir.path(),
            "### Proposal ??? broken header without status label\n",
        );
        assert_eq!(handle_with_input(dir.path(), "{}"), HookOutput::Silent);
    }

    #[test]
    fn managed_intake_stop_blocks_missing_or_stale_checkpoint_without_posting_board() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let home = temp.path().join("home");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&home).unwrap();
        let recovery_id = "stop-durability-recovery";
        let session_id = "stop-durability-session";
        let root_id = "stop-durability-root";
        let _home = ScopedEnvVar::set("HOME", &home);
        let _userprofile = ScopedEnvVar::set("USERPROFILE", &home);
        let _recovery = ScopedEnvVar::set(gwt_agent::GWT_RECOVERY_ID_ENV, recovery_id);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);
        let _provider = ScopedEnvVar::set("CODEX_THREAD_ID", root_id);
        let repo_id = gwt_core::paths::project_scope_hash(&repo).to_string();
        let store =
            RecoveryStore::for_project_dir(home.join(".gwt").join("projects").join(&repo_id));
        store
            .create(
                CreateRecovery {
                    recovery_id: recovery_id.to_string(),
                    session_id: session_id.to_string(),
                    repo_id,
                    session_kind: RecoverySessionKind::Intake,
                    worktree_path: repo.clone(),
                    launch_base_ref: None,
                    launch_base_oid: "b".repeat(40),
                    launch_head_oid: "b".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "test Stop durability".to_string(),
                    created_at: Utc::now(),
                },
                "stop-durability-create",
            )
            .unwrap();
        store
            .bind_root(
                recovery_id,
                ProviderRootBinding {
                    root_id: root_id.to_string(),
                    session_tree_id: None,
                    quality: BindingQuality::Verified,
                    bound_at: Utc::now(),
                },
                "stop-durability-bind",
            )
            .unwrap();
        store
            .record_root_input(
                recovery_id,
                root_id,
                "turn-1",
                "private root input",
                "stop-durability-root-1",
            )
            .unwrap();

        assert_stop_block(
            handle_with_input(&repo, r#"{"stop_hook_active":false}"#),
            &["no semantic discussion checkpoint", "discussion.update"],
        );
        let missing = store.load(recovery_id).unwrap().unwrap();
        assert!(missing.board_outbox.is_empty());
        assert!(missing.board_entry_ids.is_empty());

        store
            .replace_checkpoint(
                recovery_id,
                root_id,
                0,
                SemanticCheckpoint {
                    summary: "Durable turn one".to_string(),
                    next_action: Some("Wait for the next root turn".to_string()),
                    as_of_turn_id: Some("turn-1".to_string()),
                    board_intents: vec![BoardMilestoneIntent {
                        entry_id: "stop-durability-entry".to_string(),
                        title: "Intake checkpoint: Stop durability".to_string(),
                        body: "Status: active\n\nSummary:\nDurable turn one".to_string(),
                        queued_at: Utc::now(),
                    }],
                    ..Default::default()
                },
                "stop-durability-checkpoint-1",
            )
            .unwrap();
        assert_stop_block(
            handle_with_input(&repo, "{}"),
            &["no canonical memo/checkpoint operation marker"],
        );
        write_durable_checkpoint_memo(&repo, recovery_id, "stop-durability-entry");
        assert_eq!(handle_with_input(&repo, "{}"), HookOutput::Silent);

        store
            .record_root_input(
                recovery_id,
                root_id,
                "turn-2",
                "new private answer",
                "stop-durability-root-2",
            )
            .unwrap();
        assert_stop_block(
            handle_with_input(&repo, "{}"),
            &[
                "uncheckpointed root turn turn-2",
                "will not reconstruct private input",
            ],
        );
        let stale = store.load(recovery_id).unwrap().unwrap();
        assert_eq!(stale.board_outbox.len(), 1);
        assert!(stale.board_entry_ids.is_empty());
        assert!(
            !gwt_core::coordination::coordination_dir(&repo).exists(),
            "Stop enforcement must never post Board"
        );
    }
}
