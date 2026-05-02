use std::io;

use gwt_git::PrStatus;
use gwt_github::SpecOpsError;

use crate::cli::{
    CliEnv, CliParseError, PrCheckItem, PrChecksSummary, PrCommand, PrCreateCall, PrReview,
    PrReviewThread, PrReviewThreadComment,
};

pub(super) fn parse(args: &[String]) -> Result<PrCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("current") => {
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::Current)
        }
        Some("create") => parse_pr_create_args(it.collect::<Vec<_>>().as_slice()),
        Some("edit") => parse_pr_edit_args(it.collect::<Vec<_>>().as_slice()),
        Some("view") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::View { number })
        }
        Some("comment") => {
            let number = super::parse_required_number(it.next())?;
            super::expect_flag(it.next(), "-f")?;
            let file = it.next().ok_or(CliParseError::MissingFlag("-f"))?.clone();
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::Comment { number, file })
        }
        Some("reviews") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::Reviews { number })
        }
        Some("review-threads") => match it.next().map(String::as_str) {
            Some("reply-and-resolve") => {
                let number = super::parse_required_number(it.next())?;
                super::expect_flag(it.next(), "-f")?;
                let file = it.next().ok_or(CliParseError::MissingFlag("-f"))?.clone();
                super::ensure_no_remaining_args(it)?;
                Ok(PrCommand::ReviewThreadsReplyAndResolve { number, file })
            }
            Some(number_arg) => {
                let number = number_arg
                    .parse()
                    .map_err(|_| CliParseError::InvalidNumber(number_arg.to_string()))?;
                super::ensure_no_remaining_args(it)?;
                Ok(PrCommand::ReviewThreads { number })
            }
            None => Err(CliParseError::Usage),
        },
        Some("checks") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(PrCommand::Checks { number })
        }
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
        None => Err(CliParseError::Usage),
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: PrCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let code = match cmd {
        PrCommand::Current => {
            match env.fetch_current_pr().map_err(super::io_as_api_error)? {
                Some(pr) => render_pr(out, &pr),
                None => out.push_str("no current pull request\n"),
            }
            0
        }
        PrCommand::Create {
            base,
            head,
            title,
            file,
            labels,
            draft,
        } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let pr = env
                .create_pr(&base, head.as_deref(), &title, &body, &labels, draft)
                .map_err(super::io_as_api_error)?;
            out.push_str("created pull request\n");
            render_pr(out, &pr);
            0
        }
        PrCommand::Edit {
            number,
            title,
            file,
            add_labels,
        } => {
            let body = file
                .as_deref()
                .map(|path| env.read_file(path).map_err(super::io_as_api_error))
                .transpose()?;
            let pr = env
                .edit_pr(number, title.as_deref(), body.as_deref(), &add_labels)
                .map_err(super::io_as_api_error)?;
            out.push_str("updated pull request\n");
            render_pr(out, &pr);
            0
        }
        PrCommand::View { number } => {
            let pr = env.fetch_pr(number).map_err(super::io_as_api_error)?;
            render_pr(out, &pr);
            0
        }
        PrCommand::Comment { number, file } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            env.comment_on_pr(number, &body)
                .map_err(super::io_as_api_error)?;
            out.push_str(&format!("created comment on PR #{number}\n"));
            0
        }
        PrCommand::Reviews { number } => {
            let reviews = env
                .fetch_pr_reviews(number)
                .map_err(super::io_as_api_error)?;
            render_pr_reviews(out, &reviews);
            0
        }
        PrCommand::ReviewThreads { number } => {
            let threads = env
                .fetch_pr_review_threads(number)
                .map_err(super::io_as_api_error)?;
            render_pr_review_threads(out, &threads);
            0
        }
        PrCommand::ReviewThreadsReplyAndResolve { number, file } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let resolved = env
                .reply_and_resolve_pr_review_threads(number, &body)
                .map_err(super::io_as_api_error)?;
            out.push_str(&format!(
                "replied to and resolved {resolved} review threads on PR #{number}\n"
            ));
            0
        }
        PrCommand::Checks { number } => {
            let report = env
                .fetch_pr_checks(number)
                .map_err(super::io_as_api_error)?;
            render_pr_checks(out, &report);
            0
        }
    };
    Ok(code)
}

