//! CLI command models used behind `gwtd` JSON envelope operations.
//!
//! SPEC-12 Phase 6: when the gwt binary is invoked with arguments starting
//! with `issue`, we treat it as a CLI call rather than a GUI launch. This
//! module owns argv parsing, dispatches to the high-level SPEC operations in
//! `gwt-github`, and writes the result to stdout/stderr.

mod actions;
mod board;
mod build;
mod commands;
pub mod daemon;
mod diagnostics;
mod discuss;
pub(crate) mod discussion;
mod env;
pub mod execution_state;
pub mod gwtd_resolver;
pub mod hook;
pub mod improvement;
pub mod improvement_contract;
mod improvement_owner;
mod improvement_store;
pub(crate) mod index;
pub(crate) mod intake_outcome;
pub(crate) mod issue;
mod issue_spec;
mod json_envelope;
pub(crate) mod memory;
pub mod open;
mod pane;
mod plan;
mod pr;
pub(crate) mod register;
pub(crate) mod search;
mod skill_state_runtime;
#[cfg(test)]
mod test_support;
mod title_summary_guard;
pub mod tray;
pub mod update;
pub mod verification_record;
mod workflow;
mod workspace;

use std::{
    io::{self},
    path::PathBuf,
};

pub use board::{BoardCommand, BoardPostCommand};
pub use commands::{IssueCommand, PrCommand};
pub use diagnostics::DiagnosticsCommand;
pub use discuss::DiscussAction;
pub use discussion::DiscussionCommand;
pub(crate) use env::ClientRef;
pub use env::{dispatch, CliEnv, DefaultCliEnv, TargetIssueCreateCall, TestEnv};
use gwt_github::{ApiError, SpecOpsError};
pub use improvement::ImprovementCommand;
pub use index::{IndexCommand, IndexScope};
pub use memory::MemoryCommand;
pub use search::SearchCommand;
pub(crate) use title_summary_guard::validate_title_summary_work_name;

/// Compact linked PR summary used by `issue.linked_prs`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LinkedPrSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
    #[serde(default)] // closes-the-issue flag; gates the completion probe (#3226)
    pub will_close_target: bool,
}

/// Compact PR check entry used by `pr.checks`.
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

/// Render-friendly aggregate used by `pr.checks`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrChecksSummary {
    pub summary: String,
    pub ci_status: String,
    pub merge_status: String,
    pub review_status: String,
    pub checks: Vec<PrCheckItem>,
}

/// PR review summary used by `pr.reviews`.
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

/// Review thread snapshot used by `pr.review_threads`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReviewThread {
    pub id: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub path: String,
    pub line: Option<u64>,
    pub comments: Vec<PrReviewThreadComment>,
}

/// Test-visible log entry for `pr.create`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCreateCall {
    pub base: String,
    pub head: Option<String>,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub draft: bool,
}

/// Test-visible log entry for `pr.edit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrEditCall {
    pub number: u64,
    pub title: Option<String>,
    pub body: Option<String>,
    pub add_labels: Vec<String>,
}

/// Top-level argv parse result for the CLI. SPEC-1942 FR-088〜092: each top
/// verb maps to one family-typed inner enum, so the parent enum stays compact
/// variants and dispatch becomes a nested match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Issue(IssueCommand),
    Pr(PrCommand),
    Actions(ActionsCommand),
    Board(BoardCommand),
    Hook(HookCommand),
    Improvement(ImprovementCommand),
    Index(IndexCommand),
    /// SPEC-3248 P7A: `intake.outcome.record` JSON operation (FR-012).
    Intake(intake_outcome::IntakeCommand),
    Diagnostics(DiagnosticsCommand),
    Memory(MemoryCommand),
    Discuss(DiscussCommand),
    Discussion(DiscussionCommand),
    /// SPEC-3248 P8a: `execution.complete` / `execution.blocked` settlement.
    Execution(execution_state::ExecutionCommand),
    Plan(PlanCommand),
    Build(BuildCommand),
    Register(RegisterCommand),
    Update(UpdateCommand),
    /// SPEC-3248 P8b: `verify.run` tool-generated verification records.
    Verify(verification_record::VerifyCommand),
    Daemon(DaemonCommand),
    Workspace(WorkspaceCommand),
    Workflow(WorkflowCommand),
    Pane(PaneCommand),
    /// SPEC #2920 FR-006: `gwt open` reads tray lock + opens browser.
    Open(open::OpenArgs),
    /// SPEC-1942 US-15: `search` JSON operation.
    Search(SearchCommand),
}

