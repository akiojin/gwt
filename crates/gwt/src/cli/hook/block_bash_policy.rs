//! `gwtd hook block-bash-policy` — consolidated PreToolUse Bash policy hook.
//!
//! Evaluates the existing Bash safety rules in a fixed order and returns the
//! first blocking decision, if any.

use std::{io::Read, path::Path};

use super::{
    block_cd_command, block_file_ops, block_git_branch_ops, block_git_dir_override, HookError,
    HookEvent, HookOutput,
};

pub fn evaluate_bash_command(command: &str, worktree_root: &Path) -> Option<HookOutput> {
    block_git_branch_ops::evaluate_bash_command(command)
        .or_else(|| block_cd_command::evaluate_bash_command(command, worktree_root))
        .or_else(|| block_file_ops::evaluate_bash_command(command, worktree_root))
        .or_else(|| block_git_dir_override::evaluate_bash_command(command))
        .or_else(|| evaluate_long_pr_ci_polling_sleep(command))
        .or_else(|| evaluate_github_workflow_cli(command))
}

pub fn evaluate(event: &HookEvent, worktree_root: &Path) -> Result<HookOutput, HookError> {
    if event.tool_name.as_deref() != Some("Bash") {
        return Ok(HookOutput::Silent);
    }
    let Some(command) = event.command() else {
        return Ok(HookOutput::Silent);
    };
    Ok(evaluate_bash_command(command, worktree_root).unwrap_or(HookOutput::Silent))
}

pub fn handle() -> Result<HookOutput, HookError> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    handle_with_input(&input)
}

pub fn handle_with_input(input: &str) -> Result<HookOutput, HookError> {
    let Some(event) = HookEvent::read_from_str(input)? else {
        return Ok(HookOutput::Silent);
    };
    let root = crate::cli::hook::worktree::detect_worktree_root();
    evaluate(&event, &root)
}

fn evaluate_github_workflow_cli(command: &str) -> Option<HookOutput> {
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
                "auth" | "repo" | "release" => continue,
                "issue" if is_blocked_issue_subcommand(tokens.get(2).copied()) => {
                    return Some(github_workflow_block_decision(command));
                }
                "pr" if is_blocked_pr_subcommand(tokens.get(2).copied()) => {
                    return Some(github_workflow_block_decision(command));
                }
                "run" if is_blocked_run_subcommand(tokens.get(2).copied()) => {
                    return Some(github_workflow_block_decision(command));
                }
                "api" if is_workflow_api_command(&segment, &tokens) => {
                    return Some(github_workflow_block_decision(command));
                }
                _ => {}
            }
        }
    }
    None
}

fn evaluate_long_pr_ci_polling_sleep(command: &str) -> Option<HookOutput> {
    let segments = super::segments::split_command_segments(command);
    let has_pr_ci_polling = segments
        .iter()
        .any(|segment| is_pr_ci_polling_segment(segment));
    if !has_pr_ci_polling {
        return None;
    }

    for segment in &segments {
        let tokens = command_tokens(segment);
        if is_long_sleep_segment(&tokens) {
            return Some(long_pr_ci_polling_sleep_block_decision(command));
        }
    }
    None
}

fn is_long_sleep_segment(tokens: &[&str]) -> bool {
    let Some(command_name) = tokens.first().copied() else {
        return false;
    };
    if normalize_command_name(command_name) != "sleep" {
        return false;
    }

    parse_sleep_args_seconds(&tokens[1..]).is_some_and(|seconds| seconds >= 120.0)
}

fn parse_sleep_args_seconds(args: &[&str]) -> Option<f64> {
    let mut total = 0.0;
    let mut parsed_any = false;
    for arg in args {
        let seconds = parse_sleep_duration_seconds(arg)?;
        total += seconds;
        parsed_any = true;
    }
    parsed_any.then_some(total)
}

fn parse_sleep_duration_seconds(duration: &str) -> Option<f64> {
    let duration = duration.trim_matches(|ch| ch == '\'' || ch == '"');
    let (numeric, multiplier) = match duration.chars().last() {
        Some('s') => (&duration[..duration.len() - 1], 1.0),
        Some('m') => (&duration[..duration.len() - 1], 60.0),
        Some('h') => (&duration[..duration.len() - 1], 60.0 * 60.0),
        Some('d') => (&duration[..duration.len() - 1], 24.0 * 60.0 * 60.0),
        _ => (duration, 1.0),
    };
    let value: f64 = numeric.parse().ok()?;
    if value.is_sign_negative() {
        return None;
    }
    Some(value * multiplier)
}

fn is_pr_ci_polling_segment(segment: &str) -> bool {
    let tokens = command_tokens(segment);
    let Some(command_name) = tokens.first().copied().map(normalize_command_name) else {
        return false;
    };

    match command_name.as_str() {
        "gwtd" => matches!(tokens.get(1).copied(), Some("pr" | "actions")),
        "gh" => tokens
            .get(1)
            .copied()
            .is_some_and(|subcommand| matches!(subcommand, "pr" | "run" | "api")),
        _ => false,
    }
}

