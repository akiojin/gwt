//! `gwt hook block-cd-command` — PreToolUse hook that forbids `cd` into
//! paths outside the current worktree root.
//!
//! Ported from the retired external `cd` policy hook.

use std::path::{Path, PathBuf};

use super::segments::split_command_segments;
use super::{BlockDecision, HookError, HookEvent};

/// Pure evaluation: given a raw Bash command and the worktree root,
/// return `Some(BlockDecision)` if any segment `cd`s outside the root.
pub fn evaluate_bash_command(command: &str, worktree_root: &Path) -> Option<BlockDecision> {
    for segment in split_command_segments(command) {
        if let Some(target) = extract_cd_target(&segment) {
            if !is_within_worktree(&target, worktree_root) {
                return Some(BlockDecision::new(
                    "\u{1F6AB} cd command outside worktree is not allowed",
                    format!(
                        "Worktree is designed to complete work within the launched directory. \
                         Directory navigation outside the worktree using cd command cannot be \
                         executed.\n\nWorktree root: {root}\nTarget path: {target}\nBlocked \
                         command: {command}\n\nInstead, use absolute paths to execute commands, \
                         e.g., 'git -C /path/to/repo status' or '/path/to/script.sh'",
                        root = worktree_root.display(),
                        target = target,
                    ),
                ));
            }
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

/// Extract the bare target argument from a `cd` invocation. Returns
/// `None` for non-`cd` segments and for `cd` with no argument (which
/// Bash treats as `cd $HOME` — always outside the worktree, but the
/// Node helper leaves that to the path comparison below).
fn extract_cd_target(segment: &str) -> Option<String> {
    // Mirrors `^(?:builtin\s+)?(?:command\s+)?cd\b\s*(.*)`.
    let s = segment.trim_start();
    let s = strip_prefix_word(s, "builtin ").unwrap_or(s);
    let s = strip_prefix_word(s, "command ").unwrap_or(s);

    let rest = if let Some(rest) = s.strip_prefix("cd ") {
        rest
    } else if s == "cd" {
        ""
    } else {
        return None;
    };

    // Take the first whitespace-separated token after `cd`.
    let first = rest.split_whitespace().next().unwrap_or("").to_string();
    Some(first)
}

fn strip_prefix_word<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    s.strip_prefix(prefix)
}

/// Match the legacy `isWithinWorktree` helper semantics.
///
/// - empty target or `~` → NOT within (force block, mirroring Node)
/// - absolute target → compare directly
/// - relative target → resolve against the current process cwd, just
///   like Node's `path.resolve`
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

    // `Path::starts_with` in Rust is component-aware: `/foo/bar`
    // starts_with `/foo` is true but `/foobar` starts_with `/foo` is
    // false. That matches the `normalizedTarget.startsWith(root + sep)`
    // Node behaviour and gives us a one-liner.
    abs == root || abs.starts_with(&root)
}

/// Lexical path normalization (`.` and `..` removed without touching the
/// filesystem). We intentionally do NOT canonicalize via `fs::canonicalize`
/// because the target may not exist at hook-evaluation time.
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
