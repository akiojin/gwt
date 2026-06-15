//! `gwtd hook workflow-policy` — hook-driven workflow gating.
//!
//! v1 keeps the policy deliberately narrow:
//!
//! - reuse the existing consolidated Bash safety policy first
//! - apply only safety guardrails that are independent of Issue/SPEC ownership
//! - block worktree escape, branch-switching, and direct GitHub workflow CLI
//!   commands before they reach the tool runtime
//! - allow transport operations such as `git push` and worktree-internal edits
//!   without an owner gate

use std::{collections::HashMap, io::Read, path::Path};

use gwt_agent::{
    session::{Session, GWT_SESSION_ID_ENV},
    types::WorkflowBypass,
};
use gwt_core::{
    paths::{gwt_cache_dir, gwt_sessions_dir},
    workspace_projection::load_workspace_projection,
};
use gwt_github::{body::SpecBody, sections::SectionName, Cache, IssueNumber};
use serde::Deserialize;

use super::{block_bash_policy, HookError, HookEvent, HookOutput};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowOwner {
    Unknown,
    Issue(u64),
    Spec(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowContext {
    pub owner: WorkflowOwner,
    pub has_plan: bool,
    pub has_tasks: bool,
    pub bypass: Option<WorkflowBypass>,
    pub title_summary_missing: bool,
    pub pending_discussion_goal: bool,
}

impl WorkflowContext {
    pub fn unknown() -> Self {
        Self {
            owner: WorkflowOwner::Unknown,
            has_plan: false,
            has_tasks: false,
            bypass: None,
            title_summary_missing: false,
            pending_discussion_goal: false,
        }
    }

    pub fn plain_issue(issue_number: u64) -> Self {
        Self {
            owner: WorkflowOwner::Issue(issue_number),
            has_plan: false,
            has_tasks: false,
            bypass: None,
            title_summary_missing: false,
            pending_discussion_goal: false,
        }
    }

    pub fn spec_issue(issue_number: u64, has_plan: bool, has_tasks: bool) -> Self {
        Self {
            owner: WorkflowOwner::Spec(issue_number),
            has_plan,
            has_tasks,
            bypass: None,
            title_summary_missing: false,
            pending_discussion_goal: false,
        }
    }

    pub fn with_bypass(bypass: WorkflowBypass) -> Self {
        Self {
            owner: WorkflowOwner::Unknown,
            has_plan: false,
            has_tasks: false,
            bypass: Some(bypass),
            title_summary_missing: false,
            pending_discussion_goal: false,
        }
    }

    pub fn with_title_summary_missing(mut self, missing: bool) -> Self {
        self.title_summary_missing = missing;
        self
    }

    pub fn with_pending_discussion_goal(mut self, pending: bool) -> Self {
        self.pending_discussion_goal = pending;
        self
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct IssueBranchLinkStore {
    branches: HashMap<String, u64>,
}

pub fn evaluate_with_context(
    event: &HookEvent,
    worktree_root: &Path,
    context: &WorkflowContext,
) -> Result<HookOutput, HookError> {
    // Safety guardrails only: branch switching, worktree escape, direct gh CLI.
    // No owner gate — git push/commit and worktree-internal edits are always allowed.
    let safety = block_bash_policy::evaluate(event, worktree_root)?;
    if safety != HookOutput::Silent {
        return Ok(safety);
    }
    let title_summary = evaluate_title_summary_guard(event, context.title_summary_missing)?;
    if title_summary != HookOutput::Silent {
        return Ok(title_summary);
    }
    let pending_goal =
        evaluate_pending_discussion_goal_guard(event, context.pending_discussion_goal)?;
    if pending_goal != HookOutput::Silent {
        return Ok(pending_goal);
    }
    Ok(HookOutput::Silent)
}

pub fn evaluate(event: &HookEvent, worktree_root: &Path) -> Result<HookOutput, HookError> {
    let context = resolve_workflow_context(worktree_root)
        .with_title_summary_missing(current_agent_workspace_identity_missing(worktree_root)?)
        .with_pending_discussion_goal(
            crate::discussion_resume::load_pending_goal(worktree_root)
                .ok()
                .flatten()
                .is_some(),
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

fn resolve_workflow_context(worktree_root: &Path) -> WorkflowContext {
    let session = load_session_from_env();
    let bypass = session.as_ref().and_then(|s| s.workflow_bypass);

    let Some(issue_number) = session
        .as_ref()
        .and_then(|session| session.linked_issue_number)
        .or_else(|| resolve_issue_from_linkage_store(worktree_root, session.as_ref()))
    else {
        let mut ctx = WorkflowContext::unknown();
        ctx.bypass = bypass;
        return ctx;
    };

    let Some(cache_root) = crate::issue_cache::issue_cache_root_for_repo_path(worktree_root) else {
        let mut ctx = WorkflowContext::plain_issue(issue_number);
        ctx.bypass = bypass;
        return ctx;
    };
    let cache = Cache::new(cache_root);
    let Some(entry) = cache.load_entry(IssueNumber(issue_number)) else {
        let mut ctx = WorkflowContext::plain_issue(issue_number);
        ctx.bypass = bypass;
        return ctx;
    };
    if !entry
        .snapshot
        .labels
        .iter()
        .any(|label| label == "gwt-spec")
    {
        let mut ctx = WorkflowContext::plain_issue(issue_number);
        ctx.bypass = bypass;
        return ctx;
    }

    let mut ctx = WorkflowContext::spec_issue(
        issue_number,
        has_nonempty_section(&entry.spec_body, "plan"),
        has_nonempty_section(&entry.spec_body, "tasks"),
    );
    ctx.bypass = bypass;
    ctx
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
    let projection_root = if session.worktree_path.exists() {
        session.worktree_path.as_path()
    } else {
        worktree_root
    };
    let Some(projection) = load_workspace_projection(projection_root)? else {
        return Ok(false);
    };
    let Some(agent) = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session.id)
    else {
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

    if is_title_sensitive_tool(event) || is_read_only_exploration_event(event) {
        return Ok(HookOutput::pre_tool_use_permission(
            "Agent Workspace identity is required before work starts",
            "Set both a short work name and current focus before exploration, implementation, or verification commands. This is required so Workspace can show which window is doing what.\n\n\
Required command shape:\n\
  gwtd workspace update --agent-session \"$GWT_SESSION_ID\" --current-focus '<current work focus>' --title-summary '<short work title>'\n\n\
Good example: --title-summary 'Agent title improvement'\n\
Bad example: --title-summary 'Agent title improvement complete'\n\n\
Use the configured narrative language for the title-summary. Keep progress, completion, blocker state, and long detail in --current-focus, --summary, or Board --body.",
        ));
    }

    Ok(HookOutput::Silent)
}

fn evaluate_pending_discussion_goal_guard(
    event: &HookEvent,
    pending_discussion_goal: bool,
) -> Result<HookOutput, HookError> {
    if !pending_discussion_goal || is_goal_start_or_bookkeeping_event(event) {
        return Ok(HookOutput::Silent);
    }
    if is_mutating_work_event(event) {
        return Ok(HookOutput::pre_tool_use_permission(
            "pending gwt-discussion Goal Start must be handled first",
            "A gwt-discussion Action Bundle has a pending gwt-discussion Goal Start. Start, skip, or record the goal failure before changing implementation state.\n\n\
Codex path: call `create_goal` with the pending Goal condition, then run `gwtd discuss goal-started --proposal \"<label>\"`.\n\
Claude Code path: run `gwtd pane send --text '/goal <condition>'`, then run `gwtd discuss goal-started --proposal \"<label>\"`.\n\
Skip path: if the user rejects or revises the Action Bundle, run `gwtd discuss goal-skipped --proposal \"<label>\" --reason '<reason>'`.\n\
Failure path: run `gwtd discuss goal-failed --proposal \"<label>\" --reason '<reason>'` and show the manual `/goal <condition>` line to the user.",
        ));
    }
    Ok(HookOutput::Silent)
}

fn is_mutating_work_event(event: &HookEvent) -> bool {
    match event.tool_name.as_deref() {
        Some("Edit" | "MultiEdit" | "Write" | "NotebookEdit" | "apply_patch") => true,
        Some("Bash") => {
            event.command().is_some()
                && !is_read_only_exploration_event(event)
                && !is_goal_start_or_bookkeeping_event(event)
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
    let segments = super::segments::split_command_segments(command);
    !segments.is_empty()
        && segments
            .iter()
            .all(|segment| is_workspace_identity_update_segment(segment))
}

fn is_workspace_identity_update_segment(segment: &str) -> bool {
    let tokens = segment_tokens(segment);
    let Some(command_name) = tokens.first().map(|token| normalize_command_name(token)) else {
        return false;
    };
    if command_name != "gwtd" {
        return false;
    }
    match tokens.as_slice() {
        [_, "workspace", "update", rest @ ..] => {
            rest.contains(&"--agent-session")
                && rest.contains(&"--title-summary")
                && rest.contains(&"--current-focus")
        }
        [_, "workspace", "ensure", rest @ ..] => {
            rest.contains(&"--agent-session")
                && rest.contains(&"--title-summary")
                && rest.contains(&"--current-focus")
        }
        _ => false,
    }
}

fn is_read_only_exploration_event(event: &HookEvent) -> bool {
    if event.tool_name.as_deref() != Some("Bash") {
        return false;
    }
    let Some(command) = event.command() else {
        return false;
    };
    if command.contains('>') || command.contains(" tee ") || command.contains("|tee ") {
        return false;
    }
    let segments = super::segments::split_command_segments(command);
    !segments.is_empty() && segments.iter().all(|segment| is_read_only_segment(segment))
}

fn is_read_only_segment(segment: &str) -> bool {
    let tokens = segment_tokens(segment);
    let Some(command_name) = tokens.first().map(|token| normalize_command_name(token)) else {
        return true;
    };
    match command_name.as_str() {
        "awk" | "cat" | "date" | "false" | "grep" | "head" | "jq" | "ls" | "nl" | "printenv"
        | "pwd" | "rg" | "tail" | "test" | "true" | "wc" | "which" | "[" => true,
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
        "git" => is_read_only_git_tokens(&tokens[1..]),
        "gwtd" => is_read_only_gwtd_tokens(&tokens[1..]),
        _ => false,
    }
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
            | "false"
            | "find"
            | "grep"
            | "head"
            | "jq"
            | "ls"
            | "nl"
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
    Path::new(token)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(token)
        .to_string()
}

fn is_read_only_git_tokens(tokens: &[&str]) -> bool {
    match tokens {
        ["cat-file" | "diff" | "log" | "ls-files" | "ls-tree" | "rev-parse" | "show" | "status", ..] => {
            true
        }
        ["branch", rest @ ..] => is_read_only_git_branch_args(rest),
        ["config", rest @ ..] => is_read_only_git_config_args(rest),
        ["remote", rest @ ..] => is_read_only_git_remote_args(rest),
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

fn resolve_issue_from_linkage_store(
    worktree_root: &Path,
    session: Option<&Session>,
) -> Option<u64> {
    let repo_hash = crate::index_worker::detect_repo_hash(worktree_root)?;
    let store_path = gwt_cache_dir()
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    let bytes = std::fs::read(&store_path).ok()?;
    let store: IssueBranchLinkStore = serde_json::from_slice(&bytes).ok()?;
    let branch = resolve_branch_name(worktree_root, session)?;
    store.branches.get(&branch).copied()
}

fn resolve_branch_name(worktree_root: &Path, session: Option<&Session>) -> Option<String> {
    if let Some(branch) = session
        .map(|session| session.branch.trim())
        .filter(|branch| !branch.is_empty())
    {
        return Some(branch.to_string());
    }

    gwt_git::Repository::discover(worktree_root)
        .ok()?
        .current_branch()
        .ok()?
}

fn has_nonempty_section(spec_body: &SpecBody, name: &str) -> bool {
    spec_body
        .sections
        .get(&SectionName(name.to_string()))
        .is_some_and(|content| !content.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use gwt_agent::{session::Session, types::AgentId};
    use gwt_github::{
        client::{IssueNumber, IssueSnapshot, IssueState, UpdatedAt},
        Cache,
    };

    use super::*;

    use gwt_core::test_support::ScopedEnvVar;

    fn init_repo(repo: &Path) {
        fs::create_dir_all(repo).expect("create repo");
        let mut init_cmd = gwt_core::process::hidden_command("git");
        init_cmd.args(["init", "--quiet"]).current_dir(repo);
        gwt_core::process::scrub_git_env(&mut init_cmd);
        let init = init_cmd.output().expect("git init");
        assert!(init.status.success(), "git init failed");

        let mut remote_cmd = gwt_core::process::hidden_command("git");
        remote_cmd
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/example/repo.git",
            ])
            .current_dir(repo);
        gwt_core::process::scrub_git_env(&mut remote_cmd);
        let remote = remote_cmd.output().expect("git remote add");
        assert!(remote.status.success(), "git remote add failed");

        let mut branch_cmd = gwt_core::process::hidden_command("git");
        branch_cmd
            .args(["checkout", "-b", "feature/coverage"])
            .current_dir(repo);
        gwt_core::process::scrub_git_env(&mut branch_cmd);
        let branch = branch_cmd.output().expect("git checkout");
        assert!(branch.status.success(), "git checkout failed");
    }

    fn issue_snapshot(number: u64, labels: &[&str], body: &str) -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(number),
            title: format!("Issue {number}"),
            body: body.to_string(),
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-04-20T00:00:00Z"),
            comments: Vec::new(),
        }
    }

    fn spec_body_with_plan_and_tasks() -> &'static str {
        r#"<!-- gwt-spec id=2001 version=1 -->
<!-- sections:
spec=body
plan=body
tasks=body
-->
<!-- artifact:spec BEGIN -->
Coverage requirements.
<!-- artifact:spec END -->

<!-- artifact:plan BEGIN -->
1. Add tests.
<!-- artifact:plan END -->

<!-- artifact:tasks BEGIN -->
- [ ] Enforce pre-push coverage.
<!-- artifact:tasks END -->
"#
    }

    fn write_issue_links(repo_path: &Path, links: &[(&str, u64)]) {
        let repo_hash = crate::index_worker::detect_repo_hash(repo_path).expect("repo hash");
        let path = gwt_cache_dir()
            .join("issue-links")
            .join(format!("{}.json", repo_hash.as_str()));
        fs::create_dir_all(path.parent().expect("issue-links dir"))
            .expect("create issue-links dir");
        let branches = links
            .iter()
            .map(|(branch, number)| ((*branch).to_string(), *number))
            .collect::<HashMap<_, _>>();
        fs::write(
            path,
            serde_json::to_vec(&serde_json::json!({ "branches": branches })).expect("json"),
        )
        .expect("write issue links");
    }

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
        assert!(detail.contains("gwtd workspace update"));
        assert!(detail.contains("--title-summary"));
        assert!(detail.contains("--agent-session"));
        assert!(detail.contains("work name"), "{detail}");
        assert!(detail.contains("which window is doing what"), "{detail}");
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
    fn title_summary_guard_allows_title_update_command() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "gwtd workspace update --agent-session sess-1 --current-focus 'Fix title visibility' --title-summary 'Agent title visibility'"
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
    fn title_summary_guard_allows_installed_gwtd_title_update_command() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "GWT_BIN_PATH=/Applications/GWT.app/Contents/MacOS/gwtd /Applications/GWT.app/Contents/MacOS/gwtd workspace update --agent-session sess-1 --current-focus 'Fix title visibility' --title-summary 'Agent title visibility'"
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
    fn title_summary_guard_blocks_chained_work_after_title_update() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "gwtd workspace update --agent-session sess-1 --current-focus 'Fix title visibility' --title-summary 'Agent title visibility' && cargo test -p gwt"
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
    fn title_summary_guard_blocks_read_only_exploration_before_identity_is_set() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "rg -n title_summary crates/gwt/src"
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
    fn title_summary_guard_blocks_read_only_git_config_before_identity_is_set() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "git config --list --show-origin"
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
    fn title_summary_guard_blocks_read_only_git_remote_before_identity_is_set() {
        let event = HookEvent {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(serde_json::json!({
                "command": "git remote -v"
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
    fn title_summary_guard_blocks_read_only_git_branch_queries_before_identity_is_set() {
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
                matches!(
                    evaluate_title_summary_guard(&event, true).expect("guard output"),
                    HookOutput::PreToolUsePermission { .. }
                ),
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
                "command": "gwtd board post --kind status --body 'Starting implementation'"
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
            std::path::Path::new("."),
            &WorkflowContext::unknown().with_pending_discussion_goal(true),
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
        assert!(detail.contains("goal-started"), "{detail}");
        assert!(detail.contains("goal-skipped"), "{detail}");

        let allowed = HookEvent {
            tool_name: Some("create_goal".to_string()),
            tool_input: Some(serde_json::json!({})),
            transcript_path: None,
            cwd: None,
        };
        assert_eq!(
            evaluate_with_context(
                &allowed,
                std::path::Path::new("."),
                &WorkflowContext::unknown().with_pending_discussion_goal(true),
            )
            .expect("allowed output"),
            HookOutput::Silent
        );
    }

    #[test]
    fn resolve_workflow_context_uses_session_cache_and_linkage_store() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _lock = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let repo = home.path().join("repo");
        init_repo(&repo);

        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("repo cache root");
        let cache = Cache::new(cache_root);
        cache
            .write_snapshot(&issue_snapshot(41, &["bug"], "plain issue body"))
            .expect("write plain issue");
        cache
            .write_snapshot(&issue_snapshot(
                42,
                &["gwt-spec", "phase/in-progress"],
                spec_body_with_plan_and_tasks(),
            ))
            .expect("write spec issue");

        let mut session = Session::new(&repo, "feature/coverage", AgentId::Codex);
        session.linked_issue_number = Some(42);
        session.workflow_bypass = Some(WorkflowBypass::Chore);
        session.save(&gwt_sessions_dir()).expect("save session");
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);

        let context = resolve_workflow_context(&repo);
        assert_eq!(context.owner, WorkflowOwner::Spec(42));
        assert!(context.has_plan);
        assert!(context.has_tasks);
        assert_eq!(context.bypass, Some(WorkflowBypass::Chore));

        let loaded = load_session_from_env().expect("session from env");
        assert_eq!(loaded.id, session.id);

        write_issue_links(&repo, &[("feature/coverage", 41)]);
        session.linked_issue_number = None;
        session.save(&gwt_sessions_dir()).expect("update session");

        let linked_issue = resolve_issue_from_linkage_store(&repo, Some(&session));
        assert_eq!(linked_issue, Some(41));
        assert_eq!(
            resolve_branch_name(&repo, Some(&session)).as_deref(),
            Some("feature/coverage")
        );

        let plain_context = resolve_workflow_context(&repo);
        assert_eq!(plain_context.owner, WorkflowOwner::Issue(41));
        assert!(!plain_context.has_plan);
        assert!(!plain_context.has_tasks);

        let spec_body = gwt_github::body::SpecBody::parse(spec_body_with_plan_and_tasks(), &[])
            .expect("parse spec body");
        assert!(has_nonempty_section(&spec_body, "plan"));
        assert!(has_nonempty_section(&spec_body, "tasks"));
        assert!(!has_nonempty_section(&spec_body, "notes"));
    }
}
