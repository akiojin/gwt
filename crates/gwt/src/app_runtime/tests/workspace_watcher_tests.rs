use super::*;

/// Phase U-5 (SPEC-2359 US-38, FR-129, FR-130): the WebSocket reconnect
/// path goes through `FrontendEvent::FrontendReady` → `frontend_sync_events`.
/// The replied `WindowCanvasState` must carry each window's `dynamic_title`
/// and `dynamic_title_detail` so the frontend's `windowDisplayTitle()` can
/// rehydrate the pane heading without waiting for another mutation. This
/// test fails if anyone strips `dynamic_title` from the projected
/// `WorkspaceView`, regressing the reconnect contract.
#[test]
fn frontend_sync_events_preserves_window_dynamic_title_for_reconnect_rehydrate() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    agent.purpose_title = Some("Initial purpose".to_string());
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let tab_mut = runtime.tab_mut("tab-1").expect("tab mut");
    tab_mut.workspace.set_dynamic_title_with_detail(
        "agent-1",
        Some("Phase U-5 rehydrate target".to_string()),
        Some("simulated stale state before reconnect".to_string()),
    );

    let events = runtime.frontend_project_sync_events("client-1", &runtime.test_context());

    let workspace_event = events
        .iter()
        .find(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. }))
        .expect("WindowCanvasState reply for FrontendReady");
    let workspace = match &workspace_event.event {
        BackendEvent::WindowCanvasState { workspace } => workspace,
        _ => unreachable!(),
    };
    let projected_window = workspace
        .tabs
        .iter()
        .find(|tab| tab.id == "tab-1")
        .and_then(|tab| {
            tab.workspace
                .windows
                .iter()
                .find(|window| window.id == combined_window_id("tab-1", "agent-1"))
        })
        .expect("agent window in projected WindowCanvasState");
    assert_eq!(
            projected_window.dynamic_title.as_deref(),
            Some("Phase U-5 rehydrate target"),
            "frontend_sync_events must include dynamic_title so reconnect rehydrate restores pane heading"
        );
    assert_eq!(
        projected_window.dynamic_title_detail.as_deref(),
        Some("simulated stale state before reconnect"),
        "frontend_sync_events must include dynamic_title_detail so tooltip survives reconnect"
    );
}

/// Phase U-5: re-asserts the diff gate from
/// `apply_workspace_projection_title_sync_skips_workspace_state_when_same_title_resyncs`
/// at the Board entrypoint. Re-posting an identical milestone (same body for
/// `current_focus`) must not emit a duplicate `WindowCanvasState` broadcast
/// on busy projections.
#[test]
fn app_runtime_board_milestone_skips_workspace_state_on_identical_resync() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260513-0343".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    save_assigned_workspace_projection_for_test(
        &repo,
        runtime
            .active_agent_sessions
            .get(&window_id)
            .expect("session"),
    )
    .expect("save projection");
    let milestone = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Stable body for current_focus",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1")
    .with_title_summary("Stable title");

    let first = runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);
    assert!(
        first
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "first Board post should broadcast WindowCanvasState: {first:?}"
    );

    let second = runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);
    assert!(
            !second
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
            "second Board post with identical current_focus must not duplicate WindowCanvasState: {second:?}"
        );
    assert!(second
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::ActiveWorkProjection { .. })));
    let projection = wait_for_active_work_projection(&mut runtime);
    assert_eq!(projection.active_agents, 1);
}

#[test]
fn app_runtime_board_milestone_ignores_legacy_title_summary_for_window_title() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    agent.purpose_title = Some("Initial purpose".to_string());
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260507-0227".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    save_assigned_workspace_projection_for_test(
        &repo,
        runtime
            .active_agent_sessions
            .get(&window_id)
            .expect("session"),
    )
    .expect("save projection");
    let long_body = "Implementing the title-summary contract across Board, Workspace, runtime synchronization, CLI parsing, hook reminders, and frontend titlebar rendering";
    let mut entry_value = serde_json::to_value(
        BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            long_body,
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1"),
    )
    .expect("entry json");
    entry_value["title_summary"] = serde_json::json!("Title summary contract");
    let milestone: BoardEntry = serde_json::from_value(entry_value).expect("milestone");

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

    let tab = runtime.tab("tab-1").expect("tab");
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent")
            .dynamic_title
            .as_deref(),
        None
    );
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent")
            .dynamic_title_detail
            .as_deref(),
        Some(long_body)
    );
}

#[test]
fn app_runtime_board_milestone_without_title_summary_keeps_existing_agent_window_title() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    agent.purpose_title = Some("Initial purpose".to_string());
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260507-0227".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    save_assigned_workspace_projection_for_test(
        &repo,
        runtime
            .active_agent_sessions
            .get(&window_id)
            .expect("session"),
    )
    .expect("save projection");
    let milestone = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "This long body should remain detail only and should not become the titlebar text",
        None,
        None,
        vec!["start-work".to_string()],
        vec!["SPEC-2359".to_string()],
    )
    .with_origin_session_id("session-1");

    runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

    let tab = runtime.tab("tab-1").expect("tab");
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent")
            .dynamic_title
            .as_deref(),
        None
    );
}

