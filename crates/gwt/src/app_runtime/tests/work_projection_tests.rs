use super::*;

#[test]
fn app_runtime_work_singleton_treats_legacy_branches_as_alias() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.create_window_events(
        &runtime.test_context(),
        WindowPreset::Branches,
        canvas_bounds(),
    );
    let legacy = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows[0]
        .clone();
    let events =
        runtime.create_window_events(&runtime.test_context(), WindowPreset::Work, canvas_bounds());
    let windows = &runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows;

    assert_eq!(windows.len(), 1, "Branches and Work must share identity");
    assert_eq!(windows[0].id, legacy.id);
    assert_eq!(windows[0].preset, WindowPreset::Branches);
    assert!(windows[0].z_index > legacy.z_index);
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
    assert!(!events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })));
}

#[test]
fn app_runtime_work_singleton_activates_inactive_grouped_work_tab() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::Work, WindowPreset::FileTree],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let work_id = combined_window_id("tab-1", "work-1");
    let file_tree_id = combined_window_id("tab-1", "file-tree-1");

    runtime.dock_window_tab_events(&file_tree_id, &work_id);
    let before = runtime.tab("tab-1").expect("tab").workspace.persisted();
    assert!(
        before
            .windows
            .iter()
            .find(|window| window.id == "file-tree-1")
            .expect("File Tree tab")
            .tab_group_active
    );
    assert!(
        !before
            .windows
            .iter()
            .find(|window| window.id == "work-1")
            .expect("inactive Work tab")
            .tab_group_active
    );

    let events =
        runtime.create_window_events(&runtime.test_context(), WindowPreset::Work, canvas_bounds());
    let after = runtime.tab("tab-1").expect("tab").workspace.persisted();
    let work = after
        .windows
        .iter()
        .find(|window| window.id == "work-1")
        .expect("reused Work tab");
    let file_tree = after
        .windows
        .iter()
        .find(|window| window.id == "file-tree-1")
        .expect("grouped File Tree tab");

    assert_eq!(after.windows.len(), 2);
    assert!(
        work.tab_group_active,
        "reuse must reveal the Work group tab"
    );
    assert!(!file_tree.tab_group_active);
    assert!(work.z_index >= file_tree.z_index);
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
}

#[test]
fn app_runtime_work_singleton_preserves_historical_duplicates() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Work, WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.create_window_events(&runtime.test_context(), WindowPreset::Work, canvas_bounds());
    let windows = &runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows;

    assert_eq!(
        windows.len(),
        2,
        "reuse must not purge historical duplicates"
    );
    assert!(windows.iter().any(|window| window.id == "work-1"));
    assert!(windows.iter().any(|window| window.id == "branches-1"));
    assert_eq!(
        windows
            .iter()
            .max_by_key(|window| window.z_index)
            .expect("focused singleton")
            .id,
        "branches-1"
    );

    assert!(runtime
        .persist_dispatcher
        .wait_idle(std::time::Duration::from_secs(5)));
    let persisted = load_workspace_state(&workspace_state_path(&repo))
        .expect("persisted historical Work windows");
    assert_eq!(persisted.windows.len(), 2);
}

#[test]
fn app_runtime_work_singleton_preserves_shell_multi_instance_runtime() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime.create_window_events(
        &runtime.test_context(),
        WindowPreset::Shell,
        canvas_bounds(),
    );
    runtime.create_window_events(
        &runtime.test_context(),
        WindowPreset::Shell,
        canvas_bounds(),
    );
    let shell_ids = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Shell)
        .map(|window| combined_window_id("tab-1", &window.id))
        .collect::<Vec<_>>();

    assert_eq!(shell_ids.len(), 2);
    assert_eq!(runtime.runtimes.len(), 2);
    for shell_id in shell_ids {
        runtime.close_window_events(&shell_id);
    }
}

// SPEC-2359 Workspace → Work → Session: a Work row (keyed by the gwt session
// id / launch) is enriched with its Session history (agent-tool conversation
// UUIDs) read from the persisted Session, with the latest marked active.
#[test]
fn workspace_work_agent_view_attaches_session_history() {
    let mut session = gwt_agent::Session::new("/tmp/wt", "feature/x", gwt_agent::AgentId::Codex);
    session.id = "work-1".to_string();
    session.agent_session_id = Some("conv-2".to_string());
    session.session_history = vec![
        gwt_agent::AgentSessionHistoryEntry {
            agent_session_id: "conv-1".to_string(),
            started_at: chrono::Utc::now(),
        },
        gwt_agent::AgentSessionHistoryEntry {
            agent_session_id: "conv-2".to_string(),
            started_at: chrono::Utc::now(),
        },
    ];
    let sessions = vec![session];
    let index = super::super::work_session_index(&sessions);

    let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
        session_id: "work-1".to_string(),
        agent_id: Some("codex".to_string()),
        display_name: Some("Codex".to_string()),
        updated_at: chrono::Utc::now(),
        attached_by: None,
    };
    let view = super::super::workspace_work_agent_view_from_ref(
        &agent_ref,
        &index,
        scanned_without_branches(),
    );

    let ids: Vec<&str> = view
        .sessions
        .iter()
        .map(|session| session.agent_session_id.as_str())
        .collect();
    assert_eq!(ids, vec!["conv-1", "conv-2"]);
    assert!(!view.sessions[0].is_active);
    assert!(view.sessions[1].is_active, "latest conversation is active");

    // A Work with no persisted Session yields an empty Session list, not a panic.
    let empty_index = super::super::work_session_index(&[]);
    let empty_view = super::super::workspace_work_agent_view_from_ref(
        &agent_ref,
        &empty_index,
        scanned_without_branches(),
    );
    assert!(empty_view.sessions.is_empty());
}

