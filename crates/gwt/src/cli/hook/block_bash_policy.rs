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
        .or_else(|| evaluate_github_mutation_sinks(command))
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
Run JSON operation `pr.checks` with `params.number:<pr>` once. If checks are still pending or queued, post the wait state with \
JSON operation `board.post` and `params.kind:\"blocked\"` plus `params.body`, then hand off instead of sleeping indefinitely.\n\n\
Blocked command: {command}"
        ),
    )
}

fn is_blocked_issue_subcommand(subcommand: Option<&str>) -> bool {
    matches!(subcommand, Some("view" | "create" | "comment"))
}

fn is_blocked_pr_subcommand(subcommand: Option<&str>) -> bool {
    // "draft" does not exist as a gh subcommand (the real reverse of
    // `gh pr ready` is `gh pr ready --undo`, already covered by "ready");
    // it is intercepted anyway so an agent guessing it by symmetry with the
    // `pr.draft` JSON operation gets redirected instead of a gh usage error.
    matches!(
        subcommand,
        Some(
            "view"
                | "create"
                | "edit"
                | "ready"
                | "draft"
                | "comment"
                | "checks"
                | "reviews"
                | "review-threads"
        )
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
- read: JSON operations `issue.view`, `issue.comments`, `issue.linked_prs`\n\
- write: JSON operations `issue.create`, `issue.comment`\n\
- PR workflow: JSON operations `pr.current`, `pr.view`, `pr.create`, `pr.edit`, `pr.ready`, `pr.draft`, `pr.comment`, `pr.checks`\n\
- PR reviews: JSON operations `pr.reviews`, `pr.review_threads`, `pr.review_threads.reply_and_resolve`\n\
- Actions logs: JSON operations `actions.logs`, `actions.job_logs`\n\
- discovery: `gwt-search`, `~/.gwt/cache/issues/<repo-hash>/`\n\n\
Blocked command: {command}"
        ),
    )
}

/// SPEC-3248 P10 (T-212/T-217 core): method-aware GitHub mutation-sink
/// classification. The workflow-endpoint block above routes issue/PR/run
/// reads to the gwtd operations; this classifier blocks WRITES to the
/// GitHub API regardless of endpoint family — `gh api` with a mutating
/// method (explicit `-X`/`--method` or field-implied POST), `gh release`
/// mutation subcommands, GraphQL mutation documents, and `curl` mutations
/// against a GitHub API host. Read-only calls outside the workflow
/// endpoints stay available. `git push` remains the sanctioned PR handoff
/// path (classified as a mutation sink but policed by the PR gates, not
/// here). Approved wrapper intents and `hub` coverage are T-217 follow-ups.
fn evaluate_github_mutation_sinks(command: &str) -> Option<HookOutput> {
    for segment in super::segments::split_command_segments(command) {
        let tokens = command_tokens(&segment);
        let Some(first) = tokens.first().copied() else {
            continue;
        };
        match normalize_command_name(first).as_str() {
            "gh" => match tokens.get(1).copied() {
                Some("api") if is_mutating_gh_api(&segment, &tokens) => {
                    return Some(github_mutation_block_decision(command));
                }
                Some("release") if is_mutating_release_subcommand(tokens.get(2).copied()) => {
                    return Some(github_mutation_block_decision(command));
                }
                _ => {}
            },
            "curl" if is_mutating_github_curl(&tokens) => {
                return Some(github_mutation_block_decision(command));
            }
            _ => {}
        }
    }
    None
}

/// Normalize a method value for comparison (agents habitually quote it).
fn normalize_method(value: &str) -> String {
    value
        .trim_matches(|ch| ch == '\'' || ch == '"')
        .to_ascii_uppercase()
}