/// SPEC-2077 command model for `daemon.*` JSON operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonCommand {
    /// `daemon.start` — bootstrap and serve the runtime daemon.
    Start,
    /// `daemon.status` — print whether a daemon is registered for cwd scope.
    Status,
    /// `daemon.subscribe` — connect to the running daemon,
    /// subscribe to one or more broadcast channels, and print received events
    /// to stdout one JSON line at a time. Useful for debugging the Phase H1+
    /// fan-out pipeline.
    Subscribe { channels: Vec<String> },
}

/// SPEC-2359 command model for `workspace.*` JSON operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCommand {
    /// `workspace.update` — update Workspace current projection and journal.
    Update {
        title: Option<String>,
        status: Option<String>,
        status_text: Option<String>,
        summary: Option<String>,
        progress_summary: Option<String>,
        next_action: Option<String>,
        owner: Option<String>,
        agent_session: Option<String>,
        current_focus: Option<String>,
        title_summary: Option<String>,
    },
    /// `workspace.candidates` — list join candidates.
    Candidates { agent_session: String },
    /// `workspace.join`.
    Join {
        agent_session: String,
        workspace_id: String,
        current_focus: Option<String>,
        title_summary: Option<String>,
    },
    /// `workspace.create`.
    Create {
        agent_session: String,
        title_summary: String,
        current_focus: Option<String>,
        spec: Option<u64>,
        issue: Option<u64>,
        split_from: Option<String>,
        boundary: Option<String>,
    },
    /// `workspace.ensure`.
    Ensure {
        agent_session: String,
        title_summary: String,
        current_focus: Option<String>,
        spec: Option<u64>,
        issue: Option<u64>,
        topic: Option<String>,
        boundary: Option<String>,
    },
    /// SPEC-2359 US-41: `workspace.projection_list` —
    /// list saved Workspace projections under `~/.gwt/projects/*/workspace/`
    /// classified by [`gwt_core::workspace_projection::workspace_projection_stale_reason`].
    ProjectionList { stale: bool, all: bool },
    /// SPEC-2359 US-41: `workspace.projection_prune` —
    /// archive / delete stale Workspace projections (FR-153, FR-154).
    ProjectionPrune { dry_run: bool, ids: Vec<String> },
}

/// SPEC-1942 command model for `actions.*` JSON operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionsCommand {
    /// `actions.logs`.
    Logs { run_id: u64 },
    /// `actions.job_logs`.
    JobLogs { job_id: u64 },
}

/// SPEC-1942 command model for managed hook argv transport and internal daemon hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookCommand {
    /// Managed hook argv entry.
    Run { name: String, rest: Vec<String> },
    /// `gwtd __internal daemon-hook <name> [args...]` — hidden helper.
    InternalDaemon { name: String, rest: Vec<String> },
    /// `hook.health` — read-only managed hook health projection.
    Health {
        runtime_state_path: Option<PathBuf>,
        profile_path: Option<PathBuf>,
        expected_hook_bin: Option<String>,
    },
    /// `hook.doctor` — managed hook health projection with optional repair.
    Doctor {
        runtime_state_path: Option<PathBuf>,
        profile_path: Option<PathBuf>,
        expected_hook_bin: Option<String>,
        repair: bool,
    },
}