#[test]
fn start_work_launch_uses_repo_global_work_items_and_worktree_local_event() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project_root = temp.path().join("workspace-home");
    let (bare, _develop_worktree) = init_managed_workspace_with_develop_worktree(&project_root);
    let worktree = project_root.join("work").join("issue-3412");
    std::fs::create_dir_all(worktree.parent().expect("worktree parent")).expect("worktree parent");
    run_git(
        &bare,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "work/issue-3412",
            worktree.to_str().expect("worktree path"),
            "develop",
        ],
    );
    let mut session = sample_active_agent_session("tab-1", "window-3412");
    session.session_id = "session-launch-3412".to_string();
    session.branch_name = "work/issue-3412".to_string();
    session.worktree_path = worktree.clone();
    session.agent_project_root = project_root.display().to_string();

    save_start_work_workspace_projection(
        &project_root,
        &session,
        "develop",
        Some(3412),
        None,
        None,
        Some(&std::collections::HashSet::from([session
            .session_id
            .clone()])),
    )
    .expect("Start Work launch publication");

    let current = gwt_core::workspace_projection::load_workspace_projection(&project_root)
        .expect("load current")
        .expect("current projection");
    let agent = current
        .latest_agent_for_session(&session.session_id)
        .expect("launch agent");
    assert!(agent.is_assigned());
    let work_id = agent.workspace_id.as_deref().expect("assigned Work id");
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
        .expect("load WorkItems")
        .expect("WorkItems");
    let work = work_items
        .work_items
        .iter()
        .find(|item| item.id == work_id)
        .expect("materialized Work");
    assert_eq!(work.owner.as_deref(), Some("Issue #3412"));
    assert!(work
        .agents
        .iter()
        .any(|agent| agent.session_id == session.session_id));
    let launch_events = load_tracked_work_events(&worktree);
    assert_eq!(
        launch_events
            .iter()
            .filter(|event| {
                event.work_item_id == work_id
                    && event.agent_session_id.as_deref() == Some(session.session_id.as_str())
                    && event.kind == gwt_core::workspace_projection::WorkEventKind::Start
            })
            .count(),
        1,
        "launch must persist exactly one matching Start event",
    );
    assert_eq!(
        materialized_project_state_sots("works.json"),
        vec![gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root)],
        "launch must not create a topology-dependent worktree WorkItems SOT"
    );
    assert_eq!(
        materialized_project_state_sots("current.json"),
        vec![gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root)],
        "launch must not create a topology-dependent worktree Project State SOT"
    );

    let tab = sample_project_tab(
        "tab-1",
        "Workspace Home",
        project_root.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .active_agent_sessions
        .insert(session.window_id.clone(), session.clone());
    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("repo-global active Work view");
    let live_rows = view
        .active_works
        .iter()
        .filter(|work| work.id == work_id)
        .collect::<Vec<_>>();
    assert_eq!(
        live_rows.len(),
        1,
        "the live Work must surface exactly once"
    );
    assert_eq!(live_rows[0].lifecycle_state, "active");
    assert_eq!(live_rows[0].active_agents, 1);
    runtime.mark_agent_session_stopped(&session.window_id);
    let stopped_items = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
        .expect("load stopped repo-global WorkItems")
        .expect("stopped repo-global WorkItems");
    let stopped = stopped_items
        .work_items
        .iter()
        .find(|item| item.id == work_id)
        .expect("stopped canonical Work");
    assert!(
        stopped
            .events
            .iter()
            .any(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Pause),
        "stop must append Pause to the same Session-bound Work"
    );
    assert_eq!(
        materialized_project_state_sots("works.json"),
        vec![gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root)],
        "stop must not create a worktree-specific WorkItems SOT"
    );
    let mut saved = gwt_core::workspace_projection::load_workspace_projection(&project_root)
        .expect("load stopped current projection")
        .expect("stopped current projection");
    saved.git_details = None;
    gwt_core::workspace_projection::save_workspace_projection(&project_root, &saved)
        .expect("clear current execution-container hint");
    let stopped_view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("stopped repo-global Work view");
    let paused_rows = stopped_view
        .active_works
        .iter()
        .filter(|work| work.id == work_id)
        .collect::<Vec<_>>();
    assert_eq!(
        paused_rows.len(),
        1,
        "repo-global WorkItems must keep one Paused Work visible"
    );
    assert_eq!(paused_rows[0].lifecycle_state, "paused");
    assert_eq!(paused_rows[0].active_agents, 0);
}

