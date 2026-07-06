//! T-112 (SPEC #1935) — workflow-policy gating tests.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use chrono::Utc;
use gwt::cli::hook::{
    event_dispatcher, gwt_self_improvement_stop, workflow_policy, HookEvent, HookOutput,
};
use gwt_agent::{session::GWT_SESSION_ID_ENV, AgentId, Session, GWT_SESSION_RUNTIME_PATH_ENV};
use gwt_core::{
    coordination::{
        post_entry, AuthorKind, BoardEntry, BoardEntryKind, BoardMention, BoardMentionTargetKind,
    },
    paths::gwt_sessions_dir,
    repo_hash::compute_repo_hash,
    test_support::{ScopedEnvVar, ScopedGwtHome},
    workspace_projection::{
        record_workspace_work_event, save_workspace_projection, WorkEvent, WorkEventKind,
        WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary, WorkspaceProjection,
        WorkspaceStatusCategory,
    },
};
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
    std::env::temp_dir().join("gwt-test-worktree")
}

fn outside_root() -> PathBuf {
    root()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("gwt-test-outside")
}

fn event(tool_name: &str, tool_input: serde_json::Value) -> HookEvent {
    serde_json::from_value(json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
    }))
    .expect("valid hook event")
}

fn evaluate(event: &HookEvent, context: workflow_policy::WorkflowContext) -> Option<HookOutput> {
    match workflow_policy::evaluate_with_context(event, Path::new(&root()), &context)
        .expect("evaluation should succeed")
    {
        HookOutput::Silent => None,
        other => Some(other),
    }
}

fn with_temp_home<T>(f: impl FnOnce(&TempDir) -> T) -> T {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("temp home");
    let _home = ScopedGwtHome::set(home.path());
    let _session_id = ScopedEnvVar::unset(GWT_SESSION_ID_ENV);

    f(&home)
}

fn write_improvement_store(repo_path: &Path, candidates: serde_json::Value) {
    let path = repo_path
        .join(".gwt")
        .join("improvements")
        .join("candidates.json");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create improvements dir");
    std::fs::write(path, candidates.to_string()).expect("write candidates");
}

fn init_repo_with_origin(remote_url: &str) -> TempDir {
    let repo = tempfile::tempdir().expect("repo");
    assert!(std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(repo.path())
        .status()
        .expect("git init")
        .success());
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["remote", "add", "origin", remote_url])
        .status()
        .expect("git remote add")
        .success());
    repo
}

#[test]
fn gwt_self_improvement_stop_blocks_high_confidence_gwt_contract_violation_in_gwt_repo() {
    let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
    write_improvement_store(
        repo.path(),
        json!({
            "candidates": [{
                "id": "impr-high",
                "created_at": "2026-06-23T00:00:00Z",
                "updated_at": "2026-06-23T00:00:00Z",
                "source": "agent-failure",
                "target_artifact": "skill",
                "classification": "gwt-caused",
                "confidence": "high",
                "state": "pending",
                "dedupe_key": "skill:gwt-discussion:self-improvement",
                "occurrences": 1,
                "sanitized_summary": "Skill failed to update after agent failure",
                "sanitized_details": "Public-safe detail",
                "evidence_digest": "Public-safe digest",
                "local_evidence": [],
                "linked_issue": null,
                "dismissed_reason": null
            }]
        }),
    );

    let output = gwt_self_improvement_stop::evaluate(repo.path(), false, false);
    let HookOutput::StopBlock { reason } = output else {
        panic!("expected StopBlock, got {output:?}");
    };
    assert!(reason.contains("impr-high"));
    assert!(reason.contains("improvement.promote_issue"));
    assert!(reason.contains("improvement.dismiss"));

    // SPEC-3247 FR-003 / AS-4: the same high-confidence candidate in an intake
    // (Curate) session must NOT block Stop — intake owns no Work and is not the
    // producing-work self-improvement loop.
    assert_eq!(
        gwt_self_improvement_stop::evaluate(repo.path(), false, true),
        HookOutput::Silent,
        "intake sessions must not be forced to handle improvement candidates"
    );
}