/// SPEC-1942 command model for update and internal updater operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCommand {
    /// Update check-only mode.
    CheckOnly,
    /// Check and, with approval, download and apply.
    Apply,
    /// `gwtd __internal apply-update ...`.
    InternalApply { rest: Vec<String> },
    /// `gwtd __internal run-installer ...`.
    InternalRunInstaller { rest: Vec<String> },
}

/// SPEC-1942 command model for `discuss.*`. Backed by the legacy
/// [`DiscussAction`] alias to keep call-sites stable.
pub type DiscussCommand = DiscussAction;

/// SPEC-1942 command model for `plan.*`. Backed by [`SkillStateAction`].
pub type PlanCommand = SkillStateAction;

/// SPEC-1942 command model for `build.*`. Backed by [`SkillStateAction`].
pub type BuildCommand = SkillStateAction;

/// SPEC-2784 command model for `register.*`. Same skill-state lifecycle.
pub type RegisterCommand = SkillStateAction;
/// Issue #3267: command model for the `workflow.bypass` JSON operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowCommand {
    /// `workflow.bypass` — arm or clear the self session's owner-guard bypass.
    Bypass { mode: workflow::WorkflowBypassMode },
}

/// SPEC-1935 / Issue #2529: command model for `gwt-agent` pane inspection and lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneCommand {
    /// `pane.list`.
    List,
    /// `pane.read`.
    Read { id: String, lines: usize },
    /// `pane.close` / `pane.stop`.
    Close { id: String },
    /// `pane.send` (SPEC-3050: self-only injection
    /// into the calling agent's own pane).
    Send { id: Option<String>, text: String },
}
/// Sub-action for `plan.*` / `build.*` (SPEC-1935 FR-014q/r).
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
    InvalidJson(String),
    InvalidNumber(String),
    MissingFlag(&'static str),
    InvalidValue {
        flag: &'static str,
        reason: &'static str,
    },
    UnknownSubcommand(String),
}

impl std::fmt::Display for CliParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliParseError::InvalidJson(message) => write!(f, "invalid JSON envelope: {message}"),
            CliParseError::Usage => write!(
                f,
                "usage: gwtd < stdin JSON envelope; e.g. {{\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{{\"status\":\"active\",\"summary\":\"<summary>\"}}}}. Managed hook transport remains gwtd hook event <Event>."
            ),
            CliParseError::InvalidNumber(s) => write!(f, "invalid issue number: {s}"),
            CliParseError::MissingFlag(flag) => write!(f, "missing required flag: {flag}"),
            CliParseError::InvalidValue { flag, reason } => {
                write!(f, "invalid value for {flag}: {reason}")
            }
            CliParseError::UnknownSubcommand(s) => write!(f, "unknown subcommand: {s}"),
        }
    }
}

impl std::error::Error for CliParseError {}

/// Determine whether the given argv (starting at the program name) should be
/// handled as a CLI invocation. Returns `true` when argv[1..] begins with
/// a supported top-level CLI verb such as `issue`, `pr`, `actions`, `board`,
/// `hook`, `discuss`, `plan`, `build`, `pane`, `update`, or `__internal`. The GUI
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
                    | "diagnostics"
                    | "memory"
                    | "lessons"
                    | "discuss"
                    | "discussion"
                    | "plan"
                    | "build"
                    | "register"
                    | "daemon"
                    | "workspace"
                    | "pane"
                    | "open"
                    | "search"
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

/// Parse a legacy `pr ...` argv slice into a [`CliCommand`].
pub fn parse_pr_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    pr::parse(args).map(CliCommand::Pr)
}

/// Parse a legacy `actions ...` argv slice into a [`CliCommand`].
pub fn parse_actions_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    actions::parse(args).map(CliCommand::Actions)
}

/// Parse a legacy `board ...` argv slice into a [`CliCommand`].
pub fn parse_board_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    board::parse(args).map(CliCommand::Board)
}