// #3426: the genesis Start Work projection must let the canonical execution
// owner override a presentation-derived Issue-kind resume context, mirroring
// the Blocked-successor path.
#[test]
fn start_work_projection_prefers_canonical_execution_owner_over_resume_context() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project_root = temp.path().join("repo");
    fs::create_dir_all(&project_root).expect("create repo");
    init_repo(&project_root);
    let mut session = sample_active_agent_session("tab-1", "window-1921");
    session.session_id = "session-launch-1921".to_string();
    session.branch_name = "work/issue-1921".to_string();
    session.worktree_path = project_root.clone();
    session.agent_project_root = project_root.display().to_string();
    let context = WorkspaceResumeContext {
        title: Some("SPEC-1921: agent management".to_string()),
        owner: Some("Issue #1921".to_string()),
        summary: None,
        next_action: None,
    };

    save_start_work_workspace_projection(
        &project_root,
        &session,
        "develop",
        Some(1921),
        Some(gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Spec,
            number: 1921,
        }),
        Some(&context),
        Some(&std::collections::HashSet::from([session
            .session_id
            .clone()])),
    )
    .expect("Start Work launch publication");

    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
        .expect("load WorkItems")
        .expect("WorkItems");
    let work = work_items
        .work_items
        .iter()
        .find(|item| {
            item.agents
                .iter()
                .any(|agent| agent.session_id == session.session_id)
        })
        .expect("materialized Work");
    assert_eq!(
        work.owner.as_deref(),
        Some("SPEC-1921"),
        "trusted execution owner must override the mis-kinded resume context"
    );
    assert_eq!(
        work.title, "SPEC-1921: agent management",
        "non-owner resume context fields stay presentation-owned"
    );
}

// #3065: the Workspace Resume context must come from the resumed branch's
// own Work item — never from the repo-shared current projection, whose
// identity may belong to a different Work.
#[test]
fn workspace_resume_context_prefers_work_item_over_shared_projection() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project_state_root = temp.path().join("workspace-home");
    let worktree = temp.path().join("workspace-home/work/foo");
    std::fs::create_dir_all(&worktree).expect("worktree dir");

    // Shared current projection carries a foreign work's identity.
    let mut shared = gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
        &project_state_root,
    );
    shared.title = "gwt-manage-pr".to_string();
    shared.owner = Some("SPEC-2359".to_string());
    shared.next_action = Some("foreign next action".to_string());
    gwt_core::workspace_projection::save_workspace_projection(&project_state_root, &shared)
        .expect("save shared projection");

    // The resumed branch has its own Work item with its own identity.
    let now = chrono::Utc::now();
    let work_id = gwt_core::workspace_projection::canonical_work_id(
        &project_state_root,
        Some("work/foo"),
        Some(&worktree),
    )
    .expect("canonical id");
    let mut event = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    event.title = Some("fix foo".to_string());
    event.owner = Some("Issue #42".to_string());
    event.next_action = Some("own next action".to_string());
    event.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/foo".to_string()),
            worktree_path: Some(worktree.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    gwt_core::workspace_projection::record_workspace_work_event(&project_state_root, event)
        .expect("record work event");

    let context = super::super::workspace_resume_context_for_work_item(
        &project_state_root,
        Some("work/foo"),
        &worktree,
    );
    assert_eq!(context.title.as_deref(), Some("fix foo"));
    assert_eq!(context.owner.as_deref(), Some("Issue #42"));
    assert_eq!(context.next_action.as_deref(), Some("own next action"));

    // Unknown container: neutral context — never the shared identity.
    let fallback = super::super::workspace_resume_context_for_work_item(
        &project_state_root,
        Some("work/unknown"),
        &project_state_root.join("work/unknown"),
    );
    assert_eq!(fallback.owner, None, "shared owner must not leak");
    assert_eq!(fallback.title, None, "shared title must not leak");
    assert_eq!(fallback.next_action, None);
}

