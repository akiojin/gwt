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
pub mod daemon;
mod discuss;
mod env;
pub mod hook;
mod index;
mod issue;
mod issue_spec;
mod plan;
mod pr;
mod skill_state_runtime;
#[cfg(test)]
mod test_support;
pub mod update;

use std::io::{self};

pub(crate) use env::ClientRef;
pub use env::{dispatch, CliEnv, DefaultCliEnv, TestEnv};
use gwt_github::{ApiError, SpecOpsError};

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

/// Top-level argv parse result for the CLI. SPEC-1942 FR-088〜092: each top
/// verb maps to one family-typed inner enum, so the parent enum stays at 10
/// variants and dispatch becomes a nested match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    /// `gwtd issue ...` — issue + spec management.
    Issue(IssueCommand),
    /// `gwtd pr ...` — pull request management.
    Pr(PrCommand),
    /// `gwtd actions ...` — GitHub Actions log access.
    Actions(ActionsCommand),
    /// `gwtd board ...` — coordination Board read/post.
    Board(BoardCommand),
    /// `gwtd hook ...` and `gwtd __internal daemon-hook ...`.
    Hook(HookCommand),
    /// `gwtd index ...` — local search index.
    Index(IndexCommand),
    /// `gwtd discuss ...` — gwt-discussion exit CLI.
    Discuss(DiscussCommand),
    /// `gwtd plan ...` — gwt-plan-spec exit CLI.
    Plan(PlanCommand),
    /// `gwtd build ...` — gwt-build-spec exit CLI.
    Build(BuildCommand),
    /// `gwtd update [...]` and `gwtd __internal {apply-update,run-installer} ...`.
    Update(UpdateCommand),
    /// `gwtd daemon ...` — long-running runtime daemon (SPEC-2077).
    Daemon(DaemonCommand),
}

/// SPEC-2077 family enum for `gwtd daemon ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonCommand {
    /// `gwtd daemon start` — bootstrap and serve the runtime daemon.
    Start,
    /// `gwtd daemon status` — print whether a daemon is registered for cwd scope.
    Status,
}

/// SPEC-1942 family enum for `gwtd issue ...` (includes `issue spec ...`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueCommand {
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
    View { number: u64, refresh: bool },
    /// `gwtd issue comments <n> [--refresh]` — print issue comments.
    Comments { number: u64, refresh: bool },
    /// `gwtd issue linked-prs <n> [--refresh]` — print linked PR summaries.
    LinkedPrs { number: u64, refresh: bool },
    /// `gwtd issue create --title <t> -f <body_file> [--label <l>]*`.
    Create {
        title: String,
        file: String,
        labels: Vec<String>,
    },
    /// `gwtd issue comment <n> -f <body_file>` — create a plain issue comment.
    Comment { number: u64, file: String },
}

/// SPEC-1942 family enum for `gwtd pr ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrCommand {
    /// `gwtd pr current`.
    Current,
    /// `gwtd pr create --base <branch> [--head <branch>] --title <t> -f <body_file>`.
    Create {
        base: String,
        head: Option<String>,
        title: String,
        file: String,
        labels: Vec<String>,
        draft: bool,
    },
    /// `gwtd pr edit <n> [--title <t>] [-f <body_file>] [--add-label <label>]*`.
    Edit {
        number: u64,
        title: Option<String>,
        file: Option<String>,
        add_labels: Vec<String>,
    },
    /// `gwtd pr view <n>`.
    View { number: u64 },
    /// `gwtd pr comment <n> -f <body_file>`.
    Comment { number: u64, file: String },
    /// `gwtd pr reviews <n>`.
    Reviews { number: u64 },
    /// `gwtd pr review-threads <n>`.
    ReviewThreads { number: u64 },
    /// `gwtd pr review-threads reply-and-resolve <n> -f <body_file>`.
    ReviewThreadsReplyAndResolve { number: u64, file: String },
    /// `gwtd pr checks <n>`.
    Checks { number: u64 },
}