/// Parse a legacy `index ...` argv slice into a [`CliCommand`].
pub fn parse_index_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    index::parse(args).map(CliCommand::Index)
}

/// Parse a legacy `diagnostics ...` argv slice into a [`CliCommand`].
pub fn parse_diagnostics_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    diagnostics::parse(args).map(CliCommand::Diagnostics)
}

/// Parse a legacy `memory ...` / `lessons ...` argv slice into a [`CliCommand`].
pub fn parse_memory_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    memory::parse(args).map(CliCommand::Memory)
}

/// Parse a legacy `discussion ...` argv slice into a [`CliCommand`].
pub fn parse_discussion_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    discussion::parse(args).map(CliCommand::Discussion)
}

/// Parse a legacy `daemon ...` argv slice into a [`CliCommand`] (SPEC-2077).
pub fn parse_daemon_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    daemon::parse(args).map(CliCommand::Daemon)
}

/// Parse a legacy `workspace ...` argv slice into a [`CliCommand`].
pub fn parse_workspace_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    workspace::parse(args).map(CliCommand::Workspace)
}

/// Parse a legacy `pane ...` argv slice into a [`CliCommand`].
pub fn parse_pane_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    pane::parse(args).map(CliCommand::Pane)
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

/// Parse the tail of a managed hook argv slice into a
/// [`CliCommand::Hook`] holding [`HookCommand::Run`].
///
/// SPEC #1942 (CORE-CLI): managed hook argv transport is the single entry
/// point for every in-binary hook handler. The known hook names are:
///
/// - `runtime-state <event>`
/// - `block-bash-policy`
/// - `forward <target>`
///
/// Unknown names still parse (we don't maintain an allowlist here) so that
/// newly added hooks don't need parser changes. Validation happens in
/// [`crate::cli::hook::run_hook`].
pub fn parse_hook_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    Ok(CliCommand::Hook(HookCommand::Run {
        name: head.clone(),
        rest: rest.to_vec(),
    }))
}

/// Parse legacy discuss argv (SPEC-1935 FR-014p).
pub fn parse_discuss_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    discuss::parse(args).map(CliCommand::Discuss)
}

/// Parse legacy plan argv (SPEC-1935 FR-014q).
pub fn parse_plan_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    parse_skill_state_args(args).map(CliCommand::Plan)
}

/// Parse legacy build argv (SPEC-1935 FR-014r).
pub fn parse_build_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    parse_skill_state_args(args).map(CliCommand::Build)
}

