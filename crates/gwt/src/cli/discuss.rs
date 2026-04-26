//! `gwtd discuss <resolve|park|reject|clear-next-question> --proposal <label>`
//!
//! Exit CLI for the `gwt-discussion` skill (SPEC-1935 FR-014p). The LLM
//! invokes these commands to mutate `.gwt/discussion.md` so the
//! `skill-discussion-stop-check` handler stops blocking Stop events.
//!
//! All commands are idempotent: calling `resolve` on an already-resolved
//! proposal, or targeting a missing label, exits successfully with a
//! short informational message. This matches the "LLM 忘れ漏れ耐性"
//! design note.

use gwt_github::SpecOpsError;

use crate::cli::{CliEnv, DiscussAction};
use crate::discussion_resume::{clear_proposal_next_question, set_proposal_status_by_label};

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    action: DiscussAction,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let worktree = env.repo_path().to_path_buf();
    match action {
        DiscussAction::Resolve { proposal } => apply_status(&worktree, &proposal, "chosen", out),
        DiscussAction::Park { proposal } => apply_status(&worktree, &proposal, "parked", out),
        DiscussAction::Reject { proposal } => apply_status(&worktree, &proposal, "rejected", out),
        DiscussAction::ClearNextQuestion { proposal } => {
            match clear_proposal_next_question(&worktree, &proposal) {
                Ok(true) => {
                    out.push_str(&format!("discuss: cleared Next Question for {proposal}\n"));
                    Ok(0)
                }
                Ok(false) => {
                    out.push_str(&format!(
                        "discuss: no active Next Question to clear for {proposal}\n"
                    ));
                    Ok(0)
                }
                Err(err) => {
                    out.push_str(&format!("discuss: clear-next-question failed: {err}\n"));
                    Ok(1)
                }
            }
        }
    }
}

fn apply_status(
    worktree: &std::path::Path,
    proposal: &str,
    new_status: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match set_proposal_status_by_label(worktree, proposal, new_status) {
        Ok(true) => {
            out.push_str(&format!("discuss: {proposal} -> [{new_status}]\n"));
            Ok(0)
        }
        Ok(false) => {
            out.push_str(&format!(
                "discuss: {proposal} is already resolved or not found (no change)\n"
            ));
            Ok(0)
        }
        Err(err) => {
            out.push_str(&format!("discuss: {new_status} failed: {err}\n"));
            Ok(1)
        }
    }
}
