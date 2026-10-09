use super::*;

#[test]
fn app_runtime_update_terminal_grid_resizes_runtime_without_workspace_broadcast() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Agent],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let agent_id = combined_window_id("tab-1", "agent-1");
    insert_test_pane_runtime(&mut runtime, &agent_id);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::UpdateTerminalGrid {
            id: agent_id.clone(),
            cols: 112,
            rows: 34,
        },
    );

    assert!(
        events
            .iter()
            .all(|event| !matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "embedded terminal grid resize must not mutate canvas geometry"
    );
    let pane = runtime
        .runtimes
        .get(&agent_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane");
    assert_eq!(pane.screen().size(), (34, 112));
}

#[test]
fn app_runtime_cycle_focus_preserves_real_fit_pty_size() {
    // Issue #2937: cycle_focus must NOT clobber the PTY size that the
    // frontend established via its real xterm fit. The backend's
    // geometry_to_pty_size approximation is only a spawn bootstrap;
    // reverting an already-fitted PTY back to it on every window switch
    // is what desyncs the child's grid from xterm and corrupts the
    // rendered terminal (recovers on manual resize).
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let bounds = canvas_bounds();
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Shell, WindowPreset::Claude],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let shell_id = combined_window_id("tab-1", "shell-1");
    let claude_id = combined_window_id("tab-1", "claude-1");
    insert_test_pane_runtime(&mut runtime, &shell_id);
    insert_test_pane_runtime(&mut runtime, &claude_id);

    // Sentinel that differs from geometry_to_pty_size for any open
    // window, so a clobber via the approximation is detectable.
    const REAL_COLS: u16 = 137;
    const REAL_ROWS: u16 = 41;
    for raw_id in ["shell-1", "claude-1"] {
        let geometry = runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .window(raw_id)
            .expect("window")
            .geometry
            .clone();
        assert_ne!(
            geometry_to_pty_size(&geometry),
            (REAL_COLS, REAL_ROWS),
            "sentinel must differ from the approximation to be meaningful",
        );
    }

    // Simulate the frontend's real xterm fit having sized each PTY.
    for window_id in [&shell_id, &claude_id] {
        runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane")
            .resize(REAL_COLS, REAL_ROWS)
            .expect("resize");
    }

    assert_eq!(
        runtime
            .cycle_focus_events(
                &runtime.test_context(),
                FocusCycleDirection::Forward,
                bounds
            )
            .len(),
        1
    );

    for window_id in [&shell_id, &claude_id] {
        let pane = runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane");
        assert_eq!(
            pane.screen().size(),
            (REAL_ROWS, REAL_COLS),
            "cycle_focus must not clobber the frontend-fitted PTY size via geometry_to_pty_size",
        );
    }
}

#[test]
fn app_runtime_activate_window_tab_preserves_real_fit_pty_size() {
    // SPEC-2008 Phase 34 / Issue #2937 companion: tab activation changes only
    // the active marker. The backend must not resize the PTY from the shared
    // group geometry, because the frontend's visible xterm fit owns the real
    // cols/rows for the revealed tab.
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Shell, WindowPreset::Claude],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let shell_id = combined_window_id("tab-1", "shell-1");
    let claude_id = combined_window_id("tab-1", "claude-1");

    assert_eq!(
        runtime.dock_window_tab_events(&shell_id, &claude_id).len(),
        1
    );
    let _ = runtime.activate_window_tab_events(&shell_id);

    insert_test_pane_runtime(&mut runtime, &shell_id);
    insert_test_pane_runtime(&mut runtime, &claude_id);

    const REAL_COLS: u16 = 173;
    const REAL_ROWS: u16 = 31;
    for raw_id in ["shell-1", "claude-1"] {
        let geometry = runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .window(raw_id)
            .expect("window")
            .geometry
            .clone();
        assert_ne!(
            geometry_to_pty_size(&geometry),
            (REAL_COLS, REAL_ROWS),
            "sentinel must differ from the shared-geometry approximation",
        );
    }

    for window_id in [&shell_id, &claude_id] {
        runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane")
            .resize(REAL_COLS, REAL_ROWS)
            .expect("resize");
    }

    let events = runtime.activate_window_tab_events(&claude_id);

    assert_eq!(events.len(), 1);
    let workspace = &runtime.tab("tab-1").expect("tab").workspace;
    assert!(
        workspace
            .window("claude-1")
            .expect("claude")
            .tab_group_active
    );
    assert!(!workspace.window("shell-1").expect("shell").tab_group_active);
    for window_id in [&shell_id, &claude_id] {
        let pane = runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane");
        assert_eq!(
            pane.screen().size(),
            (REAL_ROWS, REAL_COLS),
            "tab activation must not clobber the frontend-fitted PTY size via geometry_to_pty_size",
        );
    }
}

#[test]
fn app_runtime_arrange_windows_does_not_clobber_real_fit_pty_size() {
    // Issue #2937 companion: arrange_windows shares the same all-window
    // resize fan-out as cycle_focus. The frontend re-fit (driven by the
    // geometry_revision bump) is the single source of truth for PTY
    // size; the backend must not revert PTYs to the approximation here.
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let bounds = canvas_bounds();
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Shell, WindowPreset::Claude],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let shell_id = combined_window_id("tab-1", "shell-1");
    let claude_id = combined_window_id("tab-1", "claude-1");
    insert_test_pane_runtime(&mut runtime, &shell_id);
    insert_test_pane_runtime(&mut runtime, &claude_id);

    const REAL_COLS: u16 = 151;
    const REAL_ROWS: u16 = 47;
    for window_id in [&shell_id, &claude_id] {
        runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane")
            .resize(REAL_COLS, REAL_ROWS)
            .expect("resize");
    }

    assert_eq!(
        runtime
            .arrange_windows_events(&runtime.test_context(), ArrangeMode::Tile, bounds)
            .len(),
        1
    );

    for window_id in [&shell_id, &claude_id] {
        let pane = runtime
            .runtimes
            .get(window_id)
            .expect("runtime")
            .pane
            .lock()
            .expect("pane");
        assert_eq!(
                pane.screen().size(),
                (REAL_ROWS, REAL_COLS),
                "arrange_windows must not clobber the frontend-fitted PTY size via geometry_to_pty_size",
            );
    }
}

#[test]
fn app_runtime_frontend_ready_replays_active_work_projection_separately_from_workspace() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        "tab-1::agent-1".to_string(),
        ActiveAgentSession {
            window_id: "tab-1::agent-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.join("../repo-work-20260504-1234"),
            agent_project_root: repo
                .join("../repo-work-20260504-1234")
                .display()
                .to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    super::super::workspace_views::reset_full_active_work_projection_builds();
    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);
    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0,
        "FrontendReady must hydrate from memory without entering the disk-backed projection builder"
    );

    assert!(matches!(
        events.get(1).map(|event| &event.event),
        Some(BackendEvent::WindowCanvasState { .. })
    ));
    let projection = events.iter().find_map(|event| match &event.event {
        BackendEvent::ActiveWorkProjection { projection } => Some(projection),
        _ => None,
    });
    let projection = projection.expect("active work projection event");
    assert_eq!(projection.status_category, "active");
    assert_eq!(projection.active_agents, 1);
    assert_eq!(projection.branch.as_deref(), Some("work/20260504-1234"));
    assert_eq!(projection.agents.len(), 1);
    assert_eq!(projection.agents[0].session_id, "session-1");
    assert_eq!(projection.agents[0].display_name, "Codex");
    assert_eq!(projection.agents[0].status_category, "active");
    assert_eq!(
        projection.agents[0].branch.as_deref(),
        Some("work/20260504-1234")
    );
    assert!(
        events.iter().all(|event| matches!(
            &event.target,
            DispatchTarget::Client(client_id) if client_id == "client-1"
        )),
        "frontend-ready projection replay must remain client-scoped"
    );
}

