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

use super::{PrQuarantineComment, PrQuarantineContext};

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

/// Issue #3891 AC-3: the raw `gh api rate_limit` payload. The endpoint is
/// free (spends neither budget), so observing the budget never consumes it.
pub fn probe_github_rate_limit_via_gh(repo_path: &Path) -> io::Result<String> {
    let output = run_gh_in(
        "gh api rate_limit",
        Some(repo_path),
        gwt_core::github_quota::RATE_LIMIT_PROBE_ARGS,
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh api rate_limit: {}",
            output.stderr.trim()
        )));
    }
    Ok(output.stdout)
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
    "number,title,state,url,headRefName,headRepository,headRepositoryOwner,createdAt,mergeable,mergeStateStatus,statusCheckRollup,reviewDecision";
const PR_LIST_FIELDS: &str = "number,title,state,url,createdAt,mergeable,mergeStateStatus,statusCheckRollup,reviewDecision,headRefName,headRepository,headRepositoryOwner";

pub fn fetch_current_pr_via_gh(repo_path: &std::path::Path) -> io::Result<Option<PrStatus>> {
    let branch = current_branch_name(repo_path)?;
    let repo = github_remote_owner_and_repo(repo_path);
    if let Some(branch) = branch.as_deref() {
        let output = run_gh_in(
            &format!("gh pr list --head {branch}"),
            Some(repo_path),
            [
                "pr",
                "list",
                "--head",
                branch,
                "--state",
                "all",
                "--json",
                PR_LIST_FIELDS,
                "--limit",
                "100",
            ],
        )?;

        if output.success() {
            let pr_values = filter_current_repo_head_prs(&output.stdout, branch, repo.as_ref())?;
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

    let value: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    if !branch
        .as_deref()
        .is_some_and(|branch| pr_value_matches_current_repo_head(&value, branch, repo.as_ref()))
    {
        return Ok(None);
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

pub(in crate::cli) fn parse_github_remote_url(remote_url: &str) -> Option<(String, String)> {
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

fn select_pr_fork_url(
    base: &serde_json::Value,
    candidates: &[serde_json::Value],
    owner: &str,
) -> io::Result<String> {
    let network_id = repository_network_id(base)?;
    let mut matched = None;
    for candidate in candidates {
        if repository_network_id(candidate)? != network_id {
            continue;
        }
        let full_name = candidate["full_name"].as_str().unwrap_or_default();
        let (actual_owner, name) = full_name.split_once('/').unwrap_or(("", ""));
        if !actual_owner.eq_ignore_ascii_case(owner)
            || name.is_empty()
            || name.contains('/')
            || !candidate["owner"]["login"]
                .as_str()
                .is_some_and(|login| login.eq_ignore_ascii_case(owner))
        {
            return Err(io::Error::other(
                "PR fork resolution returned a mismatched repository owner",
            ));
        }
        let url = candidate["clone_url"].as_str().unwrap_or_default();
        if !url.starts_with("https://github.com/")
            || !parse_github_remote_url(url).is_some_and(|(url_owner, url_name)| {
                url_owner.eq_ignore_ascii_case(actual_owner) && url_name.eq_ignore_ascii_case(name)
            })
        {
            return Err(io::Error::other(
                "PR fork resolution cannot prove the repository clone URL",
            ));
        }
        if matched.replace(url.to_string()).is_some() {
            return Err(io::Error::other("PR fork resolution is ambiguous: multiple repositories match the requested owner and target network"));
        }
    }
    matched.ok_or_else(|| io::Error::other("PR fork resolution found no accessible repository for the requested owner in the target network"))
}

fn repository_network_id(repository: &serde_json::Value) -> io::Result<u64> {
    let id = match repository["fork"].as_bool() {
        Some(true) => repository["source"]["id"].as_u64(),
        Some(false) => repository["id"].as_u64(),
        None => None,
    };
    id.filter(|id| *id != 0).ok_or_else(|| {
        io::Error::other("PR fork resolution cannot prove the repository network identity")
    })
}

/// Resolve `owner:branch` against the target's actual network, including renamed
/// and indirect forks. GraphQL `Repository.forks` only lists direct forks, so
/// enumerate the requested owner's forks and compare REST's root `source.id`.
pub fn resolve_pr_fork_url_via_gh(
    repo_slug: &str,
    repo_path: &Path,
    owner: &str,
) -> io::Result<String> {
    let fetch_repository = |slug: &str| -> io::Result<serde_json::Value> {
        let (repo_owner, name) = slug
            .split_once('/')
            .filter(|(repo_owner, name)| {
                !repo_owner.is_empty() && !name.is_empty() && !name.contains('/')
            })
            .ok_or_else(|| {
                io::Error::other("PR fork resolution received an invalid repository identity")
            })?;
        let endpoint = format!(
            "repos/{}/{}",
            encode_path_segment(repo_owner),
            encode_path_segment(name)
        );
        let output = run_gh_in(
            "gh api PR fork repository",
            Some(repo_path),
            ["api", endpoint.as_str()],
        )?;
        if !output.success() {
            return Err(io::Error::other(format!(
                "PR fork resolution: {}",
                output.stderr.trim()
            )));
        }
        serde_json::from_str(&output.stdout).map_err(io::Error::other)
    };
    let base = fetch_repository(repo_slug)?;
    repository_network_id(&base)?;
    let root = if base["fork"] == true {
        &base["source"]
    } else {
        &base
    };
    // An owner cannot own both a network root and a fork of that same network.
    if root["owner"]["login"]
        .as_str()
        .is_some_and(|login| login.eq_ignore_ascii_case(owner))
    {
        return select_pr_fork_url(&base, std::slice::from_ref(root), owner);
    }
    let query = "query($owner:String!,$endCursor:String){repositoryOwner(login:$owner){login repositories(first:100,after:$endCursor,isFork:true,ownerAffiliations:[OWNER]){nodes{nameWithOwner} pageInfo{hasNextPage endCursor}}}}";
    let output = run_gh_in(
        "gh api PR fork owner repositories",
        Some(repo_path),
        [
            "api",
            "graphql",
            "--paginate",
            "--slurp",
            "-f",
            &format!("query={query}"),
            "-f",
            &format!("owner={owner}"),
        ],
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "PR fork resolution: {}",
            output.stderr.trim()
        )));
    }
    let pages: Vec<serde_json::Value> =
        serde_json::from_str(&output.stdout).map_err(io::Error::other)?;
    let mut candidates = Vec::new();
    for page in pages {
        let repository_owner = &page["data"]["repositoryOwner"];
        if page
            .get("errors")
            .is_some_and(|errors| errors.as_array().is_none_or(|errors| !errors.is_empty()))
            || !repository_owner["login"]
                .as_str()
                .is_some_and(|login| login.eq_ignore_ascii_case(owner))
        {
            return Err(io::Error::other(
                "PR fork resolution could not read the exact requested owner",
            ));
        }
        let nodes = repository_owner["repositories"]["nodes"]
            .as_array()
            .ok_or_else(|| {
                io::Error::other("PR fork resolution returned an incomplete repository list")
            })?;
        for node in nodes {
            let slug = node["nameWithOwner"].as_str().ok_or_else(|| {
                io::Error::other("PR fork resolution returned a repository without its exact name")
            })?;
            if !slug
                .split_once('/')
                .is_some_and(|(actual_owner, _)| actual_owner.eq_ignore_ascii_case(owner))
            {
                return Err(io::Error::other(
                    "PR fork resolution returned a repository owned by another account",
                ));
            }
            let candidate = fetch_repository(slug)?;
            if !candidate["full_name"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(slug))
            {
                return Err(io::Error::other("PR fork repository identity changed during resolution; retry against its current name"));
            }
            candidates.push(candidate);
        }
    }
    select_pr_fork_url(&base, &candidates, owner)
}

pub fn create_pr_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    request: &PrCreateCall,
) -> io::Result<PrStatus> {
    let mut args = vec![
        "pr".to_string(),
        "create".to_string(),
        "--repo".to_string(),
        repo_slug.to_string(),
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

pub fn fetch_pr_head_sha_via_gh(
    repo_slug: &str,
    repo_path: &Path,
    number: u64,
) -> io::Result<Option<String>> {
    let endpoint = format!("repos/{repo_slug}/pulls/{number}");
    let output = run_gh_in(
        "gh pr head comparison",
        Some(repo_path),
        ["api", endpoint.as_str()],
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "PR head comparison read failed: {}",
            output.stderr.trim()
        )));
    }
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).map_err(io::Error::other)?;
    Ok(value
        .pointer("/head/sha")
        .and_then(serde_json::Value::as_str)
        .filter(|sha| !sha.is_empty())
        .map(str::to_string))
}

