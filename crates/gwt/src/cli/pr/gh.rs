//! `gh` CLI wrappers for PR family commands (SPEC-1942 SC-027 split).
//!
//! Hosts every helper that shells out to the `gh` binary or graphql endpoint:
//! pull request fetch / create / edit / comment, review + review-thread queries,
//! review-thread reply-and-resolve, PR checks, plus a few pure parsers that
//! only make sense alongside the gh response payloads.
//!
//! All helpers are `pub(super)` so the parent `cli::pr` module can re-export
//! them and `cli::env` can call them via `super::pr::*`.
//!
//! Every spawn flows through `gwt_core::process_console::spawn_logged_blocking`
//! so the canonical log captures `gwt.process.summary` events for each
//! invocation (SPEC-1924 FR-039 / FR-040).

use std::ffi::OsStr;
use std::io;
use std::path::Path;

use gwt_core::process_console::{spawn_logged_blocking, ProcessKind, SpawnOptions, SpawnOutput};
use gwt_git::PrStatus;

use crate::cli::{
    PrCheckItem, PrChecksSummary, PrCreateCall, PrReview, PrReviewThread, PrReviewThreadComment,
};

fn run_gh_in<I, S>(label: &str, repo_path: Option<&Path>, args: I) -> io::Result<SpawnOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let hub = gwt_core::process_console::global();
    let args_vec: Vec<std::ffi::OsString> =
        args.into_iter().map(|s| s.as_ref().to_owned()).collect();
    let mut options = SpawnOptions::new(label);
    if let Some(dir) = repo_path {
        options = options.current_dir(dir);
    }
    spawn_logged_blocking(&hub, ProcessKind::Gh, "gh", &args_vec, options)
}

fn run_gh<I, S>(label: &str, args: I) -> io::Result<SpawnOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_gh_in(label, None, args)
}

fn run_git_in<I, S>(label: &str, repo_path: &Path, args: I) -> io::Result<SpawnOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let hub = gwt_core::process_console::global();
    let args_vec: Vec<std::ffi::OsString> =
        args.into_iter().map(|s| s.as_ref().to_owned()).collect();
    let options = SpawnOptions::new(label).current_dir(repo_path);
    spawn_logged_blocking(&hub, ProcessKind::Git, "git", &args_vec, options)
}

const PR_STATUS_FIELDS: &str =
    "number,title,state,url,createdAt,mergeable,mergeStateStatus,statusCheckRollup,reviewDecision";
const PR_LIST_FIELDS: &str = "number,title,state,url,createdAt,mergeable,mergeStateStatus,statusCheckRollup,reviewDecision,headRefName,headRepository,headRepositoryOwner";

pub fn fetch_current_pr_via_gh(repo_path: &std::path::Path) -> io::Result<Option<PrStatus>> {
    if let Some(branch) = current_branch_name(repo_path)? {
        let repo = github_remote_owner_and_repo(repo_path);
        let output = run_gh_in(
            &format!("gh pr list --head {branch}"),
            Some(repo_path),
            [
                "pr",
                "list",
                "--head",
                branch.as_str(),
                "--state",
                "all",
                "--json",
                PR_LIST_FIELDS,
                "--limit",
                "100",
            ],
        )?;

        if output.success() {
            let pr_values = filter_current_repo_head_prs(&output.stdout, &branch, repo.as_ref())?;
            if !pr_values.is_empty() {
                let filtered_stdout = serde_json::to_string(&pr_values)
                    .map_err(|err| io::Error::other(err.to_string()))?;
                let prs = gwt_git::pr_status::parse_pr_list_json(&filtered_stdout)
                    .map_err(|err| io::Error::other(err.to_string()))?;
                if let Some(pr) = gwt_git::pr_status::latest_pr_by_created_at(prs) {
                    return Ok(Some(pr));
                }
            }
        }
    }

    let output = run_gh_in(
        "gh pr view",
        Some(repo_path),
        ["pr", "view", "--json", PR_STATUS_FIELDS],
    )?;

    if !output.success() {
        let trimmed = output.stderr.trim();
        let lowered = trimmed.to_ascii_lowercase();
        if lowered.contains("no pull requests found")
            || lowered.contains("no pull request found")
            || lowered.contains("could not resolve to a pull request")
        {
            return Ok(None);
        }
        return Err(io::Error::other(format!("gh pr view: {trimmed}")));
    }

    let pr = gwt_git::pr_status::parse_pr_status_json(&output.stdout)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(Some(pr))
}

