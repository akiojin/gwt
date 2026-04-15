//! T-112 (SPEC #1935) — workflow-policy gating tests.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use gwt::cli::hook::{workflow_policy, HookEvent};
use gwt_agent::{session::GWT_SESSION_ID_ENV, AgentId, Session};
use gwt_core::{paths::gwt_sessions_dir, repo_hash::compute_repo_hash};
use gwt_github::{
    client::{IssueNumber, IssueSnapshot, IssueState, UpdatedAt},
    Cache,
};
use serde_json::json;
use tempfile::TempDir;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn root() -> PathBuf {
    PathBuf::from("/tmp/gwt-test-worktree")
}

fn event(tool_name: &str, tool_input: serde_json::Value) -> HookEvent {
    serde_json::from_value(json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
    }))
    .expect("valid hook event")
}

fn evaluate(
    event: &HookEvent,
    context: workflow_policy::WorkflowContext,
) -> Option<gwt::cli::hook::BlockDecision> {
    workflow_policy::evaluate_with_context(event, Path::new(&root()), &context)
        .expect("evaluation should succeed")
}

fn with_temp_home<T>(f: impl FnOnce(&TempDir) -> T) -> T {
    let _guard = env_lock().lock().expect("env lock");
    let home = tempfile::tempdir().expect("temp home");
    let previous_home = std::env::var_os("HOME");
    let previous_session_id = std::env::var_os(GWT_SESSION_ID_ENV);
    std::env::set_var("HOME", home.path());
    std::env::remove_var(GWT_SESSION_ID_ENV);

    let result = f(&home);

    if let Some(value) = previous_home {
        std::env::set_var("HOME", value);
    } else {
        std::env::remove_var("HOME");
    }
    if let Some(value) = previous_session_id {
        std::env::set_var(GWT_SESSION_ID_ENV, value);
    } else {
        std::env::remove_var(GWT_SESSION_ID_ENV);
    }

    result
}

fn init_repo(home: &TempDir) -> PathBuf {
    let repo_path = home.path().join("repo");
    std::fs::create_dir_all(&repo_path).expect("create repo dir");
    assert!(std::process::Command::new("git")
        .arg("init")
        .arg(&repo_path)
        .status()
        .expect("git init")
        .success());
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(&repo_path)
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/gwt-test.git"
        ])
        .status()
        .expect("git remote add")
        .success());
    repo_path
}

fn seed_issue_cache(
    repo_path: &Path,
    issue_number: u64,
    labels: Vec<&str>,
    plan: &str,
    tasks: &str,
) {
    let repo_hash = compute_repo_hash("https://github.com/example/gwt-test.git");
    let cache_root = repo_path
        .parent()
        .expect("repo parent")
        .join(".gwt/cache/issues")
        .join(repo_hash.as_str());
    let cache = Cache::new(cache_root);
    let body = format!(
        "<!-- gwt-spec id={issue_number} version=1 -->\n\
<!-- sections:\n\
spec=body\n\
plan=body\n\
tasks=body\n\
-->\n\
<!-- artifact:spec BEGIN -->\n\
Workflow policy\n\
<!-- artifact:spec END -->\n\
<!-- artifact:plan BEGIN -->\n\
{plan}\n\
<!-- artifact:plan END -->\n\
<!-- artifact:tasks BEGIN -->\n\
{tasks}\n\
<!-- artifact:tasks END -->\n"
    );
    cache
        .write_snapshot(&IssueSnapshot {
            number: IssueNumber(issue_number),
            title: format!("Issue {issue_number}"),
            body,
            labels: labels.into_iter().map(str::to_string).collect(),
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-04-13T00:00:00Z"),
            comments: vec![],
        })
        .expect("seed issue cache");
}

fn save_session(repo_path: &Path, branch: &str, linked_issue_number: Option<u64>) -> String {
    let mut session = Session::new(repo_path, branch, AgentId::Codex);
    session.id = "session-workflow-policy".to_string();
    session.linked_issue_number = linked_issue_number;
    session.save(&gwt_sessions_dir()).expect("save session");
    session.id
}

