//! CLI dispatch for `gwtd issue spec ...` subcommands.
//!
//! SPEC-12 Phase 6: when the gwt binary is invoked with arguments starting
//! with `issue`, we treat it as a CLI call rather than a GUI launch. This
//! module owns argv parsing, dispatches to the high-level SPEC operations in
//! `gwt-github`, and writes the result to stdout/stderr.
//!
//! Supported commands:
//!
//! - `gwtd issue spec <n>` — print every section for an issue
//! - `gwtd issue spec <n> --section <name>` — print one section only
//! - `gwtd issue spec <n> --edit <name> -f <file>` — replace one section
//!   from a file (`-` means stdin)
//! - `gwtd issue spec <n> --edit spec --json [-f <file>] [--replace]` —
//!   structured JSON update for the spec section
//! - `gwtd issue spec list [--phase <name>] [--state open|closed]` —
//!   list SPEC-labeled issues
//! - `gwtd issue spec create --json --title <t> [-f <file>]` —
//!   create a SPEC from structured JSON
//! - `gwtd issue spec <n> --rename <title>` — rename the Issue title

mod actions;
mod board;
mod build;
mod discuss;
mod env;
pub mod hook;
mod index;
mod issue;
mod issue_spec;
mod plan;
mod pr;
mod skill_state_runtime;
pub mod update;

use std::{
    fs,
    io::{self},
    path::PathBuf,
    process::Command,
};

pub(crate) use board::parse as parse_board_args;
pub(crate) use env::ClientRef;
pub use env::{dispatch, CliEnv, DefaultCliEnv, TestEnv};
use gwt_git::PrStatus;
use gwt_github::{
    cache::write_atomic, ApiError, Cache, IssueClient, IssueNumber, IssueSnapshot, IssueState,
    SpecOpsError,
};
pub(crate) use index::parse as parse_index_args;

/// Compact linked PR summary used by `gwtd issue linked-prs`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LinkedPrSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
}

/// Compact PR check entry used by `gwtd pr checks`.
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

/// Render-friendly aggregate used by `gwtd pr checks`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrChecksSummary {
    pub summary: String,
    pub ci_status: String,
    pub merge_status: String,
    pub review_status: String,
    pub checks: Vec<PrCheckItem>,
}

/// PR review summary used by `gwtd pr reviews`.
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

/// Review thread snapshot used by `gwtd pr review-threads`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReviewThread {
    pub id: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub path: String,
    pub line: Option<u64>,
    pub comments: Vec<PrReviewThreadComment>,
}

/// Test-visible log entry for `gwtd pr create`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCreateCall {
    pub base: String,
    pub head: Option<String>,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub draft: bool,
}

