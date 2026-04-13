//! CLI dispatch for `gwt issue spec ...` subcommands.
//!
//! SPEC-12 Phase 6: when the gwt binary is invoked with arguments starting
//! with `issue`, we treat it as a CLI call rather than a TUI launch. This
//! module owns argv parsing, dispatches to the high-level SPEC operations in
//! `gwt-github`, and writes the result to stdout/stderr.
//!
//! Supported commands:
//!
//! - `gwt issue spec <n>` — print every section for an issue
//! - `gwt issue spec <n> --section <name>` — print one section only
//! - `gwt issue spec <n> --edit <name> -f <file>` — replace one section
//!   from a file (`-` means stdin)
//! - `gwt issue spec list [--phase <name>] [--state open|closed]` —
//!   list SPEC-labeled issues
//!
//! Missing (deferred to next cycle): `pull`, `create`, `repair`,
//! `migrate-specs`.

mod actions;
mod env;
pub mod hook;
mod issue;
mod pr;

use std::fs;
use std::io::{self};
use std::path::PathBuf;
use std::process::Command;

use gwt_git::PrStatus;
use gwt_github::{
    cache::write_atomic, ApiError, Cache, IssueClient, IssueNumber, IssueSnapshot, IssueState,
    SpecOpsError,
};

pub(crate) use env::ClientRef;
pub use env::{dispatch, CliEnv, DefaultCliEnv, TestEnv};

/// Compact linked PR summary used by `gwt issue linked-prs`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LinkedPrSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
}

/// Compact PR check entry used by `gwt pr checks`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrCheckItem {
    pub name: String,
    pub state: String,
    pub conclusion: String,
    pub url: String,
    pub started_at: String,
    pub completed_at: String,
    pub workflow: String,
}

/// Render-friendly aggregate used by `gwt pr checks`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrChecksSummary {
    pub summary: String,
    pub ci_status: String,
    pub merge_status: String,
    pub review_status: String,
    pub checks: Vec<PrCheckItem>,
}

/// PR review summary used by `gwt pr reviews`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReview {
    pub id: String,
    pub state: String,
    pub body: String,
    pub submitted_at: String,
    pub author: String,
}

/// Single comment inside a review thread.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReviewThreadComment {
    pub id: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub author: String,
}

/// Review thread snapshot used by `gwt pr review-threads`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReviewThread {
    pub id: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub path: String,
    pub line: Option<u64>,
    pub comments: Vec<PrReviewThreadComment>,
}

/// Test-visible log entry for `gwt pr create`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCreateCall {
    pub base: String,
    pub head: Option<String>,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub draft: bool,
}

/// Test-visible log entry for `gwt pr edit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrEditCall {
    pub number: u64,
    pub title: Option<String>,
    pub body: Option<String>,
    pub add_labels: Vec<String>,
}

