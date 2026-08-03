//! `gwtd hook workflow-policy` — hook-driven workflow gating.
//!
//! The policy is deliberately narrow:
//!
//! - reuse the existing consolidated Bash safety policy first
//! - block worktree escape, branch-switching, and direct GitHub workflow CLI
//!   commands before they reach the tool runtime
//! - block direct edits of trusted execution/evidence state files
//! - require the Agent Workspace identity (title) before work starts
//! - hold a pending gwt-discussion Goal Start until it is handled
//!
//! Owner/SPEC linkage and lane membership never gate tool calls: the owner
//! guard and the intake lane code-edit guard were removed by SPEC #3245
//! (FR-002 / FR-009) after their default-deny classification kept
//! false-positive-blocking legitimate ownerless work. SPEC-first / TDD
//! discipline is carried by skills and guidance, not by this hook.

use std::{io::Read, path::Path};

use gwt_agent::session::{Session, GWT_SESSION_ID_ENV};
use gwt_core::{paths::gwt_sessions_dir, workspace_projection::load_workspace_projection};

use crate::discussion_resume::PendingDiscussionGoal;

use super::{block_bash_policy, HookError, HookEvent, HookOutput};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowContext {
    pub title_summary_missing: bool,
    pub pending_discussion_goal: Option<PendingDiscussionGoal>,
}

impl WorkflowContext {
    /// A context with nothing pending. The name survives from the removed
    /// owner-resolution era so the extensive hook test corpus keeps reading
    /// naturally; it is exactly `Self::default()`.
    pub fn unknown() -> Self {
        Self::default()
    }

    pub fn with_title_summary_missing(mut self, missing: bool) -> Self {
        self.title_summary_missing = missing;
        self
    }

    pub fn with_pending_discussion_goal(mut self, pending: Option<PendingDiscussionGoal>) -> Self {
        self.pending_discussion_goal = pending;
        self
    }
}

pub fn evaluate_with_context(
    event: &HookEvent,
    worktree_root: &Path,
    context: &WorkflowContext,
) -> Result<HookOutput, HookError> {
    let safety = block_bash_policy::evaluate(event, worktree_root)?;
    if safety != HookOutput::Silent {
        return Ok(safety);
    }
    let trusted_state = evaluate_trusted_state_write_guard(event)?;
    if trusted_state != HookOutput::Silent {
        return Ok(trusted_state);
    }
    let title_summary = evaluate_title_summary_guard(event, context.title_summary_missing)?;
    if title_summary != HookOutput::Silent {
        return Ok(title_summary);
    }
    let pending_goal =
        evaluate_pending_discussion_goal_guard(event, context.pending_discussion_goal.as_ref())?;
    if pending_goal != HookOutput::Silent {
        return Ok(pending_goal);
    }
    Ok(HookOutput::Silent)
}