/// True when the GraphQL document in the segment contains a mutation
/// OPERATION — not merely the word "mutation" inside a search string or an
/// introspection field. Heuristic: a word-bounded `mutation` keyword
/// followed (after optional name and variable definitions) by `{`.
fn graphql_contains_mutation_operation(segment: &str) -> bool {
    let lowered = segment.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut search_from = 0;
    while let Some(offset) = lowered[search_from..].find("mutation") {
        let start = search_from + offset;
        let end = start + "mutation".len();
        search_from = end;
        if start > 0 && is_word(bytes[start - 1]) {
            continue;
        }
        if end < bytes.len() && is_word(bytes[end]) {
            continue;
        }
        let mut rest = lowered[end..].trim_start();
        // Optional operation name.
        let name_len = rest
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(rest.len());
        rest = rest[name_len..].trim_start();
        // Optional variable definitions.
        if let Some(stripped) = rest.strip_prefix('(') {
            let Some(close) = stripped.find(')') else {
                continue;
            };
            rest = stripped[close + 1..].trim_start();
        }
        if rest.starts_with('{') {
            return true;
        }
    }
    false
}

/// `gh api` method semantics: explicit `-X`/`--method` wins; otherwise any
/// body field (`-f`/`-F`/`--input`) makes gh POST. GraphQL is mutating when
/// the document contains a mutation operation; a file-based query
/// (`--input`) cannot be classified and is refused fail-closed.
fn is_mutating_gh_api(segment: &str, tokens: &[&str]) -> bool {
    if gh_api_target(tokens) == Some("graphql") {
        let opaque_query = tokens
            .iter()
            .any(|token| *token == "--input" || token.starts_with("--input="));
        return opaque_query || graphql_contains_mutation_operation(segment);
    }
    let mut method: Option<String> = None;
    let mut has_body_field = false;
    let mut i = 2;
    while i < tokens.len() {
        let token = tokens[i];
        match token {
            "-X" | "--method" => {
                method = tokens.get(i + 1).map(|value| normalize_method(value));
                i += 2;
                continue;
            }
            "-f" | "--field" | "-F" | "--raw-field" | "--input" => {
                has_body_field = true;
                i += 2;
                continue;
            }
            _ => {}
        }
        // `--flag=value` and attached short-option spellings (`-XPOST`,
        // `-fkey=value`) must not slip past the classifier.
        if let Some(value) = token.strip_prefix("--method=") {
            method = Some(normalize_method(value));
        } else if token.starts_with("--field=")
            || token.starts_with("--raw-field=")
            || token.starts_with("--input=")
        {
            has_body_field = true;
        } else if let Some(cluster) = token
            .strip_prefix('-')
            .filter(|rest| !rest.starts_with('-'))
        {
            for (idx, ch) in cluster.char_indices() {
                match ch {
                    'X' => {
                        let attached = &cluster[idx + 1..];
                        method = if attached.is_empty() {
                            i += 1;
                            tokens.get(i).map(|value| normalize_method(value))
                        } else {
                            Some(normalize_method(attached))
                        };
                        break;
                    }
                    'f' | 'F' => {
                        has_body_field = true;
                        break;
                    }
                    _ => {}
                }
            }
        }
        i += 1;
    }
    match method.as_deref() {
        Some("GET" | "HEAD") => false,
        Some(_) => true,
        None => has_body_field,
    }
}

fn is_mutating_release_subcommand(subcommand: Option<&str>) -> bool {
    matches!(
        subcommand,
        Some("create" | "edit" | "delete" | "delete-asset" | "upload")
    )
}