/// Edit a PR's base / title / body / labels via REST rather than `gh pr edit`.
///
/// `gh pr edit` prefetches repository + assignee metadata whose query touches an
/// org-scoped `login` field, so it fails with
/// `The 'login' field requires one of the following scopes: ['read:org']` when
/// the token lacks `read:org` — even though `gh pr create` succeeds with the
/// same token (Issue #3201). Routing base/title/body through
/// `PATCH /repos/{owner}/{repo}/pulls/{number}` and additive labels through
/// `POST /repos/{owner}/{repo}/issues/{number}/labels` only requires the `repo`
/// scope, keeping `pr.edit` scope-symmetric with `pr.create`.
pub fn edit_pr_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    number: u64,
    base: Option<&str>,
    title: Option<&str>,
    body: Option<&str>,
    add_labels: &[String],
) -> io::Result<PrStatus> {
    // Fail closed on unknown labels before any mutation: the REST labels
    // endpoint silently auto-creates missing labels, unlike the old
    // `gh pr edit --add-label` which rejected typos before applying anything.
    for label in add_labels {
        let endpoint = format!("repos/{repo_slug}/labels/{}", encode_path_segment(label));
        let output = run_gh_in(
            "gh pr edit label-check",
            Some(repo_path),
            ["api", endpoint.as_str()],
        )?;
        if !output.success() {
            return Err(io::Error::other(format!(
                "gh pr edit: label '{label}' lookup failed: {}",
                output.stderr.trim()
            )));
        }
    }

    if base.is_some() || title.is_some() || body.is_some() {
        let endpoint = format!("repos/{repo_slug}/pulls/{number}");
        let mut args = vec![
            "api".to_string(),
            "--method".to_string(),
            "PATCH".to_string(),
            endpoint,
        ];
        if let Some(base) = base {
            args.push("-f".to_string());
            args.push(format!("base={base}"));
        }
        if let Some(title) = title {
            args.push("-f".to_string());
            args.push(format!("title={title}"));
        }
        if let Some(body) = body {
            args.push("-f".to_string());
            args.push(format!("body={body}"));
        }
        let output = run_gh_in("gh pr edit", Some(repo_path), &args)?;
        if !output.success() {
            return Err(io::Error::other(format!(
                "gh pr edit: {}",
                output.stderr.trim()
            )));
        }
    }

    if !add_labels.is_empty() {
        let endpoint = format!("repos/{repo_slug}/issues/{number}/labels");
        let mut args = vec![
            "api".to_string(),
            "--method".to_string(),
            "POST".to_string(),
            endpoint,
        ];
        for label in add_labels {
            args.push("-f".to_string());
            args.push(format!("labels[]={label}"));
        }
        let output = run_gh_in("gh pr edit add-label", Some(repo_path), &args)?;
        if !output.success() {
            return Err(io::Error::other(format!(
                "gh pr edit: {}",
                output.stderr.trim()
            )));
        }
    }

    gwt_git::pr_status::fetch_pr_status(repo_slug, number)
        .map_err(|err| io::Error::other(err.to_string()))
}