pub fn evaluate(event: &HookEvent, worktree_root: &Path) -> Result<HookOutput, HookError> {
    let context = WorkflowContext::default()
        .with_title_summary_missing(current_agent_workspace_identity_missing(worktree_root)?)
        .with_pending_discussion_goal(
            crate::discussion_resume::load_pending_goal(worktree_root)
                .ok()
                .flatten(),
        );
    evaluate_with_context(event, worktree_root, &context)
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

fn load_session_from_env() -> Option<Session> {
    let session_id = std::env::var(GWT_SESSION_ID_ENV).ok()?;
    let session_path = gwt_sessions_dir().join(format!("{session_id}.toml"));
    Session::load_and_migrate(&session_path).ok()
}

fn current_agent_workspace_identity_missing(worktree_root: &Path) -> Result<bool, HookError> {
    let Some(session) = load_session_from_env() else {
        return Ok(false);
    };
    // The title requirement is meaningful only for the same Session/container
    // that workspace.update itself can mutate. A stale ambient Session must
    // not brick an unrelated cwd, repository, or branch before the user can
    // inspect and recover it.
    if crate::agent_project_state::resolve_session_work_mutation_target(worktree_root, &session.id)
        .is_err()
    {
        return Ok(false);
    }
    let projection_root = if session.worktree_path.exists() {
        session.worktree_path.as_path()
    } else {
        worktree_root
    };
    let Some(projection) = load_workspace_projection(projection_root)? else {
        return Ok(false);
    };
    let Some(agent) = projection.latest_agent_for_session(&session.id) else {
        return Ok(false);
    };
    if agent.is_unassigned() {
        return Ok(false);
    }
    let title_summary_missing = agent
        .title_summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none();
    let current_focus_missing = agent
        .current_focus
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none();
    Ok(title_summary_missing || current_focus_missing)
}

fn evaluate_title_summary_guard(
    event: &HookEvent,
    title_summary_missing: bool,
) -> Result<HookOutput, HookError> {
    if !title_summary_missing {
        return Ok(HookOutput::Silent);
    }

    if is_workspace_identity_update_event(event) {
        return Ok(HookOutput::Silent);
    }

    if is_title_sensitive_tool(event) && !is_read_only_exploration_event(event) {
        return Ok(HookOutput::pre_tool_use_permission(
            "Agent Workspace identity is required before work starts",
            "Set both a short work name and current focus before exploration, implementation, or verification commands. This is required so Workspace can show which window is doing what.\n\n\
Required command shape:\n\
  gwtd <<'JSON'\n\
  {\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"<short work title>\",\"current_focus\":\"<current work focus>\"}}\n\
  JSON\n\n\
Good example: \"purpose\":\"Agent title improvement\"\n\
Bad example: \"purpose\":\"Agent title improvement complete\"\n\n\
Use the configured narrative language for the purpose. Keep progress, completion, blocker state, and long detail in current_focus, summary, or Board body.",
        ));
    }

    Ok(HookOutput::Silent)
}

fn evaluate_pending_discussion_goal_guard(
    event: &HookEvent,
    pending_discussion_goal: Option<&PendingDiscussionGoal>,
) -> Result<HookOutput, HookError> {
    let Some(goal) = pending_discussion_goal else {
        return Ok(HookOutput::Silent);
    };
    if is_goal_start_or_bookkeeping_event(event) {
        return Ok(HookOutput::Silent);
    }
    if is_mutating_work_event(event) {
        return Ok(HookOutput::pre_tool_use_permission(
            "pending gwt-discussion Goal Start must be handled first",
            format!(
                "A gwt-discussion Action Bundle has a pending gwt-discussion Goal Start. Start, skip, or record the goal failure before changing implementation state.\n\n\
Proposal: {label} - {title}\n\
Goal condition: {condition}\n\n\
Codex path: call `create_goal` with the Goal condition above as the objective, then run JSON operation `discuss.goal_started` with `params.proposal:\"{label}\"`.\n\
Claude Code path: run JSON operation `pane.send` with the `/goal {condition}` text, then run JSON operation `discuss.goal_started` with `params.proposal:\"{label}\"`.\n\
Skip path: if the user rejects or revises the Action Bundle, run JSON operation `discuss.goal_skipped` with `params.proposal:\"{label}\"` and a reason.\n\
Failure path: run JSON operation `discuss.goal_failed` with `params.proposal:\"{label}\"` and a reason, then show the manual `/goal {condition}` line to the user.",
                label = goal.proposal_label,
                title = goal.proposal_title,
                condition = goal.condition,
            ),
        ));
    }
    Ok(HookOutput::Silent)
}

/// SPEC-3248 P9a (T-120): the execution/evidence state files are written
/// only by their canonical gwtd operations. Direct edits through the file
/// tools are blocked in every lane — an edited record would fail integrity
/// validation at the gates anyway, so the deny message routes to the
/// canonical operations up front. (Bash-level writes are out of reach of
/// path-based blocking and remain covered by the integrity hashes.)
const TRUSTED_STATE_FILE_NAMES: &[&str] = &[
    "execution-control.json",
    "execution-generation-pointer.json",
    "generation-ledger.json",
    "execution-repair-audit.json",
    "verification-run.json",
    "verification-plan.json",
    "intake-outcome.json",
    "action-obligations.json",
    "action-obligation-revival.json",
];

fn evaluate_trusted_state_write_guard(event: &HookEvent) -> Result<HookOutput, HookError> {
    if !matches!(
        event.tool_name.as_deref(),
        Some("Edit" | "MultiEdit" | "Write" | "NotebookEdit" | "apply_patch")
    ) {
        return Ok(HookOutput::Silent);
    }
    let paths = event_target_paths(event);
    let targets_trusted_state = paths.iter().any(|path| {
        // Lowercase so case-variant spellings on case-insensitive
        // filesystems cannot slip past the match.
        let normalized = path.replace('\\', "/").to_lowercase();
        TRUSTED_STATE_FILE_NAMES.iter().any(|name| {
            // Worktree mirror (P9a) and the repo-scoped trusted store
            // copy under `~/.gwt/projects/<hash>/trusted/<key>/` (P9b).
            normalized.ends_with(&format!(".gwt/skill-state/{name}"))
                || (normalized.contains("/.gwt/projects/")
                    && normalized.contains("/trusted/")
                    && normalized.ends_with(&format!("/{name}")))
        })
    });
    if !targets_trusted_state {
        return Ok(HookOutput::Silent);
    }
    Ok(HookOutput::pre_tool_use_permission(
        "Execution/evidence state files are written only by their canonical operations",
        "This file is trusted execution/evidence state (SPEC-3248 P9a/P9b) — the worktree mirror and its repo-scoped trusted store copy alike. Direct edits are ignored or rejected at the completion/PR gates, so do not edit it. \
Use the canonical JSON operations instead: `execution.complete` / `execution.blocked` / `execution.adopt` / `execution.repair` / `execution.reopen` for execution authority, `verify.plan` / `verify.run` for verification plans and records, and `intake.outcome.record` for intake outcomes.",
    ))
}

fn event_target_paths(event: &HookEvent) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(path) = event
        .tool_input
        .as_ref()
        .and_then(|input| input.get("file_path"))
        .and_then(serde_json::Value::as_str)
    {
        paths.push(path.to_string());
    }
    if event.tool_name.as_deref() == Some("apply_patch") {
        if let Some(input) = event.tool_input.as_ref() {
            if let Some(patch) = input.as_str() {
                paths.extend(apply_patch_target_paths(patch));
            }
            for key in ["patch", "input", "content", "cmd", "command"] {
                if let Some(patch) = input.get(key).and_then(serde_json::Value::as_str) {
                    paths.extend(apply_patch_target_paths(patch));
                }
            }
        }
    }
    paths
}

fn apply_patch_target_paths(patch: &str) -> Vec<String> {
    const PREFIXES: &[&str] = &[
        "*** Add File: ",
        "*** Delete File: ",
        "*** Update File: ",
        "*** Move to: ",
    ];

    patch
        .lines()
        .filter_map(|line| {
            PREFIXES
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix))
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToOwned::to_owned)
        })
        .collect()
}

pub(crate) fn is_mutating_work_event(event: &HookEvent) -> bool {
    match event.tool_name.as_deref() {
        Some("Edit" | "MultiEdit" | "Write" | "NotebookEdit" | "apply_patch") => true,
        Some("Bash") => {
            event.command().is_some()
                && !is_read_only_exploration_event(event)
                && !is_goal_start_or_bookkeeping_event(event)
                && !event
                    .command()
                    .is_some_and(is_standalone_json_envelope_command)
        }
        _ => false,
    }
}

fn is_goal_start_or_bookkeeping_event(event: &HookEvent) -> bool {
    match event.tool_name.as_deref() {
        Some("create_goal" | "functions.create_goal") => true,
        Some("Bash") => event.command().is_some_and(command_segments_are_goal_safe),
        _ => false,
    }
}