#[test]
fn app_runtime_geometry_focus_dock_and_activate_never_build_disk_projection() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir_all(&first).expect("create first project");
    fs::create_dir_all(&second).expect("create second project");
    let tabs = vec![
        sample_project_tab(
            "tab-1",
            "First",
            first,
            ProjectKind::Git,
            &[WindowPreset::Shell],
        ),
        sample_project_tab(
            "tab-2",
            "Second",
            second,
            ProjectKind::Git,
            &[WindowPreset::Shell, WindowPreset::Claude],
        ),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));
    let shell_id = combined_window_id("tab-2", "shell-1");
    let claude_id = combined_window_id("tab-2", "claude-1");
    let bounds = canvas_bounds();

    super::super::workspace_views::reset_full_active_work_projection_builds();
    assert!(!runtime
        .focus_window_events(&shell_id, Some(bounds.clone()))
        .is_empty());
    assert!(!runtime
        .update_window_geometry_events(&shell_id, bounds.clone(), 120, 36, None)
        .is_empty());
    assert!(!runtime
        .dock_window_tab_events(&claude_id, &shell_id)
        .is_empty());
    assert!(!runtime.activate_window_tab_events(&claude_id).is_empty());

    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0,
        "UpdateWindowGeometry/FocusWindow/DockWindowTab/ActivateWindowTab must stay cache-only \
         and never enter the disk-backed projection builder",
    );
}

#[test]
fn app_runtime_authoritative_empty_projection_replaces_stale_replay_cache() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        "tab-1::agent-1".to_string(),
        ActiveAgentSession {
            window_id: "tab-1::agent-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/stale-cache".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.clone(),
            agent_project_root: repo.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    assert!(runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .is_some());
    assert!(runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow()
        .contains_key("tab-1"));

    runtime.active_agent_sessions.clear();
    let empty = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("authoritative empty projection");
    assert!(empty.active_works.is_empty());
    assert!(
        runtime
            .project_state_for_tab("tab-1")
            .unwrap()
            .active_work_projection_payload_cache
            .borrow()
            .contains_key("tab-1"),
        "an authoritative empty rebuild replaces the stale prepared replay"
    );
}

#[test]
fn app_runtime_select_project_tab_preserves_other_project_wizard() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other).expect("create other");
    let tabs = vec![
        sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::NonRepo,
            &[WindowPreset::Branches],
        ),
        sample_project_tab(
            "tab-2",
            "Other",
            other,
            ProjectKind::NonRepo,
            &[WindowPreset::FileTree],
        ),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));

    let events = runtime.select_project_tab_events("tab-2");

    assert_eq!(events.len(), 3);
    let other_context = runtime.project_context("tab-2").unwrap();
    assert!(events.iter().all(|event| matches!(&event.target, DispatchTarget::Project(key) if key == &other_context.project_key)));
    assert!(matches!(
        events[0].event,
        BackendEvent::WindowCanvasState { .. }
    ));
    assert!(matches!(
        events[1].event,
        BackendEvent::ActiveWorkProjection { .. }
    ));
    assert!(matches!(events[2].event, BackendEvent::PmStatus { .. }));
    let original_context = runtime.project_context("tab-1").unwrap();
    assert!(
        runtime
            .project_state(&original_context)
            .unwrap()
            .launch_wizard
            .is_some(),
        "selecting another project must preserve the owner's wizard"
    );
}

#[test]
fn app_runtime_select_project_tab_emits_active_work_projection_for_new_active_tab() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other).expect("create other");
    let tabs = vec![
        sample_project_tab("tab-1", "Repo", repo, ProjectKind::NonRepo, &[]),
        sample_project_tab("tab-2", "Other", other, ProjectKind::NonRepo, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));

    let events = runtime.select_project_tab_events("tab-2");

    let projection = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjection { projection } => Some(projection),
            _ => None,
        })
        .expect("ActiveWorkProjection broadcast for newly selected tab");
    assert_eq!(
        projection.id, "tab-2",
        "projection must reflect the newly active tab"
    );
}

#[test]
fn project_prepare_open_returns_before_repo_restore_and_preserves_cancel() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let current = temp.path().join("current");
    let selected = temp.path().join("selected");
    fs::create_dir_all(&current).expect("current project");
    fs::create_dir_all(&selected).expect("selected project");
    let tab = sample_project_tab("tab-current", "Current", current, ProjectKind::NonRepo, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-current"));
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;

    let before_tabs = runtime.tabs.len();
    let before_active = runtime.active_tab_id.clone();
    let before_recent = runtime.recent_projects.clone();
    let events = runtime.open_project_dialog_selection_events(Some(selected));

    assert!(
        events.is_empty(),
        "open prepare must return without a broadcast"
    );
    assert_eq!(runtime.tabs.len(), before_tabs);
    assert_eq!(runtime.active_tab_id, before_active);
    assert_eq!(runtime.recent_projects, before_recent);
    assert_eq!(queued_tasks.lock().expect("task queue").len(), 1);
    assert!(recorded_events.lock().expect("event log").is_empty());

    let cancel_events = runtime.open_project_dialog_selection_events(None);
    assert!(cancel_events.is_empty());
    assert_eq!(runtime.tabs.len(), before_tabs);
    assert_eq!(runtime.active_tab_id, before_active);
    assert_eq!(runtime.recent_projects, before_recent);

    drain_queued_blocking_tasks(&queued_tasks);
    let prepared = take_project_navigation_completion(&recorded_events);
    assert!(
        !runtime
            .handle_project_navigation_prepared(prepared)
            .is_empty(),
        "cancelling a later picker must preserve the project open already queued"
    );
    assert_eq!(runtime.tabs.len(), before_tabs + 1);
}

#[test]
fn project_picker_requests_queue_work_and_cancel_replies_once_to_the_client() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let (spawner, queued) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let started = runtime.open_project_dialog_events("picker-client");
    let BackendEvent::PickerStarted { request_id, .. } = started[0].event else {
        panic!("picker must acknowledge pending without opening a native dialog inline");
    };
    assert_eq!(queued.lock().unwrap().len(), 1);
    let duplicate = runtime.select_clone_project_parent_events("picker-client");
    assert!(matches!(
        duplicate[0].event,
        BackendEvent::PickerBusy { .. }
    ));
    assert_eq!(
        queued.lock().unwrap().len(),
        1,
        "repeat must not queue another dialog"
    );
    let result =
        runtime.handle_project_picker_finished("picker-client", request_id, true, Ok(None));
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].target,
        gwt::project_transport::DispatchTarget::Client("picker-client".into())
    );
    assert!(
        matches!(result[0].event, BackendEvent::PickerCancelled { request_id: id, .. } if id == request_id)
    );
    assert!(runtime
        .handle_project_picker_finished("picker-client", request_id, true, Ok(None))
        .is_empty());
    assert!(matches!(
        runtime.select_clone_project_parent_events("picker-client")[0].event,
        BackendEvent::PickerStarted { .. }
    ));
}

#[test]
fn project_picker_timeout_keeps_native_worker_busy_until_it_exits() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let (spawner, queued) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let started = runtime.open_project_dialog_events("client");
    let BackendEvent::PickerStarted { request_id, .. } = started[0].event else {
        panic!("started")
    };
    let timeout = runtime.handle_project_picker_finished(
        "client",
        request_id,
        false,
        Err("Folder selection timed out".into()),
    );
    assert_eq!(timeout.len(), 1);
    assert!(matches!(timeout[0].event, BackendEvent::PickerError { .. }));
    assert!(
        matches!(
            runtime.open_project_dialog_events("client")[0].event,
            BackendEvent::PickerBusy { .. }
        ),
        "timeout must not permit another native dialog while the first is alive"
    );
    assert_eq!(queued.lock().unwrap().len(), 1);
    assert!(
        runtime
            .handle_project_picker_finished(
                "client",
                request_id,
                true,
                Ok(Some(temp.path().into())),
            )
            .is_empty(),
        "late selection must not open a project or send a second terminal reply"
    );
    assert_eq!(queued.lock().unwrap().len(), 1);
    assert!(matches!(
        runtime.open_project_dialog_events("client")[0].event,
        BackendEvent::PickerStarted { .. }
    ));
}

