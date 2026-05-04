//! `gh` CLI wrappers for PR family commands (SPEC-1942 SC-027 split).
//!
//! Hosts every helper that shells out to the `gh` binary or graphql endpoint:
//! pull request fetch / create / edit / comment, review + review-thread queries,
//! review-thread reply-and-resolve, PR checks, plus a few pure parsers that
//! only make sense alongside the gh response payloads.
//!
//! All helpers are `pub(super)` so the parent `cli::pr` module can re-export
//! them and `cli::env` can call them via `super::pr::*`.

use std::io;

use gwt_git::PrStatus;

use crate::cli::{
    PrCheckItem, PrChecksSummary, PrCreateCall, PrReview, PrReviewThread, PrReviewThreadComment,
};

pub fn fetch_current_pr_via_gh(repo_path: &std::path::Path) -> io::Result<Option<PrStatus>> {
    let output = gwt_core::process::hidden_command("gh")
        .args([
            "pr",
            "view",
            "--json",
            "number,title,state,url,mergeable,mergeStateStatus,statusCheckRollup,reviewDecision",
        ])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        let lowered = trimmed.to_ascii_lowercase();
        if lowered.contains("no pull requests found")
            || lowered.contains("no pull request found")
            || lowered.contains("could not resolve to a pull request")
        {
            return Ok(None);
        }
        return Err(io::Error::other(format!("gh pr view: {trimmed}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pr = gwt_git::pr_status::parse_pr_status_json(&stdout)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(Some(pr))
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

    let output = gwt_core::process::hidden_command("gh")
        .args(&args)
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh pr create: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let url = extract_pr_url(&stdout).ok_or_else(|| {
        io::Error::other(format!("gh pr create: missing PR URL in output: {stdout}"))
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

    let output = gwt_core::process::hidden_command("gh")
        .args(&args)
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh pr edit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
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
    let output = gwt_core::process::hidden_command("gh")
        .args(["pr", "comment", &number.to_string(), "--body", body])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh pr comment: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

pub fn fetch_pr_reviews_via_gh(owner: &str, repo: &str, number: u64) -> io::Result<Vec<PrReview>> {
    let endpoint = format!("repos/{owner}/{repo}/pulls/{number}/reviews");
    let output = gwt_core::process::hidden_command("gh")
        .args(["api", &endpoint])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh api {endpoint}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let values: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)
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
    let output = gwt_core::process::hidden_command("gh")
        .args([
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
        ])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh api graphql: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
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
            let reply = gwt_core::process::hidden_command("gh")
                .args([
                    "api",
                    "graphql",
                    "-f",
                    &format!("query={reply_mutation}"),
                    "-f",
                    &format!("threadId={}", thread.id),
                    "-f",
                    &format!("body={body}"),
                ])
                .output()?;
            if !reply.status.success() {
                return Err(io::Error::other(format!(
                    "gh api graphql reply: {}",
                    String::from_utf8_lossy(&reply.stderr).trim()
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
        let resolve = gwt_core::process::hidden_command("gh")
            .args([
                "api",
                "graphql",
                "-f",
                &format!("query={resolve_mutation}"),
                "-f",
                &format!("threadId={}", thread.id),
            ])
            .output()?;
        if !resolve.status.success() {
            if fetch_pr_review_thread_state_via_gh(owner, repo, number, &thread.id)?
                .as_ref()
                .is_some_and(|thread| thread.is_resolved)
            {
                resolved_count += 1;
                continue;
            }
            return Err(io::Error::other(format!(
                "gh api graphql resolve: {}",
                String::from_utf8_lossy(&resolve.stderr).trim()
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
    let mut output = gwt_core::process::hidden_command("gh")
        .args([
            "pr",
            "checks",
            &number.to_string(),
            "--json",
            &primary_fields.join(","),
        ])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let available = parse_available_fields(&stderr);
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
                output = gwt_core::process::hidden_command("gh")
                    .args([
                        "pr",
                        "checks",
                        &number.to_string(),
                        "--json",
                        &selected.join(","),
                    ])
                    .current_dir(repo_path)
                    .output()?;
            }
        }
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let checks = parse_pr_checks_items_response(&stdout, &stderr, output.status.success())?;
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