#[test]
fn app_runtime_workspace_projection_change_updates_agent_window_title_summary() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut tab_workspace = empty_workspace_state();
    let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
    agent.title = "Codex".to_string();
    tab_workspace.windows.push(agent);
    tab_workspace.next_z_index = 2;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260510-0900".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection
        .agents
        .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: "session-1".to_string(),
            window_id: Some(window_id),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some(
                "Implement mandatory Agent title summary updates for Workspace".to_string(),
            ),
            title_summary: Some("Agent title summary guard".to_string()),
            worktree_path: Some(repo.clone()),
            branch: Some("work/20260510-0900".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let events = commit_workspace_watcher_update(&mut runtime, &repo, &projection);

    // Issue #3783 keeps this watcher path cache-only on purpose: it merges the
    // already-loaded payload into the last materialized view instead of
    // decoding Session/WorkItems, because a full rebuild here blocks every pane
    // request. So read the projection out of the broadcast it just published
    // rather than waiting for a background rebuild this path must not schedule.
    let refreshed_projection = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjectionPatch { projection } => Some(projection.clone()),
            _ => None,
        })
        .expect("cache-only projection broadcast");
    assert_eq!(refreshed_projection.active_agents, 1);
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Agent title summary guard")
    );
    assert_eq!(
        agent_window.dynamic_title_detail.as_deref(),
        Some("Implement mandatory Agent title summary updates for Workspace")
    );
}

#[test]
fn workspace_watcher_defers_membership_and_titles_until_worker_completion() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Prepared watcher title"),
        Some("Prepared watcher focus"),
    );

    let events = runtime.handle_workspace_projection_changed_events(&repo, &projection);

    assert!(
        events.is_empty(),
        "Tao must only schedule the watcher preparation"
    );
    assert_eq!(tasks.lock().expect("queued tasks").len(), 1);
    assert!(runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow()
        .is_empty());
    assert!(runtime
        .tab("tab-1")
        .unwrap()
        .workspace
        .window("agent-1")
        .unwrap()
        .dynamic_title
        .is_none());
}

#[test]
fn workspace_watcher_rejects_superseded_and_closed_pane_patches() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let (proxy, recorded) = AppEventProxy::stub();
    runtime.proxy = proxy;
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let mut projection =
        apply_title_sync_sample_projection(&repo, &window_id, Some("Old watcher title"), None);
    runtime.handle_workspace_projection_changed_events(&repo, &projection);
    projection.agents[0].title_summary = Some("Latest watcher title".into());
    runtime.handle_workspace_projection_changed_events(&repo, &projection);
    let run_next = || tasks.lock().unwrap().remove(0)();
    let take_prepared = || match into_recorded_project_payload(recorded.lock().unwrap().remove(0)) {
        UserEvent::WorkspaceProjectionPatchPrepared(prepared) => *prepared,
        event => panic!("unexpected event: {event:?}"),
    };
    run_next();
    assert!(runtime
        .apply_workspace_projection_patch(take_prepared())
        .is_none());
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "an older result must not spawn another reload"
    );
    run_next();
    assert!(runtime
        .apply_workspace_projection_patch(take_prepared())
        .is_some());
    assert_eq!(
        runtime
            .tab("tab-1")
            .unwrap()
            .workspace
            .window("agent-1")
            .unwrap()
            .dynamic_title
            .as_deref(),
        Some("Latest watcher title")
    );
    recorded.lock().unwrap().clear();

    projection.agents[0].title_summary = Some("Stale close title".into());
    runtime.handle_workspace_projection_changed_events(&repo, &projection);
    run_next();
    let stale = take_prepared();
    runtime.active_agent_sessions.clear();
    assert!(runtime.close_window_outcome(&window_id).closed);
    assert!(runtime.apply_workspace_projection_patch(stale).is_none());
    let state = runtime.project_state_for_tab("tab-1").unwrap();
    assert_eq!(
        state
            .active_work_projection_cache
            .borrow()
            .get("tab-1")
            .unwrap()
            .agents[0]
            .title_summary
            .as_deref(),
        Some("Latest watcher title")
    );
}

#[test]
fn workspace_projection_changed_uses_supplied_snapshot_instead_of_rereading_disk() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let supplied = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("fresh payload title"),
        Some("fresh payload focus"),
    );
    let stale_disk = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("stale disk title"),
        Some("stale disk focus"),
    );
    gwt_core::workspace_projection::save_workspace_projection(&repo, &stale_disk)
        .expect("save stale projection");

    let events = commit_workspace_watcher_update(&mut runtime, &repo, &supplied);

    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjectionPatch { .. })));
    let agent_window = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .window("agent-1")
        .expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("fresh payload title")
    );
    assert_eq!(
        agent_window.dynamic_title_detail.as_deref(),
        Some("fresh payload focus")
    );
}

#[test]
fn apply_workspace_projection_title_sync_writes_dynamic_title() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Canonical orchestration"),
        Some("Implement apply_workspace_projection_title_sync"),
    );

    let _events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Canonical orchestration"),
        "dynamic_title should reflect projection.agents[<i>].title_summary"
    );
    assert_eq!(
        agent_window.dynamic_title_detail.as_deref(),
        Some("Implement apply_workspace_projection_title_sync"),
        "dynamic_title_detail should reflect projection.agents[<i>].current_focus"
    );
}

