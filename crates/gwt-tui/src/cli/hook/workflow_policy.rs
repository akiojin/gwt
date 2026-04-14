//! `gwt hook workflow-policy` — hook-driven workflow gating.
//!
//! v1 keeps the policy deliberately narrow:
//!
//! - reuse the existing consolidated Bash safety policy first
//! - block mutating tool calls when no owner Issue/SPEC is linked
//! - if the owner is a `gwt-spec` Issue, require non-empty `plan` and `tasks`
//!   sections before code implementation proceeds
//! - allow read-only investigation and docs/chore-style edits

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gwt_agent::session::{Session, GWT_SESSION_ID_ENV};
use gwt_agent::types::WorkflowBypass;
use gwt_core::paths::{gwt_cache_dir, gwt_sessions_dir};
use gwt_github::{body::SpecBody, sections::SectionName, Cache, IssueNumber};
use serde::Deserialize;

use super::{block_bash_policy, block_file_ops, BlockDecision, HookError, HookEvent};

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
}

impl WorkflowContext {
    pub fn unknown() -> Self {
        Self {
            owner: WorkflowOwner::Unknown,
            has_plan: false,
            has_tasks: false,
            bypass: None,
        }
    }

    pub fn plain_issue(issue_number: u64) -> Self {
        Self {
            owner: WorkflowOwner::Issue(issue_number),
            has_plan: false,
            has_tasks: false,
            bypass: None,
        }
    }

    pub fn spec_issue(issue_number: u64, has_plan: bool, has_tasks: bool) -> Self {
        Self {
            owner: WorkflowOwner::Spec(issue_number),
            has_plan,
            has_tasks,
            bypass: None,
        }
    }

