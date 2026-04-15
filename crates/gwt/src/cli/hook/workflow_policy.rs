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
use std::path::Path;

use gwt_agent::session::{Session, GWT_SESSION_ID_ENV};
use gwt_agent::types::WorkflowBypass;
use gwt_core::paths::{gwt_cache_dir, gwt_sessions_dir};
use gwt_github::{body::SpecBody, sections::SectionName, Cache, IssueNumber};
use serde::Deserialize;

use super::{block_bash_policy, BlockDecision, HookError, HookEvent};

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
    _context: &WorkflowContext,
) -> Result<Option<BlockDecision>, HookError> {
    // Safety guardrails only: branch switching, worktree escape, direct gh CLI.
    // No owner gate — git push/commit and worktree-internal edits are always allowed.
    block_bash_policy::evaluate(event, worktree_root)
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