fn normalize_command_name(token: &str) -> String {
    let token = token.trim_matches(|ch| ch == '\'' || ch == '"');
    Path::new(token)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(token)
        .to_string()
}

fn long_pr_ci_polling_sleep_block_decision(command: &str) -> HookOutput {
    HookOutput::pre_tool_use_permission(
        "Long PR/CI polling sleeps are not allowed",
        format!(
            "Do not keep Claude Code idle while waiting for PR or CI state changes.\n\n\
Run `gwtd pr checks <number>` once. If checks are still pending or queued, post the wait state with \
`gwtd board post --kind blocked --body '<what is pending and how to resume>'`, then hand off instead of sleeping indefinitely.\n\n\
Blocked command: {command}"
        ),
    )
}

fn is_blocked_issue_subcommand(subcommand: Option<&str>) -> bool {
    matches!(subcommand, Some("view" | "create" | "comment"))
}

fn is_blocked_pr_subcommand(subcommand: Option<&str>) -> bool {
    matches!(
        subcommand,
        Some("view" | "create" | "edit" | "comment" | "checks" | "reviews" | "review-threads")
    )
}

fn is_blocked_run_subcommand(subcommand: Option<&str>) -> bool {
    matches!(subcommand, Some("view"))
}

fn github_workflow_block_decision(command: &str) -> HookOutput {
    HookOutput::pre_tool_use_permission(
        "\u{1F6AB} Direct GitHub workflow CLI commands are not allowed",
        format!(
            "Use the gwt workflow surfaces instead of direct `gh issue`, `gh pr`, `gh run`, or workflow-focused `gh api` commands.\n\n\
Recommended alternatives:\n\
- read: `gwtd issue view <number>`, `gwtd issue comments <number>`, `gwtd issue linked-prs <number>`\n\
- write: `gwtd issue create --title ... -f <file>`, `gwtd issue comment <number> -f <file>`\n\
- PR workflow: `gwtd pr current`, `gwtd pr view <number>`, `gwtd pr comment <number> -f <file>`, `gwtd pr checks <number>`\n\
- PR reviews: `gwtd pr reviews <number>`, `gwtd pr review-threads <number>`, `gwtd pr review-threads reply-and-resolve <number> -f <file>`\n\
- Actions logs: `gwtd actions logs --run <id>`, `gwtd actions job-logs --job <id>`\n\
- discovery: `gwt-search`, `~/.gwt/cache/issues/<repo-hash>/`\n\n\
Blocked command: {command}"
        ),
    )
}

fn command_tokens(segment: &str) -> Vec<&str> {
    let raw: Vec<&str> = segment.split_whitespace().collect();
    let mut start = 0;

    while raw
        .get(start)
        .is_some_and(|token| matches!(*token, "do" | "then"))
    {
        start += 1;
    }

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

fn is_workflow_api_command(segment: &str, tokens: &[&str]) -> bool {
    let Some(target) = gh_api_target(tokens) else {
        return false;
    };

    if target == "graphql" {
        let lowered = segment.to_ascii_lowercase();
        return lowered.contains("issue(")
            || lowered.contains("issues(")
            || lowered.contains("updateissue")
            || lowered.contains("closeissue")
            || lowered.contains("reopenissue")
            || lowered.contains("pullrequest(")
            || lowered.contains("pullrequests(")
            || lowered.contains("reviews(")
            || lowered.contains("reviewthreads")
            || lowered.contains("workflowrun")
            || lowered.contains("workflowruns")
            || lowered.contains("checkrun")
            || lowered.contains("checkruns")
            || lowered.contains("checksuite")
            || lowered.contains("checksuites");
    }

    let lowered = target.to_ascii_lowercase();
    lowered.contains("/issues")
        || lowered.contains("/pulls")
        || lowered.contains("/actions/runs")
        || lowered.contains("/actions/jobs")
        || lowered.contains("/check-runs")
        || lowered.contains("/check-suites")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_long_sleep_before_gwtd_pr_polling() {
        let decision = evaluate_bash_command(
            "sleep 280 && /Applications/GWT.app/Contents/MacOS/gwtd pr view 123",
            Path::new("/worktree"),
        )
        .expect("expected long PR polling sleep to be blocked");

        assert_eq!(
            decision.summary(),
            "Long PR/CI polling sleeps are not allowed"
        );
        assert!(decision.detail().contains("gwtd pr checks <number>"));
    }

    #[test]
    fn blocks_long_sleep_before_gh_run_polling() {
        let decision = evaluate_bash_command(
            "sleep 280 && gh run view 123456 --log",
            Path::new("/worktree"),
        )
        .expect("expected long GitHub Actions polling sleep to be blocked");

        assert_eq!(
            decision.summary(),
            "Long PR/CI polling sleeps are not allowed"
        );
    }

    #[test]
    fn allows_short_sleep_before_gwtd_pr_check() {
        let decision =
            evaluate_bash_command("sleep 30 && gwtd pr checks 123", Path::new("/worktree"));

        assert!(decision.is_none());
    }
}