/// Test-visible log entry for `gwtd pr edit`.
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
    /// `gwtd issue spec <n>` — print all sections.
    SpecReadAll { number: u64 },
    /// `gwtd issue spec <n> --section <name>` — print a single section.
    SpecReadSection { number: u64, section: String },
    /// `gwtd issue spec <n> --edit <name> -f <file>` — replace a section.
    SpecEditSection {
        number: u64,
        section: String,
        file: String,
    },
    /// `gwtd issue spec <n> --edit spec --json [-f <file>] [--replace]`.
    SpecEditSectionJson {
        number: u64,
        section: String,
        file: Option<String>,
        replace: bool,
    },
    /// `gwtd issue spec list [--phase <name>] [--state open|closed]`.
    SpecList {
        phase: Option<String>,
        state: Option<String>,
    },
    /// `gwtd issue spec create --title <t> -f <body_file> [--label <l>]*`.
    SpecCreate {
        title: String,
        file: String,
        labels: Vec<String>,
    },
    /// `gwtd issue spec create --json --title <t> [-f <file>] [--label <l>]*`.
    SpecCreateJson {
        title: String,
        file: Option<String>,
        labels: Vec<String>,
    },
    /// `gwtd issue spec create --help`.
    SpecCreateHelp,
    /// `gwtd issue spec pull [--all | <n>...]` — refresh cache from server.
    SpecPull { all: bool, numbers: Vec<u64> },
    /// `gwtd issue spec repair <n>` — clear cache and re-fetch from server.
    SpecRepair { number: u64 },
    /// `gwtd issue spec <n> --rename <title>` — update the Issue title.
    SpecRename { number: u64, title: String },
    /// `gwtd issue view <n> [--refresh]` — print a plain issue from cache/live.
    IssueView { number: u64, refresh: bool },
    /// `gwtd issue comments <n> [--refresh]` — print issue comments.
    IssueComments { number: u64, refresh: bool },
    /// `gwtd issue linked-prs <n> [--refresh]` — print linked PR summaries.
    IssueLinkedPrs { number: u64, refresh: bool },
    /// `gwtd issue create --title <t> -f <body_file> [--label <l>]*`.
    IssueCreate {
        title: String,
        file: String,
        labels: Vec<String>,
    },
    /// `gwtd issue comment <n> -f <body_file>` — create a plain issue comment.
    IssueComment { number: u64, file: String },
    /// `gwtd pr current` — print the PR associated with the current branch.
    PrCurrent,
    /// `gwtd pr create --base <branch> [--head <branch>] --title <t> -f <body_file>`.
    PrCreate {
        base: String,
        head: Option<String>,
        title: String,
        file: String,
        labels: Vec<String>,
        draft: bool,
    },
    /// `gwtd pr edit <n> [--title <t>] [-f <body_file>] [--add-label <label>]*`.
    PrEdit {
        number: u64,
        title: Option<String>,
        file: Option<String>,
        add_labels: Vec<String>,
    },
    /// `gwtd pr view <n>` — print a PR by number.
    PrView { number: u64 },
    /// `gwtd pr comment <n> -f <body_file>` — create a PR issue comment.
    PrComment { number: u64, file: String },
    /// `gwtd pr reviews <n>` — print PR review summaries.
    PrReviews { number: u64 },
    /// `gwtd pr review-threads <n>` — print review thread snapshots.
    PrReviewThreads { number: u64 },
    /// `gwtd pr review-threads reply-and-resolve <n> -f <body_file>`.
    PrReviewThreadsReplyAndResolve { number: u64, file: String },
    /// `gwtd pr checks <n>` — print PR checks and summary.
    PrChecks { number: u64 },
    /// `gwtd actions logs --run <id>` — print raw GitHub Actions run logs.
    ActionsLogs { run_id: u64 },
    /// `gwtd actions job-logs --job <id>` — print raw GitHub Actions job logs.
    ActionsJobLogs { job_id: u64 },
    /// `gwtd board show [--json]`.
    BoardShow { json: bool },
    /// `gwtd board post --kind <kind> (--body <text> | -f <file>)`.
    BoardPost {
        kind: String,
        body: Option<String>,
        file: Option<String>,
        parent: Option<String>,
        topics: Vec<String>,
        owners: Vec<String>,
    },
    /// `gwtd index status`.
    IndexStatus,
    /// `gwtd index rebuild [--scope <scope>]`.
    IndexRebuild { scope: IndexScope },
    /// `gwtd hook <name> [args...]` — dispatch to an in-binary hook handler.
    ///
    /// See SPEC #1942 (CORE-CLI) — replaces retired external hook scripts
    /// and inline shell hooks in `.claude/settings.local.json`.
    Hook { name: String, rest: Vec<String> },
    /// `gwtd update [--check]` — check for a new gwt release and optionally apply it.
    Update { check_only: bool },
    /// `gwtd __internal apply-update ...` — internal helper: replace the binary then restart.
    InternalApplyUpdate { rest: Vec<String> },
    /// `gwtd __internal run-installer ...` — internal helper: run DMG/MSI installer then restart.
    InternalRunInstaller { rest: Vec<String> },
    /// `gwtd __internal daemon-hook <name> [args...]` — hidden helper used by the front door.
    InternalDaemonHook { name: String, rest: Vec<String> },
    /// `gwtd discuss <resolve|park|reject|clear-next-question> --proposal <label>`.
    Discuss(DiscussAction),
    /// `gwtd plan <start|phase|complete|abort> --spec <n> [...]`.
    Plan(SkillStateAction),
    /// `gwtd build <start|phase|complete|abort> --spec <n> [...]`.
    Build(SkillStateAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexScope {
    All,
    Issues,
    Specs,
    Files,
    FilesDocs,
}

/// Sub-action for `gwtd discuss ...` (SPEC-1935 FR-014p).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscussAction {
    Resolve { proposal: String },
    Park { proposal: String },
    Reject { proposal: String },
    ClearNextQuestion { proposal: String },
}

/// Sub-action for `gwtd plan ...` / `gwtd build ...` (SPEC-1935 FR-014q/r).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillStateAction {
    Start { spec: u64 },
    Phase { spec: u64, label: String },
    Complete { spec: u64 },
    Abort { spec: u64, reason: Option<String> },
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
                "usage: gwtd issue spec <n> [--section <name>|--rename <title>|--edit <name> (-f <file>|--json [-f <file>] [--replace])] | gwtd issue spec list [--phase <p>] [--state open|closed] | gwtd issue spec create (--title <t> -f <file> | --json --title <t> [-f <file>] | --help) [--label <l>]* | gwtd issue view|comments|linked-prs <n> [--refresh] | gwtd issue create --title <t> -f <file> [--label <l>]* | gwtd issue comment <n> -f <file> | gwtd pr current|create --base <b> [--head <h>] --title <t> -f <file> [--label <l>]* [--draft]|edit <n> [--title <t>] [-f <file>] [--add-label <l>]*|view <n>|comment <n> -f <file>|reviews <n>|review-threads <n>|review-threads reply-and-resolve <n> -f <file>|checks <n> | gwtd actions logs --run <id> | gwtd actions job-logs --job <id> | gwtd board show [--json] | gwtd board post --kind <kind> (--body <text> | -f <file>) [--parent <id>] [--topic <t>]* [--owner <n>]* | gwtd index status|rebuild [--scope all|issues|specs|files|files-docs]"
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
/// a supported top-level CLI verb such as `issue`, `pr`, `actions`, `board`,
/// `hook`, `discuss`, `plan`, `build`, `update`, or `__internal`. The GUI
/// launcher keeps its legacy behaviour (positional repo path) for any other
/// shape.
pub fn should_dispatch_cli(args: &[String]) -> bool {
    args.get(1)
        .map(|s| {
            matches!(
                s.as_str(),
                "issue"
                    | "pr"
                    | "actions"
                    | "board"
                    | "hook"
                    | "update"
                    | "__internal"
                    | "index"
                    | "discuss"
                    | "plan"
                    | "build"
            )
        })
        .unwrap_or(false)
}

/// Parse an argv slice into a [`CliCommand`]. The slice should start from
/// the first post-subcommand argument — i.e. if the caller received
/// `["gwt", "issue", "spec", "2001"]`, they pass `["spec", "2001"]`.
pub fn parse_issue_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    issue::parse(args)
}

/// Parse an argv slice into a `gwtd pr ...` [`CliCommand`].
pub fn parse_pr_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    pr::parse(args)
}