// #3065: launch saves retain only live agent sessions in the shared
// projection so dead entries stop accumulating ("765 active agents").
#[test]
fn save_workspace_launch_projection_retains_only_live_agents() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    let mut session = sample_active_agent_session("tab-1", "win-1");
    session.worktree_path = repo.clone();

    let work_id = gwt_core::workspace_projection::canonical_work_id(
        &repo,
        Some(&session.branch_name),
        Some(&session.worktree_path),
    )
    .expect("canonical id");
    let mut shared =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    shared.id = work_id.clone();
    shared
        .agents
        .push(workspace_agent_summary_for_test("dead-1", Some(&work_id)));
    gwt_core::workspace_projection::save_workspace_projection(&repo, &shared)
        .expect("save shared projection");

    let live: std::collections::HashSet<String> =
        std::iter::once(session.session_id.clone()).collect();
    let context = WorkspaceResumeContext {
        title: None,
        owner: None,
        summary: None,
        next_action: None,
    };
    save_workspace_launch_projection(
        &repo,
        &session,
        Some("develop"),
        None,
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&live),
    )
    .expect("save launch projection");

    let stored = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load")
        .expect("projection exists");
    assert!(
        stored
            .agents
            .iter()
            .all(|agent| agent.session_id != "dead-1"),
        "dead agent session is dropped"
    );
    assert!(
        stored
            .agents
            .iter()
            .any(|agent| agent.session_id == session.session_id),
        "launching session is kept"
    );
    assert_eq!(stored.status_text, "Codex is running");
}

// SPEC-2359 US-80 (FR-427/FR-429): a Start-Work Shell registers as a
// first-class Work and is not pruned when an agent later launches on another
// branch.
#[test]
fn save_shell_work_projection_registers_shell_and_survives_agent_launch() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");

    // シナリオ1: registering a Start-Work Shell makes it a Work in the projection.
    let empty: std::collections::HashSet<String> = std::collections::HashSet::new();
    super::super::save_shell_work_projection(
        &repo,
        "tab-1:shell-3",
        Some(repo.join("wt-shell")),
        Some("work/shell-x".to_string()),
        &empty,
    )
    .expect("register shell work");

    let stored = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load")
        .expect("projection exists");
    let shell = stored
        .agents
        .iter()
        .find(|agent| agent.is_shell_work())
        .expect("shell work present");
    assert_eq!(shell.session_id, "tab-1:shell-3");
    assert_eq!(shell.display_name, "Shell");
    assert_eq!(shell.branch.as_deref(), Some("work/shell-x"));

    // シナリオ3 / FR-429: launching an agent on a different branch keeps the
    // running Shell Work.
    let session = sample_active_agent_session("tab-2", "win-2");
    save_assigned_workspace_projection_for_test(&repo, &session).expect("agent launch save");

    let after = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load")
        .expect("projection exists");
    assert!(
        after
            .agents
            .iter()
            .any(|agent| agent.is_shell_work() && agent.session_id == "tab-1:shell-3"),
        "agent launch on another branch must keep the Shell Work"
    );
    assert!(
        after
            .agents
            .iter()
            .any(|agent| !agent.is_shell_work() && agent.session_id == session.session_id),
        "the launched agent is present"
    );
}

// SPEC-2359 US-80 (FR-429): the Active Work broadcast rebuild prunes dead
// agents but must keep Shell Works (no agent session), otherwise a
// just-registered shell would vanish from the broadcast.
#[test]
fn retain_live_workspace_agents_keeps_shell_work_with_no_live_sessions() {
    let repo = std::path::Path::new("/repo");
    let now = chrono::Utc::now();
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo);
    projection.agents.push(
        gwt_core::workspace_projection::WorkspaceAgentSummary::shell_work(
            "tab-1:shell-3",
            Some(repo.join("wt-x")),
            Some("work/x".to_string()),
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            now,
        ),
    );
    projection
        .agents
        .push(workspace_agent_summary_for_test("dead-agent", None));

    super::super::retain_live_workspace_agents(&mut projection, &[], now);

    assert!(
        projection.agents.iter().any(|agent| agent.is_shell_work()),
        "shell work survives the broadcast retain with no live sessions"
    );
    assert!(
        !projection
            .agents
            .iter()
            .any(|agent| agent.session_id == "dead-agent"),
        "dead agent is pruned"
    );
    assert!(
        projection.has_current_agents(),
        "a lone shell keeps the projection out of the idle reset"
    );
}

// SPEC-2359 US-80 (FR-430, シナリオ1/2): a Shell Work summary surfaces as a
// Work row in the Active Work projection view, and an agent on the same branch
// groups into the same Work.
#[test]
fn active_work_view_surfaces_shell_work_and_groups_with_same_branch_agent() {
    let repo = std::path::Path::new("/repo");
    let now = chrono::Utc::now();
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo);
    projection.agents.push(
        gwt_core::workspace_projection::WorkspaceAgentSummary::shell_work(
            "tab-1:shell-3",
            Some(repo.join("wt-x")),
            Some("work/x".to_string()),
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            now,
        ),
    );

    let view = super::super::active_work_projection_from_saved(projection.clone());
    assert_eq!(
        view.active_agents, 1,
        "the shell work surfaces as one active work row"
    );

    // Same-branch agent groups into one Work (FR-430 / シナリオ2).
    let mut agent = workspace_agent_summary_for_test("agent-1", None);
    agent.branch = Some("work/x".to_string());
    agent.worktree_path = Some(repo.join("wt-x"));
    projection.agents.push(agent);
    let grouped = super::super::active_work_projection_from_saved(projection);
    assert_eq!(
        grouped.active_agents, 2,
        "shell + agent on the same branch both appear as active rows"
    );
}