fn filter_current_repo_head_prs(
    stdout: &str,
    branch: &str,
    repo: Option<&(String, String)>,
) -> io::Result<Vec<serde_json::Value>> {
    let values: Vec<serde_json::Value> = serde_json::from_str(stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    Ok(values
        .into_iter()
        .filter(|value| pr_value_matches_current_repo_head(value, branch, repo))
        .collect())
}

fn pr_value_matches_current_repo_head(
    value: &serde_json::Value,
    branch: &str,
    repo: Option<&(String, String)>,
) -> bool {
    if value.get("headRefName").and_then(serde_json::Value::as_str) != Some(branch) {
        return false;
    }
    let Some((owner, repo_name)) = repo else {
        return false;
    };
    let Some(head_owner) = pr_head_owner_login(value) else {
        return false;
    };
    let Some(head_repo_name) = pr_head_repository_name(value) else {
        return false;
    };
    head_owner.eq_ignore_ascii_case(owner) && head_repo_name.eq_ignore_ascii_case(repo_name)
}

fn pr_head_owner_login(value: &serde_json::Value) -> Option<&str> {
    let owner = value.get("headRepositoryOwner")?;
    owner.as_str().or_else(|| {
        owner
            .get("login")
            .and_then(serde_json::Value::as_str)
            .or_else(|| owner.get("name").and_then(serde_json::Value::as_str))
    })
}

fn pr_head_repository_name(value: &serde_json::Value) -> Option<&str> {
    let repository = value.get("headRepository")?;
    repository.as_str().or_else(|| {
        repository
            .get("name")
            .and_then(serde_json::Value::as_str)
            .or_else(|| repository.get("repo").and_then(serde_json::Value::as_str))
    })
}

fn current_branch_name(repo_path: &std::path::Path) -> io::Result<Option<String>> {
    let output = run_git_in(
        "git branch --show-current",
        repo_path,
        ["branch", "--show-current"],
    )?;
    if !output.success() {
        return Ok(None);
    }
    let branch = output.stdout.trim().to_string();
    Ok((!branch.is_empty()).then_some(branch))
}

pub(super) fn github_remote_owner_and_repo(
    repo_path: &std::path::Path,
) -> Option<(String, String)> {
    let output = run_git_in(
        "git remote get-url origin",
        repo_path,
        ["remote", "get-url", "origin"],
    )
    .ok()?;
    if !output.success() {
        return None;
    }
    parse_github_remote_url(output.stdout.trim())
}

fn parse_github_remote_url(remote_url: &str) -> Option<(String, String)> {
    let path = remote_url
        .strip_prefix("https://github.com/")
        .or_else(|| remote_url.strip_prefix("http://github.com/"))
        .or_else(|| remote_url.strip_prefix("git@github.com:"))
        .or_else(|| remote_url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

pub fn create_pr_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    request: &PrCreateCall,
) -> io::Result<PrStatus> {
    let mut args = vec![
        "pr".to_string(),
        "create".to_string(),
        "--base".to_string(),
        request.base.clone(),
        "--title".to_string(),
        request.title.clone(),
        "--body".to_string(),
        request.body.clone(),
    ];
    if let Some(head) = &request.head {
        args.push("--head".to_string());
        args.push(head.clone());
    }
    for label in &request.labels {
        args.push("--label".to_string());
        args.push(label.clone());
    }
    if request.draft {
        args.push("--draft".to_string());
    }

    let output = run_gh_in("gh pr create", Some(repo_path), &args)?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh pr create: {}",
            output.stderr.trim()
        )));
    }

    let url = extract_pr_url(&output.stdout).ok_or_else(|| {
        io::Error::other(format!(
            "gh pr create: missing PR URL in output: {}",
            output.stdout
        ))
    })?;
    let number = parse_pr_number_from_url(&url)
        .ok_or_else(|| io::Error::other(format!("gh pr create: invalid PR URL: {url}")))?;
    gwt_git::pr_status::fetch_pr_status(repo_slug, number)
        .map_err(|err| io::Error::other(err.to_string()))
}

pub fn edit_pr_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    number: u64,
    title: Option<&str>,
    body: Option<&str>,
    add_labels: &[String],
) -> io::Result<PrStatus> {
    let mut args = vec!["pr".to_string(), "edit".to_string(), number.to_string()];
    if let Some(title) = title {
        args.push("--title".to_string());
        args.push(title.to_string());
    }
    if let Some(body) = body {
        args.push("--body".to_string());
        args.push(body.to_string());
    }
    for label in add_labels {
        args.push("--add-label".to_string());
        args.push(label.clone());
    }

    let output = run_gh_in("gh pr edit", Some(repo_path), &args)?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh pr edit: {}",
            output.stderr.trim()
        )));
    }
    gwt_git::pr_status::fetch_pr_status(repo_slug, number)
        .map_err(|err| io::Error::other(err.to_string()))
}

pub fn extract_pr_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("https://"))
        .map(ToOwned::to_owned)
}

pub fn parse_pr_number_from_url(url: &str) -> Option<u64> {
    url.trim_end_matches('/').rsplit('/').next()?.parse().ok()
}