#[test]
fn project_picker_selected_error_and_stale_results_preserve_request_identity() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
    let started = runtime.select_clone_project_parent_events("client");
    let BackendEvent::PickerStarted { request_id, .. } = started[0].event else {
        panic!("started")
    };
    let selected = runtime.handle_project_picker_finished(
        "client",
        request_id,
        true,
        Ok(Some(temp.path().into())),
    );
    assert_eq!(selected.len(), 1);
    assert!(
        matches!(&selected[0].event, BackendEvent::PickerSelected { purpose, path, .. }
        if purpose == "clone_parent" && path == &temp.path().display().to_string())
    );
    let next = runtime.open_project_dialog_events("client");
    let BackendEvent::PickerStarted {
        request_id: next_id,
        ..
    } = next[0].event
    else {
        panic!("started")
    };
    assert_ne!(request_id, next_id);
    assert!(runtime
        .handle_project_picker_finished("client", request_id, true, Ok(None))
        .is_empty());
    let timeout = runtime.handle_project_picker_finished(
        "client",
        next_id,
        false,
        Err("Folder selection timed out".into()),
    );
    assert_eq!(timeout.len(), 1);
    assert!(
        matches!(&timeout[0].event, BackendEvent::PickerError { request_id, message, .. }
        if *request_id == next_id && message.contains("timed out"))
    );
    assert!(runtime
        .handle_project_picker_finished("client", next_id, true, Ok(Some(temp.path().into())))
        .is_empty());
}

#[test]
fn project_picker_spawn_failure_clears_pending_and_search_runs_off_thread() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    runtime.blocking_tasks = BlockingTaskSpawner::failing("worker unavailable");
    let failed = runtime.open_project_dialog_events("client");
    assert_eq!(failed.len(), 2);
    assert!(
        matches!(&failed[1].event, BackendEvent::PickerError { message, .. } if message == "worker unavailable")
    );
    let (spawner, queued) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    assert!(matches!(
        runtime.open_project_dialog_events("client")[0].event,
        BackendEvent::PickerStarted { .. }
    ));
    let search = runtime.github_repository_search_events("client", "gwt");
    assert!(search.is_empty(), "gh must run on the queued worker");
    assert_eq!(queued.lock().unwrap().len(), 2);
}

#[test]
fn project_prepare_open_loads_window_restore_plan_in_the_worker() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let current = temp.path().join("current");
    let selected = temp.path().join("selected");
    fs::create_dir_all(&current).expect("create current project");
    fs::create_dir_all(&selected).expect("create selected project");
    fs::create_dir_all(temp.path().join("repo.git/worktrees/selected"))
        .expect("create linked-worktree metadata");
    fs::write(
        selected.join(".git"),
        "gitdir: ../repo.git/worktrees/selected\n",
    )
    .expect("write linked-worktree marker");
    let tabs = vec![sample_project_tab(
        "tab-current",
        "Current",
        current,
        ProjectKind::NonRepo,
        &[],
    )];
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), tabs, Some("tab-current"));
    let (blocking_tasks, queue) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;

    let mut workspace = empty_workspace_state();
    workspace.windows.push(sample_window(
        "shell-restore",
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    ));
    save_workspace_state(&workspace_state_path(&selected), &workspace)
        .expect("save selected workspace");

    assert!(runtime.open_project_path_events(selected).is_empty());
    drain_queued_blocking_tasks(&queue);
    let prepared = take_project_navigation_completion(&recorded_events);
    let ProjectNavigationPayload::Open(open) = prepared.result.expect("prepared project") else {
        panic!("expected open payload");
    };

    assert_eq!(
        open.window_restores.len(),
        1,
        "the worker must load the restore plan before main-thread commit"
    );
}

#[test]
fn project_prepare_open_commits_once_and_reuses_an_existing_project_key() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let first = temp.path().join("first");
    let existing = temp.path().join("existing");
    fs::create_dir_all(&first).expect("first project");
    fs::create_dir_all(&existing).expect("existing project");
    let tabs = vec![
        sample_project_tab("tab-first", "First", first, ProjectKind::NonRepo, &[]),
        sample_project_tab(
            "tab-existing",
            "Existing",
            existing.clone(),
            ProjectKind::NonRepo,
            &[],
        ),
    ];
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), tabs, Some("tab-first"));
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;

    assert!(runtime
        .open_project_path_with_request_events(existing, Some("client-open-1".to_string()))
        .is_empty());
    drain_queued_blocking_tasks(&queued_tasks);
    let prepared = take_project_navigation_completion(&recorded_events);
    let events = runtime.handle_project_navigation_prepared(prepared.clone());

    assert_eq!(
        runtime.tabs.len(),
        2,
        "existing ProjectKey must not duplicate a tab"
    );
    assert_eq!(runtime.active_tab_id.as_deref(), Some("tab-existing"));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ProjectOpened { project_key, request_id, .. }
            if project_key == &runtime.project_tab_incarnations["tab-existing"].project_key.to_string()
                && request_id.as_deref() == Some("client-open-1")
    )), "opening an existing project must produce an explicit Hub completion");
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
    assert!(
        runtime
            .handle_project_navigation_prepared(prepared)
            .is_empty(),
        "a completion is single-use"
    );
}

#[test]
fn project_prepare_clone_completion_uses_the_shared_async_open_contract() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let current = temp.path().join("current");
    let cloned = temp.path().join("cloned");
    fs::create_dir_all(&current).expect("current project");
    fs::create_dir_all(&cloned).expect("cloned project");
    let tabs = vec![sample_project_tab(
        "tab-current",
        "Current",
        current,
        ProjectKind::NonRepo,
        &[],
    )];
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), tabs, Some("tab-current"));
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;

    assert!(runtime.handle_clone_project_done(&cloned).is_empty());
    assert_eq!(
        queued_tasks.lock().expect("task queue").len(),
        1,
        "clone completion must enter the same worker-backed open API"
    );
    assert_eq!(
        runtime.tabs.len(),
        1,
        "clone completion must not open inline"
    );

    drain_queued_blocking_tasks(&queued_tasks);
    let prepared = take_project_navigation_completion(&recorded_events);
    let events = runtime.handle_project_navigation_prepared(prepared);

    assert_eq!(runtime.tabs.len(), 2);
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::CloneProjectDone { workspace_home }
            if workspace_home == &cloned.display().to_string()
    )));
    let active_tab = runtime
        .active_tab_id
        .as_ref()
        .and_then(|tab_id| runtime.project_tab_incarnations.get(tab_id))
        .expect("active project incarnation");
    let canonical_key: gwt_core::repo_hash::ProjectKey =
        gwt_core::paths::resolve_project_scope(&cloned).hash;
    assert_eq!(
        active_tab.project_key, canonical_key,
        "the consumer must keep RepoHash as the only project identity"
    );
}

#[test]
fn project_prepare_spawn_failure_is_visible_for_open_and_switch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let first = temp.path().join("first");
    let pending = temp.path().join("pending");
    for path in [&first, &pending] {
        fs::create_dir_all(path).expect("project");
    }
    let tabs = vec![
        sample_project_tab(
            "tab-first",
            "First",
            first.clone(),
            ProjectKind::NonRepo,
            &[],
        ),
        sample_project_tab("tab-second", "Second", first, ProjectKind::NonRepo, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-first"));
    runtime.blocking_tasks = BlockingTaskSpawner::failing("worker unavailable");

    let open_events =
        runtime.open_project_path_with_request_events(pending, Some("client-open-2".to_string()));
    assert!(open_events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ProjectOpenError { message, request_id } if message == "worker unavailable" && request_id.as_deref() == Some("client-open-2")
    )));
    assert_eq!(runtime.active_tab_id.as_deref(), Some("tab-first"));

    let switch_events = runtime.select_project_tab_events("tab-second");
    assert_eq!(runtime.active_tab_id.as_deref(), Some("tab-second"));
    assert!(switch_events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ProjectOpenError { message, .. } if message == "worker unavailable"
    )));
}