fn parse_pr_create_args(args: &[&String]) -> Result<PrCommand, CliParseError> {
    let mut base: Option<String> = None;
    let mut head: Option<String> = None;
    let mut title: Option<String> = None;
    let mut file: Option<String> = None;
    let mut labels: Vec<String> = Vec::new();
    let mut draft = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--base" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--base"));
                }
                base = Some(args[i].clone());
            }
            "--head" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--head"));
                }
                head = Some(args[i].clone());
            }
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title"));
                }
                title = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            "--label" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--label"));
                }
                labels.push(args[i].clone());
            }
            "--draft" => draft = true,
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }
    Ok(PrCommand::Create {
        base: base.ok_or(CliParseError::MissingFlag("--base"))?,
        head,
        title: title.ok_or(CliParseError::MissingFlag("--title"))?,
        file: file.ok_or(CliParseError::MissingFlag("-f"))?,
        labels,
        draft,
    })
}

fn parse_pr_edit_args(args: &[&String]) -> Result<PrCommand, CliParseError> {
    let Some(number_arg) = args.first() else {
        return Err(CliParseError::Usage);
    };
    let number = number_arg
        .parse()
        .map_err(|_| CliParseError::InvalidNumber((*number_arg).clone()))?;
    let mut title: Option<String> = None;
    let mut file: Option<String> = None;
    let mut add_labels: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title"));
                }
                title = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            "--add-label" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--add-label"));
                }
                add_labels.push(args[i].clone());
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }
    if title.is_none() && file.is_none() && add_labels.is_empty() {
        return Err(CliParseError::Usage);
    }
    Ok(PrCommand::Edit {
        number,
        title,
        file,
        add_labels,
    })
}

pub(super) fn render_pr(out: &mut String, pr: &PrStatus) {
    out.push_str(&format!("#{} [{}] {}\n", pr.number, pr.state, pr.title));
    out.push_str(&format!("url: {}\n", pr.url));
    out.push_str(&format!("ci: {}\n", pr.ci_status));
    out.push_str(&format!("mergeable: {}\n", pr.effective_merge_status()));
    out.push_str(&format!("merge_state: {}\n", pr.merge_state_status));
    out.push_str(&format!("review: {}\n", pr.review_status));
}

pub(super) fn render_pr_checks(out: &mut String, summary: &PrChecksSummary) {
    out.push_str(&format!("summary: {}\n", summary.summary));
    out.push_str(&format!("ci: {}\n", summary.ci_status));
    out.push_str(&format!("merge: {}\n", summary.merge_status));
    out.push_str(&format!("review: {}\n", summary.review_status));
    if summary.checks.is_empty() {
        out.push_str("no checks\n");
        return;
    }
    for check in &summary.checks {
        out.push_str(&format!(
            "- {} [{} / {}]\n",
            check.name, check.state, check.conclusion
        ));
        if !check.workflow.is_empty() {
            out.push_str(&format!("  workflow: {}\n", check.workflow));
        }
        if !check.url.is_empty() {
            out.push_str(&format!("  url: {}\n", check.url));
        }
    }
}

pub(super) fn render_pr_reviews(out: &mut String, reviews: &[PrReview]) {
    if reviews.is_empty() {
        out.push_str("no reviews\n");
        return;
    }
    for review in reviews {
        out.push_str(&format!(
            "=== review:{} [{}] by {} at {} ===\n",
            review.id, review.state, review.author, review.submitted_at
        ));
        if !review.body.is_empty() {
            out.push_str(&review.body);
            out.push('\n');
        }
    }
}