#[test]
fn apply_workspace_projection_title_sync_emits_active_work_projection_for_active_tab() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Materialize an old cache first. The watcher payload remains the source
    // for title sync, and the cache-only Work broadcast must merge the fresh
    // payload rather than replaying this stale snapshot.
    let mut old_projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Old cached purpose"),
        Some("Old cached focus"),
    );
    let mut removed = old_projection.agents[0].clone();
    removed.session_id = "session-removed".to_string();
    removed.window_id = Some("tab-1::removed".to_string());
    old_projection.agents.push(removed);
    let mut cached_view =
        super::super::workspace_views::active_work_projection_from_saved(old_projection);
    let retained_session = gwt::WorkspaceHistorySessionView {
        agent_session_id: "conversation-session-1".to_string(),
        started_at: "2026-08-29T00:00:00Z".to_string(),
        is_active: true,
        resumable: true,
    };
    for agent in cached_view
        .agents
        .iter_mut()
        .chain(cached_view.unassigned_agents.iter_mut())
    {
        if agent.session_id == "session-1" {
            agent.sessions = vec![retained_session.clone()];
        }
    }
    for work in &mut cached_view.active_works {
        for agent in &mut work.agents {
            if agent.session_id == "session-1" {
                agent.sessions = vec![retained_session.clone()];
            }
        }
        for child in &mut work.works {
            for agent in &mut child.agents {
                if agent.session_id == "session-1" {
                    agent.sessions = vec![retained_session.clone()];
                }
            }
        }
    }
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow_mut()
        .insert("tab-1".to_string(), cached_view);
    let mut projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Fresh watcher purpose"),
        Some("Fresh watcher focus"),
    );
    projection.agents[0].status_category =
        gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked;
    let mut added = projection.agents[0].clone();
    added.session_id = "session-added".to_string();
    added.window_id = Some("tab-1::added".to_string());
    added.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
    projection.agents.push(added);
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    super::super::workspace_views::reset_full_active_work_projection_builds();
    let events = commit_workspace_watcher_update(&mut runtime, &repo, &projection);

    let active_work = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjectionPatch { projection } => Some(projection.as_ref()),
            _ => None,
        })
        .expect("expected bounded ActiveWorkProjection patch");
    let fresh_agents = active_work
        .agents
        .iter()
        .chain(active_work.unassigned_agents.iter())
        .chain(
            active_work
                .active_works
                .iter()
                .flat_map(|work| work.agents.iter()),
        )
        .chain(
            active_work
                .active_works
                .iter()
                .flat_map(|work| work.works.iter())
                .flat_map(|work| work.agents.iter()),
        )
        .filter(|agent| agent.session_id == "session-1")
        .collect::<Vec<_>>();
    assert!(
        !fresh_agents.is_empty(),
        "fresh projection must retain its agent"
    );
    assert!(fresh_agents.iter().all(|agent| {
        agent.current_focus.as_deref() == Some("Fresh watcher focus")
            && agent.title_summary.as_deref() == Some("Fresh watcher purpose")
            && agent.status_category == "blocked"
            && agent.sessions.is_empty()
    }));
    assert!(runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow()
        .get("tab-1")
        .into_iter()
        .flat_map(|projection| projection.agents.iter())
        .filter(|agent| agent.session_id == "session-1")
        .all(|agent| agent.sessions == [retained_session.clone()]));
    let all_session_ids = active_work
        .agents
        .iter()
        .chain(active_work.unassigned_agents.iter())
        .chain(
            active_work
                .active_works
                .iter()
                .flat_map(|work| work.agents.iter()),
        )
        .chain(
            active_work
                .active_works
                .iter()
                .flat_map(|work| work.works.iter())
                .flat_map(|work| work.agents.iter()),
        )
        .map(|agent| agent.session_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(all_session_ids.contains("session-added"));
    assert!(
        !all_session_ids.contains("session-removed"),
        "fresh watcher membership must remove agents absent from current.json"
    );
    assert_eq!(active_work.active_agents, 1);
    assert_eq!(active_work.blocked_agents, 1);
    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0,
        "a projection watcher callback must replay cache-only on the GUI event loop"
    );
}

#[test]
fn workspace_projection_changed_initializes_cold_cache_from_authoritative_membership() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    assert!(runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow()
        .is_empty());

    let mut projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Fresh cold-cache purpose"),
        Some("Fresh cold-cache focus"),
    );
    projection.agents[0].session_id = "session-added".to_string();
    projection.agents[0].window_id = Some("tab-1::added".to_string());

    let events = commit_workspace_watcher_update(&mut runtime, &repo, &projection);

    let active_work = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjectionPatch { projection } => Some(projection.as_ref()),
            _ => None,
        })
        .expect("expected bounded ActiveWorkProjection patch");
    let session_ids = active_work
        .agents
        .iter()
        .chain(active_work.unassigned_agents.iter())
        .chain(
            active_work
                .active_works
                .iter()
                .flat_map(|work| work.agents.iter()),
        )
        .chain(
            active_work
                .active_works
                .iter()
                .flat_map(|work| work.works.iter())
                .flat_map(|work| work.agents.iter()),
        )
        .map(|agent| agent.session_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(session_ids.contains("session-added"));
    assert!(
        !session_ids.contains("session-1"),
        "cold cache must not fall back to stale active_agent_sessions membership"
    );
    assert!(runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow()
        .contains_key("tab-1"));
}

