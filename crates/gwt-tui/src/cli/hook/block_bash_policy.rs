//! `gwt hook block-bash-policy` — consolidated PreToolUse Bash policy hook.
//!
//! Evaluates the existing Bash safety rules in a fixed order and returns the
//! first blocking decision, if any.

use std::path::Path;

use super::{
    block_cd_command, block_file_ops, block_git_branch_ops, block_git_dir_override, BlockDecision,
    HookError, HookEvent,
};

pub fn evaluate_bash_command(command: &str, worktree_root: &Path) -> Option<BlockDecision> {
    block_git_branch_ops::evaluate_bash_command(command)
        .or_else(|| block_cd_command::evaluate_bash_command(command, worktree_root))
        .or_else(|| block_file_ops::evaluate_bash_command(command, worktree_root))
        .or_else(|| block_git_dir_override::evaluate_bash_command(command))
}

pub fn evaluate(
    event: &HookEvent,
    worktree_root: &Path,
) -> Result<Option<BlockDecision>, HookError> {
    if event.tool_name.as_deref() != Some("Bash") {
        return Ok(None);
    }
    let Some(command) = event.command() else {
        return Ok(None);
    };
    Ok(evaluate_bash_command(command, worktree_root))
}

pub fn handle() -> Result<Option<BlockDecision>, HookError> {
    let Some(event) = HookEvent::read_from_stdin()? else {
        return Ok(None);
    };
    let root = crate::cli::hook::worktree::detect_worktree_root();
    evaluate(&event, &root)
}
