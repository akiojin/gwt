//! `gwt hook block-file-ops` — PreToolUse hook that forbids file-system
//! mutations outside the current worktree.
//!
//! Ported from the retired external file-op policy hook.

use std::path::{Path, PathBuf};

use super::{segments::split_command_segments, BlockDecision, HookError, HookEvent};

const FILE_OPS: &[&str] = &["mkdir", "rmdir", "rm", "touch", "cp", "mv"];

pub fn evaluate_bash_command(command: &str, worktree_root: &Path) -> Option<BlockDecision> {
    for segment in split_command_segments(command) {
        let Some(op) = segment_starts_with_file_op(&segment) else {
            continue;
        };
        for file_path in extract_file_paths(&segment) {
            if file_path.is_empty() {
                continue;
            }
            if !is_within_worktree(&file_path, worktree_root) {
                return Some(BlockDecision::new(
                    "\u{1F6AB} File operations outside worktree are not allowed",
                    format!(
                        "Worktree is designed to complete work within the launched directory. \
                         File operations outside the worktree cannot be executed.\n\nWorktree \
                         root: {root}\nTarget path: {target}\nBlocked command: {command}\n\n\
                         Instead, use absolute paths within worktree, e.g., 'mkdir ./new-dir' or \
                         'rm ./file.txt'",
                        root = worktree_root.display(),
                        target = file_path,
                    ),
                ));
            }
            let _ = op;
        }
    }
    None
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

fn segment_starts_with_file_op(segment: &str) -> Option<&'static str> {
    let first = segment.split_whitespace().next()?;
    FILE_OPS.iter().copied().find(|&op| first == op)
}

/// Mirror the Node `extractFilePaths` helper: every whitespace-separated
/// token after the command name that does not start with `-`.
fn extract_file_paths(segment: &str) -> Vec<String> {
    segment
        .split_whitespace()
        .skip(1)
        .filter(|t| !t.starts_with('-'))
        .map(|t| t.to_string())
        .collect()
}

fn is_within_worktree(target: &str, worktree_root: &Path) -> bool {
    if target.is_empty() || target == "~" {
        return false;
    }
    let target_path = PathBuf::from(target);
    let abs = if target_path.is_absolute() {
        target_path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(target_path)
    };
    let abs = normalize(&abs);
    let root = normalize(worktree_root);
    abs == root || abs.starts_with(&root)
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        use std::path::Component::*;
        match comp {
            ParentDir => {
                out.pop();
            }
            CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