/// Parse an argv slice into a `gwtd actions ...` [`CliCommand`].
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

/// Parse the tail of a `gwtd hook ...` argv slice into a [`CliCommand::Hook`].
///
/// SPEC #1942 (CORE-CLI): `gwtd hook <name> [args...]` is the single entry
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

/// Parse `gwtd discuss <action> --proposal <label>` (SPEC-1935 FR-014p).
pub fn parse_discuss_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    let proposal = parse_named_string(rest, "--proposal")?;
    let action = match head.as_str() {
        "resolve" => DiscussAction::Resolve { proposal },
        "park" => DiscussAction::Park { proposal },
        "reject" => DiscussAction::Reject { proposal },
        "clear-next-question" => DiscussAction::ClearNextQuestion { proposal },
        other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
    };
    Ok(CliCommand::Discuss(action))
}

/// Parse `gwtd plan <action> --spec <n> [...]` (SPEC-1935 FR-014q).
pub fn parse_plan_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    parse_skill_state_args(args).map(CliCommand::Plan)
}

/// Parse `gwtd build <action> --spec <n> [...]` (SPEC-1935 FR-014r).
pub fn parse_build_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    parse_skill_state_args(args).map(CliCommand::Build)
}

fn parse_skill_state_args(args: &[String]) -> Result<SkillStateAction, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    match head.as_str() {
        "start" => {
            let spec = parse_named_u64(rest, "--spec")?;
            Ok(SkillStateAction::Start { spec })
        }
        "phase" => {
            let spec = parse_named_u64(rest, "--spec")?;
            let label = parse_named_string(rest, "--label")?;
            Ok(SkillStateAction::Phase { spec, label })
        }
        "complete" => {
            let spec = parse_named_u64(rest, "--spec")?;
            Ok(SkillStateAction::Complete { spec })
        }
        "abort" => {
            let spec = parse_named_u64(rest, "--spec")?;
            let reason = parse_optional_named_string(rest, "--reason");
            Ok(SkillStateAction::Abort { spec, reason })
        }
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn parse_named_string(args: &[String], flag: &'static str) -> Result<String, CliParseError> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag {
            let value = args.get(i + 1).ok_or(CliParseError::MissingFlag(flag))?;
            return Ok(value.clone());
        }
        i += 1;
    }
    Err(CliParseError::MissingFlag(flag))
}

fn parse_optional_named_string(args: &[String], flag: &'static str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag {
            return args.get(i + 1).cloned();
        }
        i += 1;
    }
    None
}

fn parse_named_u64(args: &[String], flag: &'static str) -> Result<u64, CliParseError> {
    let raw = parse_named_string(args, flag)?;
    raw.parse().map_err(|_| CliParseError::InvalidNumber(raw))
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
        | CliCommand::SpecEditSectionJson { .. }
        | CliCommand::SpecList { .. }
        | CliCommand::SpecCreate { .. }
        | CliCommand::SpecCreateJson { .. }
        | CliCommand::SpecCreateHelp
        | CliCommand::SpecPull { .. }
        | CliCommand::SpecRepair { .. }
        | CliCommand::SpecRename { .. }
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
        cmd @ (CliCommand::BoardShow { .. } | CliCommand::BoardPost { .. }) => {
            board::run(env, cmd, &mut out)?
        }
        cmd @ (CliCommand::IndexStatus | CliCommand::IndexRebuild { .. }) => {
            index::run(env, cmd, &mut out)?
        }
        CliCommand::Discuss(action) => discuss::run(env, action, &mut out)?,
        CliCommand::Plan(action) => plan::run(env, action, &mut out)?,
        CliCommand::Build(action) => build::run(env, action, &mut out)?,
        CliCommand::Hook { name, rest } => {
            return run_hook(env, &name, &rest);
        }
        CliCommand::InternalDaemonHook { name, rest } => {
            return run_daemon_hook(env, &name, &rest);
        }
        CliCommand::Update { check_only } => {
            let cmd = if check_only {
                update::UpdateCommand::CheckOnly
            } else {
                update::UpdateCommand::Apply
            };
            std::process::exit(update::run(cmd));
        }
        CliCommand::InternalApplyUpdate { rest } => {
            std::process::exit(update::run_internal_apply_update(&rest));
        }
        CliCommand::InternalRunInstaller { rest } => {
            std::process::exit(update::run_internal_run_installer(&rest));
        }
    };
    let _ = env.stdout().write_all(out.as_bytes());
    Ok(code)
}

fn io_as_api_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

#[cfg(test)]
pub(crate) fn fake_gh_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
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
    out.push_str(&format!("mergeable: {}\n", pr.effective_merge_status()));
    out.push_str(&format!("merge_state: {}\n", pr.merge_state_status));
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

/// Dispatch a `gwtd hook <name> [args...]` invocation.
///
/// SPEC-2077 Phase 2: `gwtd hook ...` remains the outward-facing surface, but
/// the front door now relays to the hidden `gwtd __internal daemon-hook ...`
/// helper so runtime evolution stays behind the same operator-facing binary.
pub fn run_hook<E: CliEnv>(env: &mut E, name: &str, rest: &[String]) -> Result<i32, SpecOpsError> {
    use crate::cli::hook::HookKind;
    let Some(_kind) = HookKind::from_name(name) else {
        let _ = writeln!(env.stderr(), "gwtd hook: unknown hook '{name}'");
        return Ok(2);
    };

    best_effort_prepare_daemon_front_door(env.repo_path());
    let stdin = env.read_stdin().map_err(io_as_api_error)?;
    let output = env
        .run_internal_command(&daemon_hook_argv(name, rest), &stdin)
        .map_err(io_as_api_error)?;
    write_internal_command_output(env, output)
}