/// `curl` against a GitHub API host with a mutating method (explicit
/// `-X`/`--request`, or implied by body/upload flags).
fn is_mutating_github_curl(tokens: &[&str]) -> bool {
    let targets_github_api = tokens.iter().any(|token| {
        let lowered = token.to_ascii_lowercase();
        lowered.contains("api.github.com") || lowered.contains("uploads.github.com")
    });
    if !targets_github_api {
        return false;
    }
    let mut method: Option<String> = None;
    let mut has_body = false;
    let mut forces_get = false;
    let mut i = 1;
    while i < tokens.len() {
        let token = tokens[i];
        match token {
            "-X" | "--request" => {
                method = tokens.get(i + 1).map(|value| normalize_method(value));
                i += 2;
                continue;
            }
            "-G" | "--get" => {
                forces_get = true;
                i += 1;
                continue;
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-urlencode" | "--json"
            | "-F" | "--form" | "-T" | "--upload-file" => {
                has_body = true;
                i += 2;
                continue;
            }
            _ => {}
        }
        if let Some(value) = token.strip_prefix("--request=") {
            method = Some(normalize_method(value));
        } else if [
            "--data=",
            "--data-raw=",
            "--data-binary=",
            "--data-urlencode=",
            "--json=",
            "--form=",
            "--upload-file=",
        ]
        .iter()
        .any(|prefix| token.starts_with(prefix))
        {
            has_body = true;
        } else if let Some(cluster) = token
            .strip_prefix('-')
            .filter(|rest| !rest.starts_with('-'))
        {
            // Attached short-option spellings (`-XPUT`, `-sSXPOST`,
            // `-d@body.json`, `-Gd q=x`): scan the cluster; booleans like
            // `G` continue, value-consuming flags end it.
            for (idx, ch) in cluster.char_indices() {
                match ch {
                    'G' => forces_get = true,
                    'X' => {
                        let attached = &cluster[idx + 1..];
                        method = if attached.is_empty() {
                            i += 1;
                            tokens.get(i).map(|value| normalize_method(value))
                        } else {
                            Some(normalize_method(attached))
                        };
                        break;
                    }
                    'd' | 'F' | 'T' => {
                        has_body = true;
                        break;
                    }
                    _ => {}
                }
            }
        }
        i += 1;
    }
    // `-G`/`--get` converts body flags into GET query parameters.
    match method.as_deref() {
        Some("GET" | "HEAD") => false,
        Some(_) => true,
        None => has_body && !forces_get,
    }
}

fn github_mutation_block_decision(command: &str) -> HookOutput {
    HookOutput::pre_tool_use_permission(
        "\u{1F6AB} Direct GitHub API mutations are not allowed",
        format!(
            "GitHub writes must go through the canonical gwt operations so the completion/PR gates and audit state see them (SPEC-3248 P10, T-217).\n\n\
Recommended alternatives:\n\
- PRs: JSON operations `pr.create`, `pr.edit`, `pr.ready`, `pr.draft`, `pr.comment`, `pr.review_threads.reply_and_resolve`\n\
- Issues/SPECs: JSON operations `issue.create`, `issue.comment`, `issue.spec.*`\n\
- lifecycle state: JSON operations `intake.outcome.record`, `execution.*`, `verify.*`\n\
- releases: the release workflow owns publishing — not agent Bash\n\n\
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
        assert!(decision.detail().contains("JSON operation `pr.checks`"));
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

    // SPEC-3248 P10 (T-212/T-217 core): GitHub API mutations are blocked
    // regardless of endpoint family; reads outside the workflow endpoints
    // stay available.
    #[test]
    fn blocks_mutating_gh_api_and_allows_reads() {
        for command in [
            "gh api -X POST repos/o/r/git/refs -f ref=refs/heads/x -f sha=abc",
            "gh api --method DELETE repos/o/r/releases/123",
            "gh api -X PUT repos/o/r/branches/main/protection --input p.json",
            "gh api -f name=bug repos/o/r/labels",
            "gh api -X PATCH repos/o/r/contents/README.md",
            "gh api --method=DELETE repos/o/r/releases/123",
            "gh api --field=name=bug repos/o/r/labels",
            "gh api -XPOST repos/o/r/forks",
            "gh api -fkey=value repos/o/r/forks",
        ] {
            let decision = evaluate_bash_command(command, Path::new("/worktree"))
                .unwrap_or_else(|| panic!("expected block: {command}"));
            assert_eq!(
                decision.summary(),
                "\u{1F6AB} Direct GitHub API mutations are not allowed",
                "{command}"
            );
            assert!(decision.detail().contains("T-217"), "{command}");
        }

        for command in [
            "gh api repos/o/r",
            "gh api repos/o/r/git/refs/heads/main",
            "gh api -X GET repos/o/r/releases",
            "gh api rate_limit",
            "gh api -XGET search/code -f q=foo",
            "gh api -X \"GET\" repos/o/r/releases",
        ] {
            assert!(
                evaluate_bash_command(command, Path::new("/worktree")).is_none(),
                "read must pass: {command}"
            );
        }
    }

    #[test]
    fn blocks_graphql_mutations_and_release_writes_but_not_reads() {
        for command in [
            r#"gh api graphql -f query='mutation { enablePullRequestAutoMerge(input: {}) { clientMutationId } }'"#,
            r#"gh api graphql -f query='mutation AddStar($id: ID!) { addStar(input: {starrableId: $id}) { clientMutationId } }'"#,
            "gh api graphql --input req.json",
            "gh release create v1.0.0 --notes x",
            "gh release delete v1.0.0 --yes",
            "gh release edit v1.0.0 --draft=false",
            "gh release upload v1.0.0 dist.tar.gz",
        ] {
            let decision = evaluate_bash_command(command, Path::new("/worktree"))
                .unwrap_or_else(|| panic!("expected block: {command}"));
            assert_eq!(
                decision.summary(),
                "\u{1F6AB} Direct GitHub API mutations are not allowed",
                "{command}"
            );
        }

        for command in [
            "gh release list --repo akiojin/gwt --limit 1",
            "gh release view v1.0.0",
            "gh release download v1.0.0",
            r#"gh api graphql -f query='query { viewer { login } }'"#,
            r#"gh api graphql -f query='query { search(query: "mutation sink", type: DISCUSSION, first: 5) { discussionCount } }'"#,
            r#"gh api graphql -f query='query { __schema { mutationType { name } } }'"#,
        ] {
            assert!(
                evaluate_bash_command(command, Path::new("/worktree")).is_none(),
                "read must pass: {command}"
            );
        }
    }

    #[test]
    fn blocks_curl_mutations_against_github_api_but_not_reads() {
        for command in [
            "curl -X POST https://api.github.com/repos/o/r/merges -d '{}'",
            "curl --request PUT https://api.github.com/repos/o/r/branches/main/protection",
            "curl -d '{\"query\":\"mutation{}\"}' https://api.github.com/graphql",
            "curl -T asset.zip https://uploads.github.com/repos/o/r/releases/1/assets",
            "curl --request=PUT https://api.github.com/repos/o/r/branches/main/protection",
            "curl --json='{}' https://api.github.com/repos/o/r/merges",
            "curl -XPUT https://api.github.com/repos/o/r/pulls/1/merge",
            "curl -sSXPOST https://api.github.com/repos/o/r/forks",
            "curl https://api.github.com/repos/o/r/forks -d@body.json",
        ] {
            let decision = evaluate_bash_command(command, Path::new("/worktree"))
                .unwrap_or_else(|| panic!("expected block: {command}"));
            assert_eq!(
                decision.summary(),
                "\u{1F6AB} Direct GitHub API mutations are not allowed",
                "{command}"
            );
        }

        for command in [
            "curl https://api.github.com/repos/o/r",
            "curl -s -I https://example.com/upload -X POST",
            "curl -X POST https://internal.example.com/hook",
            "curl -G https://api.github.com/search/code -d q=foo",
            "curl -Gd q=foo https://api.github.com/search/code",
            "curl -X 'GET' https://api.github.com/repos/o/r",
        ] {
            assert!(
                evaluate_bash_command(command, Path::new("/worktree")).is_none(),
                "must pass: {command}"
            );
        }
    }

    // git push stays sanctioned — it is the PR handoff path and the PR
    // gates own its policy (T-212 classification note).
    #[test]
    fn git_push_stays_allowed() {
        assert!(
            evaluate_bash_command("git push origin work/issue-1", Path::new("/worktree")).is_none()
        );
    }
}
