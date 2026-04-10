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
        .or_else(|| evaluate_github_issue_cli(command))
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

fn evaluate_github_issue_cli(command: &str) -> Option<BlockDecision> {
    for segment in super::segments::split_command_segments(command) {
        let tokens = command_tokens(&segment);
        let Some(command_name) = tokens.first().copied() else {
            continue;
        };
        if command_name != "gh" {
            continue;
        }

        if let Some(subcommand) = tokens.get(1).copied() {
            match subcommand {
                "issue" if is_blocked_issue_subcommand(tokens.get(2).copied()) => {
                    return Some(github_issue_block_decision(command));
                }
                "api" if is_issue_api_command(&segment, &tokens) => {
                    return Some(github_issue_block_decision(command));
                }
                _ => {}
            }
        }
    }
    None
}

fn is_blocked_issue_subcommand(subcommand: Option<&str>) -> bool {
    matches!(subcommand, Some("view" | "create" | "comment"))
}

fn github_issue_block_decision(command: &str) -> BlockDecision {
    BlockDecision::new(
        "\u{1F6AB} Direct GitHub Issue CLI commands are not allowed",
        format!(
            "Use the gwt Issue surface instead of direct `gh issue` / issue-focused `gh api` commands.\n\n\
Recommended alternatives:\n\
- read: `gwt issue view <number>`, `gwt issue comments <number>`, `gwt issue linked-prs <number>`\n\
- write: `gwt issue create --title ... -f <file>`, `gwt issue comment <number> -f <file>`\n\
- discovery: `gwt-search`, `~/.gwt/cache/issues/`\n\n\
Blocked command: {command}"
        ),
    )
}

fn command_tokens(segment: &str) -> Vec<&str> {
    let raw: Vec<&str> = segment.split_whitespace().collect();
    let mut start = 0;

    if raw.get(start) == Some(&"env") {
        start += 1;
    }

    while start < raw.len() && is_env_assignment(raw[start]) {
        start += 1;
    }

    raw[start..].to_vec()
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_issue_api_command(segment: &str, tokens: &[&str]) -> bool {
    let Some(target) = gh_api_target(tokens) else {
        return false;
    };

    if target == "graphql" {
        let lowered = segment.to_ascii_lowercase();
        return lowered.contains("issue(")
            || lowered.contains("issues(")
            || lowered.contains("updateissue")
            || lowered.contains("closeissue")
            || lowered.contains("reopenissue");
    }

    let lowered = target.to_ascii_lowercase();
    lowered.contains("/issues")
}

fn gh_api_target<'a>(tokens: &'a [&'a str]) -> Option<&'a str> {
    let mut i = 2;
    while i < tokens.len() {
        let token = tokens[i];
        if !token.starts_with('-') {
            return Some(token);
        }

        let consumes_value = matches!(
            token,
            "-H" | "--header"
                | "-f"
                | "--field"
                | "-F"
                | "--raw-field"
                | "-X"
                | "--method"
                | "--input"
                | "--jq"
                | "-q"
                | "--hostname"
                | "--cache"
        );
        i += if consumes_value { 2 } else { 1 };
    }
    None
}