/// Close a PR without deleting its branch, recording any supplied comment first.
pub fn close_pr_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    number: u64,
    comment: Option<&str>,
) -> io::Result<PrStatus> {
    if let Some(comment) = comment {
        let number = number.to_string();
        let output = run_gh_in(
            "gh pr close comment",
            Some(repo_path),
            [
                "pr",
                "comment",
                number.as_str(),
                "--repo",
                repo_slug,
                "--body",
                comment,
            ],
        )?;
        if !output.success() {
            return Err(io::Error::other(format!(
                "gh pr close comment: {}",
                output.stderr.trim()
            )));
        }
    }
    let endpoint = format!("repos/{repo_slug}/pulls/{number}");
    let output = run_gh_in(
        "gh pr close",
        Some(repo_path),
        [
            "api",
            "--method",
            "PATCH",
            endpoint.as_str(),
            "-f",
            "state=closed",
        ],
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh pr close: {}",
            output.stderr.trim()
        )));
    }
    gwt_git::pr_status::fetch_pr_status(repo_slug, number)
        .map_err(|err| io::Error::other(err.to_string()))
}

/// Percent-encode a single URL path segment (RFC 3986 unreserved set kept
/// as-is) so label names with spaces or symbols stay one segment.
fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            other => {
                encoded.push_str(&format!("%{other:02X}"));
            }
        }
    }
    encoded
}