/// Top-level argv parse result for the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    /// `gwt issue spec <n>` — print all sections.
    SpecReadAll { number: u64 },
    /// `gwt issue spec <n> --section <name>` — print a single section.
    SpecReadSection { number: u64, section: String },
    /// `gwt issue spec <n> --edit <name> -f <file>` — replace a section.
    SpecEditSection {
        number: u64,
        section: String,
        file: String,
    },
    /// `gwt issue spec list [--phase <name>] [--state open|closed]`.
    SpecList {
        phase: Option<String>,
        state: Option<String>,
    },
    /// `gwt issue spec create --title <t> -f <body_file> [--label <l>]*`.
    SpecCreate {
        title: String,
        file: String,
        labels: Vec<String>,
    },
    /// `gwt issue spec pull [--all | <n>...]` — refresh cache from server.
    SpecPull { all: bool, numbers: Vec<u64> },
    /// `gwt issue spec repair <n>` — clear cache and re-fetch from server.
    SpecRepair { number: u64 },
    /// `gwt issue view <n> [--refresh]` — print a plain issue from cache/live.
    IssueView { number: u64, refresh: bool },
    /// `gwt issue comments <n> [--refresh]` — print issue comments.
    IssueComments { number: u64, refresh: bool },
    /// `gwt issue linked-prs <n> [--refresh]` — print linked PR summaries.
    IssueLinkedPrs { number: u64, refresh: bool },
    /// `gwt issue create --title <t> -f <body_file> [--label <l>]*`.
    IssueCreate {
        title: String,
        file: String,
        labels: Vec<String>,
    },
    /// `gwt issue comment <n> -f <body_file>` — create a plain issue comment.
    IssueComment { number: u64, file: String },
    /// `gwt pr current` — print the PR associated with the current branch.
    PrCurrent,
    /// `gwt pr create --base <branch> [--head <branch>] --title <t> -f <body_file>`.
    PrCreate {
        base: String,
        head: Option<String>,
        title: String,
        file: String,
        labels: Vec<String>,
        draft: bool,
    },
    /// `gwt pr edit <n> [--title <t>] [-f <body_file>] [--add-label <label>]*`.
    PrEdit {
        number: u64,
        title: Option<String>,
        file: Option<String>,
        add_labels: Vec<String>,
    },
    /// `gwt pr view <n>` — print a PR by number.
    PrView { number: u64 },
    /// `gwt pr comment <n> -f <body_file>` — create a PR issue comment.
    PrComment { number: u64, file: String },
    /// `gwt pr reviews <n>` — print PR review summaries.
    PrReviews { number: u64 },
    /// `gwt pr review-threads <n>` — print review thread snapshots.
    PrReviewThreads { number: u64 },
    /// `gwt pr review-threads reply-and-resolve <n> -f <body_file>`.
    PrReviewThreadsReplyAndResolve { number: u64, file: String },
    /// `gwt pr checks <n>` — print PR checks and summary.
    PrChecks { number: u64 },
    /// `gwt actions logs --run <id>` — print raw GitHub Actions run logs.
    ActionsLogs { run_id: u64 },
    /// `gwt actions job-logs --job <id>` — print raw GitHub Actions job logs.
    ActionsJobLogs { job_id: u64 },
    /// `gwt hook <name> [args...]` — dispatch to an in-binary hook handler.
    ///
    /// See SPEC #1942 (CORE-CLI) — replaces `.claude/hooks/scripts/gwt-*.mjs`
    /// and inline shell hooks in `.claude/settings.local.json`.
    Hook { name: String, rest: Vec<String> },
}

/// Errors surfaced by argv parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliParseError {
    Usage,
    InvalidNumber(String),
    MissingFlag(&'static str),
    UnknownSubcommand(String),
}

impl std::fmt::Display for CliParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliParseError::Usage => write!(
                f,
                "usage: gwt issue spec <n> [--section <name>|--edit <name> -f <file>] | gwt issue spec list [--phase <p>] [--state open|closed] | gwt issue view|comments|linked-prs <n> [--refresh] | gwt issue create --title <t> -f <file> [--label <l>]* | gwt issue comment <n> -f <file> | gwt pr current|create --base <b> [--head <h>] --title <t> -f <file> [--label <l>]* [--draft]|edit <n> [--title <t>] [-f <file>] [--add-label <l>]*|view <n>|comment <n> -f <file>|reviews <n>|review-threads <n>|review-threads reply-and-resolve <n> -f <file>|checks <n> | gwt actions logs --run <id> | gwt actions job-logs --job <id>"
            ),
            CliParseError::InvalidNumber(s) => write!(f, "invalid issue number: {s}"),
            CliParseError::MissingFlag(flag) => write!(f, "missing required flag: {flag}"),
            CliParseError::UnknownSubcommand(s) => write!(f, "unknown subcommand: {s}"),
        }
    }
}

impl std::error::Error for CliParseError {}

/// Determine whether the given argv (starting at the program name) should be
/// handled as a CLI invocation. Returns `true` when argv[1..] begins with
/// `issue`, `pr`, `actions`, or `hook`. The TUI launcher keeps its legacy
/// behaviour (positional repo path) for any other shape.
pub fn should_dispatch_cli(args: &[String]) -> bool {
    args.get(1)
        .map(|s| matches!(s.as_str(), "issue" | "pr" | "actions" | "hook"))
        .unwrap_or(false)
}

/// Parse an argv slice into a [`CliCommand`]. The slice should start from
/// the first post-subcommand argument — i.e. if the caller received
/// `["gwt", "issue", "spec", "2001"]`, they pass `["spec", "2001"]`.
pub fn parse_issue_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    issue::parse(args)
}

/// Parse an argv slice into a `gwt pr ...` [`CliCommand`].
pub fn parse_pr_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    pr::parse(args)
}