/// Issue #3783 cache reconciliation: closing one active peer must not demote a
/// Work that still has a blocked peer. Blocked is a current runtime state, so
/// the lifecycle remains active until both active and blocked counts are zero.
#[test]
fn cached_close_preserves_blocked_peer_and_active_lifecycle() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (runtime, window_id) = apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let mut projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Closing peer"),
        Some("Closing peer focus"),
    );
    let mut blocked = projection.agents[0].clone();
    blocked.session_id = "session-blocked-peer".to_string();
    blocked.window_id = Some("tab-1::agent-blocked".to_string());
    blocked.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked;
    blocked.current_focus = Some("Waiting for review".to_string());
    projection.agents.push(blocked);
    let view = super::super::workspace_views::active_work_projection_from_saved(projection);
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow_mut()
        .insert("tab-1".to_string(), view);
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_payload_cache
        .borrow_mut()
        .insert("tab-1".to_string(), Arc::from("pre-close-payload"));

    runtime.mark_cached_active_work_session_stopped("tab-1", "session-1", &window_id);

    assert!(
        !runtime
            .project_state_for_tab("tab-1")
            .unwrap()
            .active_work_projection_payload_cache
            .borrow()
            .contains_key("tab-1"),
        "a cache-only lifecycle patch must invalidate the older wire payload"
    );

    let cache = runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow();
    let view = cache.get("tab-1").expect("cached projection");
    assert_eq!(view.active_agents, 0);
    assert_eq!(view.blocked_agents, 1);
    assert_eq!(view.status_category, "blocked");
    let work = view.active_works.first().expect("active Work row");
    assert_eq!(work.blocked_agents, 1);
    assert_eq!(work.lifecycle_state, "active");
    assert_eq!(work.status_category, "blocked");
}

#[test]
fn apply_workspace_projection_title_sync_returns_no_events_without_active_tab() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) = apply_title_sync_setup_tab_and_runtime(repo.clone(), None);
    let projection =
        apply_title_sync_sample_projection(&repo, &window_id, Some("No active tab"), None);

    let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

    assert!(
        !events.iter().any(|event| matches!(
            event.event,
            BackendEvent::ActiveWorkProjection { .. }
                | BackendEvent::ActiveWorkProjectionPatch { .. }
        )),
        "without an active tab, ActiveWorkProjection broadcast must be skipped"
    );
    // Even without an active tab, the in-memory dynamic_title should still
    // be synced so the next workspace_state broadcast carries it.
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("No active tab"),
        "dynamic_title must be set in-memory regardless of active_tab routing"
    );
}

#[test]
fn apply_workspace_projection_title_sync_emits_workspace_state_when_dynamic_title_changed() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Phase U-2 WindowCanvasState assertion"),
        None,
    );
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

    // Phase U-2 (SPEC-2359 US-26): a workspace update path that mutates
    // an in-memory dynamic_title MUST broadcast WindowCanvasState in the
    // same batch so the frontend's `windowData.dynamic_title` and the
    // pane heading `windowDisplayTitle` refresh without waiting for the
    // next hook event or window structure change.
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "expected WindowCanvasState broadcast when dynamic_title changed: {events:?}"
    );
}

#[test]
fn apply_workspace_projection_title_sync_skips_workspace_state_when_nothing_changed() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Drop the active_agent_sessions entry AND erase the projection's
    // window_id so neither the fast path nor the Phase U-3 fallback
    // can resolve a window. The WindowCanvasState broadcast should be
    // skipped to avoid forcing a frontend re-render for a no-op update.
    runtime.active_agent_sessions.clear();
    let mut projection =
        apply_title_sync_sample_projection(&repo, "tab-1::agent-1", Some("No-op"), None);
    projection.agents[0].window_id = None;
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "WindowCanvasState must be skipped when in-memory dynamic_title did not change: {events:?}"
    );
}

#[test]
fn apply_workspace_projection_title_sync_skips_workspace_state_when_same_title_resyncs() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Stable title"),
        Some("Stable focus"),
    );
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    // First sync: dynamic_title transitions from None → "Stable title".
    let first = runtime.apply_workspace_projection_title_sync(&repo, &projection);
    assert!(
        first
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "first sync should broadcast WindowCanvasState: {first:?}"
    );

    // Second sync with the same projection: nothing diffs, so the
    // WindowCanvasState broadcast must be suppressed to avoid forcing a
    // full frontend re-render on busy projections (Codex review P2).
    let second = runtime.apply_workspace_projection_title_sync(&repo, &projection);
    assert!(
        !second
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "second sync with identical title must not broadcast WindowCanvasState: {second:?}"
    );
    // ActiveWorkProjection still commits on the background continuation.
    assert!(second
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::ActiveWorkProjection { .. })));
    let active_work = wait_for_active_work_projection(&mut runtime);
    assert_eq!(active_work.active_agents, 1);
}