#[test]
fn gwt_self_improvement_stop_ignores_low_confidence_or_handled_candidates() {
    let repo = init_repo_with_origin("git@github.com:akiojin/gwt.git");
    write_improvement_store(
        repo.path(),
        json!({
            "candidates": [
                {
                    "id": "impr-low",
                    "created_at": "2026-06-23T00:00:00Z",
                    "updated_at": "2026-06-23T00:00:00Z",
                    "source": "agent-failure",
                    "target_artifact": "skill",
                    "classification": "gwt-caused",
                    "confidence": "low",
                    "state": "pending",
                    "dedupe_key": "skill:low",
                    "occurrences": 1,
                    "sanitized_summary": "Low confidence",
                    "sanitized_details": null,
                    "evidence_digest": null,
                    "local_evidence": [],
                    "linked_issue": null,
                    "dismissed_reason": null
                },
                {
                    "id": "impr-promoted",
                    "created_at": "2026-06-23T00:00:00Z",
                    "updated_at": "2026-06-23T00:00:00Z",
                    "source": "agent-failure",
                    "target_artifact": "skill",
                    "classification": "gwt-caused",
                    "confidence": "high",
                    "state": "promoted",
                    "dedupe_key": "skill:promoted",
                    "occurrences": 1,
                    "sanitized_summary": "Already promoted",
                    "sanitized_details": null,
                    "evidence_digest": null,
                    "local_evidence": [],
                    "linked_issue": {"number": 1, "url": "https://github.com/akiojin/gwt/issues/1", "repository": "akiojin/gwt"},
                    "dismissed_reason": null
                }
            ]
        }),
    );

    assert_eq!(
        gwt_self_improvement_stop::evaluate(repo.path(), false, false),
        HookOutput::Silent
    );
}

#[test]
fn gwt_self_improvement_stop_is_noop_outside_gwt_repo() {
    let repo = init_repo_with_origin("https://github.com/example/target-project.git");
    write_improvement_store(
        repo.path(),
        json!({
            "candidates": [{
                "id": "impr-target",
                "created_at": "2026-06-23T00:00:00Z",
                "updated_at": "2026-06-23T00:00:00Z",
                "source": "agent-failure",
                "target_artifact": "skill",
                "classification": "gwt-caused",
                "confidence": "high",
                "state": "pending",
                "dedupe_key": "target:skill",
                "occurrences": 1,
                "sanitized_summary": "Target project saw a gwt hook problem",
                "sanitized_details": null,
                "evidence_digest": null,
                "local_evidence": [],
                "linked_issue": null,
                "dismissed_reason": null
            }]
        }),
    );

    assert_eq!(
        gwt_self_improvement_stop::evaluate(repo.path(), false, false),
        HookOutput::Silent
    );
}

#[test]
fn gwt_self_improvement_stop_respects_stop_hook_active() {
    let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
    write_improvement_store(
        repo.path(),
        json!({
            "candidates": [{
                "id": "impr-active-stop",
                "created_at": "2026-06-23T00:00:00Z",
                "updated_at": "2026-06-23T00:00:00Z",
                "source": "agent-failure",
                "target_artifact": "hook",
                "classification": "gwt-caused",
                "confidence": "high",
                "state": "pending",
                "dedupe_key": "hook:active-stop",
                "occurrences": 1,
                "sanitized_summary": "Stop hook recursion must not block again",
                "sanitized_details": null,
                "evidence_digest": null,
                "local_evidence": [],
                "linked_issue": null,
                "dismissed_reason": null
            }]
        }),
    );

    assert_eq!(
        gwt_self_improvement_stop::evaluate(repo.path(), true, false),
        HookOutput::Silent
    );
}