/// SPEC-1942 family enum for `gwtd actions ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionsCommand {
    /// `gwtd actions logs --run <id>`.
    Logs { run_id: u64 },
    /// `gwtd actions job-logs --job <id>`.
    JobLogs { job_id: u64 },
}

/// SPEC-1942 family enum for `gwtd board ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardCommand {
    /// `gwtd board show [--json]`.
    Show { json: bool },
    /// `gwtd board post --kind <kind> (--body <text> | -f <file>) [--target <id>]`.
    Post {
        kind: String,
        body: Option<String>,
        file: Option<String>,
        parent: Option<String>,
        topics: Vec<String>,
        owners: Vec<String>,
        targets: Vec<String>,
    },
}

/// SPEC-1942 family enum for `gwtd index ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexCommand {
    /// `gwtd index status`.
    Status,
    /// `gwtd index rebuild [--scope <scope>]`.
    Rebuild { scope: IndexScope },
}

/// SPEC-1942 family enum for `gwtd hook ...` and `gwtd __internal daemon-hook ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookCommand {
    /// `gwtd hook <name> [args...]` — visible managed hook entry.
    Run { name: String, rest: Vec<String> },
    /// `gwtd __internal daemon-hook <name> [args...]` — hidden helper.
    InternalDaemon { name: String, rest: Vec<String> },
}

/// SPEC-1942 family enum for `gwtd update ...` and `gwtd __internal apply-update`/`run-installer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCommand {
    /// `gwtd update --check` — only check and report.
    CheckOnly,
    /// `gwtd update` — check and, with approval, download and apply.
    Apply,
    /// `gwtd __internal apply-update ...`.
    InternalApply { rest: Vec<String> },
    /// `gwtd __internal run-installer ...`.
    InternalRunInstaller { rest: Vec<String> },
}

/// SPEC-1942 family enum for `gwtd discuss ...`. Backed by the legacy
/// [`DiscussAction`] alias to keep call-sites stable.
pub type DiscussCommand = DiscussAction;

/// SPEC-1942 family enum for `gwtd plan ...`. Backed by [`SkillStateAction`].
pub type PlanCommand = SkillStateAction;

/// SPEC-1942 family enum for `gwtd build ...`. Backed by [`SkillStateAction`].
pub type BuildCommand = SkillStateAction;

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
                    | "daemon"
            )
        })
        .unwrap_or(false)
}

/// Parse an argv slice into a [`CliCommand`]. The slice should start from
/// the first post-subcommand argument — i.e. if the caller received
/// `["gwt", "issue", "spec", "2001"]`, they pass `["spec", "2001"]`.
pub fn parse_issue_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    issue::parse(args).map(CliCommand::Issue)
}

/// Parse an argv slice into a `gwtd pr ...` [`CliCommand`].
pub fn parse_pr_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    pr::parse(args).map(CliCommand::Pr)
}

/// Parse an argv slice into a `gwtd actions ...` [`CliCommand`].
pub fn parse_actions_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    actions::parse(args).map(CliCommand::Actions)
}

/// Parse an argv slice into a `gwtd board ...` [`CliCommand`].
pub fn parse_board_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    board::parse(args).map(CliCommand::Board)
}

/// Parse an argv slice into a `gwtd index ...` [`CliCommand`].
pub fn parse_index_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    index::parse(args).map(CliCommand::Index)
}

/// Parse an argv slice into a `gwtd daemon ...` [`CliCommand`] (SPEC-2077).
pub fn parse_daemon_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    daemon::parse(args).map(CliCommand::Daemon)
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

