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
};

pub(crate) use env::ClientRef;
pub use env::{dispatch, CliEnv, DefaultCliEnv, TestEnv};
use gwt_github::{cache::write_atomic, ApiError, Cache, IssueClient, IssueNumber, SpecOpsError};

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
            return run_hook(env, &name, &rest);
        }
        CliCommand::Hook(HookCommand::InternalDaemon { name, rest }) => {
            return run_daemon_hook(env, &name, &rest);
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

fn fetch_actions_run_log_via_gh(repo_path: &std::path::Path, run_id: u64) -> io::Result<String> {
    let output = gwt_core::process::hidden_command("gh")
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
    let output = gwt_core::process::hidden_command("gh")
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
/// SPEC-2077 hot path tightening: `gwtd hook ...` remains the outward-facing
/// surface, but dispatches in-process so every Claude/Codex hook event does not
/// pay for a second `gwtd __internal daemon-hook ...` process spawn. The hidden
/// internal command stays available as a compatibility route.
pub fn run_hook<E: CliEnv>(env: &mut E, name: &str, rest: &[String]) -> Result<i32, SpecOpsError> {
    run_daemon_hook(env, name, rest)
}

#[cfg(test)]
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

#[cfg(test)]
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

pub fn prepare_daemon_front_door_for_path(project_root: &std::path::Path) -> Result<(), String> {
    if !project_root.exists() {
        return Ok(());
    }

    refresh_managed_assets_for_hook_front_door(project_root)?;

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

fn refresh_managed_assets_for_hook_front_door(
    project_root: &std::path::Path,
) -> Result<(), String> {
    if gwt_git::Repository::discover(project_root).is_err() {
        return Ok(());
    }
    crate::managed_assets::refresh_managed_gwt_assets_for_worktree(project_root)
        .map_err(|err| err.to_string())
}

pub fn run_daemon_hook<E: CliEnv>(
    env: &mut E,
    name: &str,
    rest: &[String],
) -> Result<i32, SpecOpsError> {
    use crate::cli::hook::{
        block_bash_policy, event_dispatcher, skill_build_spec_stop_check,
        skill_discussion_stop_check, skill_plan_spec_stop_check, workflow_policy, HookKind,
        HookOutput,
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
        HookKind::Event => {
            let Some(event) = rest.first() else {
                let _ = writeln!(env.stderr(), "gwtd hook event: missing <event> argument");
                return Ok(2);
            };
            let cwd = env.repo_path().to_path_buf();
            let current_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok();
            match event_dispatcher::handle_with_input(
                event,
                &stdin,
                &cwd,
                current_session.as_deref(),
            ) {
                Ok(output) => Ok(emit_hook_output(env, &output)),
                Err(err) => Ok(emit_hook_error(env, name, err)),
            }
        }
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

        assert!(!crate::cli::pr::should_reply_to_review_thread(
            &thread,
            "Fixed in latest commit."
        ));
        assert!(crate::cli::pr::should_resolve_review_thread(&thread));
    }

    #[test]
    fn review_thread_reply_is_skipped_for_resolved_or_outdated_threads() {
        let mut resolved = sample_thread();
        resolved.is_resolved = true;
        assert!(!crate::cli::pr::should_reply_to_review_thread(
            &resolved, "reply"
        ));
        assert!(!crate::cli::pr::should_resolve_review_thread(&resolved));

        let mut outdated = sample_thread();
        outdated.is_outdated = true;
        assert!(!crate::cli::pr::should_reply_to_review_thread(
            &outdated, "reply"
        ));
        assert!(!crate::cli::pr::should_resolve_review_thread(&outdated));
    }

    #[test]
    fn pr_checks_response_returns_error_when_gh_fails() {
        let err =
            crate::cli::pr::parse_pr_checks_items_response("", "auth failed", false).unwrap_err();
        assert!(
            err.to_string().contains("gh pr checks: auth failed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn pr_checks_response_parses_success_payload() {
        let items = crate::cli::pr::parse_pr_checks_items_response(
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

    fn sample_pr_status() -> gwt_git::PrStatus {
        gwt_git::PrStatus {
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
    fn hook_front_door_refresh_migrates_stale_multi_hook_settings() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempdir().expect("tempdir");
        let status = gwt_core::process::hidden_command("git")
            .arg("init")
            .arg("-q")
            .current_dir(temp.path())
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");

        let hook_bin = temp.path().join("bin/gwtd");
        fs::create_dir_all(hook_bin.parent().unwrap()).expect("create bin dir");
        fs::write(&hook_bin, "#!/bin/sh\n").expect("write hook bin");
        let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &hook_bin);

        let settings_path = temp.path().join(".claude/settings.local.json");
        fs::create_dir_all(settings_path.parent().unwrap()).expect("create .claude");
        fs::write(
            &settings_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "hooks": {
                    "PreToolUse": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": "'/tmp/bunx-old/gwtd' hook runtime-state PreToolUse",
                                    "type": "command"
                                }
                            ]
                        },
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": "'/tmp/bunx-old/gwtd' hook forward",
                                    "type": "command"
                                }
                            ]
                        },
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": "'/tmp/bunx-old/gwtd' hook workflow-policy",
                                    "type": "command"
                                }
                            ]
                        }
                    ]
                }
            }))
            .unwrap(),
        )
        .expect("write stale settings");

        refresh_managed_assets_for_hook_front_door(temp.path()).expect("refresh hook assets");

        let content = fs::read_to_string(&settings_path).expect("read settings");
        let value: serde_json::Value = serde_json::from_str(&content).expect("settings json");
        let commands = commands_for_event(&value, "PreToolUse");
        assert_eq!(
            commands.len(),
            1,
            "stale split hook entries must collapse to one dispatcher: {commands:?}"
        );
        assert!(
            commands[0].contains(" hook event PreToolUse"),
            "dispatcher command expected, got: {commands:?}"
        );
        assert!(
            !content.contains("/tmp/bunx-old"),
            "stale temporary hook binary path must be removed, got: {content}"
        );
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

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn commands_for_event<'a>(value: &'a serde_json::Value, event: &str) -> Vec<&'a str> {
        value["hooks"][event]
            .as_array()
            .unwrap_or_else(|| panic!("hooks missing for event {event}"))
            .iter()
            .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
            .filter_map(|hook| hook["command"].as_str())
            .collect()
    }

    #[test]
    fn render_helpers_cover_empty_states_and_url_parsing_fallbacks() {
        let issue = IssueSnapshot {
            comments: Vec::new(),
            ..sample_issue_snapshot()
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

            let current = crate::cli::pr::fetch_current_pr_via_gh(repo_path)
                .expect("current pr")
                .expect("current pr exists");
            assert_eq!(current.number, 12);
            assert_eq!(current.merge_state_status, "CLEAN");

            let created = crate::cli::pr::create_pr_via_gh(
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

            let edited = crate::cli::pr::edit_pr_via_gh(
                "akiojin/gwt",
                repo_path,
                12,
                Some("Edited"),
                Some("Updated body"),
                &["tested".to_string()],
            )
            .expect("edit pr");
            assert_eq!(edited.number, 12);

            crate::cli::pr::comment_on_pr_via_gh(repo_path, 12, "done").expect("comment");

            let reviews =
                crate::cli::pr::fetch_pr_reviews_via_gh("akiojin", "gwt", 12).expect("reviews");
            assert_eq!(reviews.len(), 1);
            assert_eq!(reviews[0].author, "reviewer");

            let threads = crate::cli::pr::fetch_pr_review_threads_via_gh("akiojin", "gwt", 12)
                .expect("review threads");
            assert_eq!(threads.len(), 2);
            assert_eq!(threads[0].line, Some(10));

            let resolved = crate::cli::pr::reply_and_resolve_pr_review_threads_via_gh(
                "akiojin", "gwt", 12, "done",
            )
            .expect("reply and resolve");
            assert_eq!(resolved, 2);

            let checks = crate::cli::pr::fetch_pr_checks_via_gh("akiojin/gwt", repo_path, 12)
                .expect("checks");
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
            assert!(crate::cli::pr::fetch_current_pr_via_gh(repo_path)
                .expect("current pr result")
                .is_none());
        });

        with_fake_gh("checks-fallback", |repo_path| {
            let checks = crate::cli::pr::fetch_pr_checks_via_gh("akiojin/gwt", repo_path, 12)
                .expect("checks");
            assert_eq!(checks.checks.len(), 1);
            assert_eq!(checks.checks[0].workflow, "coverage");
            assert_eq!(checks.checks[0].url, "https://example.test/checks/12");
        });

        with_fake_gh("behind", |repo_path| {
            let current = crate::cli::pr::fetch_current_pr_via_gh(repo_path)
                .expect("current pr")
                .expect("current pr exists");
            assert_eq!(current.effective_merge_status(), "BEHIND");

            let checks = crate::cli::pr::fetch_pr_checks_via_gh("akiojin/gwt", repo_path, 12)
                .expect("checks");
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
            let resolved = crate::cli::pr::reply_and_resolve_pr_review_threads_via_gh(
                "akiojin", "gwt", 12, "done",
            )
            .expect("resolved after retry");
            assert_eq!(resolved, 2);
        });
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