#[test]
fn common_stop_dispatcher_does_not_run_gwt_self_improvement_stop() {
    let _env_guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _session_id = ScopedEnvVar::unset(GWT_SESSION_ID_ENV);
    let _runtime_path = ScopedEnvVar::unset(GWT_SESSION_RUNTIME_PATH_ENV);
    let repo = init_repo_with_origin("https://github.com/akiojin/gwt.git");
    write_improvement_store(
        repo.path(),
        json!({
            "candidates": [{
                "id": "impr-common-stop",
                "created_at": "2026-06-23T00:00:00Z",
                "updated_at": "2026-06-23T00:00:00Z",
                "source": "agent-failure",
                "target_artifact": "hook",
                "classification": "gwt-caused",
                "confidence": "high",
                "state": "pending",
                "dedupe_key": "hook:common-stop",
                "occurrences": 1,
                "sanitized_summary": "Common Stop dispatcher must not own self-improvement",
                "sanitized_details": null,
                "evidence_digest": null,
                "local_evidence": [],
                "linked_issue": null,
                "dismissed_reason": null
            }]
        }),
    );

    let output = event_dispatcher::handle_with_input("Stop", "{}", repo.path(), None)
        .expect("Stop dispatch");
    assert!(
        !matches!(output, HookOutput::StopBlock { .. }),
        "self-improvement must be invoked by direct gwt repo hook config, not common Stop dispatcher: {output:?}"
    );
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

fn seed_workspace_agent_title(repo_path: &Path, session_id: &str) {
    let mut projection = WorkspaceProjection::default_for_project(repo_path);
    projection.agents.push(workspace_agent(
        session_id,
        "Testing workflow policy",
        "Workflow policy test",
    ));
    save_workspace_projection(repo_path, &projection).expect("save workspace projection");
}

fn workspace_agent(
    session_id: &str,
    current_focus: &str,
    title_summary: &str,
) -> WorkspaceAgentSummary {
    WorkspaceAgentSummary {
        session_id: session_id.to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: Some(current_focus.to_string()),
        title_summary: Some(title_summary.to_string()),
        worktree_path: None,
        branch: Some("feature/workflow".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some("workspace-existing".to_string()),
        updated_at: Utc::now(),
    }
}

fn unassigned_workspace_agent(session_id: &str) -> WorkspaceAgentSummary {
    WorkspaceAgentSummary {
        session_id: session_id.to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: None,
        branch: Some("work/unassigned".to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
        workspace_id: None,
        updated_at: Utc::now(),
    }
}

fn seed_workspace_agents(
    repo_path: &Path,
    current_session_id: &str,
    current_title: &str,
    other_session_id: &str,
    other_title: &str,
) {
    let mut projection = WorkspaceProjection::default_for_project(repo_path);
    projection.title = "Workspace semantic coordination".to_string();
    projection.status_category = WorkspaceStatusCategory::Active;
    projection.summary = Some("Coordinate same-work detection across agents".to_string());
    projection.agents.push(workspace_agent(
        current_session_id,
        "Implement Workspace semantic coordination gate",
        current_title,
    ));
    projection.agents.push(workspace_agent(
        other_session_id,
        "Implement duplicate Workspace semantic coordination protection",
        other_title,
    ));
    save_workspace_projection(repo_path, &projection).expect("save workspace projection");
}

fn seed_workspace_current_agent(repo_path: &Path, session_id: &str, title: &str, focus: &str) {
    let mut projection = WorkspaceProjection::default_for_project(repo_path);
    projection
        .agents
        .push(workspace_agent(session_id, focus, title));
    save_workspace_projection(repo_path, &projection).expect("save workspace projection");
}

fn seed_workspace_work_item(
    repo_path: &Path,
    work_item_id: &str,
    kind: WorkEventKind,
    title: &str,
    session_id: &str,
) {
    let mut event = WorkEvent::new(kind, work_item_id, Utc::now());
    event.title = Some(title.to_string());
    event.intent = Some("Implement Workspace WorkItem lifecycle history".to_string());
    event.summary =
        Some("Workspace WorkItem history should be joined instead of duplicated.".to_string());
    event.status_category = Some(match kind {
        WorkEventKind::Done => WorkspaceStatusCategory::Done,
        _ => WorkspaceStatusCategory::Active,
    });
    event.agent_session_id = Some(session_id.to_string());
    event.agent_id = Some("codex".to_string());
    event.display_name = Some("Codex".to_string());
    record_workspace_work_event(repo_path, event).expect("record workspace work item");
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
fn blocks_worktree_internal_edit_without_owner() {
    let wt = root();
    let event = event(
        "Edit",
        json!({ "file_path": format!("{}/src/lib.rs", wt.display()), "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("worktree-internal implementation edit must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn blocks_worktree_internal_edit_with_relative_path() {
    let event = event(
        "Edit",
        json!({ "file_path": "src/lib.rs", "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("relative implementation edit must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn blocks_edit_outside_worktree_without_owner() {
    let event = event(
        "Edit",
        json!({ "file_path": "/outside/project/src/lib.rs", "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("owner guard should block mutating edit without owner");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn blocks_docs_edit_outside_worktree_without_owner() {
    let event = event(
        "Edit",
        json!({ "file_path": "/outside/project/README.md", "old_string": "x", "new_string": "y" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("owner guard should block outside-worktree docs edit");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
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
fn allows_docs_only_apply_patch_without_owner_as_chore_exemption() {
    let event = event(
        "apply_patch",
        json!({
            "patch": "*** Begin Patch\n*** Update File: README.md\n@@\n-old\n+new\n*** End Patch\n"
        }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    assert!(
        decision.is_none(),
        "docs-only apply_patch changes should stay allowed"
    );
}

#[test]
fn blocks_source_apply_patch_without_owner() {
    let event = event(
        "apply_patch",
        json!({
            "patch": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch\n"
        }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("source apply_patch without owner must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn allows_docs_only_apply_patch_for_spec_owner_before_plan_refresh() {
    let event = event(
        "apply_patch",
        json!({
            "patch": "*** Begin Patch\n*** Update File: docs/hooks.md\n@@\n-old\n+new\n*** End Patch\n"
        }),
    );
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, false, false),
    );
    assert!(
        decision.is_none(),
        "docs-only patch should not require spec plan/tasks"
    );
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
        json!({ "command": format!("touch {}/src/lib.rs", root().display()) }),
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
        json!({ "command": format!("rm -f {}/.gwt/memory/constitution.md", root().display()) }),
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
fn blocks_git_commit_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "git add . && git commit -m 'chore: release'" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("git commit without owner must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
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
fn blocks_sed_in_place_without_owner() {
    let event = event(
        "Bash",
        json!({ "command": "sed -i 's/old/new/' Cargo.toml" }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("sed -i without owner must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
}

#[test]
fn blocks_shell_redirect_without_owner() {
    let event = event("Bash", json!({ "command": "echo '1.0.0' > version.txt" }));
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown());
    let decision = decision.expect("shell redirect without owner must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("Owner Issue/SPEC"));
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
    let event = event(
        "Bash",
        json!({ "command": format!("rm -rf {}", outside_root().display()) }),
    );
    let decision = evaluate(&event, workflow_policy::WorkflowContext::unknown())
        .expect("out-of-worktree file ops must be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("outside worktree"));
}

#[test]
fn reuses_legacy_bash_policy_rules_before_spec_gate() {
    let event = event("Bash", json!({ "command": "gh issue view 1935" }));
    let decision = evaluate(
        &event,
        workflow_policy::WorkflowContext::spec_issue(1935, true, true),
    )
    .expect("issue cli must still be blocked");
    assert!(decision
        .permission_decision_reason()
        .contains("GitHub workflow CLI"));
}

#[test]
fn evaluate_resolves_spec_owner_from_session_cache() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        seed_issue_cache(&repo_path, 1935, vec!["gwt-spec"], "", "- [ ] T-001");
        let session_id = save_session(&repo_path, "feature/workflow", Some(1935));
        seed_workspace_agent_title(&repo_path, &session_id);
        std::env::set_var(GWT_SESSION_ID_ENV, session_id);

        let event = event("Bash", json!({ "command": "git push" }));
        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");
        assert!(
            matches!(decision, HookOutput::Silent),
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
        seed_workspace_agent_title(&repo_path, &session_id);
        seed_issue_linkage(&repo_path, "feature/workflow", 1942);
        std::env::set_var(GWT_SESSION_ID_ENV, session_id);

        let event = event(
            "Write",
            json!({ "file_path": "src/lib.rs", "content": "fn x() {}\n" }),
        );
        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");
        assert!(
            matches!(decision, HookOutput::Silent),
            "plain issue owner from linkage store should allow implementation"
        );
    });
}

#[test]
fn similar_active_workspace_does_not_hard_block_mutation() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        seed_workspace_agents(
            &repo_path,
            &session_id,
            "Workspace semantic coordination gate",
            "session-other",
            "Workspace semantic coordination duplicate guard",
        );

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "active Workspace similarity is coordination context; duplicate prevention belongs to explicit workspace affiliation"
        );
    });
}

#[test]
fn allows_mutation_after_split_claim_targets_matching_workspace_agent() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        seed_workspace_agents(
            &repo_path,
            &session_id,
            "Workspace semantic coordination gate",
            "session-other",
            "Workspace semantic coordination duplicate guard",
        );

        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Claim,
            "Split accepted for same Workspace work.\n\nBoundary: current session owns workflow-policy tests and policy gate only.",
            None,
            None,
            vec!["workspace-semantic-coordination".to_string()],
            vec!["2359".to_string()],
        )
        .with_origin_session_id(session_id.clone())
        .with_mention(BoardMention::new(
            BoardMentionTargetKind::Session,
            "session-other",
        ));
        post_entry(&repo_path, entry).expect("post split claim");

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Boundary-targeted split claim should allow disjoint implementation"
        );
    });
}

#[test]
fn active_board_claim_does_not_hard_block_mutation() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        projection.agents.push(workspace_agent(
            &session_id,
            "Implement Workspace semantic coordination gate",
            "Workspace semantic coordination gate",
        ));
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Other Codex",
            BoardEntryKind::Claim,
            "Implement Workspace semantic coordination duplicate guard for active agents.",
            None,
            None,
            vec!["workspace-semantic-coordination".to_string()],
            vec!["2359".to_string()],
        )
        .with_origin_session_id("session-other");
        post_entry(&repo_path, entry).expect("post active claim");

        let event = event(
            "Write",
            json!({ "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs", "content": "x" }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "active Board claims should coordinate duplicate risk without blocking unrelated tool execution"
        );
    });
}

#[test]
fn unassigned_agent_does_not_inherit_projection_title_for_duplicate_gate() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/unassigned", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        projection.title = "Workspace affiliation fix".to_string();
        projection.summary = Some("Stale project-level workspace title".to_string());
        projection.status_category = WorkspaceStatusCategory::Active;
        projection
            .agents
            .push(unassigned_workspace_agent(&session_id));
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Other Codex",
            BoardEntryKind::Claim,
            "Workspace affiliation fix is in progress on another branch.",
            None,
            None,
            vec!["workspace-materialization".to_string()],
            vec!["2359".to_string()],
        )
        .with_origin_session_id("session-other");
        post_entry(&repo_path, entry).expect("post stale active claim");

        let event = event(
            "Write",
            json!({ "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs", "content": "x" }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Unassigned Agents must not inherit stale projection-level title as duplicate-gate intent"
        );
    });
}

#[test]
fn does_not_block_when_active_board_claim_is_audienced_to_other_workspace() {
    // SPEC-2359 FR-099 / SC-031: a claim audienced only to a different
    // Workspace must not gate the current Agent. With Codex's
    // affiliation field landed, the current Agent is assigned to
    // `workspace-existing` (per workspace_agent helper); the claim
    // audienced to `ws-other-only` does not intersect, so the gate
    // must stay silent.
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        projection.agents.push(workspace_agent(
            &session_id,
            "Implement Workspace audience scoped gate",
            "Workspace audience scoped gate",
        ));
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Other Codex",
            BoardEntryKind::Claim,
            "Implement Workspace audience scoped gate for active agents.",
            None,
            None,
            vec!["workspace-audience".to_string()],
            vec!["2359".to_string()],
        )
        .with_origin_session_id("session-other")
        .with_audience(vec!["ws-other-only".to_string()]);
        post_entry(&repo_path, entry).expect("post audienced claim");

        let event = event(
            "Write",
            json!({ "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs", "content": "x" }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        match decision {
            HookOutput::PreToolUsePermission { detail, .. } => {
                panic!(
                    "audience-only claim must not block the current Agent when audience does not intersect: {detail}"
                );
            }
            HookOutput::Silent => {}
            other => panic!("expected silent allow, got {other:?}"),
        }
    });
}

#[test]
fn unassigned_agent_without_title_summary_is_not_title_blocked() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/unassigned", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        projection
            .agents
            .push(unassigned_workspace_agent(&session_id));
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt/src/lib.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Unassigned Agents must not be blocked as missing title-summary"
        );
    });
}

#[test]
fn actionable_unassigned_agent_can_mutate_without_forced_workspace_affiliation() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/unassigned", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        let mut agent = unassigned_workspace_agent(&session_id);
        agent.title_summary = Some("Workspace materialization".to_string());
        agent.current_focus = Some("Ensure actionable intent enters a Workspace".to_string());
        projection.agents.push(agent);
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt/src/cli/workspace.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Unassigned is a valid coordination state; affiliation is explicit and optional"
        );
    });
}

#[test]
fn actionable_unassigned_agent_can_run_workspace_ensure_command() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/unassigned", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        let mut agent = unassigned_workspace_agent(&session_id);
        agent.title_summary = Some("Workspace materialization".to_string());
        agent.current_focus = Some("Ensure actionable intent enters a Workspace".to_string());
        projection.agents.push(agent);
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let event = event(
            "Bash",
            json!({
                "command": "gwtd <<'JSON'\n{\"schema_version\":1,\"operation\":\"workspace.ensure\",\"params\":{\"agent_session\":\"$GWT_SESSION_ID\",\"purpose\":\"Workspace materialization\",\"current_focus\":\"Ensure actionable intent enters a Workspace\",\"spec\":2359}}\nJSON"
            }),
        );

        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Workspace ensure must remain allowed so the Agent can repair affiliation"
        );
    });
}

