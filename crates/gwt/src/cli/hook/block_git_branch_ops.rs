//! `gwt hook block-git-branch-ops` — PreToolUse hook that forbids
//! branch-altering git commands inside a worktree.
//!
//! Ported 1:1 from the retired external branch-policy hook.
//! The pure evaluation logic ([`evaluate_bash_command`]) takes a raw
//! Bash command string and returns `Some(BlockDecision)` when the hook
//! must veto the tool call, or `None` when it should allow. The CLI
//! wrapper ([`handle`]) reads the stdin event, extracts the command,
//! and delegates.

use std::sync::OnceLock;

use regex::Regex;

use super::segments::split_command_segments;
use super::{BlockDecision, HookError, HookEvent};

/// Evaluate a single raw command string. Returns `Some` if any segment
/// triggers a block rule, `None` if every segment is allowed.
pub fn evaluate_bash_command(command: &str) -> Option<BlockDecision> {
    for segment in split_command_segments(command) {
        if let Some(decision) = evaluate_segment(&segment, command) {
            return Some(decision);
        }
    }
    None
}

/// Wire-up from a parsed [`HookEvent`] stdin payload. Non-Bash tool calls
/// are unconditionally allowed.
pub fn evaluate(event: &HookEvent) -> Result<Option<BlockDecision>, HookError> {
    if event.tool_name.as_deref() != Some("Bash") {
        return Ok(None);
    }
    let Some(command) = event.command() else {
        return Ok(None);
    };
    Ok(evaluate_bash_command(command))
}

/// Production entry point. Reads the event from stdin and evaluates it.
pub fn handle() -> Result<Option<BlockDecision>, HookError> {
    let Some(event) = HookEvent::read_from_stdin()? else {
        return Ok(None);
    };
    evaluate(&event)
}

// ---------------------------------------------------------------------------
// Segment-level rules
// ---------------------------------------------------------------------------

fn evaluate_segment(segment: &str, original: &str) -> Option<BlockDecision> {
    // Rule 1: interactive rebase against origin/main.
    if starts_with_git_rebase(segment)
        && has_interactive_flag(segment)
        && targets_origin_main(segment)
    {
        return Some(BlockDecision::new(
            "\u{1F6AB} Interactive rebase against origin/main is not allowed",
            format!(
                "Interactive rebase against origin/main initiated by LLMs is blocked because it \
                 frequently fails and disrupts sessions.\n\nBlocked command: {original}"
            ),
        ));
    }

    // Everything below only applies to segments that begin with `git`.
    if !starts_with_git(segment) {
        return None;
    }

    // Rule 2: checkout / switch — block branch switching, allow
    // explicit file-level operations with `-- <file>`.
    if mentions_checkout_or_switch(segment) && !is_file_level_checkout(segment) {
        return Some(BlockDecision::new(
            "\u{1F6AB} Branch switching commands (checkout/switch) are not allowed",
            format!(
                "Worktree is designed to complete work on the launched branch. Branch operations \
                 such as git checkout and git switch cannot be executed.\n\nBlocked command: \
                 {original}"
            ),
        ));
    }

    // Rule 3: git branch — read-only forms OK, anything else blocked.
    if let Some(branch_args) = match_git_branch_subcommand(segment) {
        if !is_read_only_git_branch(branch_args) {
            return Some(BlockDecision::new(
                "\u{1F6AB} Branch modification commands are not allowed",
                format!(
                    "Worktree is designed to complete work on the launched branch. Destructive \
                     branch operations such as git branch -d, git branch -m cannot be \
                     executed.\n\nBlocked command: {original}"
                ),
            ));
        }
        // Read-only branch form: fall through to the next segment check.
        return None;
    }

    // Rule 4: git worktree — always blocked.
    if matches_git_worktree_subcommand(segment) {
        return Some(BlockDecision::new(
            "\u{1F6AB} Worktree commands are not allowed",
            format!(
                "Worktree management operations such as git worktree add/remove cannot be \
                 executed from within a worktree.\n\nBlocked command: {original}"
            ),
        ));
    }

    None
}