#[test]
fn project_prepare_open_drops_stale_completion_after_newer_tab_selection() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let pending = temp.path().join("pending");
    for path in [&first, &second, &pending] {
        fs::create_dir_all(path).expect("project");
    }
    let tabs = vec![
        sample_project_tab("tab-first", "First", first, ProjectKind::NonRepo, &[]),
        sample_project_tab("tab-second", "Second", second, ProjectKind::NonRepo, &[]),
    ];
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), tabs, Some("tab-first"));
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;

    assert!(runtime.open_project_path_events(pending.clone()).is_empty());
    let open_task = queued_tasks
        .lock()
        .expect("task queue")
        .pop()
        .expect("open prepare task");
    let select_events = runtime.select_project_tab_events("tab-second");
    assert_eq!(runtime.active_tab_id.as_deref(), Some("tab-second"));
    assert!(select_events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));

    open_task();
    let prepared = take_project_navigation_completion(&recorded_events);
    assert!(
        runtime
            .handle_project_navigation_prepared(prepared)
            .is_empty(),
        "the newer selection owns navigation authority"
    );
    assert_eq!(runtime.active_tab_id.as_deref(), Some("tab-second"));
    assert!(!runtime
        .tabs
        .iter()
        .any(|tab| same_worktree_path(&tab.project_root, &pending)));
}

#[test]
fn project_prepare_switch_drops_completion_after_close_and_generation_change() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir_all(&first).expect("first project");
    fs::create_dir_all(&second).expect("second project");
    let tabs = vec![
        sample_project_tab("tab-first", "First", first, ProjectKind::NonRepo, &[]),
        sample_project_tab("tab-second", "Second", second, ProjectKind::NonRepo, &[]),
    ];
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), tabs, Some("tab-first"));
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;

    runtime.select_project_tab_events("tab-second");
    drain_queued_blocking_tasks(&queued_tasks);
    let prepared = take_project_navigation_completion(&recorded_events);
    runtime.close_project_tab_events("tab-second");

    assert!(
        runtime
            .handle_project_navigation_prepared(prepared)
            .is_empty(),
        "a closed project incarnation must reject its switch completion"
    );
    assert!(runtime.project_context("tab-second").is_none());
    assert!(runtime.project_context("tab-first").is_some());

    runtime.select_project_tab_events("tab-first");
    drain_queued_blocking_tasks(&queued_tasks);
    let prepared = take_project_navigation_completion(&recorded_events);
    runtime.refresh_project_tab_incarnation("tab-first");
    assert!(
        runtime
            .handle_project_navigation_prepared(prepared)
            .is_empty(),
        "a migration/root generation update must reject the old completion"
    );
}

/// SPEC #3170 T-963: the SPEC #3287 project-per-tab consumer must reach
/// project open/switch only through the shared prepare → generation-checked
/// commit contract. A completion whose payload names a different project than
/// the request's incarnation is a foreign-project completion and must never
/// commit or broadcast.
#[test]
fn project_navigation_commit_rejects_a_foreign_project_key_payload() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir_all(&first).expect("first project");
    fs::create_dir_all(&second).expect("second project");
    let foreign_key = gwt_core::paths::resolve_project_scope(&first).hash;
    let tabs = vec![
        sample_project_tab("tab-first", "First", first, ProjectKind::NonRepo, &[]),
        sample_project_tab(
            "tab-second",
            "Second",
            second.clone(),
            ProjectKind::NonRepo,
            &[],
        ),
    ];
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), tabs, Some("tab-first"));
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;

    runtime.select_project_tab_events("tab-second");
    drain_queued_blocking_tasks(&queued_tasks);
    let mut prepared = take_project_navigation_completion(&recorded_events);
    prepared.result = Ok(ProjectNavigationPayload::Switch(PreparedProjectSwitch {
        tab_id: "tab-second".to_string(),
        project_key: foreign_key,
    }));

    assert!(
        runtime
            .handle_project_navigation_prepared(prepared)
            .is_empty(),
        "a completion carrying another project's ProjectKey must not broadcast"
    );
    assert!(
        runtime.pending_project_navigation.is_some(),
        "a foreign-project completion must not settle the reserved navigation request"
    );

    // The matching completion for the same request still commits, proving the
    // rejection above is keyed on the payload's ProjectKey and not on the
    // request having gone stale.
    let prepared = ProjectNavigationPrepared {
        request: runtime
            .pending_project_navigation
            .clone()
            .expect("reserved navigation request"),
        result: Ok(ProjectNavigationPayload::Switch(PreparedProjectSwitch {
            tab_id: "tab-second".to_string(),
            project_key: gwt_core::paths::resolve_project_scope(&second).hash,
        })),
    };
    runtime.handle_project_navigation_prepared(prepared);
    assert!(
        runtime.pending_project_navigation.is_none(),
        "the matching completion must settle the reserved navigation request"
    );
}

/// SPEC #3170 T-962: moving the heavy switch apply off the tao thread must not
/// downgrade what the switch broadcasts. `worktree_form` is only ever `Unknown`
/// on disk, so a persisted-only projection would strip the ephemeral /
/// branch-backed chrome from every live agent window on each tab switch.
#[test]
fn select_project_tab_broadcast_keeps_the_resolved_agent_worktree_form() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir_all(&first).expect("first project");
    fs::create_dir_all(&second).expect("second project");
    let tabs = vec![
        sample_project_tab("tab-first", "First", first, ProjectKind::NonRepo, &[]),
        sample_project_tab_with_window_at(
            "tab-second",
            "agent-1",
            second.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        ),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-first"));
    let (blocking_tasks, _queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;
    let window_id = combined_window_id("tab-second", "agent-1");
    let mut session = sample_active_agent_session("tab-second", &window_id);
    session.worktree_path = second;
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);

    let events = runtime.select_project_tab_events("tab-second");

    let workspace = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::WindowCanvasState { workspace } => Some(workspace),
            _ => None,
        })
        .expect("WindowCanvasState broadcast for the selected tab");
    let form = workspace
        .tabs
        .iter()
        .find(|tab| tab.id == "tab-second")
        .and_then(|tab| {
            tab.workspace
                .windows
                .iter()
                .find(|window| window.id == window_id)
        })
        .map(|window| window.worktree_form)
        .expect("projected agent window");
    assert_eq!(
        form,
        gwt::WindowWorktreeForm::BranchBacked,
        "a tab switch must keep the resolved worktree form instead of reporting `Unknown`"
    );
}

/// SPEC #3170 T-963: pin the single-entry contract itself so a later
/// project-per-tab consumer cannot reintroduce a synchronous open fallback or
/// a second project identity beside `project_tab_incarnations`.
#[test]
fn project_navigation_is_the_only_project_open_and_switch_entry() {
    let source = include_str!("../project_tabs.rs");
    assert!(
        !source.contains("fn open_project_path("),
        "the synchronous open fallback must stay removed; open goes through prepare → commit"
    );
    for entry in [
        "fn open_project_dialog_selection_events",
        "fn open_project_path_events",
        "fn handle_clone_project_done",
    ] {
        let body = source
            .split(entry)
            .nth(1)
            .and_then(|tail| tail.split("\n    pub(crate) fn ").next())
            .unwrap_or_else(|| panic!("{entry} body"));
        assert!(
            body.contains("request_project_open"),
            "{entry} must funnel into the shared asynchronous project open request"
        );
    }
    let commit = source
        .split("fn commit_prepared_project_open")
        .nth(1)
        .and_then(|tail| {
            tail.split("\n    fn remember_prepared_recent_project")
                .next()
        })
        .expect("commit body");
    assert!(
        commit.contains("incarnation.project_key == prepared.project_key"),
        "duplicate-open detection must use the canonical ProjectKey, not a second identity"
    );
    let dispatch = source
        .split("fn handle_project_navigation_prepared")
        .nth(1)
        .and_then(|tail| tail.split("\n    fn commit_prepared_project_open").next())
        .expect("dispatch body");
    assert!(
        dispatch.contains("project_navigation_request_is_current"),
        "every completion must be generation-checked before it commits or broadcasts"
    );
}

#[test]
fn app_runtime_select_project_tab_broadcasts_fresh_project_pm_status() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other).expect("create other");
    let tabs = vec![
        sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]),
        sample_project_tab("tab-2", "Other", other.clone(), ProjectKind::Git, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&other);
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        // Seed the legacy/manual-file case below the runtime floor. PmStatus
        // exposes the effective interval, never a value the loop will reject.
        prefs.settings.loop_interval_secs = 5;
    })
    .expect("seed newly selected project's latest prefs");

    let events = runtime.select_project_tab_events("tab-2");

    let interval = events
        .iter()
        .find_map(|outbound| match (&outbound.target, &outbound.event) {
            (
                DispatchTarget::Project(key),
                BackendEvent::PmStatus {
                    loop_interval_secs, ..
                },
            ) if Some(key) == runtime.project_key_for_tab("tab-2") => Some(*loop_interval_secs),
            _ => None,
        })
        .expect("tab switch must broadcast the newly active project's pm_status");
    assert_eq!(
        interval, 10,
        "status must reload and clamp that project's persisted interval"
    );
}