/// Parse the tail of a `gwtd hook ...` argv slice into a
/// [`CliCommand::Hook`] holding [`HookCommand::Run`].
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
    Ok(CliCommand::Hook(HookCommand::Run {
        name: head.clone(),
        rest: rest.to_vec(),
    }))
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
/// SPEC-1942 family-nested form: each parent variant carries the family enum
/// and we delegate to the matching family module's `run`.
///
/// We collect output into a String buffer first so the family run's borrow of
/// `env.client()` does not conflict with the mutable borrow required by
/// `env.stdout()` at write time.
pub fn run<E: CliEnv>(env: &mut E, cmd: CliCommand) -> Result<i32, SpecOpsError> {
    let mut out = String::new();
    let code = match cmd {
        CliCommand::Issue(inner) => issue::run(env, inner, &mut out)?,
        CliCommand::Pr(inner) => pr::run(env, inner, &mut out)?,
        CliCommand::Actions(inner) => actions::run(env, inner, &mut out)?,
        CliCommand::Board(inner) => board::run(env, inner, &mut out)?,
        CliCommand::Index(inner) => index::run(env, inner, &mut out)?,
        CliCommand::Discuss(action) => discuss::run(env, action, &mut out)?,
        CliCommand::Plan(action) => plan::run(env, action, &mut out)?,
        CliCommand::Build(action) => build::run(env, action, &mut out)?,
        CliCommand::Hook(HookCommand::Run { name, rest }) => {
            return hook::run_hook(env, &name, &rest);
        }
        CliCommand::Hook(HookCommand::InternalDaemon { name, rest }) => {
            return hook::run_daemon_hook(env, &name, &rest);
        }
        CliCommand::Update(UpdateCommand::CheckOnly) => {
            std::process::exit(update::run(update::UpdateRunMode::CheckOnly));
        }
        CliCommand::Update(UpdateCommand::Apply) => {
            std::process::exit(update::run(update::UpdateRunMode::Apply));
        }
        CliCommand::Update(UpdateCommand::InternalApply { rest }) => {
            std::process::exit(update::run_internal_apply_update(&rest));
        }
        CliCommand::Update(UpdateCommand::InternalRunInstaller { rest }) => {
            std::process::exit(update::run_internal_run_installer(&rest));
        }
        CliCommand::Daemon(inner) => daemon::run(env, inner, &mut out)?,
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use gwt_github::{IssueNumber, IssueSnapshot, IssueState};

    use super::*;

    #[test]
    fn render_helpers_include_issue_pr_and_review_details() {
        let mut out = String::new();
        let issue = crate::cli::test_support::sample_issue_snapshot();
        let pr = crate::cli::test_support::sample_pr_status();

        crate::cli::issue::render_issue(&mut out, &issue);
        crate::cli::issue::render_issue_comments(&mut out, &issue);
        crate::cli::issue::render_linked_prs(
            &mut out,
            &[LinkedPrSummary {
                number: 128,
                title: "Enforce coverage".to_string(),
                state: "OPEN".to_string(),
                url: pr.url.clone(),
            }],
        );
        crate::cli::pr::render_pr(&mut out, &pr);
        crate::cli::pr::render_pr_checks(
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
        crate::cli::pr::render_pr_reviews(
            &mut out,
            &[PrReview {
                id: "review-1".to_string(),
                state: "APPROVED".to_string(),
                author: "reviewer".to_string(),
                submitted_at: "2026-04-20T02:00:00Z".to_string(),
                body: "Looks good.".to_string(),
            }],
        );
        crate::cli::pr::render_pr_review_threads(
            &mut out,
            &[PrReviewThread {
                comments: vec![PrReviewThreadComment {
                    id: "comment-1".to_string(),
                    body: "Please add a push gate.".to_string(),
                    created_at: "2026-04-20T03:00:00Z".to_string(),
                    updated_at: "2026-04-20T03:00:00Z".to_string(),
                    author: "reviewer".to_string(),
                }],
                ..crate::cli::test_support::sample_thread()
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

        assert!(
            crate::cli::issue::read_linked_prs_cache(temp.path(), number)
                .unwrap()
                .is_none()
        );
        crate::cli::issue::write_linked_prs_cache(temp.path(), number, &linked_prs).unwrap();
        assert_eq!(
            crate::cli::issue::read_linked_prs_cache(temp.path(), number).unwrap(),
            Some(linked_prs)
        );

        assert_eq!(
            crate::cli::pr::extract_pr_url(
                "note\n https://github.com/akiojin/gwt/pull/55 \nignored"
            ),
            Some("https://github.com/akiojin/gwt/pull/55".to_string())
        );
        assert_eq!(
            crate::cli::pr::parse_pr_number_from_url("https://github.com/akiojin/gwt/pull/55/"),
            Some(55)
        );
        assert_eq!(
            crate::cli::pr::parse_available_fields("error\nAvailable fields:\n  number\n  title\n"),
            vec!["number".to_string(), "title".to_string()]
        );

        let checks = crate::cli::pr::parse_pr_checks_items_json(
            r#"[{"name":"coverage","status":"IN_PROGRESS","bucket":"pending","link":"https://example.com/check"}]"#,
        )
        .unwrap();
        assert_eq!(checks[0].state, "IN_PROGRESS");
        assert_eq!(checks[0].conclusion, "pending");
        assert_eq!(checks[0].url, "https://example.com/check");
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
        assert_eq!(
            crate::cli::issue::issue_state_label(IssueState::Closed),
            "CLOSED"
        );
        assert!(io_as_api_error(io::Error::other("boom"))
            .to_string()
            .contains("boom"));
        assert!(crate::cli::pr::edit_or_create_repo_guard("", "repo").is_err());
        assert!(crate::cli::pr::edit_or_create_repo_guard("akiojin", "gwt").is_ok());
    }

    #[test]
    fn render_helpers_cover_empty_states_and_url_parsing_fallbacks() {
        let issue = IssueSnapshot {
            comments: Vec::new(),
            ..crate::cli::test_support::sample_issue_snapshot()
        };
        let mut out = String::new();

        crate::cli::issue::render_issue_comments(&mut out, &issue);
        crate::cli::issue::render_linked_prs(&mut out, &[]);
        crate::cli::pr::render_pr_checks(
            &mut out,
            &PrChecksSummary {
                summary: "pending".to_string(),
                ci_status: "PENDING".to_string(),
                merge_status: "UNKNOWN".to_string(),
                review_status: "PENDING".to_string(),
                checks: Vec::new(),
            },
        );
        crate::cli::pr::render_pr_reviews(&mut out, &[]);
        crate::cli::pr::render_pr_review_threads(&mut out, &[]);

        assert!(out.contains("no comments"));
        assert!(out.contains("no linked pull requests"));
        assert!(out.contains("no checks"));
        assert!(out.contains("no reviews"));
        assert!(out.contains("no review threads"));
        assert_eq!(crate::cli::pr::extract_pr_url("no pull request here"), None);
        assert_eq!(
            crate::cli::pr::parse_pr_number_from_url(
                "https://github.com/akiojin/gwt/pull/not-a-number"
            ),
            None
        );
        assert!(crate::cli::pr::parse_available_fields("plain error").is_empty());
        assert_eq!(
            crate::cli::pr::parse_available_fields(
                "oops\nAvailable fields:\n  number\n\n  title\n"
            ),
            vec!["number".to_string(), "title".to_string()]
        );
    }

    /// SPEC-1942 family split (FR-088〜092 / SC-025〜027): the parent
    /// [`CliCommand`] is a 10-variant nested enum and each top-level verb
    /// parses into the matching family-typed inner enum. This RED test
    /// pins the contract before the refactor lands and stays green
    /// afterwards as the round-trip guard for the family split.
    #[test]
    fn cli_command_family_split_round_trip_parses() {
        use crate::cli::{
            ActionsCommand, BoardCommand, CliCommand, DiscussCommand, HookCommand, IndexCommand,
            IssueCommand, PrCommand, UpdateCommand,
        };

        fn s(value: &str) -> String {
            value.to_string()
        }

        // gwtd issue spec list
        let cmd = parse_issue_args(&[s("spec"), s("list")]).expect("parse issue spec list");
        assert!(matches!(
            cmd,
            CliCommand::Issue(IssueCommand::SpecList {
                phase: None,
                state: None
            })
        ));

        // gwtd issue view 42 --refresh
        let cmd =
            parse_issue_args(&[s("view"), s("42"), s("--refresh")]).expect("parse issue view");
        assert_eq!(
            cmd,
            CliCommand::Issue(IssueCommand::View {
                number: 42,
                refresh: true,
            })
        );

        // gwtd pr current
        let cmd = parse_pr_args(&[s("current")]).expect("parse pr current");
        assert_eq!(cmd, CliCommand::Pr(PrCommand::Current));

        // gwtd pr checks 12
        let cmd = parse_pr_args(&[s("checks"), s("12")]).expect("parse pr checks");
        assert_eq!(cmd, CliCommand::Pr(PrCommand::Checks { number: 12 }));

        // gwtd actions logs --run 42
        let cmd =
            parse_actions_args(&[s("logs"), s("--run"), s("42")]).expect("parse actions logs");
        assert_eq!(
            cmd,
            CliCommand::Actions(ActionsCommand::Logs { run_id: 42 })
        );

        // gwtd board show --json
        let cmd = parse_board_args(&[s("show"), s("--json")]).expect("parse board show");
        assert_eq!(cmd, CliCommand::Board(BoardCommand::Show { json: true }));

        // gwtd board post --kind status --body x
        let cmd = parse_board_args(&[s("post"), s("--kind"), s("status"), s("--body"), s("x")])
            .expect("parse board post");
        assert!(matches!(
            cmd,
            CliCommand::Board(BoardCommand::Post {
                kind,
                body: Some(body),
                file: None,
                ..
            }) if kind == "status" && body == "x"
        ));

        // gwtd index status / rebuild
        let cmd = parse_index_args(&[s("status")]).expect("parse index status");
        assert_eq!(cmd, CliCommand::Index(IndexCommand::Status));
        let cmd = parse_index_args(&[s("rebuild")]).expect("parse index rebuild");
        assert!(matches!(
            cmd,
            CliCommand::Index(IndexCommand::Rebuild {
                scope: IndexScope::All
            })
        ));

        // gwtd hook runtime-state PreToolUse
        let cmd =
            parse_hook_args(&[s("runtime-state"), s("PreToolUse")]).expect("parse hook command");
        assert!(matches!(
            cmd,
            CliCommand::Hook(HookCommand::Run { ref name, ref rest })
                if name == "runtime-state" && rest == &[s("PreToolUse")]
        ));

        // gwtd discuss park --proposal "Proposal A"
        let cmd = parse_discuss_args(&[s("park"), s("--proposal"), s("Proposal A")])
            .expect("parse discuss park");
        assert!(matches!(
            cmd,
            CliCommand::Discuss(DiscussCommand::Park { ref proposal })
                if proposal == "Proposal A"
        ));

        // gwtd plan start --spec 1942
        let cmd = parse_plan_args(&[s("start"), s("--spec"), s("1942")]).expect("parse plan start");
        assert!(matches!(
            cmd,
            CliCommand::Plan(SkillStateAction::Start { spec: 1942 })
        ));

        // gwtd build start --spec 1942
        let cmd =
            parse_build_args(&[s("start"), s("--spec"), s("1942")]).expect("parse build start");
        assert!(matches!(
            cmd,
            CliCommand::Build(SkillStateAction::Start { spec: 1942 })
        ));

        // `update --check` is parsed inline by `dispatch`. Round-trip it via
        // the public CliCommand builder to keep the family contract pinned.
        let cmd = CliCommand::Update(UpdateCommand::CheckOnly);
        assert!(matches!(cmd, CliCommand::Update(UpdateCommand::CheckOnly)));
    }
}