/// Parse an argv slice into a `gwt actions ...` [`CliCommand`].
pub fn parse_actions_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    actions::parse(args)
}

fn expect_flag(arg: Option<&String>, expected: &'static str) -> Result<(), CliParseError> {
    match arg.map(String::as_str) {
        Some(flag) if flag == expected => Ok(()),
        Some(flag) => Err(CliParseError::UnknownSubcommand(flag.to_string())),
        None => Err(CliParseError::MissingFlag(expected)),
    }
}

fn parse_required_number(arg: Option<&String>) -> Result<u64, CliParseError> {
    let value = arg.ok_or(CliParseError::Usage)?;
    value
        .parse()
        .map_err(|_| CliParseError::InvalidNumber(value.clone()))
}

fn ensure_no_remaining_args<'a>(
    mut args: impl Iterator<Item = &'a String>,
) -> Result<(), CliParseError> {
    if args.next().is_some() {
        return Err(CliParseError::Usage);
    }
    Ok(())
}

/// Parse the tail of a `gwt hook ...` argv slice into a [`CliCommand::Hook`].
///
/// SPEC #1942 (CORE-CLI): `gwt hook <name> [args...]` is the single entry
/// point for every in-binary hook handler. The known hook names are:
///
/// - `runtime-state <event>`
/// - `block-bash-policy`
/// - `forward <target>`
///
/// Unknown names still parse (we don't maintain an allowlist here) so that
/// newly added hooks don't need parser changes. Validation happens in
/// [`run_hook`].
pub fn parse_hook_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    Ok(CliCommand::Hook {
        name: head.clone(),
        rest: rest.to_vec(),
    })
}

/// Dispatch a parsed [`CliCommand`] against the given [`CliEnv`].
///
/// We collect output into a String buffer first so the [`SpecOps`] borrow of
/// `env.client()` does not conflict with the mutable borrow required by
/// `env.stdout()` at write time.
pub fn run<E: CliEnv>(env: &mut E, cmd: CliCommand) -> Result<i32, SpecOpsError> {
    let mut out = String::new();
    let code = match cmd {
        cmd @ (CliCommand::SpecReadAll { .. }
        | CliCommand::SpecReadSection { .. }
        | CliCommand::SpecEditSection { .. }
        | CliCommand::SpecList { .. }
        | CliCommand::SpecCreate { .. }
        | CliCommand::SpecPull { .. }
        | CliCommand::SpecRepair { .. }
        | CliCommand::IssueView { .. }
        | CliCommand::IssueComments { .. }
        | CliCommand::IssueLinkedPrs { .. }
        | CliCommand::IssueCreate { .. }
        | CliCommand::IssueComment { .. }) => issue::run(env, cmd, &mut out)?,
        cmd @ (CliCommand::PrCurrent
        | CliCommand::PrCreate { .. }
        | CliCommand::PrEdit { .. }
        | CliCommand::PrView { .. }
        | CliCommand::PrComment { .. }
        | CliCommand::PrReviews { .. }
        | CliCommand::PrReviewThreads { .. }
        | CliCommand::PrReviewThreadsReplyAndResolve { .. }
        | CliCommand::PrChecks { .. }) => pr::run(env, cmd, &mut out)?,
        cmd @ (CliCommand::ActionsLogs { .. } | CliCommand::ActionsJobLogs { .. }) => {
            actions::run(env, cmd, &mut out)?
        }
        CliCommand::Hook { name, rest } => {
            return run_hook(env, &name, &rest);
        }
    };
    let _ = env.stdout().write_all(out.as_bytes());
    Ok(code)
}

fn io_as_api_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

fn issue_state_label(state: IssueState) -> &'static str {
    match state {
        IssueState::Open => "OPEN",
        IssueState::Closed => "CLOSED",
    }
}

fn render_issue(out: &mut String, snapshot: &IssueSnapshot) {
    out.push_str(&format!(
        "#{} [{}] {}\n",
        snapshot.number.0,
        issue_state_label(snapshot.state),
        snapshot.title
    ));
    if !snapshot.labels.is_empty() {
        out.push_str(&format!("labels: {}\n", snapshot.labels.join(", ")));
    }
    out.push_str(&format!("updated_at: {}\n\n", snapshot.updated_at.0));
    if !snapshot.body.is_empty() {
        out.push_str(snapshot.body.trim_end_matches('\n'));
        out.push('\n');
    }
}