#[test]
fn app_runtime_close_project_preserves_other_project_pm_status() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other).expect("create other");
    let tabs = vec![
        sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]),
        sample_project_tab("tab-2", "Other", other.clone(), ProjectKind::Git, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&other);
    gwt::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
        prefs.settings.loop_interval_secs = 23;
    })
    .expect("seed fallback project's prefs");

    let events = runtime.close_project_tab_events("tab-1");

    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::PmStatus { .. })),
        "closing one project must not publish another project's PM state"
    );
    assert!(matches!(
        runtime.pm_status_event(&runtime.project_context("tab-2").unwrap()),
        BackendEvent::PmStatus {
            available: true,
            loop_interval_secs: 23,
            ..
        }
    ));
}

#[test]
fn app_runtime_close_last_project_clears_canvas_and_hub_catalog() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let context = runtime.test_context();
    let events = runtime.close_project_tab_events("tab-1");

    assert!(events
        .iter()
        .any(|event| matches!((&event.target, &event.event),
        (DispatchTarget::Project(key), BackendEvent::ProjectClosed { project_key })
        if key == &context.project_key && project_key == context.project_key.as_str())));
    assert!(events.iter().any(|event| matches!(&event.event,
        BackendEvent::HubState { hub } if hub.projects.is_empty())));
    assert!(runtime.project_state(&context).is_none());
}

#[test]
fn pm_status_event_uses_sixty_second_default_for_missing_prefs() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let status = runtime.pm_status_event(&runtime.test_context());

    let loop_interval_secs = match status {
        BackendEvent::PmStatus {
            available: true,
            loop_interval_secs,
            ..
        } => loop_interval_secs,
        other => panic!("expected PM status, got {other:?}"),
    };
    assert_eq!(loop_interval_secs, 60);
}

#[test]
fn app_runtime_close_project_does_not_publish_another_project_projection() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other).expect("create other");
    let tabs = vec![
        sample_project_tab("tab-1", "Repo", repo, ProjectKind::NonRepo, &[]),
        sample_project_tab("tab-2", "Other", other, ProjectKind::NonRepo, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));

    let events = runtime.close_project_tab_events("tab-1");

    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
        "closing A must not redirect A's client to B's projection"
    );
    assert!(events.iter().any(
        |event| matches!(&event.event, BackendEvent::HubState { hub }
        if hub.projects.len() == 1 && hub.projects[0].id == "tab-2")
    ));
}

#[test]
fn app_runtime_window_focus_refreshes_only_its_owner_project() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&other).expect("create other");
    let tabs = vec![
        sample_project_tab_with_window_at(
            "tab-1",
            "shell-1",
            repo.clone(),
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        ),
        sample_project_tab_with_window_at(
            "tab-2",
            "shell-2",
            other.clone(),
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        ),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));
    runtime.rebuild_window_lookup();
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));

    let events = runtime.focus_window_events(&combined_window_id("tab-2", "shell-2"), None);

    assert!(
        runtime
            .project_state(&runtime.project_context("tab-1").unwrap())
            .unwrap()
            .launch_wizard
            .is_some(),
        "focusing B must preserve A's wizard"
    );
    assert_eq!(events.len(), 1);
    assert!(matches!((&events[0].target, &events[0].event),
        (DispatchTarget::Project(key), BackendEvent::WindowCanvasState { workspace })
        if Some(key) == runtime.project_key_for_tab("tab-2") && workspace.active_tab_id.as_deref() == Some("tab-2")));
}

#[test]
fn app_runtime_open_project_path_emits_active_work_projection_for_new_tab() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tabs = vec![sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[],
    )];
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), tabs, Some("tab-1"));

    let other = temp.path().join("other-project");
    fs::create_dir_all(&other).expect("create other");

    assert!(runtime.open_project_path_events(other.clone()).is_empty());
    let prepared = take_project_navigation_completion(&recorded_events);
    let events = runtime.handle_project_navigation_prepared(prepared);

    assert!(
        events
            .iter()
            .any(|event| matches!(&event.event, BackendEvent::ActiveWorkProjection { .. })),
        "opening a new project must emit ActiveWorkProjection for the new active tab"
    );
}

#[test]
fn app_runtime_open_project_path_broadcasts_pm_status_exactly_once() {
    let _pm_gate = super::super::pm::test_gate::PmEnsureTestGuard::enable();
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    disable_pm_auto_start(&repo);
    let (mut runtime, recorded_events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;

    // SPEC #3170: the open is prepared off the tao thread; the PM snapshot
    // is aggregated once at the generation-checked commit.
    assert!(runtime.open_project_path_events(repo).is_empty());
    let events = commit_pending_project_navigation(&mut runtime, &queued_tasks, &recorded_events);

    assert_eq!(
        events
            .iter()
            .filter(|outbound| matches!(outbound.event, BackendEvent::PmStatus { .. }))
            .count(),
        1,
        "open project must emit one canonical PM settings snapshot"
    );
}

#[test]
fn app_runtime_runtime_status_uses_lightweight_events_for_non_structural_status() {
    // Scoped HOME: with FR-382 the projection broadcast also fires when home
    // work records exist, so this lightweight-path assertion must not depend
    // on whatever ~/.gwt state the developer machine has accumulated (#3022).
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");

    let events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("boom".to_string()),
    );

    assert_eq!(events.len(), 2);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "non-structural runtime status changes must not force a full workspace_state"
    );
    assert!(
        matches!(&events[0].target, DispatchTarget::Project(key) if key == &runtime.test_context().project_key)
    );
    assert!(matches!(
        &events[0].event,
        BackendEvent::WindowState { window_id: id, state }
            if id == &window_id && *state == WindowProcessStatus::Error
    ));
    assert!(
        matches!(&events[1].target, DispatchTarget::Project(key) if key == &runtime.test_context().project_key)
    );
    assert!(matches!(
        &events[1].event,
        BackendEvent::TerminalStatus { id, status, detail, .. }
            if id == &window_id
                && *status == WindowProcessStatus::Error
                && detail.as_deref() == Some("boom")
    ));
}

#[test]
fn app_runtime_open_launch_wizard_uses_cached_previous_profile_without_hydrating() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");

    let mut session = gwt_agent::Session::new(&repo, "feature/demo", gwt_agent::AgentId::Codex);
    session.model = Some("gpt-5.5".to_string());
    session.reasoning_level = Some("high".to_string());
    session.tool_version = Some("latest".to_string());
    session.session_mode = gwt_agent::SessionMode::Continue;
    session.skip_permissions = false;
    session.codex_fast_mode = true;
    session.save(&sessions_dir).expect("save session");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "feature/demo", None, None)
        .expect("open launch wizard");

    let view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .wizard
        .view();
    assert!(!view.is_hydrating);
    assert_eq!(view.selected_agent_id, "codex");
    assert_eq!(view.selected_model, "gpt-5.5");
    assert_eq!(view.selected_reasoning, "high");
    assert_eq!(view.selected_execution_mode, "continue");
    // L2 interprets the legacy permission preference as the fixed value.
    assert!(view.skip_permissions);
    // Launch choices remain hidden.
    assert!(!view.show_skip_permissions);
    assert!(!view.fast_mode);
}

#[test]
fn app_runtime_open_launch_wizard_does_not_probe_branch_worktree_for_docker_context() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "repo\n").expect("readme");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-qm", "init"]);
    run_git(&repo, &["branch", "feature/docker"]);

    let branch_worktree = temp.path().join("repo-feature-docker");
    let branch_worktree_arg = branch_worktree.to_string_lossy().to_string();
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            &branch_worktree_arg,
            "feature/docker",
        ],
    );
    fs::write(
        branch_worktree.join("docker-compose.yml"),
        "services:\n  app:\n    image: alpine:3.20\n",
    )
    .expect("compose");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "feature/docker", None, None)
        .expect("open launch wizard");

    let wizard = &runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard;
    assert!(wizard.context.worktree_path.is_none());
    assert!(same_worktree_path(&wizard.context.quick_start_root, &repo));
    let view = wizard.view();
    assert!(!view.runtime_context_resolved);
    assert!(!view.show_runtime_target);
    assert!(view.selected_docker_service.is_none());
}