#[test]
fn image_paste_prepare_uses_drop_files_relative_path_reference() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let payload = base64::engine::general_purpose::STANDARD.encode(b"image-bytes");
    let agent_root = temp.path().display().to_string();

    let prepared = super::super::prepare_image_paste_file(
        temp.path(),
        &agent_root,
        &payload,
        "image/png",
        Some("../Screen Shot.png"),
        "20260507-160000",
    )
    .expect("prepare image paste");
    let expected_path = temp
        .path()
        .join(".gwt")
        .join("drop-files")
        .join("20260507-160000-screen-shot.png");

    assert_eq!(prepared.bytes.as_deref(), Some(&b"image-bytes"[..]));
    assert_eq!(prepared.storage_path, expected_path);
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260507-160000-screen-shot.png"
    );
    assert_eq!(
        prepared.prompt_text,
        "Image file: .gwt/drop-files/20260507-160000-screen-shot.png"
    );
}

#[test]
fn image_paste_prepare_uses_docker_project_root_reference() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let payload = base64::engine::general_purpose::STANDARD.encode(b"jpeg-bytes");

    let prepared = super::super::prepare_image_paste_file(
        temp.path(),
        "/workspace/project",
        &payload,
        "image/jpeg",
        Some("Clipboard Image"),
        "20260507-160001",
    )
    .expect("prepare docker image paste");

    assert_eq!(
        prepared.storage_path,
        temp.path()
            .join(".gwt")
            .join("drop-files")
            .join("20260507-160001-clipboard-image.jpg")
    );
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260507-160001-clipboard-image.jpg"
    );
    assert_eq!(
        prepared.prompt_text,
        "Image file: .gwt/drop-files/20260507-160001-clipboard-image.jpg"
    );
}

#[test]
fn image_paste_prepare_rejects_unsupported_mime_and_empty_payload() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let payload = base64::engine::general_purpose::STANDARD.encode(b"gif-bytes");

    let unsupported = super::super::prepare_image_paste_file(
        temp.path(),
        "/workspace/project",
        &payload,
        "image/gif",
        Some("unsupported.gif"),
        "20260507-160002",
    );
    assert!(matches!(
        unsupported,
        Err(super::super::ImagePasteError::UnsupportedMimeType(mime)) if mime == "image/gif"
    ));

    let empty = super::super::prepare_image_paste_file(
        temp.path(),
        "/workspace/project",
        "",
        "image/png",
        None,
        "20260507-160003",
    );
    assert!(matches!(
        empty,
        Err(super::super::ImagePasteError::EmptyPayload)
    ));
}

#[test]
fn image_paste_event_saves_file_under_worktree() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/image-paste".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let payload = base64::engine::general_purpose::STANDARD.encode(b"webp-bytes");
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "paste_image",
        "id": window_id,
        "data_base64": payload,
        "mime_type": "image/webp",
        "filename": "capture.webp"
    }))
    .expect("deserialize paste image event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect paste files");
    assert_eq!(files.len(), 1, "expected one saved image");
    assert!(
        !worktree.join(".gwt").join("paste-images").exists(),
        "new image paste files must not be written to legacy paste-images"
    );
    let saved_path = files[0].path();
    assert_eq!(
        saved_path.extension().and_then(|ext| ext.to_str()),
        Some("webp")
    );
    assert_eq!(
        fs::read(saved_path).expect("read saved image"),
        b"webp-bytes"
    );
}

#[test]
fn uploaded_image_paste_event_saves_file_under_drop_files() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let uploaded_path = temp.path().join("image-upload.tmp");
    fs::write(&uploaded_path, b"png-upload").expect("write uploaded image");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime
        .attachment_uploads
        .insert(
            "image-upload-1".to_string(),
            UploadedAttachment {
                path: uploaded_path.clone(),
                filename: "Screenshot.png".to_string(),
                mime_type: Some("image/png".to_string()),
                size: 10,
            },
        )
        .expect("register image upload");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/image-paste".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "paste_image_uploaded",
        "id": window_id,
        "upload_id": "image-upload-1",
        "mime_type": "image/png",
        "filename": "Screenshot.png",
        "size": 10
    }))
    .expect("deserialize uploaded paste image event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect drop files");
    assert_eq!(files.len(), 1, "expected one saved uploaded image");
    assert_eq!(
        fs::read(files[0].path()).expect("read saved image"),
        b"png-upload"
    );
    assert!(
        !uploaded_path.exists(),
        "uploaded temp image should be removed"
    );
    assert!(
        !worktree.join(".gwt").join("paste-images").exists(),
        "uploaded image paste must not create legacy paste-images"
    );
}

