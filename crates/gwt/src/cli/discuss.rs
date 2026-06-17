//! `discuss.*` JSON exit operations.
//!
//! Exit CLI for the `gwt-discussion` skill (SPEC-1935 FR-014p). The LLM
//! invokes these operations to mutate `.gwt/work/discussions.md` so the
//! `skill-discussion-stop-check` handler stops blocking Stop events. Legacy
//! `.gwt/discussion.md` is read only as a fallback and canonicalized on
//! mutation.
//!
//! All operations are idempotent: calling `resolve` on an already-resolved
//! proposal, or targeting a missing label, exits successfully with a
//! short informational message. This matches the "LLM 忘れ漏れ耐性"
//! design note.

use gwt_github::SpecOpsError;

use crate::cli::{CliEnv, CliParseError};
use crate::discussion_resume::{
    clear_proposal_next_question, proposal_evidence_blocker_by_label,
    set_proposal_goal_pending_by_label, set_proposal_goal_state_by_label,
    set_proposal_status_by_label,
};

/// Sub-action for `discuss.*` operations (SPEC-1935 FR-014p).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscussAction {
    Resolve {
        proposal: String,
    },
    Park {
        proposal: String,
    },
    Reject {
        proposal: String,
    },
    ClearNextQuestion {
        proposal: String,
    },
    GoalPending {
        proposal: String,
        condition_file: std::path::PathBuf,
    },
    GoalPendingBody {
        proposal: String,
        condition: String,
    },
    GoalStarted {
        proposal: String,
    },
    GoalFailed {
        proposal: String,
        reason: String,
    },
    GoalSkipped {
        proposal: String,
        reason: String,
    },
}

pub(super) fn parse(args: &[String]) -> Result<DiscussAction, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    let proposal = parse_named_string(rest, "--proposal")?;
    match head.as_str() {
        "resolve" => Ok(DiscussAction::Resolve { proposal }),
        "park" => Ok(DiscussAction::Park { proposal }),
        "reject" => Ok(DiscussAction::Reject { proposal }),
        "clear-next-question" => Ok(DiscussAction::ClearNextQuestion { proposal }),
        "goal-pending" => Ok(DiscussAction::GoalPending {
            proposal,
            condition_file: parse_named_path(rest, "-f")?,
        }),
        "goal-started" => Ok(DiscussAction::GoalStarted { proposal }),
        "goal-failed" => Ok(DiscussAction::GoalFailed {
            proposal,
            reason: parse_named_string(rest, "--reason")?,
        }),
        "goal-skipped" => Ok(DiscussAction::GoalSkipped {
            proposal,
            reason: parse_named_string(rest, "--reason")?,
        }),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn parse_named_string(args: &[String], flag: &'static str) -> Result<String, CliParseError> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag {
            let value = args.get(i + 1).ok_or(CliParseError::MissingFlag(flag))?;
            return Ok(value.clone());
        }
        i += 1;
    }
    Err(CliParseError::MissingFlag(flag))
}

fn parse_named_path(
    args: &[String],
    flag: &'static str,
) -> Result<std::path::PathBuf, CliParseError> {
    parse_named_string(args, flag).map(std::path::PathBuf::from)
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    action: DiscussAction,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let worktree = env.repo_path().to_path_buf();
    match action {
        DiscussAction::Resolve { proposal } => {
            match proposal_evidence_blocker_by_label(&worktree, &proposal) {
                Ok(Some(reason)) => {
                    out.push_str(&format!(
                        "discuss: cannot resolve {proposal}; Evidence Gate incomplete: {reason}\n"
                    ));
                    Ok(2)
                }
                Ok(None) => apply_status(&worktree, &proposal, "chosen", out),
                Err(err) => {
                    out.push_str(&format!("discuss: evidence gate check failed: {err}\n"));
                    Ok(1)
                }
            }
        }
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
        DiscussAction::GoalPending {
            proposal,
            condition_file,
        } => {
            let condition = match std::fs::read_to_string(&condition_file) {
                Ok(condition) => condition,
                Err(err) => {
                    out.push_str(&format!(
                        "discuss: failed to read goal condition file {}: {err}\n",
                        condition_file.display()
                    ));
                    return Ok(1);
                }
            };
            apply_goal_pending(&worktree, &proposal, &condition, out)
        }
        DiscussAction::GoalPendingBody {
            proposal,
            condition,
        } => apply_goal_pending(&worktree, &proposal, &condition, out),
        DiscussAction::GoalStarted { proposal } => {
            apply_goal_state(&worktree, &proposal, "started", out)
        }
        DiscussAction::GoalFailed { proposal, reason } => {
            apply_goal_state(&worktree, &proposal, &format!("failed({reason})"), out)
        }
        DiscussAction::GoalSkipped { proposal, reason } => {
            apply_goal_state(&worktree, &proposal, &format!("skipped({reason})"), out)
        }
    }
}

fn apply_goal_pending(
    worktree: &std::path::Path,
    proposal: &str,
    condition: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match set_proposal_goal_pending_by_label(worktree, proposal, condition) {
        Ok(true) => {
            out.push_str(&format!("discuss: {proposal} goal -> pending\n"));
            Ok(0)
        }
        Ok(false) => {
            out.push_str(&format!(
                "discuss: {proposal} is not eligible for goal pending (no change)\n"
            ));
            Ok(0)
        }
        Err(err) => {
            out.push_str(&format!("discuss: goal pending failed: {err}\n"));
            Ok(1)
        }
    }
}

fn apply_goal_state(
    worktree: &std::path::Path,
    proposal: &str,
    state: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match set_proposal_goal_state_by_label(worktree, proposal, state) {
        Ok(true) => {
            out.push_str(&format!("discuss: {proposal} goal -> {state}\n"));
            Ok(0)
        }
        Ok(false) => {
            out.push_str(&format!(
                "discuss: {proposal} is not eligible for goal state update (no change)\n"
            ));
            Ok(1)
        }
        Err(err) => {
            out.push_str(&format!("discuss: goal state update failed: {err}\n"));
            Ok(1)
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