pub(super) fn render_pr_review_threads(out: &mut String, threads: &[PrReviewThread]) {
    if threads.is_empty() {
        out.push_str("no review threads\n");
        return;
    }
    for thread in threads {
        out.push_str(&format!(
            "=== thread:{} resolved={} outdated={} path={} line={} ===\n",
            thread.id,
            thread.is_resolved,
            thread.is_outdated,
            if thread.path.is_empty() {
                "-"
            } else {
                thread.path.as_str()
            },
            thread
                .line
                .map(|line| line.to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
        for comment in &thread.comments {
            out.push_str(&format!(
                "--- comment:{} by {} ({}) ---\n{}\n",
                comment.id, comment.author, comment.updated_at, comment.body
            ));
        }
    }
}

pub(super) fn fetch_current_pr_via_gh(repo_path: &std::path::Path) -> io::Result<Option<PrStatus>> {
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

pub(super) fn create_pr_via_gh(
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

pub(super) fn edit_pr_via_gh(
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

pub(super) fn extract_pr_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("https://"))
        .map(ToOwned::to_owned)
}

pub(super) fn parse_pr_number_from_url(url: &str) -> Option<u64> {
    url.trim_end_matches('/').rsplit('/').next()?.parse().ok()
}

pub(super) fn comment_on_pr_via_gh(
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

pub(super) fn fetch_pr_reviews_via_gh(
    owner: &str,
    repo: &str,
    number: u64,
) -> io::Result<Vec<PrReview>> {
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
                .and_then(|v| v.as_i64())
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

pub(super) fn fetch_pr_review_threads_via_gh(
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
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            is_outdated: node
                .get("isOutdated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            path: node
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            line: node.get("line").and_then(|v| v.as_u64()),
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

pub(super) fn reply_and_resolve_pr_review_threads_via_gh(
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

pub(super) fn fetch_pr_checks_via_gh(
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

pub(super) fn fetch_pr_review_thread_state_via_gh(
    owner: &str,
    repo: &str,
    number: u64,
    thread_id: &str,
) -> io::Result<Option<PrReviewThread>> {
    Ok(fetch_pr_review_threads_via_gh(owner, repo, number)?
        .into_iter()
        .find(|thread| thread.id == thread_id))
}

pub(super) fn review_thread_has_comment_body(thread: &PrReviewThread, body: &str) -> bool {
    thread.comments.iter().any(|comment| comment.body == body)
}

pub(super) fn should_reply_to_review_thread(thread: &PrReviewThread, body: &str) -> bool {
    should_resolve_review_thread(thread) && !review_thread_has_comment_body(thread, body)
}

pub(super) fn should_resolve_review_thread(thread: &PrReviewThread) -> bool {
    !thread.is_resolved && !thread.is_outdated
}

pub(super) fn parse_pr_checks_items_json(
    json: &str,
) -> Result<Vec<PrCheckItem>, serde_json::Error> {
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

pub(super) fn parse_pr_checks_items_response(
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

pub(super) fn parse_available_fields(message: &str) -> Vec<String> {
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

pub(super) fn edit_or_create_repo_guard(owner: &str, repo: &str) -> io::Result<()> {
    if owner.is_empty() || repo.is_empty() {
        return Err(io::Error::other(
            "missing repository context for PR create/edit operation",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    fn seeded_pr() -> gwt_git::PrStatus {
        gwt_git::PrStatus {
            number: 7,
            title: "CLI family split".to_string(),
            state: gwt_git::pr_status::PrState::Open,
            url: "https://example.com/pr/7".to_string(),
            ci_status: "SUCCESS".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            review_status: "APPROVED".to_string(),
        }
    }

    #[test]
    fn pr_family_parse_directly_handles_current() {
        let cmd = parse(&[s("current")]).expect("parse pr family command");
        assert_eq!(cmd, PrCommand::Current);
    }

    #[test]
    fn pr_family_run_directly_renders_current_pr() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.seed_current_pr(Some(seeded_pr()));

        let mut out = String::new();
        let code = run(&mut env, PrCommand::Current, &mut out).expect("run pr family");

        assert_eq!(code, 0);
        assert!(out.contains("#7 [OPEN] CLI family split"));
        assert_eq!(env.pr_current_call_count, 1);
        assert!(env.client.call_log().is_empty());
    }

    // -------------------------------------------------------------------
    // SPEC-1942 SC-025 follow-up: PR-family helper tests relocated from
    // cli.rs. Shared fake-gh harness lives in cli/test_support.rs.
    // -------------------------------------------------------------------

    use crate::cli::test_support::{sample_thread, with_fake_gh};
    use crate::cli::PrCreateCall;
    use std::io;

    #[test]
    fn review_thread_reply_is_skipped_for_duplicate_body() {
        let mut thread = sample_thread();
        thread.comments.push(crate::cli::PrReviewThreadComment {
            id: "comment-1".to_string(),
            body: "Fixed in latest commit.".to_string(),
            created_at: "2026-04-10T00:00:00Z".to_string(),
            updated_at: "2026-04-10T00:00:00Z".to_string(),
            author: "reviewer".to_string(),
        });

        assert!(!should_reply_to_review_thread(
            &thread,
            "Fixed in latest commit."
        ));
        assert!(should_resolve_review_thread(&thread));
    }

    #[test]
    fn review_thread_reply_is_skipped_for_resolved_or_outdated_threads() {
        let mut resolved = sample_thread();
        resolved.is_resolved = true;
        assert!(!should_reply_to_review_thread(&resolved, "reply"));
        assert!(!should_resolve_review_thread(&resolved));

        let mut outdated = sample_thread();
        outdated.is_outdated = true;
        assert!(!should_reply_to_review_thread(&outdated, "reply"));
        assert!(!should_resolve_review_thread(&outdated));
    }

    #[test]
    fn pr_checks_response_returns_error_when_gh_fails() {
        let err = parse_pr_checks_items_response("", "auth failed", false).unwrap_err();
        assert!(
            err.to_string().contains("gh pr checks: auth failed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pr_checks_response_parses_success_payload() {
        let items = parse_pr_checks_items_response(
            r#"[{"name":"test","state":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://example.com","startedAt":"2026-04-10T00:00:00Z","completedAt":"2026-04-10T00:01:00Z","workflow":"CI"}]"#,
            "",
            true,
        )
        .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "test");
        assert_eq!(items[0].conclusion, "SUCCESS");
    }

    #[test]
    fn gh_wrappers_parse_successful_responses() {
        with_fake_gh("success", |repo_path| {
            let linked = crate::cli::issue::fetch_linked_prs_via_gh(
                "akiojin",
                "gwt",
                gwt_github::IssueNumber(42),
            )
            .expect("linked");
            assert_eq!(linked.len(), 2);
            assert_eq!(linked[0].number, 12);
            assert_eq!(linked[1].state, "MERGED");

            let current = fetch_current_pr_via_gh(repo_path)
                .expect("current pr")
                .expect("current pr exists");
            assert_eq!(current.number, 12);
            assert_eq!(current.merge_state_status, "CLEAN");

            let created = create_pr_via_gh(
                "akiojin/gwt",
                repo_path,
                &PrCreateCall {
                    base: "develop".to_string(),
                    head: Some("feature/coverage".to_string()),
                    title: "Raise coverage".to_string(),
                    body: "Body".to_string(),
                    labels: vec!["coverage".to_string()],
                    draft: true,
                },
            )
            .expect("create pr");
            assert_eq!(created.number, 12);

            let edited = edit_pr_via_gh(
                "akiojin/gwt",
                repo_path,
                12,
                Some("Edited"),
                Some("Updated body"),
                &["tested".to_string()],
            )
            .expect("edit pr");
            assert_eq!(edited.number, 12);

            comment_on_pr_via_gh(repo_path, 12, "done").expect("comment");

            let reviews = fetch_pr_reviews_via_gh("akiojin", "gwt", 12).expect("reviews");
            assert_eq!(reviews.len(), 1);
            assert_eq!(reviews[0].author, "reviewer");

            let threads =
                fetch_pr_review_threads_via_gh("akiojin", "gwt", 12).expect("review threads");
            assert_eq!(threads.len(), 2);
            assert_eq!(threads[0].line, Some(10));

            let resolved = reply_and_resolve_pr_review_threads_via_gh("akiojin", "gwt", 12, "done")
                .expect("reply and resolve");
            assert_eq!(resolved, 2);

            let checks = fetch_pr_checks_via_gh("akiojin/gwt", repo_path, 12).expect("checks");
            assert!(checks.summary.contains("PR #12"));
            assert_eq!(checks.checks.len(), 1);
            assert_eq!(checks.checks[0].conclusion, "SUCCESS");

            let run_log =
                crate::cli::actions::fetch_actions_run_log_via_gh(repo_path, 90).expect("run log");
            assert_eq!(run_log.trim(), "run log 90");

            let job_log =
                crate::cli::actions::fetch_actions_job_log_via_gh("akiojin", "gwt", repo_path, 91)
                    .expect("job log");
            assert_eq!(job_log, "job log 91");
        });
    }

    #[test]
    fn gh_wrappers_cover_none_fallback_and_zip_error_paths() {
        with_fake_gh("no-current-pr", |repo_path| {
            assert!(fetch_current_pr_via_gh(repo_path)
                .expect("current pr result")
                .is_none());
        });

        with_fake_gh("checks-fallback", |repo_path| {
            let checks = fetch_pr_checks_via_gh("akiojin/gwt", repo_path, 12).expect("checks");
            assert_eq!(checks.checks.len(), 1);
            assert_eq!(checks.checks[0].workflow, "coverage");
            assert_eq!(checks.checks[0].url, "https://example.test/checks/12");
        });

        with_fake_gh("behind", |repo_path| {
            let current = fetch_current_pr_via_gh(repo_path)
                .expect("current pr")
                .expect("current pr exists");
            assert_eq!(current.effective_merge_status(), "BEHIND");

            let checks = fetch_pr_checks_via_gh("akiojin/gwt", repo_path, 12).expect("checks");
            assert!(checks.summary.contains("Merge: BEHIND"));
            assert_eq!(checks.merge_status, "BEHIND");
        });

        with_fake_gh("job-log-zip", |repo_path| {
            let err =
                crate::cli::actions::fetch_actions_job_log_via_gh("akiojin", "gwt", repo_path, 91)
                    .expect_err("zip");
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert!(err.to_string().contains("zip archive"));
        });
    }

    #[test]
    fn gh_wrappers_tolerate_resolve_failure_after_remote_state_changes() {
        with_fake_gh("resolve-fails-but-resolved", |_repo_path| {
            let resolved = reply_and_resolve_pr_review_threads_via_gh("akiojin", "gwt", 12, "done")
                .expect("resolved after retry");
            assert_eq!(resolved, 2);
        });
    }
}