#[test]
fn handle_workspace_projection_changed_events_broadcasts_workspace_state_for_pane_heading() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Pane heading via WindowCanvasState"),
        Some("triggered by workspace.update params.purpose"),
    );
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let events = commit_workspace_watcher_update(&mut runtime, &repo, &projection);

    // The original handler returned only ActiveWorkProjection. Phase
    // U-2 promotes it to also broadcast WindowCanvasState in one batch so
    // the pane heading refreshes immediately after `workspace.update`.
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "handle_workspace_projection_changed_events must broadcast WindowCanvasState: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjectionPatch { .. })),
        "ActiveWorkProjection broadcast must still fire: {events:?}"
    );
    // Cache-only by design (Issue #3783): assert the broadcast this path just
    // published rather than waiting for a background rebuild it must not
    // schedule, since a full decode here would block every pane request.
    let active_work = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjectionPatch { projection } => Some(projection.clone()),
            _ => None,
        })
        .expect("cache-only projection broadcast");
    assert_eq!(active_work.active_agents, 1);
}

#[test]
fn handle_workspace_projection_changed_events_syncs_title_from_canonical_project_root() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project_root = temp.path().join("workspace-home");
    let worktree = project_root.join("work").join("20260601-0934");
    fs::create_dir_all(&worktree).expect("worktree");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(project_root.clone(), Some("tab-1"));
    runtime
        .active_agent_sessions
        .get_mut(&window_id)
        .expect("active session")
        .worktree_path = worktree.clone();
    let mut projection = apply_title_sync_sample_projection(
        &project_root,
        &window_id,
        Some("Canonical Project State title"),
        Some("Agent worktree differs from Project State root"),
    );
    projection.agents[0].worktree_path = Some(worktree);
    gwt_core::workspace_projection::save_workspace_projection(&project_root, &projection)
        .expect("save projection");

    let events = commit_workspace_watcher_update(&mut runtime, &project_root, &projection);

    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "canonical Project State root updates must broadcast WindowCanvasState: {events:?}"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Canonical Project State title")
    );
    assert_eq!(
        agent_window.dynamic_title_detail.as_deref(),
        Some("Agent worktree differs from Project State root")
    );
}

#[test]
fn sync_agent_window_titles_falls_back_to_projection_window_id_when_active_agent_sessions_missing()
{
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Phase U-3: simulate a session that gwt's launch tracking has not
    // registered (e.g. GUI restarted after launch, or session started
    // outside gwt's launch path). The window exists in workspace state
    // and the projection knows its window_id + worktree_path, but
    // active_agent_sessions is empty.
    runtime.active_agent_sessions.clear();
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Phase U-3 backfill via projection window_id"),
        Some("ensures untracked sessions still update pane heading"),
    );

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        changed,
        "Phase U-3: sync must return true when projection-driven fallback resolves the window"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Phase U-3 backfill via projection window_id"),
        "dynamic_title must propagate even when active_agent_sessions is empty"
    );
    assert_eq!(
        agent_window.dynamic_title_detail.as_deref(),
        Some("ensures untracked sessions still update pane heading")
    );
}

#[test]
fn sync_agent_window_titles_skips_fallback_when_projection_worktree_mismatches() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let other_repo = temp.path().join("other-repo");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other_repo).expect("create other repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    runtime.active_agent_sessions.clear();
    // Projection claims the agent lives in a *different* worktree.
    // Phase U-3 fallback must refuse to touch local windows in that
    // case, otherwise cross-worktree titles could leak.
    let mut projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Cross-worktree title leak guard"),
        None,
    );
    projection.agents[0].worktree_path = Some(other_repo);

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        !changed,
        "fallback must refuse cross-worktree window updates"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert!(
        agent_window.dynamic_title.is_none(),
        "dynamic_title must stay None when projection.agents[<i>].worktree_path mismatches"
    );
}

#[test]
fn sync_agent_window_titles_skips_fallback_when_projection_window_id_unknown_locally() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    runtime.active_agent_sessions.clear();
    // Projection references a window_id that is NOT present in any
    // local tab. The fallback must short-circuit instead of producing
    // a phantom window update.
    let mut projection =
        apply_title_sync_sample_projection(&repo, "tab-1::agent-1", Some("phantom"), None);
    projection.agents[0].window_id = Some("tab-99::ghost".to_string());

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        !changed,
        "fallback must require the projected window_id to exist locally"
    );
}

#[test]
fn sync_agent_window_titles_returns_false_when_no_resolution_path_exists() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Drop the active_agent_sessions entry AND erase the projection's
    // window_id so neither the fast path nor the Phase U-3 fallback
    // can resolve this agent's window. The sync must then be a no-op.
    runtime.active_agent_sessions.clear();
    let mut projection = apply_title_sync_sample_projection(
        &repo,
        "tab-1::agent-1",
        Some("Should not propagate"),
        None,
    );
    projection.agents[0].window_id = None;

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        !changed,
        "sync must return false when no in-memory window was touched"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert!(
            agent_window.dynamic_title.is_none(),
            "dynamic_title must stay None when neither active_agent_sessions nor projection window_id resolve"
        );
}

