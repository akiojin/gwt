//! `gwt hook block-git-dir-override` — PreToolUse hook that forbids
//! overriding `GIT_DIR` or `GIT_WORK_TREE` via the Bash command the
//! agent is about to run.
//!
//! Ported from the retired external git-dir policy hook.

use std::sync::OnceLock;

use regex::Regex;

use super::{BlockDecision, HookError, HookEvent};

pub fn evaluate_bash_command(command: &str) -> Option<BlockDecision> {
    if re_git_dir().is_match(command) {
        return Some(BlockDecision::new(
            "\u{1F6AB} GIT_DIR environment variable override is not allowed",
            format!(
                "Modifying GIT_DIR in a worktree environment can cause unintended repository \
                 operations.\n\nBlocked command: {command}\n\nWorktrees have their own .git file \
                 pointing to the main repository worktree directory. Overriding GIT_DIR may \
                 break this relationship and cause git commands to operate on the wrong \
                 repository."
            ),
        ));
    }
    if re_git_work_tree().is_match(command) {
        return Some(BlockDecision::new(
            "\u{1F6AB} GIT_WORK_TREE environment variable override is not allowed",
            format!(
                "Modifying GIT_WORK_TREE in a worktree environment can cause unintended \
                 repository operations.\n\nBlocked command: {command}\n\nWorktrees have their \
                 own working directory configuration. Overriding GIT_WORK_TREE may cause git \
                 commands to operate on the wrong directory."
            ),
        ));
    }
    None
}

pub fn evaluate(event: &HookEvent) -> Result<Option<BlockDecision>, HookError> {
    if event.tool_name.as_deref() != Some("Bash") {
        return Ok(None);
    }
    let Some(command) = event.command() else {
        return Ok(None);
    };
    Ok(evaluate_bash_command(command))
}

pub fn handle() -> Result<Option<BlockDecision>, HookError> {
    let Some(event) = HookEvent::read_from_stdin()? else {
        return Ok(None);
    };
    evaluate(&event)
}

fn re_git_dir() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| {
        Regex::new(
            r"(^|[;&|]|\s)(export\s+)?GIT_DIR\s*=|env\s+[^;]*GIT_DIR\s*=|declare\s+-x\s+GIT_DIR\s*=",
        )
        .unwrap()
    })
}

fn re_git_work_tree() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| {
        Regex::new(
            r"(^|[;&|]|\s)(export\s+)?GIT_WORK_TREE\s*=|env\s+[^;]*GIT_WORK_TREE\s*=|declare\s+-x\s+GIT_WORK_TREE\s*=",
        )
        .unwrap()
    })
}