pub fn comment_on_pr_via_gh(
    repo_path: &std::path::Path,
    number: u64,
    body: &str,
) -> io::Result<()> {
    let number_str = number.to_string();
    let output = run_gh_in(
        &format!("gh pr comment {number}"),
        Some(repo_path),
        ["pr", "comment", number_str.as_str(), "--body", body],
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh pr comment: {}",
            output.stderr.trim()
        )));
    }
    Ok(())
}

pub fn fetch_pr_reviews_via_gh(owner: &str, repo: &str, number: u64) -> io::Result<Vec<PrReview>> {
    let endpoint = format!("repos/{owner}/{repo}/pulls/{number}/reviews");
    let output = run_gh(&format!("gh api {endpoint}"), ["api", endpoint.as_str()])?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh api {endpoint}: {}",
            output.stderr.trim()
        )));
    }

    let values: Vec<serde_json::Value> = serde_json::from_str(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    Ok(values
        .into_iter()
        .map(|value| PrReview {
            id: value
                .get("id")
                .and_then(serde_json::Value::as_i64)
                .map(|v| v.to_string())
                .or_else(|| {
                    value
                        .get("node_id")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned)
                })
                .unwrap_or_default(),
            state: value
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            body: value
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            submitted_at: value
                .get("submitted_at")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            author: value
                .get("user")
                .and_then(|v| v.get("login"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })
        .collect())
}

pub fn fetch_pr_review_threads_via_gh(
    owner: &str,
    repo: &str,
    number: u64,
) -> io::Result<Vec<PrReviewThread>> {
    let query = r#"
query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      reviewThreads(first: 100) {
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          comments(first: 100) {
            nodes {
              id
              body
              createdAt
              updatedAt
              author { login }
            }
          }
        }
      }
    }
  }
}
"#;
    let output = run_gh(
        "gh api graphql reviewThreads",
        [
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-f",
            &format!("owner={owner}"),
            "-f",
            &format!("repo={repo}"),
            "-F",
            &format!("number={number}"),
        ],
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh api graphql: {}",
            output.stderr.trim()
        )));
    }

    let value: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    let nodes = value
        .get("data")
        .and_then(|v| v.get("repository"))
        .and_then(|v| v.get("pullRequest"))
        .and_then(|v| v.get("reviewThreads"))
        .and_then(|v| v.get("nodes"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(nodes
        .into_iter()
        .map(|node| PrReviewThread {
            id: node
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            is_resolved: node
                .get("isResolved")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            is_outdated: node
                .get("isOutdated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            path: node
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            line: node.get("line").and_then(serde_json::Value::as_u64),
            comments: node
                .get("comments")
                .and_then(|v| v.get("nodes"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|comment| PrReviewThreadComment {
                    id: comment
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    body: comment
                        .get("body")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    created_at: comment
                        .get("createdAt")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    updated_at: comment
                        .get("updatedAt")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    author: comment
                        .get("author")
                        .and_then(|v| v.get("login"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                })
                .collect(),
        })
        .collect())
}

pub fn reply_and_resolve_pr_review_threads_via_gh(
    owner: &str,
    repo: &str,
    number: u64,
    body: &str,
) -> io::Result<usize> {
    let unresolved: Vec<PrReviewThread> = fetch_pr_review_threads_via_gh(owner, repo, number)?
        .into_iter()
        .filter(should_resolve_review_thread)
        .collect();

    let mut resolved_count = 0;
    for thread in &unresolved {
        let Some(current_thread) =
            fetch_pr_review_thread_state_via_gh(owner, repo, number, &thread.id)?
        else {
            continue;
        };
        if !should_resolve_review_thread(&current_thread) {
            continue;
        }

        let reply_mutation = r#"
mutation($threadId: ID!, $body: String!) {
  addPullRequestReviewThreadReply(input: {
    pullRequestReviewThreadId: $threadId,
    body: $body
  }) {
    comment { id }
  }
}
"#;
        if should_reply_to_review_thread(&current_thread, body) {
            let reply = run_gh(
                "gh api graphql reply",
                [
                    "api",
                    "graphql",
                    "-f",
                    &format!("query={reply_mutation}"),
                    "-f",
                    &format!("threadId={}", thread.id),
                    "-f",
                    &format!("body={body}"),
                ],
            )?;
            if !reply.success() {
                return Err(io::Error::other(format!(
                    "gh api graphql reply: {}",
                    reply.stderr.trim()
                )));
            }
        }

        let resolve_mutation = r#"
mutation($threadId: ID!) {
  resolveReviewThread(input: { threadId: $threadId }) {
    thread { id isResolved }
  }
}
"#;
        let resolve = run_gh(
            "gh api graphql resolve",
            [
                "api",
                "graphql",
                "-f",
                &format!("query={resolve_mutation}"),
                "-f",
                &format!("threadId={}", thread.id),
            ],
        )?;
        if !resolve.success() {
            if fetch_pr_review_thread_state_via_gh(owner, repo, number, &thread.id)?
                .as_ref()
                .is_some_and(|thread| thread.is_resolved)
            {
                resolved_count += 1;
                continue;
            }
            return Err(io::Error::other(format!(
                "gh api graphql resolve: {}",
                resolve.stderr.trim()
            )));
        }

        resolved_count += 1;
    }

    Ok(resolved_count)
}

pub fn fetch_pr_checks_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    number: u64,
) -> io::Result<PrChecksSummary> {
    let pr = gwt_git::pr_status::fetch_pr_status(repo_slug, number)
        .map_err(|err| io::Error::other(err.to_string()))?;

    let primary_fields = [
        "name",
        "state",
        "conclusion",
        "detailsUrl",
        "startedAt",
        "completedAt",
    ];
    let number_str = number.to_string();
    let mut output = run_gh_in(
        &format!("gh pr checks {number}"),
        Some(repo_path),
        [
            "pr",
            "checks",
            number_str.as_str(),
            "--json",
            &primary_fields.join(","),
        ],
    )?;

    if !output.success() {
        let available = parse_available_fields(&output.stderr);
        if !available.is_empty() {
            let fallback_fields = [
                "name",
                "state",
                "bucket",
                "link",
                "startedAt",
                "completedAt",
                "workflow",
            ];
            let selected: Vec<&str> = fallback_fields
                .iter()
                .copied()
                .filter(|field| available.iter().any(|candidate| candidate == field))
                .collect();
            if !selected.is_empty() {
                output = run_gh_in(
                    &format!("gh pr checks {number} fallback"),
                    Some(repo_path),
                    [
                        "pr",
                        "checks",
                        number_str.as_str(),
                        "--json",
                        &selected.join(","),
                    ],
                )?;
            }
        }
    }

    let checks = parse_pr_checks_items_response(&output.stdout, &output.stderr, output.success())?;
    let merge_status = pr.effective_merge_status().to_string();

    Ok(PrChecksSummary {
        summary: format!(
            "PR #{} | CI: {} | Merge: {} | Review: {}",
            pr.number, pr.ci_status, merge_status, pr.review_status
        ),
        ci_status: pr.ci_status,
        merge_status,
        review_status: pr.review_status,
        checks,
    })
}

pub fn fetch_pr_review_thread_state_via_gh(
    owner: &str,
    repo: &str,
    number: u64,
    thread_id: &str,
) -> io::Result<Option<PrReviewThread>> {
    Ok(fetch_pr_review_threads_via_gh(owner, repo, number)?
        .into_iter()
        .find(|thread| thread.id == thread_id))
}

pub fn review_thread_has_comment_body(thread: &PrReviewThread, body: &str) -> bool {
    thread.comments.iter().any(|comment| comment.body == body)
}

pub fn should_reply_to_review_thread(thread: &PrReviewThread, body: &str) -> bool {
    should_resolve_review_thread(thread) && !review_thread_has_comment_body(thread, body)
}

pub fn should_resolve_review_thread(thread: &PrReviewThread) -> bool {
    !thread.is_resolved && !thread.is_outdated
}

pub fn parse_pr_checks_items_json(json: &str) -> Result<Vec<PrCheckItem>, serde_json::Error> {
    let values: Vec<serde_json::Value> = serde_json::from_str(json)?;
    Ok(values
        .into_iter()
        .map(|value| PrCheckItem {
            name: value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            state: value
                .get("state")
                .or_else(|| value.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            conclusion: value
                .get("conclusion")
                .or_else(|| value.get("bucket"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            url: value
                .get("detailsUrl")
                .or_else(|| value.get("link"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            started_at: value
                .get("startedAt")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            completed_at: value
                .get("completedAt")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            workflow: value
                .get("workflow")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })
        .collect())
}

pub fn parse_pr_checks_items_response(
    stdout: &str,
    stderr: &str,
    success: bool,
) -> io::Result<Vec<PrCheckItem>> {
    if !success {
        return Err(io::Error::other(format!("gh pr checks: {}", stderr.trim())));
    }

    parse_pr_checks_items_json(stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))
}

pub fn parse_available_fields(message: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut collecting = false;
    for line in message.lines() {
        if line.contains("Available fields:") {
            collecting = true;
            continue;
        }
        if !collecting {
            continue;
        }
        let field = line.trim();
        if field.is_empty() {
            continue;
        }
        fields.push(field.to_string());
    }
    fields
}

pub fn edit_or_create_repo_guard(owner: &str, repo: &str) -> io::Result<()> {
    if owner.is_empty() || repo.is_empty() {
        return Err(io::Error::other(
            "missing repository context for PR create/edit operation",
        ));
    }
    Ok(())
}