fn render_issue_comments(out: &mut String, snapshot: &IssueSnapshot) {
    if snapshot.comments.is_empty() {
        out.push_str("no comments\n");
        return;
    }
    for comment in &snapshot.comments {
        out.push_str(&format!(
            "=== comment:{} ({}) ===\n{}\n",
            comment.id.0, comment.updated_at.0, comment.body
        ));
    }
}

fn render_linked_prs(out: &mut String, linked_prs: &[LinkedPrSummary]) {
    if linked_prs.is_empty() {
        out.push_str("no linked pull requests\n");
        return;
    }
    for pr in linked_prs {
        out.push_str(&format!(
            "#{} [{}] {}\n{}\n",
            pr.number, pr.state, pr.title, pr.url
        ));
    }
}

fn render_pr(out: &mut String, pr: &PrStatus) {
    out.push_str(&format!("#{} [{}] {}\n", pr.number, pr.state, pr.title));
    out.push_str(&format!("url: {}\n", pr.url));
    out.push_str(&format!("ci: {}\n", pr.ci_status));
    out.push_str(&format!("mergeable: {}\n", pr.mergeable));
    out.push_str(&format!("review: {}\n", pr.review_status));
}

fn render_pr_checks(out: &mut String, summary: &PrChecksSummary) {
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

fn render_pr_reviews(out: &mut String, reviews: &[PrReview]) {
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

fn render_pr_review_threads(out: &mut String, threads: &[PrReviewThread]) {
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

fn load_or_refresh_issue<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
) -> Result<gwt_github::CacheEntry, SpecOpsError> {
    let cache = Cache::new(env.cache_root());
    if !refresh {
        if let Some(entry) = cache.load_entry(number) {
            return Ok(entry);
        }
    }
    refresh_issue_cache(env, number)
}

fn refresh_issue_cache<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
) -> Result<gwt_github::CacheEntry, SpecOpsError> {
    let snapshot = match env.client().fetch(number, None)? {
        gwt_github::FetchResult::Updated(snapshot) => snapshot,
        gwt_github::FetchResult::NotModified => {
            return Cache::new(env.cache_root())
                .load_entry(number)
                .ok_or_else(|| SpecOpsError::SectionNotFound(format!("issue {}", number.0)));
        }
    };
    let cache = Cache::new(env.cache_root());
    cache.write_snapshot(&snapshot)?;
    cache
        .load_entry(number)
        .ok_or_else(|| SpecOpsError::SectionNotFound(format!("issue {}", number.0)))
}

fn load_or_refresh_linked_prs<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
) -> Result<Vec<LinkedPrSummary>, SpecOpsError> {
    let cache_root = env.cache_root();
    if !refresh {
        if let Some(cached) = read_linked_prs_cache(&cache_root, number)? {
            return Ok(cached);
        }
    }
    let linked_prs = env.fetch_linked_prs(number).map_err(io_as_api_error)?;
    write_linked_prs_cache(&cache_root, number, &linked_prs)?;
    Ok(linked_prs)
}

fn linked_prs_cache_path(cache_root: &std::path::Path, number: IssueNumber) -> PathBuf {
    cache_root
        .join(number.0.to_string())
        .join("linked_prs.json")
}

fn read_linked_prs_cache(
    cache_root: &std::path::Path,
    number: IssueNumber,
) -> Result<Option<Vec<LinkedPrSummary>>, SpecOpsError> {
    let path = linked_prs_cache_path(cache_root, number);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let parsed = serde_json::from_str(&text)
                .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
            Ok(Some(parsed))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(io_as_api_error(err)),
    }
}

fn write_linked_prs_cache(
    cache_root: &std::path::Path,
    number: IssueNumber,
    linked_prs: &[LinkedPrSummary],
) -> Result<(), SpecOpsError> {
    let bytes = serde_json::to_vec_pretty(linked_prs)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
    write_atomic(&linked_prs_cache_path(cache_root, number), &bytes).map_err(io_as_api_error)
}