#[test]
fn image_paste_event_ignores_non_agent_terminal_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "shell-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    let payload = base64::engine::general_purpose::STANDARD.encode(b"png-bytes");
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "paste_image",
        "id": window_id,
        "data_base64": payload,
        "mime_type": "image/png",
        "filename": "capture.png"
    }))
    .expect("deserialize paste image event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    assert!(
        !worktree.join(".gwt").join("drop-files").exists(),
        "non-agent terminal paste must not create image files"
    );
}

#[test]
fn file_attachment_prepare_copies_host_native_path_under_drop_files() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let source = temp.path().join("report.pdf");
    fs::write(&source, b"host-pdf").expect("write source file");

    let prepared = super::super::prepare_file_attachment(
        &worktree,
        &worktree.display().to_string(),
        gwt_agent::LaunchRuntimeTarget::Host,
        &gwt::FileAttachment::NativePath {
            path: source.display().to_string(),
        },
        "20260524-attach",
        ContentLimits::default(),
        &AttachmentUploadStore::in_system_temp(),
    )
    .expect("prepare host native path attachment");

    let expected_path = worktree
        .join(".gwt")
        .join("drop-files")
        .join("20260524-attach-report.pdf");
    assert_eq!(prepared.bytes, None);
    assert_eq!(prepared.source_path.as_deref(), Some(source.as_path()));
    assert_eq!(
        prepared.storage_path.as_deref(),
        Some(expected_path.as_path())
    );
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260524-attach-report.pdf"
    );
    assert_eq!(
        super::super::format_file_attachment_prompt(&[prepared.agent_path]),
        "File: \".gwt/drop-files/20260524-attach-report.pdf\""
    );
}

#[test]
fn file_attachment_prepare_copies_inline_file_under_worktree() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let payload = base64::engine::general_purpose::STANDARD.encode(b"notes-bytes");

    let prepared = super::super::prepare_file_attachment(
        &worktree,
        &worktree.display().to_string(),
        gwt_agent::LaunchRuntimeTarget::Host,
        &gwt::FileAttachment::Inline {
            filename: "../Notes 2026.txt".to_string(),
            mime_type: Some("application/octet-stream".to_string()),
            size: 11,
            data_base64: payload,
        },
        "20260524-inline",
        ContentLimits::default(),
        &AttachmentUploadStore::in_system_temp(),
    )
    .expect("prepare inline file attachment");

    let expected_path = worktree
        .join(".gwt")
        .join("drop-files")
        .join("20260524-inline-notes-2026.txt");
    assert_eq!(prepared.bytes.as_deref(), Some(&b"notes-bytes"[..]));
    assert_eq!(
        prepared.storage_path.as_deref(),
        Some(expected_path.as_path())
    );
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260524-inline-notes-2026.txt"
    );
}

#[test]
fn file_attachment_prepare_preserves_japanese_unicode_basename() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let payload = base64::engine::general_purpose::STANDARD.encode(b"unicode-notes");

    let prepared = super::super::prepare_file_attachment(
        &worktree,
        &worktree.display().to_string(),
        gwt_agent::LaunchRuntimeTarget::Host,
        &gwt::FileAttachment::Inline {
            filename: "../資料 日本語.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            size: 13,
            data_base64: payload,
        },
        "20260604-inline",
        ContentLimits::default(),
        &AttachmentUploadStore::in_system_temp(),
    )
    .expect("prepare unicode filename attachment");

    assert_eq!(
        prepared.storage_path.as_deref(),
        Some(
            worktree
                .join(".gwt")
                .join("drop-files")
                .join("20260604-inline-資料-日本語.txt")
                .as_path()
        )
    );
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260604-inline-資料-日本語.txt"
    );
    assert_eq!(
        super::super::format_file_attachment_prompt(&[prepared.agent_path]),
        "File: \".gwt/drop-files/20260604-inline-資料-日本語.txt\""
    );
}

#[test]
fn file_attachment_prepare_copies_native_file_for_docker_agent_path() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let source = temp.path().join("Data Set.bin");
    fs::write(&source, b"docker-bytes").expect("write source file");

    let prepared = super::super::prepare_file_attachment(
        &worktree,
        "/workspace/project",
        gwt_agent::LaunchRuntimeTarget::Docker,
        &gwt::FileAttachment::NativePath {
            path: source.display().to_string(),
        },
        "20260524-docker",
        ContentLimits::default(),
        &AttachmentUploadStore::in_system_temp(),
    )
    .expect("prepare docker native file attachment");

    assert_eq!(prepared.bytes, None);
    assert_eq!(prepared.source_path.as_deref(), Some(source.as_path()));
    assert_eq!(
        prepared.storage_path.as_deref(),
        Some(
            worktree
                .join(".gwt")
                .join("drop-files")
                .join("20260524-docker-data-set.bin")
                .as_path()
        )
    );
    assert_eq!(
        prepared.agent_path,
        ".gwt/drop-files/20260524-docker-data-set.bin"
    );
}