fn seed_issue_linkage(repo_path: &Path, branch: &str, issue_number: u64) {
    let repo_hash = compute_repo_hash("https://github.com/example/gwt-test.git");
    let store_path = repo_path
        .parent()
        .expect("repo parent")
        .join(".gwt/cache/issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    std::fs::create_dir_all(store_path.parent().expect("store parent")).expect("create store dir");
    std::fs::write(
        store_path,
        serde_json::to_vec_pretty(&json!({
            "branches": {
                branch: issue_number,
            }
        }))
        .expect("serialize linkage store"),
    )
    .expect("write linkage store");
}

#[test]
fn allows_read_only_tools_without_owner() {
    let event = event("Read", json!({ "file_path": "src/lib.rs" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "read-only tools must stay allowed");
}

#[test]
fn allows_worktree_internal_edit_without_owner() {
    let wt = root();
    let event = event(
        "Edit",
        json!({ "file_path": format!("{}/src/lib.rs", wt.display()), "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "worktree-internal edits must be allowed without owner"
    );
}

#[test]
fn allows_worktree_internal_edit_with_relative_path() {
    let event = event(
        "Edit",
        json!({ "file_path": "src/lib.rs", "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "relative worktree-internal edits must be allowed without owner"
    );
}

#[test]
fn allows_edit_outside_worktree_without_owner() {
    // Edit/Write path control is handled by Claude Code permissions, not by
    // workflow-policy. The hook only enforces Bash safety guardrails.
    let event = event(
        "Edit",
        json!({ "file_path": "/outside/project/src/lib.rs", "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "Edit outside worktree is not gated by workflow-policy"
    );
}

#[test]
fn allows_docs_edits_without_owner_as_chore_exemption() {
    let event = event(
        "Edit",
        json!({ "file_path": "README.md", "old_string": "old", "new_string": "new" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(decision.is_none(), "docs-only changes should stay allowed");
}

#[test]
fn allows_mutation_for_plain_issue_owner() {
    let event = event(
        "Write",
        json!({ "file_path": "src/lib.rs", "content": "fn x() {}\n" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::plain_issue(1942));
    assert!(
        decision.is_none(),
        "plain issue flow must not require spec plan/tasks"
    );
}

#[test]
fn allows_git_push_even_for_spec_without_plan() {
    let event = event("Bash", json!({ "command": "git push" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, false, true),
    );
    assert!(
        decision.is_none(),
        "git push is transport and must not be gated by plan/tasks"
    );
}

#[test]
fn allows_git_push_even_for_spec_without_tasks() {
    let event = event("Bash", json!({ "command": "git push" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, true, false),
    );
    assert!(
        decision.is_none(),
        "git push is transport and must not be gated by plan/tasks"
    );
}

#[test]
fn allows_spec_owner_when_plan_and_tasks_exist() {
    let event = event("Bash", json!({ "command": "git push origin main" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, true, true),
    );
    assert!(
        decision.is_none(),
        "ready spec owner should allow external ops"
    );
}

#[test]
fn allows_verification_bash_even_without_owner() {
    let event = event("Bash", json!({ "command": "cargo test -p gwt" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "verification commands should not be blocked by the workflow gate"
    );
}

#[test]
fn allows_worktree_touch_bash_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "touch /tmp/gwt-test-worktree/src/lib.rs" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "worktree-local file ops should bypass the owner gate"
    );
}

#[test]
fn allows_worktree_rm_bash_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "rm -f /tmp/gwt-test-worktree/.gwt/memory/constitution.md" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "worktree-local file ops should bypass the owner gate"
    );
}

#[test]
fn allows_cargo_fmt_without_owner() {
    let event = event("Bash", json!({ "command": "cargo fmt" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "cargo fmt is worktree-internal and must be allowed"
    );
}

#[test]
fn allows_git_commit_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "git add . && git commit -m 'chore: release'" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "git add/commit are worktree-internal and must be allowed"
    );
}

#[test]
fn allows_git_push_without_owner() {
    let event = event("Bash", json!({ "command": "git push origin main" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "git push is a transport operation and must not be gated by owner"
    );
}

#[test]
fn allows_git_push_with_session_bypass() {
    let event = event("Bash", json!({ "command": "git push origin main" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::with_bypass(gwt_agent::types::WorkflowBypass::Release),
    );
    assert!(decision.is_none(), "session bypass must allow git push");
}

#[test]
fn allows_git_push_with_chore_bypass() {
    let event = event("Bash", json!({ "command": "git push" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::with_bypass(gwt_agent::types::WorkflowBypass::Chore),
    );
    assert!(decision.is_none(), "chore bypass must allow git push");
}

#[test]
fn allows_sed_in_place_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "sed -i 's/old/new/' Cargo.toml" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "sed -i is worktree-internal and must be allowed"
    );
}

#[test]
fn allows_shell_redirect_without_owner() {
    let event = event("Bash", json!({ "command": "echo '1.0.0' > version.txt" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "shell redirects are worktree-internal and must be allowed"
    );
}

#[test]
fn allows_git_push_in_chained_command() {
    let event = event(
        "Bash",
        json!({ "command": "cargo fmt && git push origin main" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "chained command with git push must not be gated by owner"
    );
}

#[test]
fn worktree_external_file_op_is_blocked_before_owner_gate() {
    let event = event("Bash", json!({ "command": "rm -rf /tmp/outside" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown())
        .expect("out-of-worktree file ops must be blocked");
    assert!(decision.reason.contains("outside worktree"));
}

#[test]
fn reuses_legacy_bash_policy_rules_before_spec_gate() {
    let event = event("Bash", json!({ "command": "gh issue view 1935" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, true, true),
    )
    .expect("issue cli must still be blocked");
    assert!(decision.reason.contains("GitHub workflow CLI"));
}

#[test]
fn evaluate_resolves_spec_owner_from_session_cache() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        seed_issue_cache(&repo_path, 1935, vec!["gwt-spec"], "", "- [ ] T-001");
        let session_id = save_session(&repo_path, "feature/workflow", Some(1935));
        std::env::set_var(GWT_SESSION_ID_ENV, session_id);

        let event = event("Bash", json!({ "command": "git push" }));
        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");
        assert!(
            decision.is_none(),
            "git push is transport and must not be gated by plan/tasks"
        );
    });
}

#[test]
fn evaluate_falls_back_to_issue_linkage_store_for_plain_issue_owner() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        seed_issue_cache(&repo_path, 1942, vec!["bug"], "n/a", "n/a");
        let session_id = save_session(&repo_path, "feature/workflow", None);
        seed_issue_linkage(&repo_path, "feature/workflow", 1942);
        std::env::set_var(GWT_SESSION_ID_ENV, session_id);

        let event = event(
            "Write",
            json!({ "file_path": "src/lib.rs", "content": "fn x() {}\n" }),
        );
        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");
        assert!(
            decision.is_none(),
            "plain issue owner from linkage store should allow implementation"
        );
    });
}