#[test]
fn app_runtime_launch_wizard_continue_resolves_runtime_context_from_worktree() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "repo\n").expect("readme");
    fs::write(
        repo.join("docker-compose.yml"),
        "services:\n  app:\n    image: alpine:3.20\n",
    )
    .expect("compose");
    run_git(&repo, &["add", "README.md", "docker-compose.yml"]);
    run_git(&repo, &["commit", "-qm", "init"]);

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "develop", None, None)
        .expect("open launch wizard");
    assert!(
        !runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .as_ref()
            .expect("wizard")
            .wizard
            .view()
            .runtime_context_resolved
    );

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        None,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].event,
        BackendEvent::LaunchWizardState { wizard: Some(_) }
    ));
    let pending_view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(pending_view.runtime_resolution_pending);
    assert!(!pending_view.runtime_context_resolved);
    assert_eq!(pending_view.primary_action_label, "Preparing...");

    wait_for_recorded_event(
        "launch wizard runtime resolution",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
        },
    );
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    assert_eq!(resolved_events.len(), 1);

    let wizard = &runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard;
    assert!(wizard
        .context
        .worktree_path
        .as_ref()
        .is_some_and(|path| same_worktree_path(path, &repo)));
    let view = wizard.view();
    assert!(!view.runtime_resolution_pending);
    assert!(view.runtime_context_resolved);
    assert!(view.show_runtime_target);
    assert_eq!(view.selected_runtime_target, "docker");
    assert_eq!(view.selected_docker_service.as_deref(), Some("app"));
}

#[test]
fn app_runtime_launch_wizard_continue_does_not_materialize_missing_worktree() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let _origin = init_git_clone_with_origin(&repo);
    fs::write(
        repo.join("docker-compose.yml"),
        "services:\n  app:\n    image: alpine:3.20\n",
    )
    .expect("compose");
    run_git(&repo, &["add", "docker-compose.yml"]);
    run_git(&repo, &["commit", "-qm", "add compose"]);
    run_git(&repo, &["push", "origin", "develop"]);

    let branch_name = "work/runtime-deferral";
    let expected_worktree = gwt_git::worktree::sibling_worktree_path(&repo, branch_name);
    assert!(
        !expected_worktree.exists(),
        "fixture branch worktree should start absent"
    );

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, branch_name, None, None)
        .expect("open launch wizard");

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        None,
    );
    assert_eq!(events.len(), 1);
    let pending_view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(pending_view.runtime_resolution_pending);
    assert_eq!(
        pending_view.runtime_resolution_message.as_deref(),
        Some("Preparing runtime context...")
    );

    wait_for_recorded_event(
        "launch wizard runtime deferral",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
        },
    );
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    assert_eq!(resolved_events.len(), 1);

    let wizard = &runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard;
    assert!(
        wizard.context.worktree_path.is_none(),
        "runtime confirmation should not resolve a newly-created target worktree"
    );
    assert!(
        !expected_worktree.exists(),
        "Runtime confirmation must not create {expected_worktree:?}"
    );
    let view = wizard.view();
    assert!(!view.runtime_resolution_pending);
    assert!(view.runtime_context_resolved);
    assert!(view.show_runtime_target);
    assert_eq!(view.selected_runtime_target, "docker");
    assert_eq!(view.selected_docker_service.as_deref(), Some("app"));
}

#[test]
fn app_runtime_start_work_parent_root_uses_develop_checkout_for_docker_context() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let workspace_home = temp.path().join("workspace");
    let (bare_repo, develop_worktree) =
        init_managed_workspace_with_develop_worktree(&workspace_home);
    fs::write(
        develop_worktree.join("docker-compose.yml"),
        "services:\n  gwt:\n    image: alpine:3.20\n",
    )
    .expect("compose");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        workspace_home.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_start_work_for_project("tab-1", &workspace_home)
        .expect("open start work");
    let branch_name = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .context
        .normalized_branch_name
        .clone();
    let expected_worktree = gwt_git::worktree::sibling_worktree_path(&bare_repo, &branch_name);
    assert!(
        !expected_worktree.exists(),
        "fixture branch worktree should start absent"
    );

    resolve_launch_wizard_runtime_confirmation(
        &mut runtime,
        &recorded_events,
        "start work parent root docker context",
    );

    let wizard = &runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard;
    assert!(
        wizard.context.worktree_path.is_none(),
        "Runtime confirmation must not resolve a missing target worktree"
    );
    assert!(
        !expected_worktree.exists(),
        "Runtime confirmation must not create {expected_worktree:?}"
    );
    let view = wizard.view();
    assert!(!view.runtime_resolution_pending);
    assert!(view.runtime_context_resolved);
    assert!(view.show_runtime_target);
    assert_eq!(view.selected_runtime_target, "docker");
    assert_eq!(view.selected_docker_service.as_deref(), Some("gwt"));
}

#[test]
fn app_runtime_start_work_parent_root_preserves_saved_host_while_showing_runtime_target() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let workspace_home = temp.path().join("workspace");
    let (_bare_repo, develop_worktree) =
        init_managed_workspace_with_develop_worktree(&workspace_home);
    fs::write(
        develop_worktree.join("docker-compose.yml"),
        "services:\n  gwt:\n    image: alpine:3.20\n",
    )
    .expect("compose");

    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut session =
        gwt_agent::Session::new(&develop_worktree, "develop", gwt_agent::AgentId::Codex);
    session.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
    session.docker_service = None;
    session.save(&sessions_dir).expect("save session");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        workspace_home.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_start_work_for_project("tab-1", &workspace_home)
        .expect("open start work");
    resolve_launch_wizard_runtime_confirmation(
        &mut runtime,
        &recorded_events,
        "start work parent root saved host",
    );

    let view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(view.runtime_context_resolved);
    assert!(view.show_runtime_target);
    assert_eq!(view.selected_runtime_target, "host");
    assert!(view.selected_docker_service.is_none());
    assert_eq!(
        view.docker_service_options
            .iter()
            .map(|option| option.value.as_str())
            .collect::<Vec<_>>(),
        vec!["gwt"]
    );
}

#[test]
fn app_runtime_launch_wizard_continue_falls_back_to_host_without_resolved_docker_context() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    run_git(&repo, &["init", "-q", "-b", "develop"]);
    run_git(&repo, &["config", "user.name", "Codex"]);
    run_git(&repo, &["config", "user.email", "codex@example.com"]);
    fs::write(repo.join("README.md"), "repo\n").expect("readme");
    run_git(&repo, &["add", "README.md"]);
    run_git(&repo, &["commit", "-qm", "init"]);

    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut session = gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex);
    session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
    session.docker_service = Some("app".to_string());
    session.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
    session.save(&sessions_dir).expect("save session");

    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .open_launch_wizard_for_branch("tab-1", &repo, "develop", None, None)
        .expect("open launch wizard");
    let phase_one = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(!phase_one.runtime_context_resolved);
    assert_eq!(phase_one.selected_runtime_target, "host");
    assert!(!phase_one.show_runtime_target);

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        None,
    );
    assert_eq!(events.len(), 1);
    let pending_view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(pending_view.runtime_resolution_pending);
    assert!(!pending_view.runtime_context_resolved);

    wait_for_recorded_event("launch wizard host fallback", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchWizardRuntimeResolved { .. }
            )
        })
    });
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    assert_eq!(resolved_events.len(), 1);

    let view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("wizard")
        .wizard
        .view();
    assert!(!view.runtime_resolution_pending);
    assert!(view.runtime_context_resolved);
    assert_eq!(view.selected_runtime_target, "host");
    assert!(!view.show_runtime_target);
    assert!(view.selected_docker_service.is_none());
}