pub(crate) fn parse_skill_state_args(args: &[String]) -> Result<SkillStateAction, CliParseError> {
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
pub(crate) fn run_collect<E: CliEnv>(
    env: &mut E,
    cmd: CliCommand,
) -> Result<(i32, String), SpecOpsError> {
    let mut out = String::new();
    let code = match cmd {
        CliCommand::Issue(inner) => issue::run(env, inner, &mut out)?,
        CliCommand::Pr(inner) => pr::run(env, inner, &mut out)?,
        CliCommand::Actions(inner) => actions::run(env, inner, &mut out)?,
        CliCommand::Board(inner) => board::run(env, inner, &mut out)?,
        CliCommand::Improvement(inner) => improvement::run(env, inner, &mut out)?,
        CliCommand::Index(inner) => index::run(env, inner, &mut out)?,
        CliCommand::Intake(inner) => intake_outcome::run(env, inner, &mut out)?,
        CliCommand::Memory(inner) => memory::run(env, inner, &mut out)?,
        CliCommand::Discuss(action) => discuss::run(env, action, &mut out)?,
        CliCommand::Discussion(inner) => discussion::run(env, inner, &mut out)?,
        CliCommand::Execution(inner) => execution_state::run(env, inner, &mut out)?,
        CliCommand::Verify(inner) => verification_record::run(env, inner, &mut out)?,
        CliCommand::Plan(action) => plan::run(env, action, &mut out)?,
        CliCommand::Build(action) => build::run(env, action, &mut out)?,
        CliCommand::Register(action) => register::run(env, action, &mut out)?,
        CliCommand::Hook(HookCommand::Run { name, rest }) => {
            let mut hook_stdout = Vec::new();
            let code = {
                let mut capture = env::StdoutCaptureEnv {
                    inner: env,
                    stdout: &mut hook_stdout,
                };
                hook::run_hook(&mut capture, &name, &rest)?
            };
            out.push_str(&String::from_utf8_lossy(&hook_stdout));
            return Ok((code, out));
        }
        CliCommand::Hook(HookCommand::InternalDaemon { name, rest }) => {
            let mut hook_stdout = Vec::new();
            let code = {
                let mut capture = env::StdoutCaptureEnv {
                    inner: env,
                    stdout: &mut hook_stdout,
                };
                hook::run_daemon_hook(&mut capture, &name, &rest)?
            };
            out.push_str(&String::from_utf8_lossy(&hook_stdout));
            return Ok((code, out));
        }
        CliCommand::Hook(HookCommand::Health {
            runtime_state_path,
            profile_path,
            expected_hook_bin,
        }) => {
            let mut input = hook::health::ManagedHookHealthInput::new(env.repo_path());
            if let Some(path) = runtime_state_path {
                input = input.with_runtime_state_path(path);
            }
            if let Some(path) = profile_path {
                input = input.with_profile_path(path);
            }
            if let Some(bin) = expected_hook_bin {
                input = input.with_expected_hook_bin(bin);
            }
            let health = hook::health::read_managed_hook_health(&input);
            out.push_str(&serde_json::to_string_pretty(&health).map_err(serde_as_api_error)?);
            out.push('\n');
            0
        }
        CliCommand::Hook(HookCommand::Doctor {
            runtime_state_path,
            profile_path,
            expected_hook_bin,
            repair,
        }) => {
            let repair = if repair {
                Some(
                    hook::health::repair_managed_hook_configs(env.repo_path())
                        .map_err(io_as_api_error)?,
                )
            } else {
                None
            };
            let mut input = hook::health::ManagedHookHealthInput::new(env.repo_path());
            if let Some(path) = runtime_state_path {
                input = input.with_runtime_state_path(path);
            }
            if let Some(path) = profile_path {
                input = input.with_profile_path(path);
            }
            if let Some(bin) = expected_hook_bin {
                input = input.with_expected_hook_bin(bin);
            }
            let health = hook::health::read_managed_hook_health(&input);
            let payload = serde_json::json!({
                "repair": repair,
                "health": health,
            });
            out.push_str(&serde_json::to_string_pretty(&payload).map_err(serde_as_api_error)?);
            out.push('\n');
            0
        }
        CliCommand::Diagnostics(inner) => diagnostics::run(env, inner, &mut out)?,
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
        CliCommand::Workspace(inner) => workspace::run(env, inner, &mut out)?,
        CliCommand::Workflow(inner) => workflow::run(env, inner, &mut out)?,
        CliCommand::Pane(inner) => pane::run(env, inner, &mut out)?,
        CliCommand::Open(args) => open::run(env, args, &mut out)?,
        CliCommand::Search(inner) => search::run(env, inner, &mut out)?,
    };
    Ok((code, out))
}

pub fn run<E: CliEnv>(env: &mut E, cmd: CliCommand) -> Result<i32, SpecOpsError> {
    let (code, out) = run_collect(env, cmd)?;
    env.stdout()
        .write_all(out.as_bytes())
        .map_err(io_as_api_error)?;
    Ok(code)
}

fn io_as_api_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

fn serde_as_api_error(err: serde_json::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

#[cfg(test)]
pub(crate) fn fake_gh_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(test)]
mod family_split_tests;

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
                will_close_target: true,
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
            will_close_target: true,
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
}