/// Resolve a PR's GraphQL node id via the REST API (`repo` scope only).
///
/// The `markPullRequestReadyForReview` / `convertPullRequestToDraft` mutations
/// require the PR node id; fetching it through
/// `GET /repos/{owner}/{repo}/pulls/{number}` avoids any `read:org` dependency.
fn fetch_pr_node_id_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    number: u64,
) -> io::Result<String> {
    let endpoint = format!("repos/{repo_slug}/pulls/{number}");
    let output = run_gh_in(
        "gh api pr node-id",
        Some(repo_path),
        ["api", endpoint.as_str()],
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh api {endpoint}: {}",
            output.stderr.trim()
        )));
    }
    let value: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    value
        .get("node_id")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| io::Error::other(format!("gh api {endpoint}: missing node_id")))
}

/// Promote a Draft PR to Ready-for-review through the GraphQL mutation
/// `markPullRequestReadyForReview` (Issue #3201). Only the `repo` scope is
/// required, so agents can complete the Draft→Ready step entirely through the
/// sanctioned `pr.ready` operation.
pub fn mark_pr_ready_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    number: u64,
) -> io::Result<PrStatus> {
    let node_id = fetch_pr_node_id_via_gh(repo_slug, repo_path, number)?;
    let mutation = r#"
mutation($id: ID!) {
  markPullRequestReadyForReview(input: { pullRequestId: $id }) {
    pullRequest { number isDraft }
  }
}
"#;
    let output = run_gh(
        "gh api graphql markPullRequestReadyForReview",
        [
            "api",
            "graphql",
            "-f",
            &format!("query={mutation}"),
            "-f",
            &format!("id={node_id}"),
        ],
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh api graphql markPullRequestReadyForReview: {}",
            output.stderr.trim()
        )));
    }
    gwt_git::pr_status::fetch_pr_status(repo_slug, number)
        .map_err(|err| io::Error::other(err.to_string()))
}

/// Convert a Ready PR back to Draft through the GraphQL mutation
/// `convertPullRequestToDraft` (Issue #3201) — the symmetric counterpart to
/// [`mark_pr_ready_via_gh`]. Only the `repo` scope is required.
pub fn convert_pr_to_draft_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    number: u64,
) -> io::Result<PrStatus> {
    let node_id = fetch_pr_node_id_via_gh(repo_slug, repo_path, number)?;
    let mutation = r#"
mutation($id: ID!) {
  convertPullRequestToDraft(input: { pullRequestId: $id }) {
    pullRequest { number isDraft }
  }
}
"#;
    let output = run_gh(
        "gh api graphql convertPullRequestToDraft",
        [
            "api",
            "graphql",
            "-f",
            &format!("query={mutation}"),
            "-f",
            &format!("id={node_id}"),
        ],
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh api graphql convertPullRequestToDraft: {}",
            output.stderr.trim()
        )));
    }
    gwt_git::pr_status::fetch_pr_status(repo_slug, number)
        .map_err(|err| io::Error::other(err.to_string()))
}

/// Whether a `updatePullRequestBranch` failure is GitHub saying the merge
/// would conflict, rather than the call itself breaking.
///
/// GitHub answers a conflicting update with an ordinary GraphQL error, so the
/// wording is the only signal available. Anything unrecognised stays an error:
/// a PM must never read an unknown failure as "conflict, owner's problem".
pub fn update_branch_failure_is_conflict(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("conflict") || message.contains("merge conflict")
}