#[test]
fn app_runtime_workspace_add_agent_opens_branch_launch_without_branches_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        "tab-1::agent-1".to_string(),
        ActiveAgentSession {
            window_id: "tab-1::agent-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.join("../repo-work-20260504-1234"),
            agent_project_root: repo
                .join("../repo-work-20260504-1234")
                .display()
                .to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenActiveWorkLaunchWizard {
            branch_name: "work/20260504-1234".to_string(),
            linked_issue_number: None,
        },
    );

    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::LaunchWizardState { wizard: Some(_) })
    ));
    let view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("active work launch wizard")
        .wizard
        .view();
    assert_eq!(view.mode, gwt::LaunchWizardMode::Branch);
    assert_eq!(view.branch_name, "work/20260504-1234");
    assert!(view.show_start_methods);
    assert!(!view.show_branch_controls);
    assert_eq!(view.live_sessions.len(), 1);
    assert_eq!(view.live_sessions[0].name, "Codex");

    let _ = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        gwt::LaunchWizardAction::UseStartMethod {
            method: gwt::LaunchWizardStartMethodKind::ConfigureAndStart,
        },
        None,
    );
    let configured_view = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("configured active work launch wizard")
        .wizard
        .view();
    assert!(!configured_view.show_start_methods);
    assert!(configured_view.show_branch_controls);
}

#[test]
fn app_runtime_live_sessions_report_composed_idle_runtime_status() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: PathBuf::from("E:/gwt/test-repo"),
            agent_project_root: "E:/gwt/test-repo".to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    runtime.handle_runtime_hook_event(runtime_hook_state("Idle", "session-1"));
    let sessions = runtime.live_sessions_for_branch("tab-1", "work/20260504-1234");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].runtime_status, WindowProcessStatus::Idle);
}

#[test]
fn app_runtime_live_sessions_report_idle_after_launch_before_first_hook() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: PathBuf::from("E:/gwt/test-repo"),
            agent_project_root: "E:/gwt/test-repo".to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let sessions = runtime.live_sessions_for_branch("tab-1", "work/20260504-1234");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].runtime_status, WindowProcessStatus::Idle);

    runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Idle",
        "SessionStart",
        "session-1",
    ));
    let sessions = runtime.live_sessions_for_branch("tab-1", "work/20260504-1234");

    assert_eq!(sessions[0].runtime_status, WindowProcessStatus::Idle);
}

#[test]
fn app_runtime_workspace_state_reports_idle_for_launched_agent_without_hook_state() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: PathBuf::from("E:/gwt/test-repo"),
            agent_project_root: "E:/gwt/test-repo".to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let view = runtime.app_state_view();
    let tab = view.tabs.iter().find(|tab| tab.id == "tab-1").unwrap();
    let window = tab
        .workspace
        .windows
        .iter()
        .find(|window| window.id == "tab-1::agent-1")
        .unwrap();

    assert_eq!(window.status, WindowProcessStatus::Idle);
}

#[test]
fn app_runtime_workspace_state_normalizes_pre_lifecycle_agent_windows() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime.app_state_view();
    let tab = view.tabs.iter().find(|tab| tab.id == "tab-1").unwrap();
    let window = tab
        .workspace
        .windows
        .iter()
        .find(|window| window.id == "tab-1::agent-1")
        .unwrap();

    assert_eq!(window.status, WindowProcessStatus::Starting);
    assert_eq!(tab.running_agent_count, 0);
    assert!(tab.running_agents.is_empty());
}

#[test]
fn app_runtime_workspace_state_normalizes_agent_kanban_board_ids() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut tab = sample_project_tab(
        "tab-1",
        "Repo",
        PathBuf::from("E:/gwt/test-repo"),
        ProjectKind::Git,
        &[WindowPreset::AgentKanban, WindowPreset::Agent],
    );
    assert!(tab.workspace.place_agent_window_in_kanban(
        "agent-1",
        "agent-kanban-1",
        gwt::AgentKanbanLane::Active,
        None,
    ));
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let view = runtime.app_state_view();
    let tab = view.tabs.iter().find(|tab| tab.id == "tab-1").unwrap();
    let agent = tab
        .workspace
        .windows
        .iter()
        .find(|window| window.id == "tab-1::agent-1")
        .expect("agent window");

    assert_eq!(
        agent.placement,
        WindowPlacement::AgentKanban {
            board_id: "tab-1::agent-kanban-1".to_string(),
            lane_id: gwt::AgentKanbanLane::Active,
            order: 0,
            collapsed: false,
        },
        "workspace wire state must use the same combined IDs for windows and Kanban board references"
    );
}

#[test]
fn app_runtime_window_list_normalizes_pre_lifecycle_agent_windows() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let BackendEvent::WindowList { windows } = runtime.list_windows_event() else {
        panic!("expected window list");
    };
    let window = windows
        .iter()
        .find(|window| window.id == "tab-1::agent-1")
        .unwrap();

    assert_eq!(window.status, WindowProcessStatus::Starting);
}

#[test]
fn app_runtime_window_list_enumerates_all_project_tabs() {
    // SPEC-3038 (2026-06-20): the Command Rail Windows popover lists windows
    // from every project tab, not just the active one, so the list matches the
    // cross-tab open-window badge.
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab_a = sample_project_tab_with_window(
        "tab-a",
        "win-a1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let tab_b = sample_project_tab_with_window(
        "tab-b",
        "win-b1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let runtime = sample_runtime(temp.path(), vec![tab_a, tab_b], Some("tab-a"));

    let BackendEvent::WindowList { windows } = runtime.list_windows_event() else {
        panic!("expected window list");
    };
    let ids: Vec<String> = windows.iter().map(|window| window.id.clone()).collect();
    assert!(
        ids.contains(&"tab-a::win-a1".to_string()),
        "active tab window must be listed: {ids:?}"
    );
    assert!(
        ids.contains(&"tab-b::win-b1".to_string()),
        "non-active tab window must also be listed: {ids:?}"
    );
    assert_eq!(windows.len(), 2, "all project-tab windows must be listed");
}

#[test]
fn app_runtime_open_launch_wizard_failure_surfaces_launch_wizard_open_error() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().join("repo"),
        ProjectKind::Git,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenLaunchWizard {
            id: "missing-window".to_string(),
            branch_name: "main".to_string(),
            linked_issue_number: None,
        },
    );

    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .is_none());
    assert!(matches!(
        events.first().map(|event| &event.target),
        Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
    ));
    assert!(matches!(
        events.first().map(|event| &event.event),
        Some(BackendEvent::LaunchWizardOpenError { title, message })
            if title == "Launch Agent" && message == "Window not found"
    ));
}

#[test]
fn app_runtime_open_launch_wizard_accepts_work_window_preset() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "work-1",
        repo,
        WindowPreset::Work,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "work-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::OpenLaunchWizard {
            id: window_id,
            branch_name: "main".to_string(),
            linked_issue_number: None,
        },
    );

    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .is_some(),
        "Launch wizard should open from a Work window preset"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(&event.event, BackendEvent::LaunchWizardOpenError { .. })),
        "Work preset must not be rejected as 'not a Work surface'"
    );
}

#[test]
fn app_runtime_resume_branch_latest_agent_accepts_work_window_preset() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "work-1",
        repo,
        WindowPreset::Work,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "work-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeBranchLatestAgent {
            id: window_id,
            branch_name: "main".to_string(),
            bounds: canvas_bounds(),
        },
    );

    let has_surface_error = events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::BranchError { message, .. }
                if message.contains("is not a Work surface")
        )
    });
    assert!(
        !has_surface_error,
        "Work preset must not be rejected as 'not a Work surface'"
    );
}

#[test]
fn app_runtime_resume_workspace_failure_surfaces_launch_wizard_open_error() {
    // SPEC-2359 / Issue #2757: Resume クリックで `resume_workspace_events`
    // が早期 return / Start Work fallback 失敗を起こした場合、frontend で
    // 可視な `LaunchWizardOpenError` を return しなければならない。
    // 旧経路は `ProjectOpenError` を broadcast していたが、project 開放中は
    // `renderProjectPicker` が hidden なので silent failure になっていた。
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Current,
            journal_id: None,
        },
    );

    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .is_none());
    assert!(
            matches!(
                events.first().map(|event| &event.target),
                Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
            ),
            "Resume failure must be replied to the originating client, not broadcast as ProjectOpenError"
        );
    assert!(
            matches!(
                events.first().map(|event| &event.event),
                Some(BackendEvent::LaunchWizardOpenError { title, message })
                    if title == "Resume Work" && !message.is_empty()
            ),
            "Resume failure must surface as LaunchWizardOpenError so Work Overview can render a visible overlay"
        );
}

