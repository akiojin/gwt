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
use gwt_core::paths::{gwt_cache_dir, gwt_sessions_dir};
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
) -> Result<HookOutput, HookError> {
    // Safety guardrails only: branch switching, worktree escape, direct gh CLI.
    // No owner gate — git push/commit and worktree-internal edits are always allowed.
    block_bash_policy::evaluate(event, worktree_root)
}

pub fn evaluate(event: &HookEvent, worktree_root: &Path) -> Result<HookOutput, HookError> {
    let context = resolve_workflow_context(worktree_root);
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
    use std::{collections::HashMap, ffi::OsString, fs};

    use gwt_agent::{session::Session, types::AgentId};
    use gwt_github::{
        client::{IssueNumber, IssueSnapshot, IssueState, UpdatedAt},
        Cache,
    };

    use super::*;

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
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

    fn init_repo(repo: &Path) {
        fs::create_dir_all(repo).expect("create repo");
        let mut init_cmd = std::process::Command::new("git");
        init_cmd.args(["init", "--quiet"]).current_dir(repo);
        gwt_core::process::scrub_git_env(&mut init_cmd);
        let init = init_cmd.output().expect("git init");
        assert!(init.status.success(), "git init failed");

        let mut remote_cmd = std::process::Command::new("git");
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

        let mut branch_cmd = std::process::Command::new("git");
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
    fn resolve_workflow_context_uses_session_cache_and_linkage_store() {
        let _lock = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
