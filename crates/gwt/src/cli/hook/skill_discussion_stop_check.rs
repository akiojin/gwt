//! `gwtd hook skill-discussion-stop-check` — Stop-block handler for the
//! `gwt-discussion` skill (SPEC-1935 Phase 10, FR-014p).
//!
//! Reads `.gwt/work/discussions.md` in the current worktree, falling back to
//! legacy `.gwt/discussion.md`, and returns `HookOutput::StopBlock` when an
//! active proposal still has a next question, evidence blocker, or depth
//! blocker. Claude Code's built-in `stop_hook_active` flag (FR-014o)
//! short-circuits this handler to prevent infinite loops.
//!
//! Fail-open policy (FR-014u): read failures produce a nonblocking warning
//! so an unreadable state file never accidentally blocks a Stop. Missing
//! files and unrecognized Markdown stay silent.

use std::{
    io::{self, Read},
    path::Path,
};

use gwt_agent::GWT_SESSION_ID_ENV;

use super::{envelope::stop_hook_active_from, HookError, HookOutput};
use crate::discussion_resume::discussion_stop_blocker;

pub fn handle() -> Result<HookOutput, HookError> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let cwd = std::env::current_dir()?;
    let current_session = std::env::var(GWT_SESSION_ID_ENV).ok();
    Ok(handle_with_input(&cwd, &input, current_session.as_deref()))
}

/// Pure core decision. Warns without blocking on read failures so the
/// Stop hook stays fail-open.
///
/// Issue #3465: `current_session_id` keeps the gate inside the session that
/// opened the discussion. The canonical log is project-scoped, so without it
/// any session's active proposal blocks every other session of the same
/// project, and the only offered exits rewrite the owner's state.
pub fn handle_with_input(
    worktree: &Path,
    input: &str,
    current_session_id: Option<&str>,
) -> HookOutput {
    if stop_hook_active_from(input) {
        return HookOutput::Silent;
    }
    let pending = match discussion_stop_blocker(worktree, current_session_id) {
        Ok(Some(pending)) => pending,
        Ok(None) => return HookOutput::Silent,
        Err(error) => {
            tracing::warn!(%error, "discussion record is unreadable");
            return HookOutput::SystemMessage(format!(
                "Warning: Discussion record is unreadable ({error}). Stop is not blocked by this infrastructure failure."
            ));
        }
    };
    let Some(question) = pending
        .next_question
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    else {
        return HookOutput::Silent;
    };

    HookOutput::stop_block(format!(
        "Discussion is still [active] on proposal \"{title}\".\n\
         Next question, evidence blocker, or depth blocker: {question}\n\
         Continue the gwt-discussion workflow (investigate → ask the user → update Discussion TODO), \
         or run JSON operation `discuss.resolve`, `discuss.park`, or `discuss.reject` with `params.proposal:\"{label}\"` to exit the discussion explicitly.",
        title = pending.proposal_title,
        question = question,
        label = pending.proposal_label,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn blocks_when_active_proposal_has_non_empty_next_question() {
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_QUESTION);
        let output = handle_with_input(dir.path(), "{}", None);
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
        let dir = tempfile::tempdir().unwrap();
        write_canonical_discussion(
            dir.path(),
            &format!(
                "# Discussions\n\n## 2026-06-17 — Managed Hooks\n\nStatus: active\n\n{}",
                ACTIVE_WITH_QUESTION
            ),
        );

        assert_stop_block(
            handle_with_input(dir.path(), "{}", None),
            &[
                "Hook-driven resume",
                "Should SessionStart or UserPromptSubmit surface the resume proposal?",
                "Proposal A",
            ],
        );
    }

    #[test]
    fn block_reason_quotes_proposal_label_for_shell_safety() {
        // Regression: default labels like `Proposal A` contain spaces.
        // The remediation command must quote the label so the LLM's
        // tool call does not split on whitespace.
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_QUESTION);
        match handle_with_input(dir.path(), "{}", None) {
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

    /// Issue #3465: a discussion entry opened by another session must not
    /// block this session's Stop.
    #[test]
    fn silent_when_canonical_entry_belongs_to_another_session() {
        let dir = tempfile::tempdir().unwrap();
        write_canonical_discussion(
            dir.path(),
            &format!(
                "# Discussions\n\n## 2026-08-04 — Other session\n\nStatus: active\nOrigin Session: sess-owner\n\n{}",
                ACTIVE_WITH_QUESTION
            ),
        );

        assert_eq!(
            handle_with_input(dir.path(), "{}", Some("sess-unrelated")),
            HookOutput::Silent
        );
        assert_stop_block(
            handle_with_input(dir.path(), "{}", Some("sess-owner")),
            &["Hook-driven resume"],
        );
    }

    #[test]
    fn silent_when_stop_hook_active_flag_is_true() {
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_QUESTION);
        assert_eq!(
            handle_with_input(dir.path(), r#"{"stop_hook_active":true}"#, None),
            HookOutput::Silent
        );
    }

    #[test]
    fn silent_when_discussion_file_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            handle_with_input(dir.path(), "{}", None),
            HookOutput::Silent
        );
    }

    #[test]
    fn silent_when_active_proposal_has_empty_next_question() {
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITHOUT_QUESTION);
        assert_eq!(
            handle_with_input(dir.path(), "{}", None),
            HookOutput::Silent
        );
    }

    #[test]
    fn blocks_when_evidence_gate_is_incomplete_without_next_question() {
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_EXIT_BLOCKER_WITHOUT_QUESTION);
        assert_stop_block(
            handle_with_input(dir.path(), "{}", None),
            &[
                "Evidence gate",
                "Exit Blockers remain unresolved",
                "Next question, evidence blocker, or depth blocker",
            ],
        );
    }

    #[test]
    fn blocks_when_depth_gate_is_incomplete_without_next_question() {
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_DEPTH_BLOCKER_WITHOUT_QUESTION);
        assert_stop_block(
            handle_with_input(dir.path(), "{}", None),
            &[
                "Depth gate",
                "Depth Gate is not complete",
                "Next question, evidence blocker, or depth blocker",
            ],
        );
    }

    #[test]
    fn silent_when_no_active_proposals_remain() {
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ALL_RESOLVED);
        assert_eq!(
            handle_with_input(dir.path(), "{}", None),
            HookOutput::Silent
        );
    }

    #[test]
    fn malformed_markdown_is_silent_and_unreadable_discussion_warns_without_blocking_stop() {
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
        // Tolerant Markdown parsing still treats an unrecognized header as absent.
        write_discussion(
            dir.path(),
            "### Proposal ??? broken header without status label\n",
        );
        assert_eq!(
            handle_with_input(dir.path(), "{}", None),
            HookOutput::Silent
        );

        let path = dir.path().join(".gwt/discussion.md");
        let unreadable = b"\xff\xfe";
        fs::write(&path, unreadable).unwrap();
        assert_eq!(
            discussion_stop_blocker(dir.path(), Some("sess-1"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        let output = handle_with_input(dir.path(), "{}", Some("sess-1"));
        let HookOutput::SystemMessage(message) = output else {
            panic!("unreadable discussion must warn without blocking Stop: {output:?}");
        };
        assert!(message.contains("Discussion"), "{message}");
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
            panic!("public Stop must retain the warning: {output:?}");
        };
        assert!(public_message.contains(&message), "{public_message}");
        assert_eq!(fs::read(&path).unwrap(), unreadable);
    }
}