#[test]
fn app_runtime_resume_workspace_without_active_tab_returns_launch_wizard_open_error() {
    // Same contract for the `Open a project before resuming work` early return.
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ResumeWorkspace {
            source: gwt::WorkspaceResumeSource::Current,
            journal_id: None,
        },
    );

    assert!(
        events.is_empty(),
        "a request without an owned project/window is rejected before mutation"
    );
}

#[test]
fn app_runtime_custom_agent_cache_refresh_rebroadcasts_open_wizard_state() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::NonRepo, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));

    let events = runtime.custom_agent_reply_with_cache_refresh(
        "client-1".to_string(),
        BackendEvent::CustomAgentDeleted {
            agent_id: "custom-agent".to_string(),
        },
    );

    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0].event,
        BackendEvent::CustomAgentDeleted { .. }
    ));
    assert!(matches!(
        events[1].event,
        BackendEvent::LaunchWizardState { wizard: Some(_) }
    ));
}

#[test]
fn app_runtime_supported_agents_lists_catalog_and_distinguishes_missing_versions() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, recorded_events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
    let options = vec![
        gwt::AgentOption {
            id: "claude".into(),
            name: "Claude Code".into(),
            available: true,
            installed_version: Some(" 2.1.0 ".into()),
            custom_agent: None,
        },
        gwt::AgentOption {
            id: "codex".into(),
            name: "Codex".into(),
            available: true,
            installed_version: None,
            custom_agent: None,
        },
    ];
    runtime.launch_wizard_cache =
        LaunchWizardMemoryCache::load_with_agent_options(&runtime.sessions_dir, options);
    let request: FrontendEvent = serde_json::from_str(r#"{"kind":"list_supported_agents"}"#)
        .expect("L3 Settings must support the read-only list request");
    let immediate = runtime.handle_frontend_event("settings-client".into(), request);
    assert!(
        immediate.is_empty(),
        "detection cache reads run off the GUI loop"
    );
    wait_for_recorded_event("supported agent list", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(event, UserEvent::Dispatch(outbound) if outbound.iter().any(|reply|
                serde_json::to_value(&reply.event).unwrap()["kind"] == "supported_agent_list"))
        })
    });
    let events = recorded_events.lock().expect("events lock");
    let payload = events
        .iter()
        .filter_map(|event| match event {
            UserEvent::Dispatch(outbound) => Some(outbound),
            _ => None,
        })
        .flatten()
        .map(|reply| serde_json::to_value(&reply.event).unwrap())
        .find(|value| value["kind"] == "supported_agent_list")
        .expect("supported agent reply");
    let rows = payload["agents"].as_array().expect("agent rows");
    assert_eq!(rows.len(), gwt_agent::builtin_agent_descriptors().len());
    for (row, descriptor) in rows.iter().zip(gwt_agent::builtin_agent_descriptors()) {
        assert_eq!(row["id"], descriptor.command);
        assert_eq!(row["name"], descriptor.display_name);
    }
    assert_eq!(rows[0]["installed"], true);
    assert_eq!(rows[0]["installed_version"], "2.1.0");
    assert_eq!(rows[1]["installed"], true);
    assert!(rows[1]["installed_version"].is_null());
    assert_eq!(rows[2]["installed"], false);
    assert!(rows[2]["installed_version"].is_null());
}

#[test]
fn issue_monitor_error_notification_keeps_project_in_ledger() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("repo");
    let tab = sample_project_tab("tab-1", "Repo", root.clone(), ProjectKind::NonRepo, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let event = BackendEvent::IssueMonitorLaunchFailed {
        issue_number: 4735,
        message: "project notification failure".into(),
    };
    let outbound = runtime
        .issue_monitor_project_notification(Some(&root), event)
        .unwrap();
    prepare_outbound_event(&outbound);
    let rows = gwt_core::error_ledger::list_since(None).unwrap();
    let row = rows
        .iter()
        .find(|row| row.message == "project notification failure")
        .unwrap();
    assert_eq!(row.target.project_root.as_deref(), root.to_str());
}

#[test]
fn app_runtime_launch_wizard_submit_failure_emits_structured_error_log() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::NonRepo,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_no_agent_launch_wizard_session("tab-1", &repo));

    let events = capture_tracing_events(|| {
        let _ = runtime.handle_launch_wizard_action(
            &runtime.test_context(),
            LaunchWizardAction::Submit,
            Some(canvas_bounds()),
        );
    });

    let rows = gwt_core::error_ledger::list_since(None).unwrap();
    let row = rows
        .iter()
        .find(|row| row.message == "No supported agent CLI was detected")
        .unwrap();
    assert_eq!(
        row.target.project_root.as_deref(),
        Some(repo.to_str().unwrap())
    );

    let event = events
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("wizard_submit")
        })
        .expect("launch wizard submit failure log");
    assert_eq!(
        event.fields.get("wizard_id").map(String::as_str),
        Some("wizard-unavailable-agent")
    );
    assert_eq!(
        event.fields.get("tab_id").map(String::as_str),
        Some("tab-1")
    );
    assert_eq!(
        event.fields.get("selected_agent_id").map(String::as_str),
        Some("")
    );
    assert_eq!(
        event.fields.get("requested_agent_id").map(String::as_str),
        Some("none")
    );
    assert_eq!(
        event
            .fields
            .get("selected_launch_target")
            .map(String::as_str),
        Some("agent")
    );
    assert_eq!(
        event.fields.get("error").map(String::as_str),
        Some("No supported agent CLI was detected")
    );
}

#[test]
fn app_runtime_launch_submit_returns_materialization_pending_before_dispatch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_start_work_confirm_session("tab-1", &repo));

    let events = runtime.handle_launch_wizard_action_for_client(
        &runtime.test_context(),
        Some("client-1"),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );

    assert_eq!(events.len(), 1);
    let BackendEvent::LaunchWizardState {
        wizard: Some(wizard),
    } = &events[0].event
    else {
        panic!("expected pending wizard state before launch dispatch: {events:?}");
    };
    assert!(wizard.launch_materialization_pending);
    assert_eq!(
        wizard.launch_materialization_message.as_deref(),
        Some("Preparing worktree...")
    );
    assert_eq!(wizard.primary_action_label, "Launching...");
    assert!(!wizard.primary_action_enabled);

    let recorded = recorded_events.lock().expect("event log");
    assert_eq!(
        recorded
            .iter()
            .filter(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
                )
            })
            .count(),
        1,
        "actual launch must be deferred to exactly one internal event",
    );
    drop(recorded);

    let duplicate_events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    assert!(
        duplicate_events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "duplicate submit should keep returning the pending wizard state",
    );
    let recorded = recorded_events.lock().expect("event log");
    assert_eq!(
        recorded
            .iter()
            .filter(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
                )
            })
            .count(),
        1,
        "duplicate submit while pending must not enqueue a second launch",
    );
}

#[test]
fn app_runtime_launch_wizard_set_agent_failure_logs_requested_agent() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::NonRepo,
        &[WindowPreset::Branches],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_no_agent_launch_wizard_session("tab-1", &repo));

    let events = capture_tracing_events(|| {
        let _ = runtime.handle_launch_wizard_action(
            &runtime.test_context(),
            LaunchWizardAction::SetAgent {
                agent_id: "codex".to_string(),
            },
            Some(canvas_bounds()),
        );
    });

    let event = events
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("agent_select")
        })
        .expect("set agent failure log");
    assert_eq!(
        event.fields.get("requested_agent_id").map(String::as_str),
        Some("codex")
    );
    assert_eq!(
        event
            .fields
            .get("selected_runtime_target")
            .map(String::as_str),
        Some("host")
    );
    assert!(!event.fields.contains_key("selected_tool_version"));
}