fn command_segments_are_goal_safe(command: &str) -> bool {
    if is_workspace_identity_update_command(command)
        || is_json_envelope_operation(
            command,
            &[
                "board.post",
                "discuss.goal_started",
                "discuss.goal-started",
                "discuss.goal_failed",
                "discuss.goal-failed",
                "discuss.goal_skipped",
                "discuss.goal-skipped",
                "pane.send",
            ],
        )
    {
        return true;
    }

    let segments = super::segments::split_command_segments(command);
    !segments.is_empty()
        && segments.iter().all(|segment| {
            is_goal_bookkeeping_segment(segment)
                || is_workspace_identity_update_segment(segment)
                || is_board_post_segment(segment)
        })
}

fn is_goal_bookkeeping_segment(segment: &str) -> bool {
    let tokens = segment_tokens(segment);
    let Some(command_name) = tokens.first().map(|token| normalize_command_name(token)) else {
        return false;
    };
    if command_name != "gwtd" {
        return false;
    }
    matches!(
        tokens.as_slice(),
        [_, "pane", "send", ..]
            | [_, "discuss", "goal-started", ..]
            | [_, "discuss", "goal-failed", ..]
            | [_, "discuss", "goal-skipped", ..]
    )
}

fn is_board_post_segment(segment: &str) -> bool {
    let tokens = segment_tokens(segment);
    let Some(command_name) = tokens.first().map(|token| normalize_command_name(token)) else {
        return false;
    };
    command_name == "gwtd" && matches!(tokens.as_slice(), [_, "board", "post", ..])
}

fn is_title_sensitive_tool(event: &HookEvent) -> bool {
    match event.tool_name.as_deref() {
        Some("Bash") => event.command().is_some(),
        Some("Edit" | "MultiEdit" | "Write" | "NotebookEdit" | "apply_patch") => true,
        _ => false,
    }
}

fn is_workspace_identity_update_event(event: &HookEvent) -> bool {
    if event.tool_name.as_deref() != Some("Bash") {
        return false;
    }
    let Some(command) = event.command() else {
        return false;
    };
    is_workspace_identity_update_command(command)
}

fn is_workspace_identity_update_command(command: &str) -> bool {
    let segments = super::segments::split_command_segments(command);
    if segments.len() == 1
        && segments
            .first()
            .is_some_and(|segment| is_gwtd_only_segment(segment))
        && is_workspace_identity_update_json_segment(command)
    {
        return true;
    }
    false
}

fn is_gwtd_only_segment(segment: &str) -> bool {
    let tokens = segment_tokens(segment);
    matches!(
        tokens.as_slice(),
        [command] if normalize_command_name(command) == "gwtd"
    )
}

fn is_workspace_identity_update_segment(segment: &str) -> bool {
    is_workspace_identity_update_json_segment(segment)
}

fn is_workspace_identity_update_json_segment(segment: &str) -> bool {
    let Some(json) = extract_json_object(segment) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };
    let Some(operation) = value.get("operation").and_then(|value| value.as_str()) else {
        return false;
    };
    if !matches!(operation, "workspace.update" | "workspace.ensure") {
        return false;
    }
    let Some(params) = value.get("params").and_then(|value| value.as_object()) else {
        return false;
    };
    params
        .get("purpose")
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.trim().is_empty())
        && params
            .get("current_focus")
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.trim().is_empty())
}

fn is_json_envelope_operation(command: &str, allowed_operations: &[&str]) -> bool {
    json_envelope_operation(command)
        .as_deref()
        .is_some_and(|operation| allowed_operations.contains(&operation))
}

fn is_standalone_json_envelope_command(command: &str) -> bool {
    json_envelope_operation(command).is_some()
}

fn is_read_only_json_envelope_command(command: &str) -> bool {
    json_envelope_operation(command)
        .as_deref()
        .is_some_and(is_read_only_json_envelope_operation)
}