fn fetch_linked_prs_via_gh(
    owner: &str,
    repo: &str,
    number: IssueNumber,
) -> io::Result<Vec<LinkedPrSummary>> {
    let query = r#"
query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      timelineItems(first: 100, itemTypes: [CROSS_REFERENCED_EVENT, CONNECTED_EVENT]) {
        nodes {
          __typename
          ... on CrossReferencedEvent {
            source {
              __typename
              ... on PullRequest {
                number
                title
                state
                url
              }
            }
          }
          ... on ConnectedEvent {
            subject {
              __typename
              ... on PullRequest {
                number
                title
                state
                url
              }
            }
          }
        }
      }
    }
  }
}
"#;

    let output = Command::new("gh")
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
            &format!("number={}", number.0),
        ])
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh api graphql failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    let nodes = value
        .get("data")
        .and_then(|v| v.get("repository"))
        .and_then(|v| v.get("issue"))
        .and_then(|v| v.get("timelineItems"))
        .and_then(|v| v.get("nodes"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for node in nodes {
        let typename = node
            .get("__typename")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let pr = match typename {
            "CrossReferencedEvent" => node.get("source"),
            "ConnectedEvent" => node.get("subject"),
            _ => None,
        };
        let Some(pr) = pr else { continue };
        if pr.get("__typename").and_then(|v| v.as_str()) != Some("PullRequest") {
            continue;
        }
        let Some(pr_number) = pr.get("number").and_then(|v| v.as_u64()) else {
            continue;
        };
        if !seen.insert(pr_number) {
            continue;
        }
        out.push(LinkedPrSummary {
            number: pr_number,
            title: pr
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            state: pr
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            url: pr
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(out)
}

fn fetch_current_pr_via_gh(repo_path: &std::path::Path) -> io::Result<Option<PrStatus>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            "--json",
            "number,title,state,url,mergeable,statusCheckRollup,reviewDecision",
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

fn create_pr_via_gh(
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

    let output = Command::new("gh")
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

fn edit_pr_via_gh(
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

    let output = Command::new("gh")
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

fn extract_pr_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("https://"))
        .map(ToOwned::to_owned)
}

fn parse_pr_number_from_url(url: &str) -> Option<u64> {
    url.trim_end_matches('/').rsplit('/').next()?.parse().ok()
}

fn comment_on_pr_via_gh(repo_path: &std::path::Path, number: u64, body: &str) -> io::Result<()> {
    let output = Command::new("gh")
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

fn fetch_pr_reviews_via_gh(owner: &str, repo: &str, number: u64) -> io::Result<Vec<PrReview>> {
    let endpoint = format!("repos/{owner}/{repo}/pulls/{number}/reviews");
    let output = Command::new("gh").args(["api", &endpoint]).output()?;
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

fn fetch_pr_review_threads_via_gh(
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
    let output = Command::new("gh")
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

fn reply_and_resolve_pr_review_threads_via_gh(
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
            let reply = Command::new("gh")
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
        let resolve = Command::new("gh")
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

fn fetch_pr_checks_via_gh(
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
    let mut output = Command::new("gh")
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
                output = Command::new("gh")
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

    Ok(PrChecksSummary {
        summary: format!(
            "PR #{} | CI: {} | Merge: {} | Review: {}",
            pr.number, pr.ci_status, pr.mergeable, pr.review_status
        ),
        ci_status: pr.ci_status,
        merge_status: pr.mergeable,
        review_status: pr.review_status,
        checks,
    })
}

fn fetch_pr_review_thread_state_via_gh(
    owner: &str,
    repo: &str,
    number: u64,
    thread_id: &str,
) -> io::Result<Option<PrReviewThread>> {
    Ok(fetch_pr_review_threads_via_gh(owner, repo, number)?
        .into_iter()
        .find(|thread| thread.id == thread_id))
}

fn review_thread_has_comment_body(thread: &PrReviewThread, body: &str) -> bool {
    thread.comments.iter().any(|comment| comment.body == body)
}

fn should_reply_to_review_thread(thread: &PrReviewThread, body: &str) -> bool {
    should_resolve_review_thread(thread) && !review_thread_has_comment_body(thread, body)
}

fn should_resolve_review_thread(thread: &PrReviewThread) -> bool {
    !thread.is_resolved && !thread.is_outdated
}

fn parse_pr_checks_items_json(json: &str) -> Result<Vec<PrCheckItem>, serde_json::Error> {
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

fn parse_pr_checks_items_response(
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

fn parse_available_fields(message: &str) -> Vec<String> {
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

fn fetch_actions_run_log_via_gh(repo_path: &std::path::Path, run_id: u64) -> io::Result<String> {
    let output = Command::new("gh")
        .args(["run", "view", &run_id.to_string(), "--log"])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh run view --log: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn fetch_actions_job_log_via_gh(
    owner: &str,
    repo: &str,
    repo_path: &std::path::Path,
    job_id: u64,
) -> io::Result<String> {
    let endpoint = format!("/repos/{owner}/{repo}/actions/jobs/{job_id}/logs");
    let output = Command::new("gh")
        .args(["api", &endpoint])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh api {endpoint}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    if output.stdout.starts_with(b"PK") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "job logs returned a zip archive; unable to parse",
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Dispatch a `gwt hook <name> [args...]` invocation.
///
/// SPEC #1942 (CORE-CLI) scope: this is the single entry point for every
/// in-binary hook handler. Each handler reads stdin (usually JSON), performs
/// its judgment, and either:
///
/// - exits 0 (allow / success, stdout empty or a human-readable status)
/// - exits 2 with a `{"decision":"block",...}` JSON on stdout (block)
///
/// Dispatches to the hook handlers in [`crate::cli::hook`]. Unknown hooks
/// exit 2 with a `gwt hook: unknown hook '<name>'` message on stderr so
/// that settings_local typos surface loudly. Runtime errors from known
/// handlers exit 1 with the error chain on stderr; they are never turned
/// into `decision=block` to avoid false positives under partial outages.
pub fn run_hook<E: CliEnv>(env: &mut E, name: &str, rest: &[String]) -> Result<i32, SpecOpsError> {
    use crate::cli::hook::{block_bash_policy, forward, runtime_state, BlockDecision, HookKind};

    let Some(kind) = HookKind::from_name(name) else {
        let _ = writeln!(env.stderr(), "gwt hook: unknown hook '{name}'");
        return Ok(2);
    };

    // Every block hook returns `Result<Option<BlockDecision>, HookError>`.
    // `emit_block_decision` serializes a decision to stdout and yields
    // the block exit code (2). `emit_hook_error` reports a handler
    // error on stderr and yields 1.
    fn emit_block_decision<E: CliEnv>(env: &mut E, decision: &BlockDecision) -> i32 {
        match serde_json::to_vec(decision) {
            Ok(bytes) => {
                let _ = env.stdout().write_all(&bytes);
                let _ = env.stdout().flush();
                2
            }
            Err(err) => {
                let _ = writeln!(
                    env.stderr(),
                    "gwt hook: failed to serialize decision: {err}"
                );
                1
            }
        }
    }
    fn emit_hook_error<E: CliEnv>(env: &mut E, name: &str, err: impl std::fmt::Display) -> i32 {
        let _ = writeln!(env.stderr(), "gwt hook {name}: {err}");
        1
    }

    match kind {
        HookKind::RuntimeState => {
            let Some(event) = rest.first() else {
                let _ = writeln!(
                    env.stderr(),
                    "gwt hook runtime-state: missing <event> argument"
                );
                return Ok(2);
            };
            match runtime_state::handle(event) {
                Ok(()) => Ok(0),
                Err(err) => Ok(emit_hook_error(env, name, err)),
            }
        }
        HookKind::BlockBashPolicy => match block_bash_policy::handle() {
            Ok(None) => Ok(0),
            Ok(Some(decision)) => Ok(emit_block_decision(env, &decision)),
            Err(err) => Ok(emit_hook_error(env, name, err)),
        },
        HookKind::Forward => match forward::handle() {
            Ok(()) => Ok(0),
            Err(err) => Ok(emit_hook_error(env, name, err)),
        },
    }
}

fn edit_or_create_repo_guard(owner: &str, repo: &str) -> io::Result<()> {
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

    fn sample_thread() -> PrReviewThread {
        PrReviewThread {
            id: "thread-1".to_string(),
            is_resolved: false,
            is_outdated: false,
            path: "src/lib.rs".to_string(),
            line: Some(12),
            comments: vec![],
        }
    }

    #[test]
    fn review_thread_reply_is_skipped_for_duplicate_body() {
        let mut thread = sample_thread();
        thread.comments.push(PrReviewThreadComment {
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
}
