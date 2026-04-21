//! `gwt hook skill-discussion-stop-check` — Stop-block handler for the
//! `gwt-discussion` skill (SPEC-1935 Phase 10, FR-014p).
//!
//! Reads `.gwt/discussion.md` in the current worktree and, when an
//! `[active]` proposal with a non-empty `Next Question:` line is found,
//! returns `HookOutput::StopBlock` so Claude Code / Codex continue the
//! agent instead of stopping. Claude Code's built-in `stop_hook_active`
//! flag (FR-014o) short-circuits this handler to prevent infinite loops.
//!
//! Fail-open policy (FR-014u): any I/O or parse failure resolves to
//! `HookOutput::Silent` so a corrupted state file never accidentally
//! blocks a Stop.

use std::{
    io::{self, Read},
    path::Path,
};

use super::{envelope::stop_hook_active_from, HookError, HookOutput};
use crate::discussion_resume::load_pending_resume;

pub fn handle() -> Result<HookOutput, HookError> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let cwd = std::env::current_dir()?;
    Ok(handle_with_input(&cwd, &input))
}

/// Pure core decision. Always returns `Silent` on any parse/IO failure
/// so the Stop hook stays fail-open.
pub fn handle_with_input(worktree: &Path, input: &str) -> HookOutput {
    if stop_hook_active_from(input) {
        return HookOutput::Silent;
    }
    let Ok(Some(pending)) = load_pending_resume(worktree) else {
        return HookOutput::Silent;
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
         Next question: {question}\n\
         Continue the gwt-discussion workflow (investigate → ask the user → update Discussion TODO), \
         or call `gwt discuss resolve|park|reject --proposal {label}` to exit the discussion explicitly.",
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
- Next Question:\n\
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
        let output = handle_with_input(dir.path(), "{}");
        assert_stop_block(
            output,
            &[
                "Hook-driven resume",
                "Should SessionStart or UserPromptSubmit surface the resume proposal?",
                "gwt discuss resolve",
                "Proposal A",
            ],
        );
    }

    #[test]
    fn silent_when_stop_hook_active_flag_is_true() {
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITH_QUESTION);
        assert_eq!(
            handle_with_input(dir.path(), r#"{"stop_hook_active":true}"#),
            HookOutput::Silent
        );
    }

    #[test]
    fn silent_when_discussion_file_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(handle_with_input(dir.path(), "{}"), HookOutput::Silent);
    }

    #[test]
    fn silent_when_active_proposal_has_empty_next_question() {
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ACTIVE_WITHOUT_QUESTION);
        assert_eq!(handle_with_input(dir.path(), "{}"), HookOutput::Silent);
    }

    #[test]
    fn silent_when_no_active_proposals_remain() {
        let dir = tempfile::tempdir().unwrap();
        write_discussion(dir.path(), ALL_RESOLVED);
        assert_eq!(handle_with_input(dir.path(), "{}"), HookOutput::Silent);
    }

    #[test]
    fn silent_when_discussion_md_is_malformed() {
        let dir = tempfile::tempdir().unwrap();
        // `parse_proposals` is tolerant, but this test future-proofs the
        // fail-open contract: any unparseable input must not block Stop.
        write_discussion(
            dir.path(),
            "### Proposal ??? broken header without status label\n",
        );
        assert_eq!(handle_with_input(dir.path(), "{}"), HookOutput::Silent);
    }
}