fn json_envelope_operation(command: &str) -> Option<String> {
    let segments = super::segments::split_command_segments(command);
    if segments.len() != 1
        || !segments
            .first()
            .is_some_and(|segment| is_gwtd_only_segment(segment))
    {
        return None;
    }
    let json = extract_json_object(command)?;
    let value = serde_json::from_str::<serde_json::Value>(json).ok()?;
    value
        .get("operation")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn is_read_only_json_envelope_operation(operation: &str) -> bool {
    matches!(
        operation,
        "workspace.candidates"
            | "workspace.projection_list"
            | "workspace.projection-list"
            | "board.show"
            | "board.config.show"
            | "board.config-show"
            | "improvement.list"
            | "issue.view"
            | "issue.comments"
            | "issue.linked_prs"
            | "issue.linked-prs"
            | "issue.spec.read"
            | "issue.spec.section"
            | "issue.spec.list"
            | "issue.monitor.status"
            | "pr.current"
            | "pr.view"
            | "pr.checks"
            | "pr.reviews"
            | "pr.review_threads"
            | "pr.review-threads"
            | "actions.logs"
            | "actions.job_logs"
            | "actions.job-logs"
            | "index.status"
            | "diagnostics.cpu"
            | "daemon.status"
            | "execution.status"
            | "execution.continue"
            | "hook.health"
            | "pane.list"
            | "pane.read"
            | "pm.status"
            | "search"
    )
}

fn extract_json_object(segment: &str) -> Option<&str> {
    // Prefer the first `{"` so shell expansions like `${GWT_BIN}` before the
    // heredoc body do not shift the extraction window off the JSON envelope.
    let start = segment.find("{\"").or_else(|| segment.find('{'))?;
    let end = segment.rfind('}')?;
    (start <= end).then_some(&segment[start..=end])
}

fn is_read_only_exploration_event(event: &HookEvent) -> bool {
    if event.tool_name.as_deref() != Some("Bash") {
        return false;
    }
    let Some(command) = event.command() else {
        return false;
    };
    if has_file_output_redirection(command) {
        return false;
    }
    if is_read_only_json_envelope_command(command) {
        return true;
    }
    let segments = super::segments::split_command_segments(command);
    !segments.is_empty() && segments.iter().all(|segment| is_read_only_segment(segment))
}

/// True when the command redirects output into a real file. Detection is
/// structural (quote-aware, heredoc bodies masked), so a `>` inside a string
/// literal or heredoc payload is data, not a redirection (issue #3265).
/// Pipeline sinks like `tee` are covered by per-segment classification —
/// `tee` is not a read-only command.
fn has_file_output_redirection(command: &str) -> bool {
    !super::segments::output_redirect_file_targets(command).is_empty()
}

fn is_read_only_segment(segment: &str) -> bool {
    let tokens = segment_tokens(segment);
    let Some(command_name) = tokens.first().map(|token| normalize_command_name(token)) else {
        return true;
    };
    match command_name.as_str() {
        "awk" | "basename" | "cat" | "cut" | "date" | "dirname" | "echo" | "false" | "grep"
        | "head" | "jq" | "ls" | "nl" | "printf" | "printenv" | "pwd" | "rg" | "sort" | "tail"
        | "test" | "tr" | "true" | "uniq" | "wc" | "which" | "[" => true,
        "find" => !tokens
            .iter()
            .any(|token| matches!(*token, "-delete" | "-exec" | "-execdir" | "-ok" | "-okdir")),
        "sed" => !tokens
            .iter()
            .any(|token| *token == "--in-place" || token.starts_with("-i")),
        "command" => matches!(tokens.get(1).copied(), Some("-v")),
        "env" => tokens
            .get(1)
            .is_none_or(|token| is_read_only_command_token(token)),
        "gh" => is_read_only_gh_tokens(&tokens[1..]),
        "git" => is_read_only_git_tokens(&tokens[1..]),
        "gwtd" => is_read_only_gwtd_tokens(&tokens[1..]),
        _ => false,
    }
}

/// Read-only `gh` queries used by release monitoring. Everything else stays
/// owner-gated (`gh run rerun`, `gh release create`, ...); note that a
/// separate block-bash policy independently restricts `gh pr` / `gh issue` /
/// `gh run view` regardless of owner state.
fn is_read_only_gh_tokens(tokens: &[&str]) -> bool {
    matches!(
        tokens,
        ["release", "view" | "list", ..] | ["run", "list", ..]
    )
}

fn segment_tokens(segment: &str) -> Vec<&str> {
    let raw = segment.split_whitespace().collect::<Vec<_>>();
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

fn is_read_only_command_token(token: &str) -> bool {
    matches!(
        normalize_command_name(token).as_str(),
        "awk"
            | "cat"
            | "date"
            | "echo"
            | "false"
            | "find"
            | "grep"
            | "head"
            | "jq"
            | "ls"
            | "nl"
            | "printf"
            | "printenv"
            | "pwd"
            | "rg"
            | "sed"
            | "tail"
            | "test"
            | "true"
            | "wc"
            | "which"
            | "["
    )
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn normalize_command_name(token: &str) -> String {
    let token = token.trim_matches(|ch| ch == '\'' || ch == '"');
    // Skills resolve gwtd through `resolve_gwt_bin` and invoke it as
    // `"$GWT_BIN"`; treat that documented convention as the gwtd command so
    // envelope classification does not depend on the invocation spelling.
    if matches!(token, "$GWT_BIN" | "${GWT_BIN}") {
        return "gwtd".to_string();
    }
    Path::new(token)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(token)
        .to_string()
}

fn is_read_only_git_tokens(tokens: &[&str]) -> bool {
    let mut subcommand_index = 0;
    loop {
        match tokens.get(subcommand_index).copied() {
            Some("-C" | "-c") if tokens.get(subcommand_index + 1).is_some() => {
                subcommand_index += 2;
            }
            Some("--no-pager" | "-P") => {
                subcommand_index += 1;
            }
            Some("-C" | "-c") => return false,
            _ => break,
        }
    }
    let tokens = &tokens[subcommand_index..];
    match tokens {
        ["cat-file" | "diff" | "log" | "ls-files" | "ls-remote" | "ls-tree" | "rev-list"
        | "rev-parse" | "show" | "status", ..] => true,
        ["branch", rest @ ..] => is_read_only_git_branch_args(rest),
        ["config", rest @ ..] => is_read_only_git_config_args(rest),
        ["remote", rest @ ..] => is_read_only_git_remote_args(rest),
        ["tag", rest @ ..] => is_read_only_git_tag_args(rest),
        _ => false,
    }
}

fn is_read_only_git_branch_args(args: &[&str]) -> bool {
    if args.iter().any(|arg| is_mutating_git_branch_arg(arg)) {
        return false;
    }

    let mut list_mode = false;
    let mut has_branch_positionals = false;
    let mut pending_read_value = false;
    for arg in args {
        if pending_read_value {
            pending_read_value = false;
            continue;
        }
        if !arg.starts_with('-') {
            has_branch_positionals = true;
            continue;
        }

        let (flag, has_inline_value) = split_flag_value(arg);
        if flag == "--list" {
            list_mode = true;
            continue;
        }
        if flag == "--no-list" {
            if has_inline_value {
                return false;
            }
            list_mode = false;
            continue;
        }
        if is_value_taking_git_branch_read_flag(flag) {
            pending_read_value = !has_inline_value;
            continue;
        }
        if let Some(shorts) = read_only_git_branch_short_flags(flag) {
            if shorts.contains('l') {
                list_mode = true;
            }
            continue;
        }
        if is_valueless_git_branch_read_flag(flag) {
            continue;
        }
        return false;
    }
    !has_branch_positionals || list_mode
}

fn split_flag_value(arg: &str) -> (&str, bool) {
    arg.split_once('=')
        .map(|(flag, _)| (flag, true))
        .unwrap_or((arg, false))
}

fn is_mutating_git_branch_arg(arg: &str) -> bool {
    const MUTATING_LONG_FLAGS: &[&str] = &[
        "--copy",
        "--delete",
        "--edit-description",
        "--move",
        "--set-upstream-to",
        "--track",
        "--unset-upstream",
    ];
    let (flag, _) = split_flag_value(arg);
    if MUTATING_LONG_FLAGS.contains(&flag) {
        return true;
    }
    arg.strip_prefix('-')
        .filter(|shorts| !shorts.starts_with('-'))
        .is_some_and(|shorts| {
            shorts
                .chars()
                .any(|ch| matches!(ch, 'c' | 'C' | 'd' | 'D' | 'm' | 'M' | 'u'))
        })
}

fn is_value_taking_git_branch_read_flag(flag: &str) -> bool {
    matches!(
        flag,
        "--abbrev"
            | "--color"
            | "--column"
            | "--contains"
            | "--format"
            | "--merged"
            | "--no-contains"
            | "--no-merged"
            | "--points-at"
            | "--sort"
    )
}

fn is_valueless_git_branch_read_flag(flag: &str) -> bool {
    matches!(
        flag,
        "--all"
            | "--ignore-case"
            | "--no-ignore-case"
            | "--no-abbrev"
            | "--no-color"
            | "--no-column"
            | "--omit-empty"
            | "--quiet"
            | "--remotes"
            | "--show-current"
            | "--verbose"
    )
}

fn read_only_git_branch_short_flags(flag: &str) -> Option<&str> {
    flag.strip_prefix('-')
        .filter(|shorts| !shorts.is_empty() && !shorts.starts_with('-'))
        .filter(|shorts| {
            shorts
                .chars()
                .all(|ch| matches!(ch, 'a' | 'i' | 'l' | 'q' | 'r' | 'v'))
        })
}

fn is_read_only_git_config_args(args: &[&str]) -> bool {
    const READ_FLAGS: &[&str] = &[
        "--get",
        "--get-all",
        "--get-color",
        "--get-colorbool",
        "--get-regexp",
        "--get-urlmatch",
        "--list",
        "--name-only",
        "-l",
    ];
    const MUTATING_FLAGS: &[&str] = &[
        "--add",
        "--edit",
        "--remove-section",
        "--rename-section",
        "--replace-all",
        "--set",
        "--unset",
        "--unset-all",
    ];
    args.iter().any(|arg| READ_FLAGS.contains(arg))
        && !args.iter().any(|arg| {
            MUTATING_FLAGS.contains(arg)
                || arg
                    .split_once('=')
                    .is_some_and(|(flag, _)| MUTATING_FLAGS.contains(&flag))
        })
}

fn is_read_only_git_remote_args(args: &[&str]) -> bool {
    matches!(args, [] | ["-v" | "--verbose"] | ["show" | "get-url", ..])
}

/// `git tag` is read-only only in list/query form. Creation (`git tag v1`),
/// deletion, and re-pointing must keep requiring an owner.
fn is_read_only_git_tag_args(args: &[&str]) -> bool {
    let mut saw_query_flag = false;
    let mut saw_positional = false;
    for arg in args {
        if let Some(flag) = arg.strip_prefix("--") {
            let name = flag.split_once('=').map_or(flag, |(name, _)| name);
            match name {
                "list" | "contains" | "no-contains" | "points-at" | "merged" | "no-merged"
                | "sort" | "format" | "column" | "no-column" | "color" | "ignore-case"
                | "omit-empty" => saw_query_flag = true,
                _ => return false,
            }
        } else if let Some(flag) = arg.strip_prefix('-') {
            match flag {
                "l" | "i" => saw_query_flag = true,
                _ if flag.starts_with('n') => saw_query_flag = true,
                _ => return false,
            }
        } else {
            saw_positional = true;
        }
    }
    saw_query_flag || !saw_positional
}

fn is_read_only_gwtd_tokens(tokens: &[&str]) -> bool {
    match tokens {
        ["board", "show", ..] => true,
        ["issue", "view" | "comments" | "linked-prs", ..] => true,
        ["issue", "spec", "list", ..] => true,
        ["issue", "spec", ..] => !tokens.iter().any(|token| {
            matches!(
                *token,
                "--edit" | "--rename" | "create" | "comment" | "view" | "comments" | "linked-prs"
            )
        }),
        ["pane", "list" | "read", ..] => true,
        ["index", "status", ..] => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    // Issue #3265: the `.gwt/` bookkeeping allowance resolves the redirect
    // word the hook sees, which is *before* the shell expands it. Anything
    // that could still expand elsewhere fails closed.
    #[test]
    fn handle_with_input_ignores_empty_and_rejects_invalid_json() {
        assert_eq!(
            handle_with_input("").expect("empty input"),
            HookOutput::Silent
        );
        assert!(matches!(
            handle_with_input("{not-json"),
            Err(HookError::Json(_))
        ));
    }

    #[test]
    fn title_summary_guard_blocks_work_before_agent_title_is_set() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "cargo test -p gwt"
            })),
            transcript_path: None,
            cwd: None,
        };

        let output = evaluate_title_summary_guard(&event, true).expect("guard output");

        let HookOutput::PreToolUsePermission { detail, .. } = output else {
            panic!("expected PreToolUsePermission");
        };
        assert!(detail.contains("gwtd <<'JSON'"));
        assert!(detail.contains(r#""operation":"workspace.update""#));
        assert!(detail.contains(r#""purpose""#));
        assert!(detail.contains("work name"), "{detail}");
        assert!(detail.contains("which window is doing what"), "{detail}");
    }

    #[test]
    fn trusted_state_write_guard_blocks_direct_edits_in_all_lanes() {
        for state_file in [
            ".gwt/skill-state/execution-control.json",
            ".gwt/skill-state/execution-generation-pointer.json",
            ".gwt/skill-state/verification-run.json",
            ".gwt/skill-state/verification-plan.json",
            ".gwt/skill-state/intake-outcome.json",
            ".gwt/skill-state/action-obligations.json",
            ".gwt/skill-state/action-obligation-revival.json",
        ] {
            let event = HookEvent {
                tool_name: Some("Edit".to_string()),
                tool_input: Some(serde_json::json!({
                    "file_path": format!("E:/work/repo/{state_file}")
                })),
                transcript_path: None,
                cwd: None,
            };
            assert!(
                matches!(
                    evaluate_trusted_state_write_guard(&event).expect("guard"),
                    HookOutput::PreToolUsePermission { .. }
                ),
                "direct edit of {state_file} must be blocked"
            );
        }

        // Windows-style separators are normalized.
        let event = HookEvent {
            tool_name: Some("Write".to_string()),
            tool_input: Some(serde_json::json!({
                "file_path": "E:\\work\\repo\\.gwt\\skill-state\\execution-control.json"
            })),
            transcript_path: None,
            cwd: None,
        };
        assert!(matches!(
            evaluate_trusted_state_write_guard(&event).expect("guard"),
            HookOutput::PreToolUsePermission { .. }
        ));

        // P9b: the repo-scoped trusted store copies are equally protected,
        // including case-variant spellings on case-insensitive filesystems.
        for trusted_path in [
            "C:/Users/u/.gwt/projects/abc123/trusted/0011223344556677/execution-control.json",
            "C:\\Users\\u\\.gwt\\projects\\abc123\\trusted\\0011223344556677\\verification-run.json",
            "C:/Users/u/.GWT/Projects/abc123/Trusted/0011223344556677/EXECUTION-CONTROL.JSON",
            "E:/work/repo/.GWT/Skill-State/Verification-Plan.json",
        ] {
            let event = HookEvent {
                tool_name: Some("Write".to_string()),
                tool_input: Some(serde_json::json!({ "file_path": trusted_path })),
                transcript_path: None,
                cwd: None,
            };
            assert!(
                matches!(
                    evaluate_trusted_state_write_guard(&event).expect("guard"),
                    HookOutput::PreToolUsePermission { .. }
                ),
                "direct edit of trusted store copy {trusted_path} must be blocked"
            );
        }

        // Ordinary files and non-file tools pass.
        let event = HookEvent {
            tool_name: Some("Edit".to_string()),
            tool_input: Some(serde_json::json!({
                "file_path": "E:/work/repo/crates/gwt/src/main.rs"
            })),
            transcript_path: None,
            cwd: None,
        };
        assert_eq!(
            evaluate_trusted_state_write_guard(&event).expect("guard"),
            HookOutput::Silent
        );
        let bash = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({ "command": "cargo test" })),
            transcript_path: None,
            cwd: None,
        };
        assert_eq!(
            evaluate_trusted_state_write_guard(&bash).expect("guard"),
            HookOutput::Silent
        );
    }

    // SPEC-3248 P7A (T-076): the intake lane still blocks production code
    // edits while the standalone gwtd JSON envelope operations that settle
    // curation — Issue/SPEC ops, `intake.outcome.record`,
    // `improvement.capture`, and `memory.add` — pass the lane guard.
    #[test]
    fn stop_gate_settlement_operations_pass_ownerless() {
        let bash_event = |command: &str| HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({ "command": command })),
            transcript_path: None,
            cwd: None,
        };

        // Intake artifact gate settlement paths (FR-017).
        let intake = tempfile::tempdir().expect("repo");
        for operation in [
            "issue.create",
            "issue.comment",
            "issue.spec.create",
            "issue.spec.edit",
            "intake.outcome.record",
        ] {
            let command = format!(
                "gwtd <<'JSON'\n{{\"schema_version\":1,\"operation\":\"{operation}\",\"params\":{{}}}}\nJSON"
            );
            assert_eq!(
                evaluate_with_context(
                    &bash_event(&command),
                    intake.path(),
                    &WorkflowContext::unknown(),
                )
                .expect("guard"),
                HookOutput::Silent,
                "intake settlement op {operation} must pass PreToolUse ownerless"
            );
        }

        // Execution-side gates (execution control, obligations, evidence,
        // PR handoff) advertise these operations in their block messages.
        let execution = tempfile::tempdir().expect("repo");
        for operation in [
            "verify.plan",
            "verify.run",
            "execution.complete",
            "execution.blocked",
            "execution.adopt",
            "pr.create",
            "pr.edit",
            "pr.ready",
            "build.complete",
        ] {
            let command = format!(
                "gwtd <<'JSON'\n{{\"schema_version\":1,\"operation\":\"{operation}\",\"params\":{{}}}}\nJSON"
            );
            assert_eq!(
                evaluate_with_context(
                    &bash_event(&command),
                    execution.path(),
                    &WorkflowContext::unknown(),
                )
                .expect("guard"),
                HookOutput::Silent,
                "execution settlement op {operation} must pass PreToolUse ownerless"
            );
        }
    }

    // #3356: read-only loops and sanctioned bookkeeping writes must not
    // require an owner; production source stays owner-gated.
    #[test]
    fn ownerless_read_only_loops_and_bookkeeping_writes_pass() {
        let repo = tempfile::tempdir().expect("repo");
        let context = WorkflowContext::unknown();

        for command in [
            "for d in crates docs scripts; do ls \"$d\"; done",
            "for f in a.json b.json; do head -c 200 \"$f\"; done",
        ] {
            let event = HookEvent {
                tool_name: Some("Bash".to_string()),
                tool_input: Some(serde_json::json!({ "command": command })),
                transcript_path: None,
                cwd: None,
            };
            assert_eq!(
                evaluate_with_context(&event, repo.path(), &context).expect("guard"),
                HookOutput::Silent,
                "read-only loop must pass ownerless: {command}"
            );
        }

        let write_event = |path: String| HookEvent {
            tool_name: Some("Write".to_string()),
            tool_input: Some(serde_json::json!({ "file_path": path })),
            transcript_path: None,
            cwd: None,
        };
        // Worktree bookkeeping (any extension) and the OS temp scratchpad
        // are sanctioned ownerless surfaces.
        for path in [
            repo.path()
                .join(".gwt/work/scratch/data.json")
                .to_string_lossy()
                .to_string(),
            repo.path()
                .join("tasks/state.json")
                .to_string_lossy()
                .to_string(),
            std::env::temp_dir()
                .join("claude/session-x/scratchpad/probe.json")
                .to_string_lossy()
                .to_string(),
        ] {
            assert_eq!(
                evaluate_with_context(&write_event(path.clone()), repo.path(), &context)
                    .expect("guard"),
                HookOutput::Silent,
                "bookkeeping write must pass ownerless: {path}"
            );
        }
        // SPEC #3245 FR-009: production source writes are no longer
        // owner-gated either — every surface above and below evaluates the
        // same way for ownerless sessions.
        let production = write_event(
            repo.path()
                .join("crates/gwt/src/main.rs")
                .to_string_lossy()
                .to_string(),
        );
        assert_eq!(
            evaluate_with_context(&production, repo.path(), &context).expect("guard"),
            HookOutput::Silent,
            "production source writes must pass ownerless after the owner guard removal"
        );
    }

    #[test]
    fn issue_monitor_json_operations_have_the_expected_policy_classification() {
        assert!(is_read_only_json_envelope_operation("issue.monitor.status"));
        for operation in [
            "issue.monitor.priority.move",
            "issue.monitor.priority.set",
            "issue.monitor.config.set",
            // SPEC-3431 FR-006: launch_now mutates priority order, so it is
            // not read-only — but it must stay ownerless-safe like its siblings.
            "issue.monitor.launch_now",
        ] {
            assert!(!is_read_only_json_envelope_operation(operation));
        }

        let repo = tempfile::tempdir().expect("repo");
        let context = WorkflowContext::unknown();
        for operation in [
            "issue.monitor.status",
            "issue.monitor.priority.move",
            "issue.monitor.priority.set",
            "issue.monitor.config.set",
            "issue.monitor.launch_now",
        ] {
            let command = format!(
                "gwtd <<'JSON'\n{{\"schema_version\":1,\"operation\":\"{operation}\",\"params\":{{}}}}\nJSON"
            );
            let event = HookEvent {
                tool_name: Some("Bash".to_string()),
                tool_input: Some(serde_json::json!({"command": command})),
                transcript_path: None,
                cwd: None,
            };
            assert_eq!(
                evaluate_with_context(&event, repo.path(), &context).expect("policy"),
                HookOutput::Silent,
                "operation must pass the ownerless execution policy: {operation}"
            );
        }
    }

    #[test]
    fn evaluate_with_context_uses_explicit_title_summary_state() {
        let repo = tempfile::tempdir().expect("repo");
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "cargo test -p gwt"
            })),
            transcript_path: None,
            cwd: None,
        };
        let context = WorkflowContext::unknown().with_title_summary_missing(true);

        assert!(matches!(
            evaluate_with_context(&event, repo.path(), &context).expect("guard output"),
            HookOutput::PreToolUsePermission { .. }
        ));
    }

    #[test]
    fn title_summary_guard_blocks_legacy_argv_title_update_command() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "gwtd workspace update --agent-session sess-1 --current-focus 'Fix title visibility' --title-summary 'Agent title visibility'"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert!(matches!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::PreToolUsePermission { .. }
        ));
    }

    #[test]
    fn title_summary_guard_allows_json_envelope_workspace_update_command() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"Agent title visibility\",\"current_focus\":\"Fix title visibility\"}}\nJSON"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert_eq!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::Silent
        );
    }

    #[test]
    fn title_summary_guard_allows_installed_gwtd_json_envelope_workspace_update_command() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "GWT_BIN_PATH=/Applications/GWT.app/Contents/MacOS/gwtd /Applications/GWT.app/Contents/MacOS/gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"Agent title visibility\",\"current_focus\":\"Fix title visibility\"}}\nJSON"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert_eq!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::Silent
        );
    }

    #[test]
    fn title_summary_guard_blocks_installed_legacy_argv_title_update_command() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "GWT_BIN_PATH=/Applications/GWT.app/Contents/MacOS/gwtd /Applications/GWT.app/Contents/MacOS/gwtd workspace update --agent-session sess-1 --current-focus 'Fix title visibility' --title-summary 'Agent title visibility'"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert!(matches!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::PreToolUsePermission { .. }
        ));
    }

    #[test]
    fn title_summary_guard_blocks_chained_work_after_title_update() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"Agent title visibility\",\"current_focus\":\"Fix title visibility\"}}\nJSON\n&& cargo test -p gwt"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert!(matches!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::PreToolUsePermission { .. }
        ));
    }

    #[test]
    fn title_summary_guard_allows_read_only_exploration_before_identity_is_set() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "rg -n title_summary crates/gwt/src"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert_eq!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::Silent
        );
    }

    #[test]
    fn title_summary_guard_allows_json_envelope_read_before_identity_is_set() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"issue.view\",\"params\":{\"number\":3253}}\nJSON"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert_eq!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::Silent
        );
    }

    #[test]
    fn title_summary_guard_allows_execution_status_before_identity_is_set() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"execution.status\",\"params\":{}}\nJSON"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert_eq!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::Silent
        );
    }

    #[test]
    fn title_summary_guard_allows_pm_status_before_identity_is_set() {
        // SPEC-3431: pm.status is read-only diagnostics and must work before
        // the session identity or an owner is established.
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"pm.status\",\"params\":{}}\nJSON"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert_eq!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::Silent
        );
    }

    #[test]
    fn title_summary_guard_allows_read_only_git_config_before_identity_is_set() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "git config --list --show-origin"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert_eq!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::Silent
        );
    }

    #[test]
    fn title_summary_guard_allows_read_only_git_remote_before_identity_is_set() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "git remote -v"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert_eq!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::Silent
        );
    }

    #[test]
    fn title_summary_guard_allows_read_only_git_after_global_options() {
        for command in [
            "git -C /tmp/repository log -1",
            "git -c color.ui=false status --short",
            "git --no-pager log -1",
            "git -P show --stat HEAD",
            "git -C /tmp/repository -c color.ui=false --no-pager log -1",
        ] {
            let event = HookEvent {
                tool_name: Some("Bash".to_string()),
                tool_input: Some(serde_json::json!({ "command": command })),
                transcript_path: None,
                cwd: None,
            };

            assert_eq!(
                evaluate_title_summary_guard(&event, true).expect("guard output"),
                HookOutput::Silent,
                "{command}"
            );
        }
    }

    #[test]
    fn title_summary_guard_allows_read_only_git_branch_queries_before_identity_is_set() {
        for command in [
            "git branch --contains HEAD",
            "git branch --points-at HEAD",
            "git branch --list 'work/*'",
            "git branch --merged main",
            "git branch --no-merged origin/develop",
            "git branch --format=%(refname:short)",
            "git branch --sort=-committerdate",
            "git branch -a",
            "git branch -v",
            "git branch -avv --contains HEAD",
            "git branch -l 'work/*'",
            "git branch -i --list 'foo*'",
            "git branch --no-list",
            "git branch -l --no-list",
            "git branch new-work --list",
        ] {
            let event = HookEvent {
                tool_name: Some("Bash".to_string()),
                tool_input: Some(serde_json::json!({
                    "command": command
                })),
                transcript_path: None,
                cwd: None,
            };

            assert!(
                evaluate_title_summary_guard(&event, true).expect("guard output")
                    == HookOutput::Silent,
                "{command}"
            );
        }
    }

    #[test]
    fn title_summary_guard_blocks_mutating_exploration_like_sed_in_place() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "sed -i '' 's/a/b/' README.md"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert!(matches!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::PreToolUsePermission { .. }
        ));
    }

    #[test]
    fn title_summary_guard_blocks_mutating_find_delete() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "find target -name '*.tmp' -delete"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert!(matches!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::PreToolUsePermission { .. }
        ));
    }

    #[test]
    fn title_summary_guard_blocks_mutating_git_config() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "git config user.name Codex"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert!(matches!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::PreToolUsePermission { .. }
        ));
    }

    #[test]
    fn title_summary_guard_blocks_mutating_git_remote() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "git remote add origin https://example.com/repo.git"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert!(matches!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::PreToolUsePermission { .. }
        ));
    }

    #[test]
    fn title_summary_guard_blocks_mutating_git_branch() {
        for command in [
            "git branch new-work",
            "git branch -D old-work",
            "git branch -df old-work",
            "git branch -l --no-list new-work",
            "git branch -l new-work HEAD --no-list",
            "git branch --list new-work --no-list",
        ] {
            let event = HookEvent {
                tool_name: Some("Bash".to_string()),
                tool_input: Some(serde_json::json!({
                    "command": command
                })),
                transcript_path: None,
                cwd: None,
            };

            assert!(
                matches!(
                    evaluate_title_summary_guard(&event, true).expect("guard output"),
                    HookOutput::PreToolUsePermission { .. }
                ),
                "{command}"
            );
        }
    }

    #[test]
    fn title_summary_guard_blocks_board_posts_without_title_summary() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"board.post\",\"params\":{\"kind\":\"status\",\"body\":\"Starting implementation\"}}\nJSON"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert!(matches!(
            evaluate_title_summary_guard(&event, true).expect("guard output"),
            HookOutput::PreToolUsePermission { .. }
        ));
    }

    #[test]
    fn title_summary_guard_is_silent_after_agent_title_is_set() {
        let event = HookEvent {
            tool_name: Some("Edit".to_string()),
            tool_input: Some(serde_json::json!({
                "file_path": "crates/gwt/src/lib.rs"
            })),
            transcript_path: None,
            cwd: None,
        };

        assert_eq!(
            evaluate_title_summary_guard(&event, false).expect("guard output"),
            HookOutput::Silent
        );
    }

    #[test]
    fn evaluate_with_context_blocks_mutating_tools_until_pending_discussion_goal_starts() {
        fn pending_goal() -> PendingDiscussionGoal {
            PendingDiscussionGoal {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Goal handoff".to_string(),
                condition: "verification handoff ready with User Verification Result recorded"
                    .to_string(),
            }
        }

        let repo = tempfile::tempdir().expect("repo");

        let event = HookEvent {
            tool_name: Some("Edit".to_string()),
            tool_input: Some(serde_json::json!({
                "file_path": "crates/gwt/src/lib.rs"
            })),
            transcript_path: None,
            cwd: None,
        };

        let output = evaluate_with_context(
            &event,
            repo.path(),
            &WorkflowContext::unknown().with_pending_discussion_goal(Some(pending_goal())),
        )
        .expect("guard output");

        let HookOutput::PreToolUsePermission { detail, .. } = output else {
            panic!("expected pending goal guard");
        };
        assert!(
            detail.contains("pending gwt-discussion Goal Start"),
            "{detail}"
        );
        assert!(detail.contains("create_goal"), "{detail}");
        assert!(detail.contains("discuss.goal_started"), "{detail}");
        assert!(detail.contains("discuss.goal_skipped"), "{detail}");
        assert!(
            detail.contains("verification handoff ready with User Verification Result recorded"),
            "{detail}"
        );

        let allowed = HookEvent {
            tool_name: Some("create_goal".to_string()),
            tool_input: Some(serde_json::json!({})),
            transcript_path: None,
            cwd: None,
        };
        assert_eq!(
            evaluate_with_context(
                &allowed,
                repo.path(),
                &WorkflowContext::unknown().with_pending_discussion_goal(Some(pending_goal())),
            )
            .expect("allowed output"),
            HookOutput::Silent
        );

        for command in [
            "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"discuss.goal_started\",\"params\":{\"proposal\":\"A\"}}\nJSON",
            "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"discuss.goal_failed\",\"params\":{\"proposal\":\"A\",\"reason\":\"cannot start\"}}\nJSON",
            "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"workspace.update\",\"params\":{\"purpose\":\"Goal setup\",\"current_focus\":\"Starting goal\"}}\nJSON",
        ] {
            let allowed = HookEvent {
                tool_name: Some("Bash".to_string()),
                tool_input: Some(serde_json::json!({ "command": command })),
                transcript_path: None,
                cwd: None,
            };
            assert_eq!(
                evaluate_with_context(
                    &allowed,
                    repo.path(),
                    &WorkflowContext::unknown().with_pending_discussion_goal(Some(pending_goal())),
                )
                .expect("allowed JSON bookkeeping output"),
                HookOutput::Silent,
                "{command}"
            );
        }
    }
}