// ---------------------------------------------------------------------
// SPEC-2359 Phase U-4: worktree-only fallback for SessionStart hook
// registered records (window_id is None, session_id does not match any
// active_agent_session). Resolves to the unique active session in the
// same worktree with the same agent_id when one exists.
// ---------------------------------------------------------------------

#[test]
fn sync_agent_window_titles_fast_path_resolves_across_unrelated_project_root() {
    // SPEC-2359 Phase U-7: when the watcher event project_root is for
    // tab A but a session lives in tab B (both share current.json
    // because they're worktrees of the same repo), the fast path must
    // still resolve B's window via session_id alone. Previously the
    // additional `same_worktree_path(session.worktree_path,
    // project_root)` filter prevented this and caused the user-visible
    // pane heading to stay on the agent_id fallback even after
    // `workspace.update` succeeded at the data
    // layer.
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let unrelated = temp.path().join("unrelated");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&unrelated).expect("create unrelated");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));

    // Projection identifies the agent by session-1 with a worktree
    // that *does not* match the unrelated project_root we will pass
    // in. The fast path should still resolve.
    let projection = apply_title_sync_sample_projection(
        &repo,
        "tab-1::agent-1",
        Some("Phase U-7 cross-tab fast path"),
        None,
    );

    let changed =
        runtime.sync_agent_window_titles_from_workspace_projection(&unrelated, &projection);

    assert!(
        changed,
        "fast path must resolve by session_id alone, independent of project_root"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Phase U-7 cross-tab fast path"),
    );
}

#[test]
fn sync_agent_window_titles_falls_back_to_worktree_when_session_id_not_active() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let mut projection = apply_title_sync_sample_projection(
        &repo,
        "tab-1::agent-1",
        Some("Phase U-4 worktree fallback"),
        Some("SessionStart-hook registered records have no window_id"),
    );
    // Simulate a SessionStart-hook registration: same worktree, same
    // agent_id (codex), but a *different* session_id and no window_id.
    // The fast path won't match by session_id, but the worktree-only
    // fallback should resolve to the in-memory Codex window.
    projection.agents[0].session_id = "out-of-band-session".to_string();
    projection.agents[0].window_id = None;

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        changed,
        "worktree fallback should resolve to the unique active session"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert_eq!(
        agent_window.dynamic_title.as_deref(),
        Some("Phase U-4 worktree fallback"),
    );
}

#[test]
fn sync_agent_window_titles_worktree_fallback_refuses_when_agent_id_mismatches() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, _window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Active session is `codex`, but the projection record (SessionStart
    // registered) claims `claude`. The fallback must refuse rather than
    // assigning a Claude title to the Codex pane.
    let mut projection = apply_title_sync_sample_projection(
        &repo,
        "tab-1::agent-1",
        Some("Wrong-agent title leak guard"),
        None,
    );
    projection.agents[0].session_id = "out-of-band-claude".to_string();
    projection.agents[0].window_id = None;
    projection.agents[0].agent_id = "claude".to_string();

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        !changed,
        "worktree fallback must require matching agent_id to disambiguate"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    let agent_window = tab.workspace.window("agent-1").expect("agent window");
    assert!(agent_window.dynamic_title.is_none());
}

#[test]
fn sync_agent_window_titles_worktree_fallback_refuses_when_multiple_sessions_match() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    // Add a second Codex session in the same worktree. The fallback
    // must refuse to pick one because the mapping would be ambiguous.
    runtime.active_agent_sessions.insert(
        "tab-1::agent-2".to_string(),
        ActiveAgentSession {
            window_id: "tab-1::agent-2".to_string(),
            session_id: "session-2".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260510-0900".to_string(),
            display_name: "Codex 2".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: String::new(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    let mut projection =
        apply_title_sync_sample_projection(&repo, &window_id, Some("Ambiguity guard"), None);
    projection.agents[0].session_id = "out-of-band-session".to_string();
    projection.agents[0].window_id = None;

    let changed = runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

    assert!(
        !changed,
        "worktree fallback must refuse when multiple sessions share the same worktree + agent_id"
    );
}

#[test]
fn app_runtime_runtime_hook_state_does_not_update_agent_window_dynamic_title() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    tab.workspace
        .set_dynamic_title("codex-1", Some("Board milestone focus".to_string()));
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));

    assert!(events
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::RuntimeHookEvent { .. })));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "non-structural runtime hook state changes must not force a full workspace_state"
    );
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowState { .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })));
    let tab = runtime.tab("tab-1").expect("tab");
    assert_eq!(
        tab.workspace
            .window("codex-1")
            .expect("codex window")
            .dynamic_title
            .as_deref(),
        Some("Board milestone focus")
    );
}

#[test]
fn app_runtime_gui_board_scope_uses_active_workspace_id_not_first_agent_workspace() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.id = "workspace-active".to_string();
    projection.agents.push(workspace_agent_summary_for_test(
        "session-other",
        Some("workspace-other"),
    ));
    projection
        .agents
        .push(workspace_agent_summary_for_test("session-active", None));
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");

    let scope = super::super::board::gui_default_board_scope_for_project(&repo)
        .expect("resolve GUI board scope");
    let post_audience = gwt::board_audience::post_audience_for_gui(&repo, &[], None, false)
        .expect("resolve post audience");

    assert_eq!(
        scope,
        BoardAudienceScope::Workspace("workspace-active".to_string())
    );
    assert_eq!(post_audience, Some(vec!["workspace-active".to_string()]));
}