#[test]
fn file_attachment_prepare_rejects_invalid_items() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let payload = base64::engine::general_purpose::STANDARD.encode(b"too-large");
    let limits = ContentLimits {
        text_max_bytes: 16,
        binary_chunk_max_bytes: 3,
    };

    let too_large = super::super::prepare_file_attachment(
        &worktree,
        "/workspace/project",
        gwt_agent::LaunchRuntimeTarget::Host,
        &gwt::FileAttachment::Inline {
            filename: "large.dat".to_string(),
            mime_type: None,
            size: 9,
            data_base64: payload,
        },
        "20260524-large",
        limits,
        &AttachmentUploadStore::in_system_temp(),
    );
    assert!(matches!(
        too_large,
        Err(super::super::FileAttachmentError::TooLarge { size: 9, limit: 3 })
    ));

    let directory = super::super::prepare_file_attachment(
        &worktree,
        "/workspace/project",
        gwt_agent::LaunchRuntimeTarget::Host,
        &gwt::FileAttachment::NativePath {
            path: temp.path().display().to_string(),
        },
        "20260524-dir",
        ContentLimits::default(),
        &AttachmentUploadStore::in_system_temp(),
    );
    assert!(matches!(
        directory,
        Err(super::super::FileAttachmentError::NotAFile(path)) if path == temp.path().display().to_string()
    ));
}

#[test]
fn file_attachment_prompt_formats_single_and_multiple_paths_without_newlines() {
    let single = super::super::format_file_attachment_prompt(&["/tmp/a\"b\nc.txt".to_string()]);
    assert_eq!(single, "File: \"/tmp/a\\\"b\\nc.txt\"");
    assert!(
        !single.contains('\n'),
        "single-file prompt must never inject a newline"
    );

    let multiple = super::super::format_file_attachment_prompt(&[
        "/tmp/a.txt".to_string(),
        "C:\\tmp\\b\r.txt".to_string(),
    ]);
    assert_eq!(
        multiple,
        "Files: [\"/tmp/a.txt\", \"C:\\\\tmp\\\\b\\r.txt\"]"
    );
    assert!(
        !multiple.contains('\r') && !multiple.contains('\n'),
        "multi-file prompt must stay on one terminal input line"
    );
}

#[test]
fn file_attachment_event_saves_inline_file_under_worktree() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/file-drop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let payload = base64::engine::general_purpose::STANDARD.encode(b"text-bytes");
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "files": [
            {
                "source": "inline",
                "filename": "notes.txt",
                "mime_type": "text/plain",
                "size": 10,
                "data_base64": payload
            }
        ]
    }))
    .expect("deserialize attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect drop files");
    assert_eq!(files.len(), 1, "expected one saved dropped file");
    assert_eq!(
        fs::read(files[0].path()).expect("read saved file"),
        b"text-bytes"
    );
}

#[test]
fn file_attachment_event_saves_native_path_under_drop_files() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let source = temp.path().join("large-host-file.bin");
    fs::write(&source, b"native-bytes").expect("write native source");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/file-drop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "files": [
            {
                "source": "native_path",
                "path": source.display().to_string()
            }
        ]
    }))
    .expect("deserialize attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect drop files");
    assert_eq!(files.len(), 1, "expected one saved native file");
    assert_eq!(
        fs::read(files[0].path()).expect("read saved file"),
        b"native-bytes"
    );
    assert!(
        files[0]
            .file_name()
            .to_string_lossy()
            .ends_with("large-host-file.bin"),
        "saved native file should keep sanitized source basename"
    );
}

#[test]
fn file_attachment_event_saves_uploaded_file_under_drop_files() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let uploaded_path = temp.path().join("upload.tmp");
    fs::write(&uploaded_path, b"uploaded-bytes").expect("write uploaded temp");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime
        .attachment_uploads
        .insert(
            "upload-1".to_string(),
            UploadedAttachment {
                path: uploaded_path.clone(),
                filename: "Browser Large.bin".to_string(),
                mime_type: Some("application/octet-stream".to_string()),
                size: 14,
            },
        )
        .expect("register upload");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/file-drop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "files": [
            {
                "source": "uploaded",
                "upload_id": "upload-1",
                "filename": "Browser Large.bin",
                "mime_type": "application/octet-stream",
                "size": 14
            }
        ]
    }))
    .expect("deserialize attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect drop files");
    assert_eq!(files.len(), 1, "expected one saved uploaded file");
    assert_eq!(
        fs::read(files[0].path()).expect("read saved file"),
        b"uploaded-bytes"
    );
    assert!(
        !uploaded_path.exists(),
        "uploaded temp file should be removed after staging"
    );
}