    pub fn with_bypass(bypass: WorkflowBypass) -> Self {
        Self {
            owner: WorkflowOwner::Unknown,
            has_plan: false,
            has_tasks: false,
            bypass: Some(bypass),
        }
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
) -> Result<Option<BlockDecision>, HookError> {
    // 1. Legacy safety (gh issue CLI, branch ops, worktree-external file ops)
    if let Some(decision) = block_bash_policy::evaluate(event, worktree_root)? {
        return Ok(Some(decision));
    }

    // 2. Docs/chore path exemption + non-mutating tools
    if is_exempt_chore_change(event) || !is_mutating_tool_call(event, worktree_root) {
        return Ok(None);
    }

    // 3. Worktree-internal operations are allowed without owner
    if is_worktree_internal_operation(event, worktree_root) {
        return Ok(None);
    }

    // 4. Session-level bypass (release/chore workflows)
    if context.bypass.is_some() {
        return Ok(None);
    }

    // 5. Owner gate (only reached for external operations like git push)
    match context.owner {
        WorkflowOwner::Unknown => Ok(Some(missing_owner_decision())),
        WorkflowOwner::Issue(_) => Ok(None),
        WorkflowOwner::Spec(issue_number) if !context.has_plan || !context.has_tasks => Ok(Some(
            missing_spec_artifacts_decision(issue_number, context.has_plan, context.has_tasks),
        )),
        WorkflowOwner::Spec(_) => Ok(None),
    }
}

pub fn evaluate(
    event: &HookEvent,
    worktree_root: &Path,
) -> Result<Option<BlockDecision>, HookError> {
    let context = resolve_workflow_context(worktree_root);
    evaluate_with_context(event, worktree_root, &context)
}

pub fn handle() -> Result<Option<BlockDecision>, HookError> {
    let Some(event) = HookEvent::read_from_stdin()? else {
        return Ok(None);
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
    Session::load(&session_path).ok()
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

fn is_worktree_internal_operation(event: &HookEvent, worktree_root: &Path) -> bool {
    match event.tool_name.as_deref() {
        Some("Edit" | "Write" | "MultiEdit") => extract_target_path(event)
            .is_some_and(|path| is_path_within_worktree(&path, worktree_root)),
        Some("Bash") => event
            .command()
            .is_some_and(|cmd| !has_external_side_effects(cmd)),
        _ => true,
    }
}

fn has_external_side_effects(command: &str) -> bool {
    for segment in super::segments::split_command_segments(command) {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if tokens.first().copied() == Some("git") && tokens.get(1).copied() == Some("push") {
            return true;
        }
    }
    false
}

fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        use std::path::Component::*;
        match comp {
            ParentDir => {
                out.pop();
            }
            CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn is_path_within_worktree(path: &Path, worktree_root: &Path) -> bool {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        worktree_root.join(path)
    };
    let abs = normalize_path(&abs);
    let root = normalize_path(worktree_root);
    abs == root || abs.starts_with(&root)
}

fn is_mutating_tool_call(event: &HookEvent, worktree_root: &Path) -> bool {
    match event.tool_name.as_deref() {
        Some("Edit" | "Write" | "MultiEdit") => true,
        Some("Bash") => event
            .command()
            .is_some_and(|command| is_mutating_bash_command(command, worktree_root)),
        _ => false,
    }
}

fn is_exempt_chore_change(event: &HookEvent) -> bool {
    matches!(
        event.tool_name.as_deref(),
        Some("Edit" | "Write" | "MultiEdit")
    ) && extract_target_path(event).is_some_and(is_docs_or_chore_path)
}

fn extract_target_path(event: &HookEvent) -> Option<PathBuf> {
    let tool_input = event.tool_input.as_ref()?;
    let raw = tool_input
        .get("file_path")
        .or_else(|| tool_input.get("path"))
        .and_then(serde_json::Value::as_str)?;
    Some(PathBuf::from(raw))
}

fn is_docs_or_chore_path(path: PathBuf) -> bool {
    let path = path.as_path();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let path_text = path.to_string_lossy();

    file_name.eq_ignore_ascii_case("AGENTS.md")
        || file_name.ends_with(".md")
        || path_text.starts_with("docs/")
        || path_text.starts_with("tasks/")
        || path_text.starts_with(".gwt/")
        || path_text.starts_with(".claude/")
        || path_text.starts_with(".codex/")
        || path_text.starts_with(".github/")
}

fn is_mutating_bash_command(command: &str, worktree_root: &Path) -> bool {
    for segment in super::segments::split_command_segments(command) {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }

        if contains_shell_write_operator(trimmed) {
            return true;
        }

        let tokens = command_tokens(trimmed);
        let Some(command_name) = tokens.first().copied() else {
            continue;
        };
        let subcommand = tokens.get(1).copied().unwrap_or_default();

        if is_worktree_local_file_op(command_name, trimmed, worktree_root) {
            continue;
        }

        match command_name {
            "touch" | "mkdir" | "rm" | "mv" | "cp" | "install" | "tee" | "truncate" => {
                return true;
            }
            "sed"
                if tokens
                    .iter()
                    .any(|token| *token == "-i" || *token == "--in-place") =>
            {
                return true;
            }
            "perl" if tokens.iter().any(|token| token.starts_with("-i")) => {
                return true;
            }
            "cargo" if subcommand == "fmt" => {
                return true;
            }
            "git" if matches!(subcommand, "add" | "commit" | "push" | "rm" | "mv") => {
                return true;
            }
            "npx" | "bunx" | "prettier" if tokens.contains(&"--write") => {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn is_worktree_local_file_op(command_name: &str, segment: &str, worktree_root: &Path) -> bool {
    matches!(
        command_name,
        "mkdir" | "rmdir" | "rm" | "touch" | "cp" | "mv"
    ) && block_file_ops::evaluate_bash_command(segment, worktree_root).is_none()
}

fn contains_shell_write_operator(command: &str) -> bool {
    command.contains(">>")
        || command.contains(">|")
        || command.contains(">")
        || command.contains("<<")
}

fn command_tokens(segment: &str) -> Vec<&str> {
    let raw: Vec<&str> = segment.split_whitespace().collect();
    let mut start = 0;

    if raw.get(start) == Some(&"env") {
        start += 1;
    }
    while start < raw.len() && is_env_assignment(raw[start]) {
        start += 1;
    }

    raw[start..].to_vec()
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn missing_owner_decision() -> BlockDecision {
    BlockDecision::new(
        "workflow owner is required before implementation",
        "This tool call would change project state before an owner Issue or SPEC is linked.\n\n\
Continue with `gwt-discussion` to settle the owner, run `gwt-search` if scope ownership is unclear, or relaunch from a linked Issue before editing code.",
    )
}

fn missing_spec_artifacts_decision(
    issue_number: u64,
    has_plan: bool,
    has_tasks: bool,
) -> BlockDecision {
    let mut missing = Vec::new();
    if !has_plan {
        missing.push("plan");
    }
    if !has_tasks {
        missing.push("tasks");
    }
    let missing_text = missing.join(" + ");
    BlockDecision::new(
        format!("linked SPEC #{issue_number} is missing {missing_text}"),
        format!(
            "Issue #{issue_number} is a `gwt-spec` owner, so implementation stays blocked until `{missing_text}` is ready.\n\n\
Use `gwt-discussion` to finalize the decision surface, then update the owner through `gwt-plan-spec` before resuming code changes."
        ),
    )
}