#[test]
fn app_runtime_board_projection_change_preserves_all_view_for_live_updates() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.id = "workspace-a".to_string();
    projection
        .agents
        .push(workspace_agent_summary_for_test("session-a", None));
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save projection");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "Workspace A update",
            None,
            None,
            vec![],
            vec![],
        )
        .with_audience(vec!["workspace-a"]),
    )
    .expect("seed workspace A update");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "Workspace B update",
            None,
            None,
            vec![],
            vec![],
        )
        .with_audience(vec!["workspace-b"]),
    )
    .expect("seed workspace B update");

    let mut tab_workspace = empty_workspace_state();
    tab_workspace.windows.push(sample_window(
        "board-all",
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    ));
    tab_workspace.windows.push(sample_window(
        "board-workspace",
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    ));
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let all_window_id = combined_window_id("tab-1", "board-all");
    let workspace_window_id = combined_window_id("tab-1", "board-workspace");
    let _ = runtime.load_board_events("client-1", &all_window_id, true);
    let _ = runtime.load_board_events("client-1", &workspace_window_id, false);

    let events = runtime.handle_board_projection_changed_events(&repo);

    let all_entries = events
        .iter()
        .find_map(|event| match event {
            OutboundEvent {
                event: BackendEvent::BoardEntries { id, entries, .. },
                ..
            } if id == &all_window_id => Some(entries),
            _ => None,
        })
        .expect("all view board entries");
    let workspace_entries = events
        .iter()
        .find_map(|event| match event {
            OutboundEvent {
                event: BackendEvent::BoardEntries { id, entries, .. },
                ..
            } if id == &workspace_window_id => Some(entries),
            _ => None,
        })
        .expect("workspace view board entries");

    assert!(
        all_entries
            .iter()
            .any(|entry| entry.body == "Workspace B update"),
        "All view live update must include other Workspace entries"
    );
    assert!(
        !workspace_entries
            .iter()
            .any(|entry| entry.body == "Workspace B update"),
        "Workspace view live update must stay scoped"
    );
}

#[test]
fn app_runtime_board_projection_change_broadcasts_to_matching_board_windows_only() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let other_repo = temp.path().join("other-repo");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other_repo).expect("create other repo");
    post_entry(
        &repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "External update",
            None,
            None,
            vec![],
            vec![],
        ),
    )
    .expect("seed matching board snapshot");
    post_entry(
        &other_repo,
        BoardEntry::new(
            AuthorKind::Agent,
            "codex",
            BoardEntryKind::Status,
            "Other project update",
            None,
            None,
            vec![],
            vec![],
        ),
    )
    .expect("seed other board snapshot");

    let mut tab_workspace = empty_workspace_state();
    tab_workspace.windows.push(sample_window(
        "board-1",
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    ));
    tab_workspace.windows.push(sample_window(
        "board-2",
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    ));
    tab_workspace.windows.push(sample_window(
        "logs-1",
        WindowPreset::Logs,
        WindowProcessStatus::Ready,
    ));
    tab_workspace.next_z_index = 4;
    let matching_tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(tab_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let other_tab = sample_project_tab_with_window_at(
        "tab-2",
        "board-3",
        other_repo,
        WindowPreset::Board,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![matching_tab, other_tab], Some("tab-1"));

    super::super::workspace_views::reset_full_active_work_projection_builds();
    let events = runtime.handle_board_projection_changed_events(&repo);

    // Issue #4406: a post that is no Work milestone changes no Work row, so the
    // refresh emits only the two Board windows and rebuilds no Active Work.
    assert_eq!(events.len(), 2);
    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0
    );
    for expected_id in [
        combined_window_id("tab-1", "board-1"),
        combined_window_id("tab-1", "board-2"),
    ] {
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Project(key),
                event: BackendEvent::BoardEntries { id, entries, .. },
                ..
            } if Some(key) == runtime.project_key_for_tab("tab-1") && *id == expected_id
                && entries.len() == 1
                && entries[0].body == "External update"
        )));
    }
    assert!(!events.iter().any(|event| matches!(
        event,
        OutboundEvent {
            event: BackendEvent::BoardEntries { id, .. },
            ..
        } if *id == combined_window_id("tab-2", "board-3")
    )));
}

#[test]
fn board_projection_refresh_applies_a_work_milestone_without_rebuilding_active_work() {
    // Issue #4406 AC-1: applying a Board refresh is the only part of a Board
    // change on the GUI event loop; a full Active Work rebuild there cost
    // seconds per post.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let (mut runtime, window_id) =
        apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
    let projection = apply_title_sync_sample_projection(
        &repo,
        &window_id,
        Some("Board milestone title"),
        Some("posted a decision"),
    );
    super::super::workspace_views::reset_full_active_work_projection_builds();

    let events = runtime.apply_board_projection_refresh(super::super::BoardProjectionRefreshed {
        context: runtime.project_context("tab-1"),
        events: Vec::new(),
        milestone: Some((repo.clone(), projection)),
    });

    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "the milestone must still refresh the pane heading: {events:?}"
    );
    let tab = runtime.tab("tab-1").expect("tab");
    assert_eq!(
        tab.workspace
            .window("agent-1")
            .expect("agent window")
            .dynamic_title
            .as_deref(),
        Some("Board milestone title")
    );
}