fn daemon_hook_argv(name: &str, rest: &[String]) -> Vec<String> {
    let mut argv = vec![
        "gwtd".to_string(),
        "__internal".to_string(),
        "daemon-hook".to_string(),
        name.to_string(),
    ];
    argv.extend(rest.iter().cloned());
    argv
}

fn write_internal_command_output<E: CliEnv>(
    env: &mut E,
    output: crate::cli::env::InternalCommandOutput,
) -> Result<i32, SpecOpsError> {
    env.stdout()
        .write_all(&output.stdout)
        .map_err(io_as_api_error)?;
    env.stdout().flush().map_err(io_as_api_error)?;
    env.stderr()
        .write_all(&output.stderr)
        .map_err(io_as_api_error)?;
    env.stderr().flush().map_err(io_as_api_error)?;
    Ok(output.status)
}

fn best_effort_prepare_daemon_front_door(project_root: &std::path::Path) {
    let _ = prepare_daemon_front_door_for_path(project_root);
}

pub fn prepare_daemon_front_door_for_path(project_root: &std::path::Path) -> Result<(), String> {
    if !project_root.exists() {
        return Ok(());
    }

    crate::index_worker::bootstrap_project_index_for_path(project_root)?;

    let scope = gwt_core::daemon::RuntimeScope::from_project_root(
        project_root,
        gwt_core::daemon::RuntimeTarget::Host,
    )
    .map_err(|err| err.to_string())?;
    let gwt_home = gwt_core::paths::gwt_home();
    let action = gwt_core::daemon::resolve_bootstrap_action(
        &gwt_home,
        &scope,
        gwt_core::daemon::DAEMON_PROTOCOL_VERSION,
        |pid| pid == std::process::id(),
    )
    .map_err(|err| err.to_string())?;

    if let gwt_core::daemon::DaemonBootstrapAction::Spawn { endpoint_path } = action {
        let endpoint = gwt_core::daemon::DaemonEndpoint::new(
            scope,
            std::process::id(),
            "internal://gwt-front-door".to_string(),
            uuid::Uuid::new_v4().to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        gwt_core::daemon::persist_endpoint(&endpoint_path, &endpoint)
            .map_err(|err| err.to_string())?;
    }

    Ok(())
}

pub fn run_daemon_hook<E: CliEnv>(
    env: &mut E,
    name: &str,
    rest: &[String],
) -> Result<i32, SpecOpsError> {
    use crate::cli::hook::{
        block_bash_policy, skill_build_spec_stop_check, skill_discussion_stop_check,
        skill_plan_spec_stop_check, workflow_policy, HookKind, HookOutput,
    };

    let Some(kind) = HookKind::from_name(name) else {
        let _ = writeln!(env.stderr(), "gwtd hook: unknown hook '{name}'");
        return Ok(2);
    };
    let stdin = env.read_stdin().map_err(io_as_api_error)?;

    fn emit_hook_output<E: CliEnv>(env: &mut E, output: &HookOutput) -> i32 {
        match output.serialize_to(env.stdout()) {
            Ok(()) => output.exit_code(),
            Err(err) => {
                let _ = writeln!(env.stderr(), "gwtd hook: failed to serialize output: {err}");
                1
            }
        }
    }
    fn emit_hook_error<E: CliEnv>(env: &mut E, name: &str, err: impl std::fmt::Display) -> i32 {
        let _ = writeln!(env.stderr(), "gwtd hook {name}: {err}");
        1
    }

    match kind {
        HookKind::RuntimeState => {
            let Some(event) = rest.first() else {
                let _ = writeln!(
                    env.stderr(),
                    "gwtd hook runtime-state: missing <event> argument"
                );
                return Ok(2);
            };
            match crate::daemon_runtime::handle_runtime_state(event, &stdin) {
                Ok(()) => Ok(0),
                Err(err) => Ok(emit_hook_error(env, name, err)),
            }
        }
        HookKind::CoordinationEvent => {
            let Some(event) = rest.first() else {
                let _ = writeln!(
                    env.stderr(),
                    "gwtd hook coordination-event: missing <event> argument"
                );
                return Ok(2);
            };
            match crate::daemon_runtime::handle_coordination_event(event, &stdin) {
                Ok(()) => Ok(0),
                Err(err) => Ok(emit_hook_error(env, name, err)),
            }
        }
        HookKind::BoardReminder => {
            let Some(event) = rest.first() else {
                let _ = writeln!(
                    env.stderr(),
                    "gwtd hook board-reminder: missing <event> argument"
                );
                return Ok(2);
            };
            match crate::cli::hook::board_reminder::handle_with_input(event, &stdin) {
                Ok(output) => Ok(emit_hook_output(env, &output)),
                Err(err) => Ok(emit_hook_error(env, name, err)),
            }
        }
        HookKind::BlockBashPolicy => match block_bash_policy::handle_with_input(&stdin) {
            Ok(output) => Ok(emit_hook_output(env, &output)),
            Err(err) => Ok(emit_hook_error(env, name, err)),
        },
        HookKind::WorkflowPolicy => match workflow_policy::handle_with_input(&stdin) {
            Ok(output) => Ok(emit_hook_output(env, &output)),
            Err(err) => Ok(emit_hook_error(env, name, err)),
        },
        HookKind::Forward => match crate::daemon_runtime::handle_forward(&stdin) {
            Ok(()) => Ok(0),
            Err(err) => Ok(emit_hook_error(env, name, err)),
        },
        HookKind::SkillDiscussionStopCheck => {
            let cwd = env.repo_path().to_path_buf();
            let output = skill_discussion_stop_check::handle_with_input(&cwd, &stdin);
            Ok(emit_hook_output(env, &output))
        }
        HookKind::SkillPlanSpecStopCheck => {
            let cwd = env.repo_path().to_path_buf();
            let current_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok();
            let output = skill_plan_spec_stop_check::handle_with_input(
                &cwd,
                &stdin,
                current_session.as_deref(),
            );
            Ok(emit_hook_output(env, &output))
        }
        HookKind::SkillBuildSpecStopCheck => {
            let cwd = env.repo_path().to_path_buf();
            let current_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok();
            let output = skill_build_spec_stop_check::handle_with_input(
                &cwd,
                &stdin,
                current_session.as_deref(),
            );
            Ok(emit_hook_output(env, &output))
        }
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
    use std::{
        env, fs,
        path::{Path, PathBuf},
    };

    use tempfile::tempdir;

    use crate::cli::env::{InternalCommandOutput, TestEnv};
    use gwt_github::{
        CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt,
    };

    use super::*;

    fn compile_fake_gh(bin_dir: &Path) {
        let source = r###"
use std::{env, fs, process::ExitCode};

fn pr_json(number: &str, title: &str) -> String {
    format!(
        "{{\"number\":{number},\"title\":\"{title}\",\"state\":\"OPEN\",\"url\":\"https://github.com/akiojin/gwt/pull/{number}\",\"mergeable\":\"MERGEABLE\",\"mergeStateStatus\":\"CLEAN\",\"statusCheckRollup\":[{{\"name\":\"ci\",\"status\":\"COMPLETED\",\"conclusion\":\"SUCCESS\"}}],\"reviewDecision\":\"APPROVED\"}}"
    )
}

fn behind_pr_json(number: &str, title: &str) -> String {
    format!(
        "{{\"number\":{number},\"title\":\"{title}\",\"state\":\"OPEN\",\"url\":\"https://github.com/akiojin/gwt/pull/{number}\",\"mergeable\":\"MERGEABLE\",\"mergeStateStatus\":\"BEHIND\",\"statusCheckRollup\":[{{\"name\":\"ci\",\"status\":\"COMPLETED\",\"conclusion\":\"SUCCESS\"}}],\"reviewDecision\":\"REVIEW_REQUIRED\"}}"
    )
}

fn review_threads_json(resolved_after_fail: bool) -> String {
    let resolved = if resolved_after_fail { "true" } else { "false" };
    r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[
{"id":"thread-1","isResolved":__RESOLVED__,"isOutdated":false,"path":"src/lib.rs","line":10,"comments":{"nodes":[{"id":"comment-1","body":"done","createdAt":"2026-04-20T00:00:00Z","updatedAt":"2026-04-20T00:00:00Z","author":{"login":"reviewer"}}]}},
{"id":"thread-2","isResolved":false,"isOutdated":false,"path":"src/main.rs","line":12,"comments":{"nodes":[{"id":"comment-2","body":"needs changes","createdAt":"2026-04-20T01:00:00Z","updatedAt":"2026-04-20T01:00:00Z","author":{"login":"reviewer"}}]}}
]}}}}}"#
        .replace("__RESOLVED__", resolved)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let mode = env::var("GWT_FAKE_GH_MODE").unwrap_or_else(|_| "success".to_string());
    let state_file = env::var("GWT_FAKE_GH_STATE_FILE").ok();

    match args.as_slice() {
        [pr, view, json_flag, ..] if pr == "pr" && view == "view" && json_flag == "--json" => {
            if mode == "no-current-pr" {
                eprintln!("no pull requests found for branch");
                return ExitCode::from(1);
            }
            if mode == "behind" {
                println!("{}", behind_pr_json("12", "Current PR"));
            } else {
                println!("{}", pr_json("12", "Current PR"));
            }
            return ExitCode::SUCCESS;
        }
        [pr, view, number, repo_flag, _, json_flag, ..]
            if pr == "pr" && view == "view" && repo_flag == "--repo" && json_flag == "--json" =>
        {
            if mode == "behind" {
                println!("{}", behind_pr_json(number, "Fetched PR"));
            } else {
                println!("{}", pr_json(number, "Fetched PR"));
            }
            return ExitCode::SUCCESS;
        }
        [pr, create, ..] if pr == "pr" && create == "create" => {
            println!("https://github.com/akiojin/gwt/pull/12");
            return ExitCode::SUCCESS;
        }
        [pr, edit, ..] if pr == "pr" && edit == "edit" => {
            return ExitCode::SUCCESS;
        }
        [pr, comment, ..] if pr == "pr" && comment == "comment" => {
            return ExitCode::SUCCESS;
        }
        [pr, checks, _, json_flag, fields] if pr == "pr" && checks == "checks" && json_flag == "--json" => {
            if mode == "checks-fallback" && !fields.contains("bucket") {
                eprintln!("unknown JSON field\nAvailable fields:\n  name\n  state\n  bucket\n  link\n  startedAt\n  completedAt\n  workflow");
                return ExitCode::from(1);
            }
            if fields.contains("bucket") {
                println!("[{{\"name\":\"CI\",\"state\":\"COMPLETED\",\"bucket\":\"pass\",\"link\":\"https://example.test/checks/12\",\"startedAt\":\"2026-04-20T00:00:00Z\",\"completedAt\":\"2026-04-20T00:01:00Z\",\"workflow\":\"coverage\"}}]");
            } else {
                println!("[{{\"name\":\"CI\",\"state\":\"COMPLETED\",\"conclusion\":\"SUCCESS\",\"detailsUrl\":\"https://example.test/checks/12\",\"startedAt\":\"2026-04-20T00:00:00Z\",\"completedAt\":\"2026-04-20T00:01:00Z\"}}]");
            }
            return ExitCode::SUCCESS;
        }
        [run, view, run_id, log_flag] if run == "run" && view == "view" && log_flag == "--log" => {
            println!("run log {run_id}");
            return ExitCode::SUCCESS;
        }
        [api, endpoint] if api == "api" && endpoint == "repos/akiojin/gwt/pulls/12/reviews" => {
            println!("[{{\"id\":42,\"state\":\"APPROVED\",\"body\":\"Looks good\",\"submitted_at\":\"2026-04-20T02:00:00Z\",\"user\":{{\"login\":\"reviewer\"}}}}]");
            return ExitCode::SUCCESS;
        }
        [api, endpoint] if api == "api" && endpoint == "/repos/akiojin/gwt/actions/jobs/91/logs" => {
            if mode == "job-log-zip" {
                print!("PKZIP");
            } else {
                print!("job log 91");
            }
            return ExitCode::SUCCESS;
        }
        [api, graphql, ..] if api == "api" && graphql == "graphql" => {
            let joined = args.join("\n");
            if joined.contains("timelineItems") {
                println!(
                    "{}",
                    r#"{"data":{"repository":{"issue":{"timelineItems":{"nodes":[
{"__typename":"CrossReferencedEvent","source":{"__typename":"PullRequest","number":12,"title":"Coverage Gate","state":"OPEN","url":"https://github.com/akiojin/gwt/pull/12"}},
{"__typename":"ConnectedEvent","subject":{"__typename":"PullRequest","number":13,"title":"Follow-up","state":"MERGED","url":"https://github.com/akiojin/gwt/pull/13"}},
{"__typename":"ConnectedEvent","subject":{"__typename":"PullRequest","number":12,"title":"Duplicate","state":"OPEN","url":"https://github.com/akiojin/gwt/pull/12"}}
]}}}}}"#
                );
                return ExitCode::SUCCESS;
            }
            if joined.contains("reviewThreads") {
                let resolved_after_fail = state_file
                    .as_deref()
                    .map(fs::metadata)
                    .transpose()
                    .ok()
                    .flatten()
                    .is_some();
                println!("{}", review_threads_json(resolved_after_fail));
                return ExitCode::SUCCESS;
            }
            if joined.contains("addPullRequestReviewThreadReply") {
                println!("{{\"data\":{{\"addPullRequestReviewThreadReply\":{{\"comment\":{{\"id\":\"reply-1\"}}}}}}}}");
                return ExitCode::SUCCESS;
            }
            if joined.contains("resolveReviewThread") {
                if mode == "resolve-fails-but-resolved" {
                    let already_failed = state_file
                        .as_deref()
                        .map(fs::metadata)
                        .transpose()
                        .ok()
                        .flatten()
                        .is_some();
                    if !already_failed {
                        if let Some(state_file) = state_file.as_deref() {
                            let _ = fs::write(state_file, "resolved");
                        }
                        eprintln!("thread already resolved");
                        return ExitCode::from(1);
                    }
                }
                println!("{{\"data\":{{\"resolveReviewThread\":{{\"thread\":{{\"id\":\"thread-1\",\"isResolved\":true}}}}}}}}");
                return ExitCode::SUCCESS;
            }
        }
        _ => {}
    }

    eprintln!("unexpected fake gh args: {args:?}");
    ExitCode::from(1)
}
"###;

        let source_path = bin_dir.join("gh.rs");
        fs::write(&source_path, source).expect("write fake gh source");
        let output_path = bin_dir.join(format!("gh{}", env::consts::EXE_SUFFIX));
        let status = std::process::Command::new("rustc")
            .arg(&source_path)
            .arg("-o")
            .arg(&output_path)
            .status()
            .expect("compile fake gh");
        assert!(status.success(), "fake gh compilation failed");
    }

    fn with_fake_gh<T>(mode: &str, test: impl FnOnce(&Path) -> T) -> T {
        let _lock = super::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempdir().expect("tempdir");
        compile_fake_gh(temp.path());

        let repo_path = temp.path().join("repo");
        fs::create_dir_all(&repo_path).expect("create repo path");

        let old_path = env::var_os("PATH");
        let old_mode = env::var_os("GWT_FAKE_GH_MODE");
        let old_state = env::var_os("GWT_FAKE_GH_STATE_FILE");
        let state_file = temp.path().join("gh-state");
        let joined_path = env::join_paths(
            std::iter::once(PathBuf::from(temp.path()))
                .chain(old_path.iter().flat_map(env::split_paths)),
        )
        .expect("join PATH");
        env::set_var("PATH", joined_path);
        env::set_var("GWT_FAKE_GH_MODE", mode);
        env::set_var("GWT_FAKE_GH_STATE_FILE", &state_file);

        let result = test(&repo_path);

        match old_path {
            Some(value) => env::set_var("PATH", value),
            None => env::remove_var("PATH"),
        }
        match old_mode {
            Some(value) => env::set_var("GWT_FAKE_GH_MODE", value),
            None => env::remove_var("GWT_FAKE_GH_MODE"),
        }
        match old_state {
            Some(value) => env::set_var("GWT_FAKE_GH_STATE_FILE", value),
            None => env::remove_var("GWT_FAKE_GH_STATE_FILE"),
        }

        result
    }

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

    fn sample_issue_snapshot() -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(42),
            title: "Coverage gate".to_string(),
            body: "Raise the project coverage gate.\n".to_string(),
            labels: vec!["gwt-spec".to_string(), "coverage".to_string()],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-04-20T00:00:00Z"),
            comments: vec![CommentSnapshot {
                id: CommentId(7),
                body: "Need more tests.".to_string(),
                updated_at: UpdatedAt::new("2026-04-20T01:00:00Z"),
            }],
        }
    }

    fn sample_pr_status() -> PrStatus {
        PrStatus {
            number: 128,
            title: "Enforce coverage".to_string(),
            state: gwt_git::pr_status::PrState::Open,
            url: "https://github.com/akiojin/gwt/pull/128".to_string(),
            ci_status: "SUCCESS".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            review_status: "APPROVED".to_string(),
        }
    }

    #[test]
    fn render_helpers_include_issue_pr_and_review_details() {
        let mut out = String::new();
        let issue = sample_issue_snapshot();
        let pr = sample_pr_status();

        render_issue(&mut out, &issue);
        render_issue_comments(&mut out, &issue);
        render_linked_prs(
            &mut out,
            &[LinkedPrSummary {
                number: 128,
                title: "Enforce coverage".to_string(),
                state: "OPEN".to_string(),
                url: pr.url.clone(),
            }],
        );
        render_pr(&mut out, &pr);
        render_pr_checks(
            &mut out,
            &PrChecksSummary {
                summary: "All checks passed".to_string(),
                ci_status: "SUCCESS".to_string(),
                merge_status: "MERGEABLE".to_string(),
                review_status: "APPROVED".to_string(),
                checks: vec![PrCheckItem {
                    name: "CI".to_string(),
                    state: "COMPLETED".to_string(),
                    conclusion: "SUCCESS".to_string(),
                    url: "https://github.com/akiojin/gwt/actions/runs/1".to_string(),
                    started_at: "2026-04-20T00:00:00Z".to_string(),
                    completed_at: "2026-04-20T00:01:00Z".to_string(),
                    workflow: "coverage".to_string(),
                }],
            },
        );
        render_pr_reviews(
            &mut out,
            &[PrReview {
                id: "review-1".to_string(),
                state: "APPROVED".to_string(),
                author: "reviewer".to_string(),
                submitted_at: "2026-04-20T02:00:00Z".to_string(),
                body: "Looks good.".to_string(),
            }],
        );
        render_pr_review_threads(
            &mut out,
            &[PrReviewThread {
                comments: vec![PrReviewThreadComment {
                    id: "comment-1".to_string(),
                    body: "Please add a push gate.".to_string(),
                    created_at: "2026-04-20T03:00:00Z".to_string(),
                    updated_at: "2026-04-20T03:00:00Z".to_string(),
                    author: "reviewer".to_string(),
                }],
                ..sample_thread()
            }],
        );

        assert!(out.contains("#42 [OPEN] Coverage gate"));
        assert!(out.contains("labels: gwt-spec, coverage"));
        assert!(out.contains("=== comment:7 (2026-04-20T01:00:00Z) ==="));
        assert!(out.contains("#128 [OPEN] Enforce coverage"));
        assert!(out.contains("ci: SUCCESS"));
        assert!(out.contains("merge_state: CLEAN"));
        assert!(out.contains("workflow: coverage"));
        assert!(out.contains("=== review:review-1 [APPROVED] by reviewer"));
        assert!(out.contains(
            "=== thread:thread-1 resolved=false outdated=false path=src/lib.rs line=12 ==="
        ));
    }

    #[test]
    fn cache_and_parse_helpers_cover_fallback_paths() {
        let temp = tempdir().expect("tempdir");
        let number = IssueNumber(77);
        let linked_prs = vec![LinkedPrSummary {
            number: 9,
            title: "Hook coverage".to_string(),
            state: "MERGED".to_string(),
            url: "https://github.com/akiojin/gwt/pull/9".to_string(),
        }];

        assert!(read_linked_prs_cache(temp.path(), number)
            .unwrap()
            .is_none());
        write_linked_prs_cache(temp.path(), number, &linked_prs).unwrap();
        assert_eq!(
            read_linked_prs_cache(temp.path(), number).unwrap(),
            Some(linked_prs)
        );

        assert_eq!(
            extract_pr_url("note\n https://github.com/akiojin/gwt/pull/55 \nignored"),
            Some("https://github.com/akiojin/gwt/pull/55".to_string())
        );
        assert_eq!(
            parse_pr_number_from_url("https://github.com/akiojin/gwt/pull/55/"),
            Some(55)
        );
        assert_eq!(
            parse_available_fields("error\nAvailable fields:\n  number\n  title\n"),
            vec!["number".to_string(), "title".to_string()]
        );

        let checks = parse_pr_checks_items_json(
            r#"[{"name":"coverage","status":"IN_PROGRESS","bucket":"pending","link":"https://example.com/check"}]"#,
        )
        .unwrap();
        assert_eq!(checks[0].state, "IN_PROGRESS");
        assert_eq!(checks[0].conclusion, "pending");
        assert_eq!(checks[0].url, "https://example.com/check");
    }

    #[test]
    fn daemon_hook_argv_and_internal_command_output_preserve_streams() {
        let argv = daemon_hook_argv(
            "runtime-state",
            &["start".to_string(), "--json".to_string()],
        );
        assert_eq!(
            argv,
            vec![
                "gwtd".to_string(),
                "__internal".to_string(),
                "daemon-hook".to_string(),
                "runtime-state".to_string(),
                "start".to_string(),
                "--json".to_string(),
            ]
        );

        let temp = tempdir().expect("tempdir");
        let mut env = TestEnv::new(temp.path().to_path_buf());
        let status = write_internal_command_output(
            &mut env,
            InternalCommandOutput {
                status: 7,
                stdout: b"stdout-bytes".to_vec(),
                stderr: b"stderr-bytes".to_vec(),
            },
        )
        .unwrap();

        assert_eq!(status, 7);
        assert_eq!(env.stdout, b"stdout-bytes");
        assert_eq!(env.stderr, b"stderr-bytes");
    }

    #[test]
    fn parse_and_guard_helpers_cover_additional_error_paths() {
        assert!(should_dispatch_cli(&[
            "gwt".to_string(),
            "issue".to_string()
        ]));
        assert!(!should_dispatch_cli(&[
            "gwt".to_string(),
            "help".to_string()
        ]));

        let check = "--check".to_string();
        assert!(expect_flag(Some(&check), "--check").is_ok());
        assert!(expect_flag(Some(&check), "--refresh").is_err());

        let number = "42".to_string();
        let bad_number = "abc".to_string();
        assert_eq!(parse_required_number(Some(&number)).unwrap(), 42);
        assert!(parse_required_number(Some(&bad_number)).is_err());
        assert!(ensure_no_remaining_args([].iter()).is_ok());
        assert!(ensure_no_remaining_args([check].iter()).is_err());

        assert!(matches!(parse_hook_args(&[]), Err(CliParseError::Usage)));
        assert_eq!(issue_state_label(IssueState::Closed), "CLOSED");
        assert!(io_as_api_error(io::Error::other("boom"))
            .to_string()
            .contains("boom"));
        assert!(edit_or_create_repo_guard("", "repo").is_err());
        assert!(edit_or_create_repo_guard("akiojin", "gwt").is_ok());
    }

    #[test]
    fn render_helpers_cover_empty_states_and_url_parsing_fallbacks() {
        let issue = IssueSnapshot {
            comments: Vec::new(),
            ..sample_issue_snapshot()
        };
        let mut out = String::new();

        render_issue_comments(&mut out, &issue);
        render_linked_prs(&mut out, &[]);
        render_pr_checks(
            &mut out,
            &PrChecksSummary {
                summary: "pending".to_string(),
                ci_status: "PENDING".to_string(),
                merge_status: "UNKNOWN".to_string(),
                review_status: "PENDING".to_string(),
                checks: Vec::new(),
            },
        );
        render_pr_reviews(&mut out, &[]);
        render_pr_review_threads(&mut out, &[]);

        assert!(out.contains("no comments"));
        assert!(out.contains("no linked pull requests"));
        assert!(out.contains("no checks"));
        assert!(out.contains("no reviews"));
        assert!(out.contains("no review threads"));
        assert_eq!(extract_pr_url("no pull request here"), None);
        assert_eq!(
            parse_pr_number_from_url("https://github.com/akiojin/gwt/pull/not-a-number"),
            None
        );
        assert!(parse_available_fields("plain error").is_empty());
        assert_eq!(
            parse_available_fields("oops\nAvailable fields:\n  number\n\n  title\n"),
            vec!["number".to_string(), "title".to_string()]
        );
    }

    #[test]
    fn cache_backed_issue_and_linked_pr_helpers_reuse_cached_data() {
        let temp = tempdir().expect("tempdir");
        let mut env = TestEnv::new(temp.path().to_path_buf());
        let snapshot = sample_issue_snapshot();
        env.client.seed(snapshot.clone());

        let loaded = load_or_refresh_issue(&mut env, snapshot.number, false).expect("load issue");
        assert_eq!(loaded.snapshot.number, snapshot.number);
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);

        let cached = load_or_refresh_issue(&mut env, snapshot.number, false).expect("cached issue");
        assert_eq!(cached.snapshot.title, snapshot.title);
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);

        env.seed_linked_prs(
            42,
            vec![LinkedPrSummary {
                number: 128,
                title: "Enforce coverage".to_string(),
                state: "OPEN".to_string(),
                url: "https://github.com/akiojin/gwt/pull/128".to_string(),
            }],
        );
        let linked =
            load_or_refresh_linked_prs(&mut env, snapshot.number, false).expect("linked prs");
        assert_eq!(linked.len(), 1);
        assert_eq!(env.linked_pr_calls(), vec![42]);

        env.clear_linked_pr_calls();
        let cached_linked = load_or_refresh_linked_prs(&mut env, snapshot.number, false)
            .expect("cached linked prs");
        assert_eq!(cached_linked.len(), 1);
        assert!(env.linked_pr_calls().is_empty());

        let cache_path = linked_prs_cache_path(temp.path(), snapshot.number);
        std::fs::create_dir_all(cache_path.parent().expect("cache dir")).expect("create cache dir");
        std::fs::write(&cache_path, "{not-json").expect("write invalid json");
        assert!(read_linked_prs_cache(temp.path(), snapshot.number).is_err());
    }

    #[test]
    fn gh_wrappers_parse_successful_responses() {
        with_fake_gh("success", |repo_path| {
            let linked =
                fetch_linked_prs_via_gh("akiojin", "gwt", IssueNumber(42)).expect("linked");
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

            let run_log = fetch_actions_run_log_via_gh(repo_path, 90).expect("run log");
            assert_eq!(run_log.trim(), "run log 90");

            let job_log =
                fetch_actions_job_log_via_gh("akiojin", "gwt", repo_path, 91).expect("job log");
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
                fetch_actions_job_log_via_gh("akiojin", "gwt", repo_path, 91).expect_err("zip");
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