/// Merge the base branch into the PR head through the GraphQL mutation
/// `updatePullRequestBranch` (SPEC #3835 AC-15). This is the PM's only way out
/// of `BEHIND`, and the one action `default_action` has been recommending
/// without an operation behind it.
///
/// A conflicting update is reported as
/// [`PrUpdateBranchOutcome::Conflicted`](super::types::PrUpdateBranchOutcome::Conflicted)
/// and pushes nothing: resolving conflicts stays the owner's work (FR-007).
pub fn update_pr_branch_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    number: u64,
) -> io::Result<super::types::PrUpdateBranchResult> {
    use super::types::{PrUpdateBranchOutcome, PrUpdateBranchResult};

    let node_id = fetch_pr_node_id_via_gh(repo_slug, repo_path, number)?;
    let mutation = r#"
mutation($id: ID!) {
  updatePullRequestBranch(input: { pullRequestId: $id }) {
    pullRequest { number }
  }
}
"#;
    let output = run_gh(
        "gh api graphql updatePullRequestBranch",
        [
            "api",
            "graphql",
            "-f",
            &format!("query={mutation}"),
            "-f",
            &format!("id={node_id}"),
        ],
    )?;
    if !output.success() {
        // `gh api graphql` reports a GraphQL-level error on stdout (the
        // `errors` array) and a transport-level one on stderr, and a
        // conflicting update is the former. Read both so a conflict is not
        // mistaken for a broken call.
        let detail = [output.stderr.trim(), output.stdout.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if update_branch_failure_is_conflict(&detail) {
            return Ok(PrUpdateBranchResult {
                number,
                outcome: PrUpdateBranchOutcome::Conflicted,
                detail,
            });
        }
        return Err(io::Error::other(format!(
            "gh api graphql updatePullRequestBranch: {detail}"
        )));
    }
    Ok(PrUpdateBranchResult {
        number,
        outcome: PrUpdateBranchOutcome::Updated,
        detail: String::new(),
    })
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

fn quarantine_body(value: &serde_json::Value, field: &str) -> io::Result<String> {
    match value.get(field) {
        Some(serde_json::Value::String(body)) => Ok(body.clone()),
        Some(serde_json::Value::Null) => Ok(String::new()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("GitHub response is missing a valid {field}"),
        )),
    }
}

fn parse_pr_quarantine_context(
    expected_number: u64,
    pr_json: &str,
    comments_json: &str,
) -> io::Result<PrQuarantineContext> {
    let pr: serde_json::Value = serde_json::from_str(pr_json)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let number = pr
        .get("number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "PR response is missing number")
        })?;
    if number != expected_number {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("PR response number #{number} does not match requested #{expected_number}"),
        ));
    }
    let body = quarantine_body(&pr, "body")?;
    let pages: Vec<Vec<serde_json::Value>> = serde_json::from_str(comments_json)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let comments = pages
        .into_iter()
        .flatten()
        .map(|value| {
            let id = value
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "PR comment response is missing durable id",
                    )
                })?;
            let body = quarantine_body(&value, "body")?;
            Ok(PrQuarantineComment { id, body })
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(PrQuarantineContext {
        number,
        body,
        comments,
    })
}