#[test]
fn background_work_scan_results_refresh_active_work_off_the_gui_event_loop() {
    // Issue #4406 AC-3: `WorkTipSubjects` and `WorkMergeStatus` are background
    // scan completions, and each rebuilt the disk-backed Active Work projection
    // on the GUI event loop — 930,665ms and 796,705ms of stall over one 20
    // minute window. They must only cache the result and ask for an off-loop
    // refresh.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let (mut runtime, events, _window_id) = active_work_off_loop_setup(temp.path(), &repo);

    super::super::workspace_views::reset_full_active_work_projection_builds();
    let tip_events = runtime.apply_work_tip_subjects(
        &repo,
        [("work/off-loop".to_string(), "tip subject".to_string())]
            .into_iter()
            .collect(),
    );
    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0,
        "WorkTipSubjects must not enter the disk-backed projection builder"
    );
    assert!(
        tip_events.is_empty(),
        "the rail update rides the off-loop refresh, not the handler return: {tip_events:?}"
    );
    assert_eq!(wait_for_active_work_prepare_completions(&events, 1), 1);
    // The broker collapses per project, so commit the tip-subject prepare
    // before asking again — otherwise the merge scan's request only lands in
    // `pending` and no second completion can be observed.
    let _ = wait_for_active_work_projection(&mut runtime);

    super::super::workspace_views::reset_full_active_work_projection_builds();
    let merge_events = runtime.apply_work_merge_status(
        &repo,
        [("work/off-loop".to_string(), chrono::Utc::now())]
            .into_iter()
            .collect(),
        HashMap::new(),
        HashSet::new(),
        HashSet::new(),
        None,
    );
    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0,
        "WorkMergeStatus must not enter the disk-backed projection builder"
    );
    assert!(merge_events.is_empty());
    assert_eq!(wait_for_active_work_prepare_completions(&events, 1), 1);

    // AC-6: the drained refresh still carries both scan results, so nothing the
    // rail showed before is lost by moving the build off the loop.
    let applied = drain_active_work_projection_refresh(&mut runtime, &repo);
    let projection = applied
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjection { projection } => Some(projection.clone()),
            _ => None,
        })
        .expect("the off-loop refresh broadcasts the rebuilt rail");
    let row = projection
        .active_works
        .iter()
        .find(|work| work.branch.as_deref() == Some("work/off-loop"))
        .expect("row");
    assert!(
        row.merged_into_base,
        "the merge scan result reached the row"
    );
    assert_eq!(row.work_summary.as_deref(), Some("tip subject"));
}

#[test]
fn workspace_state_load_failure_keeps_rail_and_replays_until_recovery() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let (mut runtime, _, _) = active_work_off_loop_setup(temp.path(), &repo);
    gwt_core::workspace_projection::save_workspace_projection(
        &repo,
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .unwrap();
    drain_active_work_projection_refresh(&mut runtime, &repo);
    let cached = || {
        runtime
            .project_state_for_tab("tab-1")
            .unwrap()
            .active_work_projection_cache
            .borrow()
            .get("tab-1")
            .cloned()
    };
    let before = serde_json::to_value(cached()).unwrap();
    let path = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&repo);
    let original = fs::read(&path).unwrap();
    fs::write(&path, b"{broken workspace").unwrap();

    let events = drain_active_work_projection_refresh(&mut runtime, &repo);
    assert!(events
        .iter()
        .any(|event| event.event.event_kind() == "workspace_state_notice"));
    assert_eq!(
        serde_json::to_value(
            runtime
                .project_state_for_tab("tab-1")
                .unwrap()
                .active_work_projection_cache
                .borrow()
                .get("tab-1")
        )
        .unwrap(),
        before
    );
    let context = runtime.project_context_for_root(&repo).unwrap();
    let replay = runtime.frontend_project_sync_events("reconnected", &context);
    assert!(replay
        .iter()
        .any(|event| event.event.event_kind() == "workspace_state_notice"));
    assert_eq!(fs::read(&path).unwrap(), b"{broken workspace");

    fs::write(&path, original).unwrap();
    let cached_refresh = drain_active_work_projection_refresh(&mut runtime, &repo);
    assert!(
        !cached_refresh.iter().any(|event| {
            matches!(
                &event.event,
                BackendEvent::WorkspaceStateNotice { notice: None }
            )
        }),
        "a cache hit must not clear the notice before a fresh load"
    );
    let Some(UserEvent::WorkspaceProjectionLoaded { .. }) =
        crate::load_workspace_projection_user_event(&repo)
    else {
        panic!("repaired canonical files must load successfully");
    };
    let recovered = runtime.handle_workspace_state_loaded(&repo);
    let notice = recovered
        .iter()
        .find(|event| event.event.event_kind() == "workspace_state_notice")
        .expect("clear the pending load notice");
    assert!(serde_json::to_value(&notice.event).unwrap()["notice"].is_null());
}