// ---------------------------------------------------------------------------
// Compiled regex cache — each pattern is compiled exactly once per process.
// ---------------------------------------------------------------------------

fn re_git_rebase() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| Regex::new(r"^git\s+rebase\b").unwrap())
}

fn re_interactive_flag() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| Regex::new(r"(?:^|\s)(-i|--interactive)(?:\s|$)").unwrap())
}

fn re_origin_main() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| Regex::new(r"(?:^|\s)origin/main(?:\s|$)").unwrap())
}

fn re_checkout_explicit_sep() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| Regex::new(r"\bcheckout\b.*\s--\s").unwrap())
}

fn re_checkout_conflict_flag() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| Regex::new(r"\bcheckout\b.*\s--(theirs|ours)\b").unwrap())
}

fn re_checkout_broad_target() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| Regex::new(r"\s--\s+[.*]").unwrap())
}

fn re_git_branch_subcommand() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| Regex::new(r"^git\s+(?:(?:-[a-zA-Z]|--[a-z-]+)\s+)*branch\b(.*)").unwrap())
}

fn re_git_worktree_subcommand() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| Regex::new(r"^git\s+(?:(?:-[a-zA-Z]|--[a-z-]+)\s+)*worktree\b").unwrap())
}

fn re_read_only_branch_flag() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| {
        Regex::new(
            r"^(--list|--show-current|--all|-a|--remotes|-r|--contains|--merged|--no-merged|--points-at|--format|--sort|--abbrev|-v|-vv|--verbose)",
        )
        .unwrap()
    })
}

// ---------------------------------------------------------------------------
// Thin predicates — all regex work is cached above.
// ---------------------------------------------------------------------------

fn starts_with_git(segment: &str) -> bool {
    // `\bgit\b` but anchored: Node uses `^git\b`, we replicate that.
    segment == "git" || segment.starts_with("git ") || segment.starts_with("git\t")
}

fn starts_with_git_rebase(segment: &str) -> bool {
    re_git_rebase().is_match(segment)
}

fn has_interactive_flag(segment: &str) -> bool {
    re_interactive_flag().is_match(segment)
}

fn targets_origin_main(segment: &str) -> bool {
    re_origin_main().is_match(segment)
}

fn mentions_checkout_or_switch(segment: &str) -> bool {
    // Mirrors `/\b(checkout|switch)\b/`.
    word_present(segment, "checkout") || word_present(segment, "switch")
}

fn word_present(haystack: &str, word: &str) -> bool {
    // Cheap `\b<word>\b` substitute — avoids compiling a regex per call.
    let bytes = haystack.as_bytes();
    let wbytes = word.as_bytes();
    if wbytes.is_empty() || bytes.len() < wbytes.len() {
        return false;
    }
    let mut i = 0;
    while i + wbytes.len() <= bytes.len() {
        if &bytes[i..i + wbytes.len()] == wbytes {
            let left_ok = i == 0 || !is_word_char(bytes[i - 1]);
            let right_idx = i + wbytes.len();
            let right_ok = right_idx == bytes.len() || !is_word_char(bytes[right_idx]);
            if left_ok && right_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_file_level_checkout(segment: &str) -> bool {
    let has_conflict = re_checkout_conflict_flag().is_match(segment);
    let has_sep = re_checkout_explicit_sep().is_match(segment);
    let has_broad = re_checkout_broad_target().is_match(segment);
    (has_conflict || has_sep) && !has_broad
}

fn match_git_branch_subcommand(segment: &str) -> Option<&str> {
    re_git_branch_subcommand()
        .captures(segment)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
}

fn matches_git_worktree_subcommand(segment: &str) -> bool {
    re_git_worktree_subcommand().is_match(segment)
}

fn is_read_only_git_branch(args: &str) -> bool {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return true;
    }
    re_read_only_branch_flag().is_match(trimmed)
}