#[test]
fn assigned_agent_without_title_summary_remains_title_blocked() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/assigned", None);
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        let mut projection = WorkspaceProjection::default_for_project(&repo_path);
        let mut agent = workspace_agent(&session_id, "Implement assigned work", "");
        agent.title_summary = None;
        projection.agents.push(agent);
        save_workspace_projection(&repo_path, &projection).expect("save workspace projection");

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt/src/lib.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision =
            workflow_policy::evaluate(&event, &repo_path).expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::PreToolUsePermission { .. }),
            "Assigned Agents still need a title-summary before implementation"
        );
    });
}

#[test]
fn incomplete_work_item_history_does_not_hard_block_mutation() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        seed_workspace_current_agent(
            &repo_path,
            &session_id,
            "Workspace WorkItem history",
            "Implement Workspace WorkItem lifecycle history",
        );
        seed_workspace_work_item(
            &repo_path,
            "workitem-existing",
            WorkEventKind::Start,
            "Workspace WorkItem history duplicate prevention",
            "session-other",
        );

        let event = event(
            "Edit",
            json!({
                "file_path": "crates/gwt-core/src/workspace_projection.rs",
                "old_string": "old",
                "new_string": "new"
            }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "incomplete Workspace history is context; explicit workspace join/create owns duplicate prevention"
        );
    });
}

#[test]
fn completed_work_item_history_does_not_block_new_related_work() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", Some(1942));
        std::env::set_var(GWT_SESSION_ID_ENV, &session_id);
        seed_workspace_current_agent(
            &repo_path,
            &session_id,
            "Workspace WorkItem history",
            "Implement Workspace WorkItem lifecycle history follow-up",
        );
        seed_workspace_work_item(
            &repo_path,
            "workitem-completed",
            WorkEventKind::Done,
            "Workspace WorkItem history",
            "session-other",
        );

        let event = event(
            "Write",
            json!({ "file_path": "crates/gwt-core/src/workspace_projection.rs", "content": "x" }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::plain_issue(1942),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "completed WorkItem history must be context only"
        );
    });
}