pub fn fetch_pr_quarantine_context_via_gh(
    owner: &str,
    repo: &str,
    repo_path: &Path,
    number: u64,
) -> io::Result<PrQuarantineContext> {
    let pr_endpoint = format!("repos/{owner}/{repo}/pulls/{number}");
    let pr = run_gh_in(
        &format!("gh api {pr_endpoint}"),
        Some(repo_path),
        ["api", pr_endpoint.as_str()],
    )?;
    if !pr.success() {
        return Err(io::Error::other(format!(
            "gh api {pr_endpoint}: {}",
            pr.stderr.trim()
        )));
    }

    let comments_endpoint = format!("repos/{owner}/{repo}/issues/{number}/comments?per_page=100");
    let comments = run_gh_in(
        &format!("gh api --paginate {comments_endpoint}"),
        Some(repo_path),
        ["api", "--paginate", "--slurp", comments_endpoint.as_str()],
    )?;
    if !comments.success() {
        return Err(io::Error::other(format!(
            "gh api {comments_endpoint}: {}",
            comments.stderr.trim()
        )));
    }

    parse_pr_quarantine_context(number, &pr.stdout, &comments.stdout)
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

    let checks = parse_pr_checks_items_response(
        &output.stdout,
        &output.stderr,
        output.success() || output.exit_code == Some(8),
    )?;
    // The details are fetched after the PR rollup: derive the headline from
    // this same snapshot instead of retaining an earlier green verdict.
    let rollup = serde_json::Value::Array(
        checks
            .iter()
            .map(|check| {
                let state = check.state.to_ascii_uppercase();
                let bucket = check.conclusion.to_ascii_lowercase();
                let conclusion = match bucket.as_str() {
                    // Modern gh exports a bucket alongside the actual state.
                    // Preserve shared semantics for NEUTRAL / EXPECTED / STALE.
                    "pass" | "fail" | "cancel" | "skipping" | "pending"
                        if !state.is_empty() && state != "COMPLETED" => None,
                    "pass" => Some("SUCCESS"),
                    "fail" => Some("FAILURE"),
                    "cancel" => Some("CANCELLED"),
                    "skipping" => Some("SKIPPED"),
                    "" => None,
                    _ => Some(check.conclusion.as_str()),
                };
                serde_json::json!({"status": check.state, "state": check.state, "conclusion": conclusion})
            })
            .collect(),
    );
    let check_counts = gwt_git::pr_status::check_counts_from_rollup(Some(&rollup));
    let unknown_required = serde_json::Value::Array(
        checks
            .iter()
            .zip(rollup.as_array().unwrap())
            .filter(|(check, _)| check.is_required.is_none())
            .map(|(_, node)| node.clone())
            .collect(),
    );
    let required_metadata_complete =
        gwt_git::pr_status::check_counts_from_rollup(Some(&unknown_required))
            .is_none_or(|counts| counts.in_progress == 0);
    let required_pending_count = (required_metadata_complete
        && checks.iter().any(|check| check.is_required.is_some()))
    .then(|| {
        let required = serde_json::Value::Array(
            checks
                .iter()
                .zip(rollup.as_array().unwrap())
                .filter(|(check, _)| check.is_required == Some(true))
                .map(|(_, node)| node.clone())
                .collect(),
        );
        gwt_git::pr_status::check_counts_from_rollup(Some(&required))
            .map_or(0, |counts| counts.in_progress)
    });
    let ci_status = check_counts
        .map_or("UNKNOWN", |counts| {
            if counts.in_progress > 0 {
                "PENDING"
            } else {
                counts.summary()
            }
        })
        .to_string();
    let mut ci_summary = ci_status.clone();
    if let Some(counts) = check_counts.filter(|counts| counts.in_progress > 0) {
        ci_summary.push_str(&format!(" ({} unfinished", counts.in_progress));
        if let Some(required) = required_pending_count {
            ci_summary.push_str(&format!(", {required} required"));
        }
        ci_summary.push(')');
    }
    let merge_status = pr.merge_state_status;

    Ok(PrChecksSummary {
        summary: format!(
            "PR #{} | CI: {} | Merge: {} | Review: {}",
            pr.number, ci_summary, merge_status, pr.review_status
        ),
        ci_status,
        merge_status,
        review_status: pr.review_status,
        checks,
        check_counts,
        required_pending_count,
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
    should_resolve_review_thread(thread)
        && !thread.is_outdated
        && !review_thread_has_comment_body(thread, body)
}

pub fn should_resolve_review_thread(thread: &PrReviewThread) -> bool {
    !thread.is_resolved
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
            is_required: value
                .get("isRequired")
                .or_else(|| value.get("is_required"))
                .and_then(serde_json::Value::as_bool),
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

#[cfg(test)]
mod tests {
    fn fork_repository(name: &str, source_id: u64) -> serde_json::Value {
        serde_json::json!({
            "id": 22,
            "full_name": format!("contributor/{name}"),
            "owner": { "login": "contributor" },
            "fork": true,
            "source": { "id": source_id },
            "clone_url": format!("https://github.com/contributor/{name}.git")
        })
    }

    #[test]
    fn fork_resolution_accepts_renamed_fork_in_exact_network() {
        let base = serde_json::json!({ "id": 7, "fork": true, "source": { "id": 1 } });
        let unrelated = fork_repository("gwt", 99);
        let renamed = fork_repository("my-renamed-fork", 1);
        assert_eq!(
            super::select_pr_fork_url(&base, &[unrelated, renamed], "contributor").unwrap(),
            "https://github.com/contributor/my-renamed-fork.git"
        );
    }

    #[test]
    fn fork_resolution_refuses_unrelated_same_name_repository() {
        let base = serde_json::json!({ "id": 1, "fork": false });
        assert!(
            super::select_pr_fork_url(&base, &[fork_repository("gwt", 99)], "contributor").is_err()
        );
    }

    #[test]
    fn fork_resolution_refuses_ambiguous_network_identity() {
        let base = serde_json::json!({ "id": 1, "fork": false });
        assert!(super::select_pr_fork_url(
            &base,
            &[fork_repository("gwt", 1), fork_repository("renamed", 1)],
            "contributor"
        )
        .is_err());
    }

    #[test]
    fn current_pr_fallback_rejects_foreign_fork() {
        crate::cli::test_support::with_fake_gh("foreign-fork-fallback", |repo| {
            for args in [
                vec!["init", "-b", "work/20260507-0808"],
                vec![
                    "remote",
                    "add",
                    "origin",
                    "https://github.com/akiojin/gwt.git",
                ],
            ] {
                assert!(gwt_core::process::hidden_command("git")
                    .args(args)
                    .current_dir(repo)
                    .status()
                    .unwrap()
                    .success());
            }
            assert!(super::fetch_current_pr_via_gh(repo).unwrap().is_none());
        });
    }

    use super::*;

    fn review_thread(is_resolved: bool, is_outdated: bool) -> PrReviewThread {
        PrReviewThread {
            id: "thread-1".to_string(),
            is_resolved,
            is_outdated,
            path: "src/lib.rs".to_string(),
            line: Some(12),
            comments: Vec::new(),
        }
    }

    /// SPEC #3835 AC-15: GitHub answers a conflicting update with an ordinary
    /// GraphQL error, so only the wording separates "the base would conflict"
    /// from "the call broke". An unrecognised failure stays an error: reading
    /// it as a conflict would tell the PM to relaunch an owner for a problem
    /// that is not theirs.
    #[test]
    fn only_a_conflict_message_is_read_as_a_conflict() {
        for message in [
            "merge conflict between base and head",
            "GraphQL: Merge conflict (updatePullRequestBranch)",
            "CONFLICT: cannot update branch",
        ] {
            assert!(
                update_branch_failure_is_conflict(message),
                "must be read as a conflict: {message}"
            );
        }
        for message in [
            "HTTP 401: Bad credentials",
            "GraphQL: Resource not accessible by integration",
            "could not resolve to a PullRequest",
            "",
        ] {
            assert!(
                !update_branch_failure_is_conflict(message),
                "must stay an error: {message}"
            );
        }
    }

    #[test]
    fn unresolved_outdated_review_threads_are_still_resolution_targets() {
        assert!(should_resolve_review_thread(&review_thread(false, true)));
        assert!(should_resolve_review_thread(&review_thread(false, false)));
        assert!(!should_resolve_review_thread(&review_thread(true, true)));
        assert!(!should_resolve_review_thread(&review_thread(true, false)));
    }

    #[test]
    fn quarantine_context_parser_flattens_every_comment_page() {
        let context = parse_pr_quarantine_context(
            42,
            r#"{"number":42,"body":"body marker"}"#,
            r#"[[{"id":1,"body":"first"}],[{"id":2,"body":"later marker"}]]"#,
        )
        .expect("parse paginated quarantine context");

        assert_eq!(context.number, 42);
        assert_eq!(context.body, "body marker");
        assert_eq!(context.comments.len(), 2);
        assert_eq!(context.comments[1].id, 2);
        assert_eq!(context.comments[1].body, "later marker");
    }

    #[test]
    fn quarantine_context_parser_rejects_incomplete_comment_identity() {
        let error = parse_pr_quarantine_context(
            42,
            r#"{"number":42,"body":null}"#,
            r#"[[{"body":"marker without durable id"}]]"#,
        )
        .expect_err("comment id must be present");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
