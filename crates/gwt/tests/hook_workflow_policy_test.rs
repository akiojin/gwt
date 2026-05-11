//! T-112 (SPEC #1935) — workflow-policy gating tests.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use chrono::Utc;
use gwt::cli::hook::{workflow_policy, HookEvent, HookOutput};
use gwt_agent::{session::GWT_SESSION_ID_ENV, AgentId, Session};
use gwt_core::{
    coordination::{
        post_entry, AuthorKind, BoardEntry, BoardEntryKind, BoardMention, BoardMentionTargetKind,
    },
    paths::gwt_sessions_dir,
    repo_hash::compute_repo_hash,
    workspace_projection::{
        record_workspace_work_event, save_workspace_projection, WorkspaceAgentAffiliationStatus,
        WorkspaceAgentSummary, WorkspaceProjection, WorkspaceStatusCategory, WorkspaceWorkEvent,
        WorkspaceWorkEventKind,
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
    kind: WorkspaceWorkEventKind,
    title: &str,
    session_id: &str,
) {
    let mut event = WorkspaceWorkEvent::new(kind, work_item_id, Utc::now());
    event.title = Some(title.to_string());
    event.intent = Some("Implement Workspace WorkItem lifecycle history".to_string());
    event.summary =
        Some("Workspace WorkItem history should be joined instead of duplicated.".to_string());
    event.status_category = Some(match kind {
        WorkspaceWorkEventKind::Done => WorkspaceStatusCategory::Done,
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
fn blocks_mutation_when_another_active_workspace_matches_current_title() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", None);
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
            &workflow_policy::WorkflowContext::unknown(),
        )
        .expect("workflow evaluation succeeds");

        let HookOutput::PreToolUsePermission { detail, .. } = decision else {
            panic!("expected active Workspace conflict to block mutation");
        };
        assert!(detail.contains("similar active Workspace"), "{detail}");
        assert!(detail.contains("session-other"), "{detail}");
        assert!(detail.contains("gwtd board post"), "{detail}");
        assert!(detail.contains("Boundary:"), "{detail}");
    });
}

#[test]
fn allows_mutation_after_split_claim_targets_matching_workspace_agent() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", None);
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
            &workflow_policy::WorkflowContext::unknown(),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "Boundary-targeted split claim should allow disjoint implementation"
        );
    });
}

#[test]
fn blocks_mutation_when_active_board_claim_matches_current_title() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", None);
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
            &workflow_policy::WorkflowContext::unknown(),
        )
        .expect("workflow evaluation succeeds");

        let HookOutput::PreToolUsePermission { detail, .. } = decision else {
            panic!("expected active Board claim conflict to block mutation");
        };
        assert!(detail.contains("active Board claim"), "{detail}");
        assert!(detail.contains("session-other"), "{detail}");
    });
}

#[test]
fn allows_mutation_when_active_board_claim_is_scoped_to_other_workspace() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", None);
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
        .with_origin_session_id("session-other")
        .with_audience(vec!["workspace-other"]);
        post_entry(&repo_path, entry).expect("post other-workspace claim");

        let event = event(
            "Write",
            json!({ "file_path": "crates/gwt/src/cli/hook/workflow_policy.rs", "content": "x" }),
        );

        let decision = workflow_policy::evaluate_with_context(
            &event,
            &repo_path,
            &workflow_policy::WorkflowContext::unknown(),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "other-Workspace Board claims must not block this Workspace"
        );
    });
}

#[test]
fn unassigned_agent_without_title_summary_is_not_title_blocked() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/unassigned", None);
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
fn blocks_mutation_when_incomplete_work_item_matches_current_title() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", None);
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
            WorkspaceWorkEventKind::Start,
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
            &workflow_policy::WorkflowContext::unknown(),
        )
        .expect("workflow evaluation succeeds");

        let HookOutput::PreToolUsePermission { detail, .. } = decision else {
            panic!("expected incomplete WorkItem conflict to block mutation");
        };
        assert!(detail.contains("incomplete Workspace"), "{detail}");
        assert!(detail.contains("session-other"), "{detail}");
        assert!(detail.contains("gwtd board post"), "{detail}");
    });
}

#[test]
fn completed_work_item_history_does_not_block_new_related_work() {
    with_temp_home(|home| {
        let repo_path = init_repo(home);
        let session_id = save_session(&repo_path, "work/current", None);
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
            WorkspaceWorkEventKind::Done,
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
            &workflow_policy::WorkflowContext::unknown(),
        )
        .expect("workflow evaluation succeeds");

        assert!(
            matches!(decision, HookOutput::Silent),
            "completed WorkItem history must be context only"
        );
    });
}