#[test]
fn file_attachment_operation_dispatches_failed_progress_without_prompt_on_stage_failure() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/file-drop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "operation_id": "attachment-op-fail",
        "files": [
            {
                "source": "native_path",
                "path": temp.path().display().to_string()
            }
        ]
    }))
    .expect("deserialize operation attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(
            events.iter().any(|event| matches!(
                &event.event,
                BackendEvent::AttachmentProgress {
                    id,
                    operation_id,
                    phase: AttachmentProgressPhase::Queued,
                    ..
                } if id == &window_id && operation_id == "attachment-op-fail"
            )),
            "operation-aware attachment handling should acknowledge queued progress immediately: {events:?}"
        );
    wait_for_recorded_event("failed attachment progress", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::Dispatch(dispatched)
                    if dispatched.iter().any(|outbound| matches!(
                        &outbound.event,
                        BackendEvent::AttachmentProgress {
                            id,
                            operation_id,
                            phase: AttachmentProgressPhase::Failed,
                            message: Some(message),
                            ..
                        } if id == &window_id
                            && operation_id == "attachment-op-fail"
                            && message.contains("not a file")
                    ))
            )
        })
    });
    {
        let events = recorded_events.lock().expect("event log");
        assert!(
            !events.iter().any(|event| matches!(
                recorded_project_payload(event),
                UserEvent::AttachmentPromptReady { .. }
            )),
            "failed staging must not enqueue terminal prompt injection"
        );
    }
}

#[test]
fn file_attachment_copy_reports_byte_progress() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let source = temp.path().join("日本語-source.bin");
    let storage = temp
        .path()
        .join("repo")
        .join(".gwt")
        .join("drop-files")
        .join("20260604-日本語-source.bin");
    let payload = vec![b'x'; 192 * 1024 + 7];
    fs::write(&source, &payload).expect("write source file");
    let prepared = super::super::PreparedFileAttachment {
        bytes: None,
        source_path: Some(source.clone()),
        remove_source_after_save: false,
        storage_path: Some(storage.clone()),
        agent_path: ".gwt/drop-files/20260604-日本語-source.bin".to_string(),
    };
    let mut progress = Vec::new();

    super::super::save_file_attachment_with_progress(&prepared, |bytes_done, bytes_total| {
        progress.push((bytes_done, bytes_total));
    })
    .expect("copy attachment with progress");

    assert_eq!(fs::read(&storage).expect("read copied file"), payload);
    assert!(
            progress.len() >= 3,
            "copy should report an initial sample, at least one chunk, and final completion: {progress:?}"
        );
    assert_eq!(progress.first(), Some(&(0, Some(192 * 1024 + 7))));
    assert_eq!(
        progress.last(),
        Some(&(192 * 1024 + 7, Some(192 * 1024 + 7)))
    );
    assert!(
        progress
            .windows(2)
            .all(|pair| pair[0].0 <= pair[1].0 && pair[0].1 == pair[1].1),
        "copy progress must be monotonic with stable total: {progress:?}"
    );
}

#[test]
fn file_attachment_event_saves_prepared_files_incrementally() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "agent-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/file-drop".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        },
    );
    let valid_payload = base64::engine::general_purpose::STANDARD.encode(b"saved-first");
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "files": [
            {
                "source": "inline",
                "filename": "first.txt",
                "mime_type": "text/plain",
                "size": 11,
                "data_base64": valid_payload
            },
            {
                "source": "inline",
                "filename": "invalid.txt",
                "mime_type": "text/plain",
                "size": 4,
                "data_base64": "not base64"
            }
        ]
    }))
    .expect("deserialize attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(
        events.is_empty(),
        "invalid later attachment must not inject a partial prompt",
    );
    let drop_dir = worktree.join(".gwt").join("drop-files");
    let files = fs::read_dir(&drop_dir)
        .expect("read drop dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect drop files");
    assert_eq!(
        files.len(),
        1,
        "first attachment should be saved before a later file fails",
    );
    assert_eq!(
        fs::read(files[0].path()).expect("read saved file"),
        b"saved-first"
    );
}

#[test]
fn file_attachment_event_ignores_non_agent_terminal_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let worktree = temp.path().join("repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let tab_id = "tab-1";
    let raw_window_id = "shell-1";
    let window_id = combined_window_id(tab_id, raw_window_id);
    let tab = sample_project_tab_with_window_at(
        tab_id,
        raw_window_id,
        worktree.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _events) = sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
    let payload = base64::engine::general_purpose::STANDARD.encode(b"ignored");
    let event: FrontendEvent = serde_json::from_value(serde_json::json!({
        "kind": "attach_files",
        "id": window_id,
        "files": [
            {
                "source": "inline",
                "filename": "ignored.txt",
                "mime_type": "text/plain",
                "size": 7,
                "data_base64": payload
            }
        ]
    }))
    .expect("deserialize attach files event");

    let events = runtime.handle_frontend_event("client-1".to_string(), event);

    assert!(events.is_empty());
    assert!(
        !worktree.join(".gwt").join("drop-files").exists(),
        "non-agent terminal file drop must not create files"
    );
}
