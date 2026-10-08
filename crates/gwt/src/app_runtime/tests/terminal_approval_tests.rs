use super::*;

#[test]
fn app_runtime_row_cleanup_candidate_exposes_merged_workspace_without_live_agent() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-merged-cleanup-row",
            chrono::Utc::now(),
        );
        event.title = Some("Merged cleanup row".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/20260615-merged-cleanup".to_string()),
                worktree_path: Some(repo.join("work/20260615-merged-cleanup")),
                pr_number: Some(3100),
                pr_url: None,
                pr_state: Some("MERGED".to_string()),
            },
        );
        event
    })
    .expect("record work");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .work_dirty_branches
        .insert(repo.clone(), HashSet::new());
    runtime
        .work_live_process_branches
        .insert(repo.clone(), HashSet::new());

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.branch.as_deref() == Some("work/20260615-merged-cleanup"))
        .expect("merged Workspace row");
    let candidate = row
        .cleanup_candidate
        .as_ref()
        .expect("eligible merged row should expose cleanup candidate");

    assert_eq!(candidate.branch, "work/20260615-merged-cleanup");
    assert_eq!(candidate.reason, "pr_merged");
    assert!(!candidate.default_delete_remote);
}

#[test]
fn app_runtime_row_cleanup_candidate_hides_grouped_live_agent_branch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-merged-grouped-row",
            chrono::Utc::now(),
        );
        event.title = Some("Merged grouped row".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/20260615-grouped-cleanup".to_string()),
                worktree_path: Some(repo.join("work/20260615-grouped-cleanup")),
                pr_number: Some(3101),
                pr_url: None,
                pr_state: Some("MERGED".to_string()),
            },
        );
        event
    })
    .expect("record work");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "codex-1",
        repo.clone(),
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .work_dirty_branches
        .insert(repo.clone(), HashSet::new());
    runtime
        .work_live_process_branches
        .insert(repo.clone(), HashSet::new());
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id,
            session_id: "session-grouped-cleanup".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260615-grouped-cleanup".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: repo.join("work/20260615-grouped-cleanup"),
            agent_project_root: repo
                .join("work/20260615-grouped-cleanup")
                .display()
                .to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.branch.as_deref() == Some("work/20260615-grouped-cleanup"))
        .expect("grouped Workspace row");

    assert!(row.merged_into_base, "merged badge remains visible");
    assert_eq!(
        row.cleanup_candidate, None,
        "grouped row with live Agent on the same branch must not be cleanable"
    );
}

#[cfg(unix)]
#[test]
fn app_runtime_row_cleanup_candidate_hides_workspace_with_live_cwd_process() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    let worktree = repo.join("work/20260616-0203");
    fs::create_dir_all(&worktree).expect("create worktree");
    gwt_core::workspace_projection::record_workspace_work_event(&repo, {
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            "work-live-cwd-cleanup-row",
            chrono::Utc::now(),
        );
        event.title = Some("Merged live cwd row".to_string());
        event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/20260616-0203".to_string()),
                worktree_path: Some(worktree.clone()),
                pr_number: Some(3108),
                pr_url: None,
                pr_state: Some("MERGED".to_string()),
            },
        );
        event
    })
    .expect("record work");
    let _child = KillOnDrop(
        gwt_core::process::hidden_command("sh")
            .arg("-c")
            .arg("sleep 30")
            .current_dir(&worktree)
            .spawn()
            .expect("spawn cwd process"),
    );
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .work_dirty_branches
        .insert(repo.clone(), HashSet::new());
    runtime.work_live_process_branches.insert(
        repo.clone(),
        HashSet::from(["work/20260616-0203".to_string()]),
    );

    let view = runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .expect("projection view");
    let row = view
        .active_works
        .iter()
        .find(|work| work.branch.as_deref() == Some("work/20260616-0203"))
        .expect("merged Workspace row");

    assert!(row.merged_into_base, "merged badge remains visible");
    assert_eq!(
        row.cleanup_candidate, None,
        "Workspace whose worktree is still an agent process cwd must not be cleanable"
    );
}

#[test]
fn app_runtime_stopped_agent_cleans_saved_projection_and_broadcasts_active_work_idle() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "codex-1",
        repo.clone(),
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    let session = ActiveAgentSession {
        window_id: window_id.clone(),
        session_id: "session-1".to_string(),
        agent_id: "codex".to_string(),
        branch_name: "work/20260506-1652".to_string(),
        display_name: "Codex".to_string(),
        worktree_path: temp.path().join("work/20260506-1652"),
        agent_project_root: temp.path().join("work/20260506-1652").display().to_string(),
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        tab_id: "tab-1".to_string(),
    };
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session.clone());
    save_start_work_workspace_projection(
        &repo,
        &session,
        "origin/main",
        None,
        None,
        None,
        Some(&std::collections::HashSet::new()),
    )
    .expect("save projection");

    let events = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Stopped,
        Some("Process exited".to_string()),
        true,
    );

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    assert!(projection.agents.is_empty());
    assert_eq!(
        projection.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Idle
    );
    assert!(events
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::ActiveWorkProjection { .. })));
    let active_work = wait_for_active_work_projection(&mut runtime);
    assert_eq!(active_work.active_agents, 0);
    assert!(active_work.agents.is_empty());
    assert_eq!(active_work.status_category, "idle");
}

#[test]
fn app_runtime_status_thread_reports_process_exit_without_reader_eof() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let captured_events = match &runtime.proxy {
        AppEventProxy::Stub(events) => events.clone(),
        AppEventProxy::Real(_) | AppEventProxy::Project { .. } => {
            panic!("sample runtime must use stub proxy")
        }
    };
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/D".to_string(),
                "/S".to_string(),
                "/C".to_string(),
                "exit /B 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };
    let pane = Arc::new(Mutex::new(
        Pane::new(
            window_id.clone(),
            command,
            args,
            80,
            24,
            HashMap::new(),
            test_pane_cwd(),
        )
        .expect("pane"),
    ));
    if cfg!(windows) {
        // Windows ConPTY may wait for this CPR response before exposing exit state.
        if let Ok(pane) = pane.lock() {
            let _ = pane.pty().write_input(b"\x1b[1;1R");
        }
    }
    let incarnation = super::super::next_window_runtime_incarnation();
    let status_thread = runtime.spawn_status_thread(window_id.clone(), incarnation, pane.clone());

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed_status = None;
    while Instant::now() < deadline {
        if let Ok(events) = captured_events.lock() {
            observed_status = events.iter().find_map(|event| match event {
                UserEvent::RuntimeStatus {
                    id,
                    incarnation: event_incarnation,
                    status,
                    detail,
                    ..
                } if id == &window_id
                    && *event_incarnation == incarnation
                    && *status == WindowProcessStatus::Stopped =>
                {
                    Some(detail.clone())
                }
                _ => None,
            });
        }
        if observed_status.is_some() {
            break;
        }
        // test-hygiene: allow-short-duration bounded polling for OS process-status event; the independent deadline bounds observation
        thread::sleep(Duration::from_millis(25));
    }
    if observed_status.is_none() {
        if let Ok(pane) = pane.lock() {
            let _ = pane.kill();
        }
    }
    let _ = status_thread.join();

    assert_eq!(observed_status.flatten().as_deref(), Some("Process exited"));
}

#[test]
fn app_runtime_runtime_hook_stopped_keeps_active_agent_window_for_host_close() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Stopped", "session-1"));

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Stopped,
            ..
        }
    )));
    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.tabs[0].workspace.window("codex-1").is_some());
}

/// Issue #3783: RuntimeHook Stop has no process-local lifecycle generation.
/// A predecessor Stop can therefore arrive after a same-address, same-Session
/// successor has registered, and must not destructively close that successor.
#[test]
fn late_runtime_hook_stop_preserves_same_session_successor_generation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.register_window("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let pty = runtime
        .runtimes
        .get(&window_id)
        .expect("live runtime")
        .pty
        .clone();
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let predecessor_generation = runtime
        .window_lifecycle_generations
        .lock()
        .expect("window lifecycle generations")
        .get(&window_id)
        .copied()
        .expect("predecessor generation");
    runtime.register_window("tab-1", "codex-1");
    let successor_generation = runtime
        .window_lifecycle_generations
        .lock()
        .expect("window lifecycle generations")
        .get(&window_id)
        .copied()
        .expect("successor generation");
    assert_ne!(predecessor_generation, successor_generation);

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Stopped", "session-1"));

    assert!(!events.is_empty(), "the stopped state is still surfaced");
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    // A Stop legitimately queues one blocking task now: Issue #3777 AC-3 moves
    // the Active Work projection rebuild off the Tao loop, so reaching a
    // terminal state schedules that background refresh. Counting queued tasks
    // therefore no longer distinguishes "scheduled a projection rebuild" from
    // "scheduled a window teardown". Run whatever was queued and re-assert the
    // window instead: a destructive finalizer would tear it down here, so this
    // tests the contract in the assertion's name directly rather than by proxy.
    let queued = std::mem::take(
        &mut *finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    for task in queued {
        task();
    }
    assert!(
        runtime.window_lookup.contains_key(&window_id),
        "a generationless Stop must not queue a destructive finalizer"
    );
    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert!(
        pty.try_wait().expect("probe child").is_none(),
        "hook dispatch must not kill the PTY inline"
    );
    assert_eq!(
        runtime
            .window_lifecycle_generations
            .lock()
            .expect("window lifecycle generations")
            .get(&window_id)
            .copied(),
        Some(successor_generation),
        "the late predecessor event must not settle the successor generation"
    );
    runtime.stop_window_runtime(&window_id);
}

#[test]
fn app_runtime_workspace_projection_surface_helper_groups_state_and_active_work_events() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let mut events = Vec::new();
    runtime.push_workspace_and_active_work_projection_broadcasts(&mut events);

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].event,
        BackendEvent::WindowCanvasState { .. }
    ));
    let projection = wait_for_active_work_projection(&mut runtime);
    assert_eq!(projection.active_agents, 1);
}

#[test]
fn app_runtime_runtime_hook_stopped_without_active_session_keeps_window_open() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Stopped", "session-1"));

    assert!(events.is_empty());
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.tabs[0].workspace.window("codex-1").is_some());
}

#[test]
fn app_runtime_runtime_state_hooks_use_status_events_without_browser_hook_event() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));

    assert!(
            !events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::RuntimeHookEvent { .. })),
            "runtime_state hooks are browser-internal noise; status events carry the visible chrome state"
        );
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowState { .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })));
}

#[test]
fn app_runtime_non_session_start_hook_keeps_agent_session_id_fallback() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let mut event = runtime_hook_state("Waiting", "foreign-gwt-session");
    event.agent_session_id = Some("session-1".to_string());

    let events = runtime.handle_runtime_hook_event(event);

    assert!(events.iter().any(|outbound| matches!(
        &outbound.event,
        BackendEvent::WindowState { window_id: id, .. } if id == &window_id
    )));
}

#[test]
fn app_runtime_coordination_hooks_still_emit_browser_hook_event() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "board-1",
        WindowPreset::Board,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_runtime_hook_event(runtime_hook_coordination_event("session-1"));

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].event,
        BackendEvent::RuntimeHookEvent { .. }
    ));
}

#[test]
fn app_runtime_browser_hook_event_strips_continue_work_readiness_nonce() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "board-1",
        WindowPreset::Board,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut event = runtime_hook_coordination_event("session-1");
    event.continuation_readiness_nonce = Some("private-ready-nonce".to_string());

    let events = runtime.handle_runtime_hook_event(event);

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::RuntimeHookEvent { event }
            if event.continuation_readiness_nonce.is_none()
    )));
    let encoded =
        serde_json::to_string(&events.iter().map(|event| &event.event).collect::<Vec<_>>())
            .expect("serialize browser events");
    assert!(!encoded.contains("private-ready-nonce"));
}

#[test]
fn app_runtime_runtime_state_bursts_emit_no_browser_hook_events() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let browser_hook_events = (0..1_000)
        .flat_map(|_| runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1")))
        .filter(|event| matches!(event.event, BackendEvent::RuntimeHookEvent { .. }))
        .count();

    assert_eq!(browser_hook_events, 0);
}

#[test]
fn app_runtime_duplicate_runtime_state_hooks_emit_status_events_only_once() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let events = (0..1_000)
        .flat_map(|_| runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1")))
        .collect::<Vec<_>>();

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, BackendEvent::RuntimeHookEvent { .. }))
            .count(),
        0
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, BackendEvent::WindowState { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, BackendEvent::TerminalStatus { .. }))
            .count(),
        1
    );
}

#[test]
fn app_runtime_approval_wait_overlay_enters_once_and_restores_hook_state() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Running);

    let entered = runtime.handle_runtime_approval_wait_state(&window_id, true);
    let duplicate = runtime.handle_runtime_approval_wait_state(&window_id, true);
    let cleared = runtime.handle_runtime_approval_wait_state(&window_id, false);

    assert!(entered.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Waiting,
            ..
        }
    )));
    assert!(
        duplicate.is_empty(),
        "a prompt redraw must not re-emit waiting"
    );
    assert!(cleared.iter().any(|event| matches!(
        event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Running,
            ..
        }
    )));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
}

#[test]
fn app_runtime_remote_approval_overlay_enters_clears_and_reenters() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Running);

    let entered = runtime.handle_daemon_runtime_approval_wait_state(&window_id, true);
    let raw_output =
        runtime.handle_daemon_runtime_output(window_id.clone(), b"partial".to_vec(), None);
    let duplicate = runtime.handle_daemon_runtime_approval_wait_state(&window_id, true);
    let cleared = runtime.handle_daemon_runtime_approval_wait_state(&window_id, false);
    let reentered = runtime.handle_daemon_runtime_approval_wait_state(&window_id, true);

    assert!(entered.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Waiting,
            ..
        }
    )));
    assert!(duplicate.is_empty());
    assert!(raw_output.iter().all(|event| !matches!(
        event.event,
        BackendEvent::WindowState { .. } | BackendEvent::TerminalStatus { .. }
    )));
    assert!(cleared.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(reentered.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Waiting,
            ..
        }
    )));
}

#[test]
fn app_runtime_approval_overlay_dedupes_when_hook_state_is_already_waiting() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Waiting);
    runtime.recompute_window_state(&window_id);

    let entered = runtime.handle_runtime_approval_wait_state(&window_id, true);
    let cleared = runtime.handle_runtime_approval_wait_state(&window_id, false);

    assert!(entered.is_empty());
    assert!(cleared.is_empty());
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Waiting)
    );
}

#[test]
fn app_runtime_persistence_excludes_only_the_approval_overlay_state() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Running);
    runtime.handle_runtime_approval_wait_state(&window_id, true);

    let live = runtime.tabs[0]
        .workspace
        .window("agent-1")
        .expect("live window")
        .status;
    let persisted = runtime.persistable_workspace_state(&runtime.tabs[0]);
    let saved = persisted
        .windows
        .iter()
        .find(|window| window.id == "agent-1")
        .expect("persisted window");
    let serialized = serde_json::to_string(&persisted).expect("serialize persisted workspace");

    assert_eq!(live, WindowProcessStatus::Waiting);
    assert_eq!(saved.status, WindowProcessStatus::Running);
    assert!(!serialized.contains("fingerprint"));
    assert!(!serialized.contains("approval"));

    let restored = WindowCanvasState::from_persisted(persisted);
    assert_eq!(
        restored.window("agent-1").expect("restored window").status,
        WindowProcessStatus::Running
    );
}

#[test]
fn app_runtime_persistence_keeps_hook_native_waiting_without_approval_overlay() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Waiting);
    runtime.recompute_window_state(&window_id);

    let persisted = runtime.persistable_workspace_state(&runtime.tabs[0]);

    assert_eq!(
        persisted
            .windows
            .iter()
            .find(|window| window.id == "agent-1")
            .expect("persisted window")
            .status,
        WindowProcessStatus::Waiting
    );
}

#[test]
fn app_runtime_output_classifies_rendered_codex_approval_prompt() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let prompt = b"Would you like to run the following command?\r\n\r\n\
        cargo test -p gwt window_state\r\n\r\n\
        > 1. Yes, proceed\r\n\
          2. No, and tell Codex what to do differently\r\n\r\n\
        Press enter to confirm or esc to cancel\r\n";
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(prompt);

    let events = runtime.handle_runtime_output(window_id.clone(), prompt.to_vec());

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Waiting,
            ..
        }
    )));
    assert!(runtime.window_approval_waiting.contains_key(&window_id));
}

#[test]
fn app_runtime_codex_directory_trust_prompt_escalates_only_monitor_owned_live_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo_without_origin(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab.workspace.set_agent_id("agent-1", "codex"));
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: window_id.clone(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let prompt = b"You are in /tmp/managed-worktree\r\n\r\n\
        Do you trust the contents of this directory? Working with untrusted contents comes with higher\r\n\
        risk of prompt injection. Trusting the directory allows project-local config, hooks, and exec\r\n\
        policies to load.\r\n\r\n\
        > 1. Yes, continue\r\n  2. No, quit\r\n\r\n  Press enter to continue\r\n";
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(prompt);

    runtime.handle_runtime_output(window_id.clone(), prompt.to_vec());

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert!(
        persisted.launched_issues.is_empty(),
        "NeedsHuman releases the slot"
    );
    assert_eq!(persisted.failed_issues.len(), 1);
    assert_eq!(persisted.failed_issues[0].issue_number, 42);
    assert_eq!(
        persisted.failed_issues[0].message,
        "Codex requires directory trust confirmation for the managed worktree"
    );
    assert!(
        !runtime.window_approval_waiting.contains_key(&window_id),
        "directory trust is a typed terminal handoff, not tool-approval Waiting"
    );
}

#[test]
fn app_runtime_directory_trust_prompt_is_inert_for_unowned_codex_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo_without_origin(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "codex-1",
        repo.clone(),
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let prompt = b"You are in /tmp/manual-worktree\r\n\r\n\
        Do you trust the contents of this directory? Working with untrusted contents comes with higher\r\n\
        risk of prompt injection. Trusting the directory allows project-local config, hooks, and exec policies to load.\r\n\r\n\
        > 1. Yes, continue\r\n  2. No, quit\r\n\r\n  Press enter to continue\r\n";
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(prompt);

    let events = runtime.handle_runtime_output(window_id.clone(), prompt.to_vec());

    assert_eq!(
        events.len(),
        2,
        "unowned output only emits terminal output and its read-only preview"
    );
    assert!(matches!(
        events[0].event,
        BackendEvent::TerminalOutput { .. }
    ));
    assert!(matches!(
        events[1].event,
        BackendEvent::TerminalPreview { .. }
    ));
    assert!(
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
            .map_or(true, |prefs| prefs.failed_issues.is_empty())
    );
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
}

#[test]
fn app_runtime_generic_agent_uses_persisted_claude_provider_for_approval_prompt() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab.workspace.set_agent_id("agent-1", "claude"));
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let prompt = b"Bash command\r\n\r\n  cargo test -p gwt\r\n\r\n\
        Do you want to proceed?\r\n\
        > 1. Yes\r\n\
          2. No, and tell Claude what to do differently\r\n\r\n\
        Enter to confirm \xc2\xb7 Esc to cancel\r\n";
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(prompt);

    let events = runtime.handle_runtime_output(window_id.clone(), prompt.to_vec());

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Waiting,
            ..
        }
    )));
    assert!(runtime.window_approval_waiting.contains_key(&window_id));
}

#[test]
fn app_runtime_terminal_navigation_and_submit_keep_wait_until_output_resolves_it() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Running);
    let prompt = b"Would you like to run the following command?\r\n\r\n\
        cargo test -p gwt\r\n\r\n\
        > 1. Yes, proceed\r\n\
          2. No, and tell Codex what to do differently\r\n\r\n\
        Press enter to confirm or esc to cancel\r\n";
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(prompt);
    runtime.handle_runtime_output(window_id.clone(), prompt.to_vec());

    assert!(runtime
        .terminal_input_events(&window_id, "\u{1b}[A")
        .is_empty());
    assert!(runtime.window_approval_waiting.contains_key(&window_id));
    let partial_redraw = runtime.observe_runtime_approval_prompt(&window_id, None);
    assert!(partial_redraw.is_empty());
    assert!(runtime.window_approval_waiting.contains_key(&window_id));

    let submitted = runtime.terminal_input_events(&window_id, "1\r");

    assert!(submitted.is_empty());
    let latch = runtime
        .window_approval_waiting
        .get(&window_id)
        .expect("approval remains latched until rendered output changes");
    assert_eq!(latch.resolving_fingerprint, latch.active_fingerprint);

    let pending = runtime.observe_runtime_approval_prompt(&window_id, None);
    assert!(pending.is_empty());
    let token = runtime.window_approval_waiting[&window_id]
        .pending_settle_token
        .expect("settle token");
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(b"\x1b[2J\x1b[HWorking\r\n");
    let cleared = runtime.handle_runtime_approval_settle(&window_id, token);
    assert!(cleared.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
}

#[test]
fn app_runtime_approval_settle_same_prompt_cancels_resolution_without_frames() {
    let (_temp, mut runtime, window_id, _prompt) = approval_settle_runtime();
    assert!(runtime
        .observe_runtime_approval_prompt(&window_id, None)
        .is_empty());
    let token = runtime.window_approval_waiting[&window_id]
        .pending_settle_token
        .expect("settle token");
    let fingerprint = runtime.window_approval_waiting[&window_id]
        .active_fingerprint
        .expect("active fingerprint");

    let redraw = runtime.observe_runtime_approval_prompt(&window_id, Some(fingerprint));
    assert!(redraw.is_empty());

    let events = runtime.handle_runtime_approval_settle(&window_id, token);

    assert!(events.is_empty());
    let latch = &runtime.window_approval_waiting[&window_id];
    assert!(!latch.resolution_started);
    assert!(latch.pending_settle_token.is_none());
}

#[test]
fn app_runtime_approval_settle_stable_progress_clears_waiting() {
    let (_temp, mut runtime, window_id, _prompt) = approval_settle_runtime();
    runtime.observe_runtime_approval_prompt(&window_id, None);
    let token = runtime.window_approval_waiting[&window_id]
        .pending_settle_token
        .expect("settle token");
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(b"\x1b[2J\x1b[HWorking\r\n");

    let events = runtime.handle_runtime_approval_settle(&window_id, token);

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
}

#[test]
fn app_runtime_approval_settle_partial_provider_evidence_keeps_waiting() {
    let (_temp, mut runtime, window_id, _prompt) = approval_settle_runtime();
    runtime.observe_runtime_approval_prompt(&window_id, None);
    let token = runtime.window_approval_waiting[&window_id]
        .pending_settle_token
        .expect("settle token");
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(b"\x1b[2J\x1b[HWould you like to run the following command?\r\n");

    let events = runtime.handle_runtime_approval_settle(&window_id, token);

    assert!(events.is_empty());
    let latch = &runtime.window_approval_waiting[&window_id];
    assert!(latch.resolution_started);
    assert!(latch.pending_settle_token.is_none());
}

#[test]
fn app_runtime_approval_settle_ignores_complete_prompt_in_scrollback_above_progress() {
    let (_temp, mut runtime, window_id, _prompt) = approval_settle_runtime();
    runtime.observe_runtime_approval_prompt(&window_id, None);
    let token = runtime.window_approval_waiting[&window_id]
        .pending_settle_token
        .expect("settle token");
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(b"\r\nRunning tests...\r\n");

    let events = runtime.handle_runtime_approval_settle(&window_id, token);

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
}

#[test]
fn app_runtime_approval_settle_stale_token_is_noop() {
    let (_temp, mut runtime, window_id, _prompt) = approval_settle_runtime();
    runtime.observe_runtime_approval_prompt(&window_id, None);
    let token = runtime.window_approval_waiting[&window_id]
        .pending_settle_token
        .expect("settle token");

    let events = runtime.handle_runtime_approval_settle(&window_id, token.wrapping_add(1));

    assert!(events.is_empty());
    assert_eq!(
        runtime.window_approval_waiting[&window_id].pending_settle_token,
        Some(token)
    );
}

#[test]
fn app_runtime_approval_settle_timer_routes_sanitized_token_event() {
    let (_temp, mut runtime, window_id, _prompt) = approval_settle_runtime();
    let proxy_events = match &runtime.proxy {
        AppEventProxy::Stub(events) => events.clone(),
        AppEventProxy::Real(_) | AppEventProxy::Project { .. } => {
            panic!("sample runtime must use stub proxy")
        }
    };
    runtime.observe_runtime_approval_prompt(&window_id, None);
    let token = runtime.window_approval_waiting[&window_id]
        .pending_settle_token
        .expect("settle token");

    wait_for_recorded_event_with_timeout(
        "runtime approval settle timer event",
        &proxy_events,
        Duration::from_secs(2),
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::RuntimeApprovalSettle { id, token: queued }
                        if id == &window_id && *queued == token
                )
            })
        },
    );
}

#[test]
fn app_runtime_different_prompt_after_submit_emits_clear_then_waiting_reentry() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Running);
    let first = b"Would you like to run the following command?\r\n first-command\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(first);
    runtime.handle_runtime_output(window_id.clone(), first.to_vec());
    runtime.terminal_input_events(&window_id, "1\r");

    let second =
        b"\x1b[2J\x1b[HWould you like to run the following command?\r\n second-command\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(second);
    let events = runtime.handle_runtime_output(window_id.clone(), second.to_vec());
    let states = events
        .iter()
        .filter_map(|event| match event.event {
            BackendEvent::WindowState { state, .. } => Some(state),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        states,
        vec![WindowProcessStatus::Running, WindowProcessStatus::Waiting]
    );
}

#[test]
fn app_runtime_pane_send_marks_resolution_before_delayed_submit() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime.window_approval_waiting.insert(
        window_id.clone(),
        ApprovalPromptLatch {
            active_fingerprint: Some(7),
            resolving_fingerprint: None,
            resolution_started: false,
            pending_settle_token: None,
        },
    );

    let events =
        runtime.pane_send_input_to_window_events("client-1".to_string(), &window_id, "1\r");

    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::PaneSendResult { ok: true, .. })));
    assert_eq!(
        runtime
            .window_approval_waiting
            .get(&window_id)
            .and_then(|latch| latch.resolving_fingerprint),
        Some(7)
    );
}

#[test]
fn app_runtime_failed_input_paths_do_not_mark_approval_resolution() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.window_approval_waiting.insert(
        window_id.clone(),
        ApprovalPromptLatch {
            active_fingerprint: Some(11),
            resolving_fingerprint: None,
            resolution_started: false,
            pending_settle_token: None,
        },
    );

    let terminal_events = runtime.terminal_input_events(&window_id, "1\r");
    let pane_events =
        runtime.pane_send_input_to_window_events("client-1".to_string(), &window_id, "1\r");

    assert!(terminal_events.is_empty());
    assert!(pane_events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::PaneSendResult { ok: false, .. })));
    let latch = runtime
        .window_approval_waiting
        .get(&window_id)
        .expect("latch");
    assert!(!latch.resolution_started);
    assert!(latch.resolving_fingerprint.is_none());
}

#[test]
fn app_runtime_custom_agent_name_containing_codex_is_not_classified_as_codex() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_agent_id("agent-1", "company-codex-wrapper"));
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let prompt = b"Would you like to run the following command?\r\n secret\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(prompt);

    let events = runtime.handle_runtime_output(window_id.clone(), prompt.to_vec());

    assert!(events.iter().all(|event| !matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Waiting,
            ..
        }
    )));
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
}

#[test]
fn app_runtime_approval_error_beats_live_hook_and_redacts_prompt_tail_everywhere() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Running);
    runtime.handle_runtime_approval_wait_state(&window_id, true);
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(b"SECRET_TOKEN=/private/path command --token sentinel\r\n");

    let events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("failed SECRET_TOKEN=/private/path".to_string()),
    );
    let serialized = format!("{events:?}");

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Error,
            ..
        }
    )));
    assert!(!serialized.contains("SECRET_TOKEN"));
    assert!(!serialized.contains("/private/path"));
    assert!(!serialized.contains("--token"));
    assert_eq!(
        runtime.window_details.get(&window_id).map(String::as_str),
        Some("Agent approval prompt ended unexpectedly")
    );
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
}

#[test]
fn app_runtime_approval_error_redacts_screen_before_output_event_sets_latch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let prompt = b"Would you like to run the following command?\r\n\
        SECRET_TOKEN=/private/path command --token sentinel\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .process_bytes(prompt);
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));

    let events = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("failed SECRET_TOKEN=/private/path".to_string()),
    );
    let serialized = format!("{events:?}");

    assert!(!serialized.contains("SECRET_TOKEN"));
    assert!(!serialized.contains("/private/path"));
    assert!(!serialized.contains("--token"));
    assert_eq!(
        runtime.window_details.get(&window_id).map(String::as_str),
        Some("Agent approval prompt ended unexpectedly")
    );
}

#[test]
fn app_runtime_progress_hook_clears_approval_wait_even_when_hook_state_is_unchanged() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Running);
    runtime.handle_runtime_approval_wait_state(&window_id, true);

    let cleared = runtime.handle_runtime_hook_event(runtime_hook_state("Running", "session-1"));

    assert!(cleared.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
}

#[test]
fn app_runtime_terminal_state_clears_approval_wait_tracking() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    runtime.handle_runtime_approval_wait_state(&window_id, true);

    let events =
        runtime.handle_runtime_status(window_id.clone(), WindowProcessStatus::Stopped, None);

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Stopped,
            ..
        }
    )));
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
}

#[test]
fn app_runtime_runtime_state_change_after_duplicate_burst_emits_status_events() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );

    let first_events =
        runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));
    let duplicate_events =
        runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));
    let changed_events =
        runtime.handle_runtime_hook_event(runtime_hook_state("Running", "session-1"));

    assert!(first_events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })));
    assert!(
        duplicate_events.is_empty(),
        "unchanged RuntimeState hooks should not fan out status events"
    );
    assert!(changed_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(changed_events.iter().any(|event| matches!(
        event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Running,
            ..
        }
    )));
}

#[test]
fn app_runtime_stopped_runtime_state_after_prior_state_keeps_window_open() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));

    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Stopped", "session-1"));

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Stopped,
            ..
        }
    )));
    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.tabs[0].workspace.window("codex-1").is_some());
}

#[test]
fn app_runtime_start_window_registers_running_process_runtime_and_pty_writer() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window = runtime.tabs[0].workspace.persisted().windows[0].clone();
    let window_id = combined_window_id("tab-1", &window.id);

    let events = runtime.start_window("tab-1", &window.id, window.preset, window.geometry.clone());

    assert_eq!(events.len(), 2);
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::WindowState { window_id: id, state }
            if id == &window_id && *state == WindowProcessStatus::Running
    )));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalStatus { id, status, detail }
            if id == &window_id
                && *status == WindowProcessStatus::Running
                && detail.is_none()
    )));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(runtime
        .pty_writers
        .read()
        .expect("pty writer registry")
        .contains_key(&window_id));

    runtime.stop_window_runtime(&window_id);
}

// SPEC-2356 安心 Addendum (FR-041): StopWindow tears down the runtime but KEEPS
// the window on the canvas, rendered as Stopped, unlike CloseWindow.
#[test]
fn stop_window_events_keeps_window_and_marks_stopped() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window = runtime.tabs[0].workspace.persisted().windows[0].clone();
    let window_id = combined_window_id("tab-1", &window.id);
    runtime.register_window("tab-1", &window.id);
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .window_pty_statuses
        .insert(window_id.clone(), WindowProcessStatus::Running);

    let events = runtime.stop_window_events(&window_id);

    // The runtime is gone (PTY killed) but the window record survives.
    assert!(!runtime.runtimes.contains_key(&window_id));
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(
        runtime.tabs[0].workspace.window(&window.id).is_some(),
        "StopWindow must keep the window on the canvas, unlike CloseWindow"
    );
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Stopped),
        "stopped window must render as Stopped"
    );
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::WindowState { window_id: id, state }
            if id == &window_id && *state == WindowProcessStatus::Stopped
    )));
}

// SPEC-2356 安心 Addendum (FR-041): StopWindow is idempotent — stopping an
// already-stopped window keeps it on the canvas and stays Stopped.
#[test]
fn stop_window_events_is_idempotent_when_already_stopped() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo,
        WindowPreset::Shell,
        WindowProcessStatus::Stopped,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    runtime.register_window("tab-1", "shell-1");

    let events = runtime.stop_window_events(&window_id);

    assert!(runtime.tabs[0].workspace.window("shell-1").is_some());
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Stopped)
    );
    // Idempotent: it still emits the authoritative Stopped status, never an error.
    assert!(events.iter().all(|event| !matches!(
        &event.event,
        BackendEvent::WindowState { state, .. } if *state == WindowProcessStatus::Error
    )));
}

// SPEC-2356 安心 Addendum (FR-041): CloseWindow vs StopWindow contract — Close
// removes the window, Stop keeps it. This guards the distinction directly.
#[test]
fn close_window_removes_while_stop_window_keeps() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[WindowPreset::Shell, WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let windows: Vec<_> = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| window.id.clone())
        .collect();
    let stop_raw = windows[0].clone();
    let close_raw = windows[1].clone();
    let stop_id = combined_window_id("tab-1", &stop_raw);
    let close_id = combined_window_id("tab-1", &close_raw);
    runtime.register_window("tab-1", &stop_raw);
    runtime.register_window("tab-1", &close_raw);

    runtime.stop_window_events(&stop_id);
    runtime.close_window_events(&close_id);

    assert!(
        runtime.tabs[0].workspace.window(&stop_raw).is_some(),
        "StopWindow keeps the window"
    );
    assert!(
        runtime.tabs[0].workspace.window(&close_raw).is_none(),
        "CloseWindow removes the window"
    );
}

// Issue #3366 — the line-level process stream (measured ≈956 msg/s under
// normal agent load) is delivered only while a Console window exists
// somewhere in the workspace. Raw `process_line` events are consumed
// exclusively by Console window controllers; the Logs window's Process
// facet reads summary log events instead. History is not lost while
// suppressed: `LoadProcessConsole` replays the ProcessConsoleHub ring
// buffer on every Console mount.
#[test]
fn process_line_events_drop_stream_without_console_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    // A Logs window alone must not subscribe the raw process stream.
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::Shell, WindowPreset::Logs],
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.process_line_events(gwt_core::process_console::ProcessLine::new(
        gwt_core::process_console::ProcessKind::Git,
        1,
        gwt_core::process_console::ProcessStream::Stdout,
        "remote: Enumerating objects",
    ));

    assert!(
        events.is_empty(),
        "no Console window anywhere → the stream must not reach the client hub"
    );
}

#[test]
fn process_line_events_broadcast_while_console_window_open_on_inactive_tab() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    fs::create_dir_all(&repo_a).expect("create repo-a");
    fs::create_dir_all(&repo_b).expect("create repo-b");
    // The Console window lives on the INACTIVE tab: its controller keeps
    // accumulating without re-requesting a snapshot when the tab becomes
    // active again, so the gate must consider every tab.
    let active = sample_project_tab(
        "tab-a",
        "A",
        repo_a,
        ProjectKind::Git,
        &[WindowPreset::Shell],
    );
    let inactive = sample_project_tab(
        "tab-b",
        "B",
        repo_b,
        ProjectKind::Git,
        &[WindowPreset::Console],
    );
    let runtime = sample_runtime(temp.path(), vec![active, inactive], Some("tab-a"));

    let events = runtime.process_line_events(gwt_core::process_console::ProcessLine::new(
        gwt_core::process_console::ProcessKind::Gh,
        7,
        gwt_core::process_console::ProcessStream::Stderr,
        "gh api rate limit",
    ));

    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0].target, DispatchTarget::Project(key) if Some(key) == runtime.project_key_for_tab("tab-b"))
    );
    assert!(matches!(
        &events[0].event,
        BackendEvent::ProcessLine { line } if line.message == "gh api rate limit"
    ));
}

// Issue #3366 — `log_entry_appended` is consumed only by Logs window
// state. `LoadLogs` re-reads the log directory on mount, so suppressing
// the live stream while no Logs window exists loses nothing.
#[test]
fn log_entry_events_drop_stream_without_logs_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    // A Console window alone must not subscribe the tracing log stream.
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::Shell, WindowPreset::Console],
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.log_entry_events(gwt_core::logging::LogEvent::new(
        LogLevel::Warn,
        "pty",
        "reader stalled",
    ));

    assert!(
        events.is_empty(),
        "no Logs window anywhere → the stream must not reach the client hub"
    );
}

#[test]
fn log_entry_events_send_global_to_consumers_and_project_only_to_owner() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo_a = temp.path().join("repo-a");
    let repo_b = temp.path().join("repo-b");
    fs::create_dir_all(&repo_a).expect("create repo-a");
    fs::create_dir_all(&repo_b).expect("create repo-b");
    let active = sample_project_tab(
        "tab-a",
        "A",
        repo_a,
        ProjectKind::Git,
        &[WindowPreset::Logs],
    );
    let inactive = sample_project_tab(
        "tab-b",
        "B",
        repo_b,
        ProjectKind::Git,
        &[WindowPreset::Logs],
    );
    let runtime = sample_runtime(temp.path(), vec![active, inactive], Some("tab-a"));

    let events = runtime.log_entry_events(gwt_core::logging::LogEvent::new(
        LogLevel::Warn,
        "pty",
        "reader stalled",
    ));

    assert_eq!(
        events.len(),
        2,
        "global diagnostics reach both Logs consumers"
    );
    let owner = runtime.project_key_for_tab("tab-b").unwrap();
    let mut scoped =
        gwt_core::logging::LogEvent::new(LogLevel::Warn, "pty", "B private diagnostic");
    scoped.project_scope = Some(owner.as_str().to_string());
    let owned_events = runtime.log_entry_events(scoped.clone());
    assert_eq!(
        owned_events.len(),
        1,
        "another project's diagnostics must not reach the A socket"
    );
    assert!(matches!(&owned_events[0].target, DispatchTarget::Project(key) if key == owner));
    scoped.project_scope = Some("unopened-project".to_string());
    assert!(runtime.log_entry_events(scoped).is_empty());
    assert!(matches!(
        &events[0].event,
        BackendEvent::LogEntryAppended { entry } if entry.message == "reader stalled"
    ));
}

// SPEC-2356 安心 Addendum (FR-042): StopAllWindows stops every running agent
// window's runtime while keeping all windows on the canvas.
#[test]
fn stop_all_windows_events_stops_every_runtime_keeping_windows() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[WindowPreset::Claude, WindowPreset::Codex],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let raw_ids: Vec<_> = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| window.id.clone())
        .collect();
    for raw in &raw_ids {
        let window_id = combined_window_id("tab-1", raw);
        runtime.register_window("tab-1", raw);
        insert_test_pane_runtime(&mut runtime, &window_id);
        runtime
            .window_pty_statuses
            .insert(window_id.clone(), WindowProcessStatus::Running);
    }

    runtime.stop_all_windows_events(&runtime.test_context());

    for raw in &raw_ids {
        let window_id = combined_window_id("tab-1", raw);
        assert!(
            !runtime.runtimes.contains_key(&window_id),
            "every agent runtime must be torn down"
        );
        assert!(
            runtime.tabs[0].workspace.window(raw).is_some(),
            "StopAllWindows must keep all windows on the canvas"
        );
        assert_eq!(
            runtime.window_status(&window_id),
            Some(WindowProcessStatus::Stopped)
        );
    }
}

// SPEC-2356 安心 Addendum (FR-044): RestartWindow relaunches a stopped process
// preset in place, preserving the window id and re-registering its runtime.
#[test]
fn restart_window_events_relaunches_stopped_process_window_in_place() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo,
        WindowPreset::Shell,
        WindowProcessStatus::Stopped,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    runtime.register_window("tab-1", "shell-1");
    assert!(!runtime.runtimes.contains_key(&window_id));

    let events = runtime.restart_window_events(&window_id);

    // Same window id, freshly-running runtime + PTY writer registered.
    assert!(
        runtime.tabs[0].workspace.window("shell-1").is_some(),
        "RestartWindow must preserve the window id"
    );
    assert!(runtime.runtimes.contains_key(&window_id));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::WindowState { window_id: id, state }
            if id == &window_id && *state == WindowProcessStatus::Running
    )));

    runtime.stop_window_runtime(&window_id);
}

#[test]
fn natural_process_exit_keeps_terminal_status_available_to_restart_guard() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo,
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    runtime.register_window("tab-1", "shell-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .window_pty_statuses
        .insert(window_id.clone(), WindowProcessStatus::Running);

    runtime.handle_runtime_status(window_id.clone(), WindowProcessStatus::Stopped, None);
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Stopped),
        "natural teardown must leave the stored terminal status readable"
    );

    let events = runtime.restart_window_events(&window_id);

    assert!(
        !events.is_empty(),
        "the restart guard must admit natural exits"
    );
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
    runtime.stop_window_runtime(&window_id);
}

// SPEC-2356 安心 Addendum (FR-044): RestartWindow only acts on stopped/errored
// windows — restarting a window that is already running is a no-op so a live
// agent is never double-spawned.
#[test]
fn restart_window_events_is_noop_when_window_already_running() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::NonRepo,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window = runtime.tabs[0].workspace.persisted().windows[0].clone();
    let window_id = combined_window_id("tab-1", &window.id);
    runtime.register_window("tab-1", &window.id);
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .window_pty_statuses
        .insert(window_id.clone(), WindowProcessStatus::Running);
    let original_pane = runtime
        .runtimes
        .get(&window_id)
        .map(|runtime| Arc::as_ptr(&runtime.pane) as usize)
        .expect("runtime present");

    let events = runtime.restart_window_events(&window_id);

    assert!(events.is_empty(), "restarting a running window is a no-op");
    let after_pane = runtime
        .runtimes
        .get(&window_id)
        .map(|runtime| Arc::as_ptr(&runtime.pane) as usize)
        .expect("runtime still present");
    assert_eq!(
        original_pane, after_pane,
        "running window's runtime must not be replaced"
    );

    runtime.stop_window_runtime(&window_id);
}

#[test]
fn app_runtime_viewport_and_geometry_updates_persist_workspace_state() {
    // Persistence flows through `workspace_state_path()` which is
    // HOME-based, so we must serialize against other HOME-touching
    // tests and pin HOME to this test's tempdir for the duration.
    // Without this guard, parallel tests that mutate HOME race
    // with our persist + load pair and the workspace file ends up
    // missing (or pointing at another test's already-cleaned
    // tempdir).
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");

    assert_eq!(
        runtime
            .update_viewport_events(
                &runtime.test_context(),
                gwt::CanvasViewport {
                    x: 12.0,
                    y: 34.0,
                    zoom: 1.25,
                }
            )
            .len(),
        1
    );
    assert_eq!(
        runtime
            .update_window_geometry_events(
                &window_id,
                WindowGeometry {
                    x: 56.0,
                    y: 78.0,
                    width: 720.0,
                    height: 480.0,
                },
                100,
                30,
                None,
            )
            .len(),
        1
    );

    assert!(
        runtime
            .persist_dispatcher
            .wait_idle(std::time::Duration::from_secs(5)),
        "persist dispatcher should drain before disk readback",
    );
    let session = load_session_state(&temp.path().join("session-state.json"))
        .expect("load persisted session state");
    // Issue #4535 AC-4: `active_tab_id` is no longer persisted.
    assert_eq!(session.legacy_active_tab_id, None);
    assert_eq!(session.tabs.len(), 1);
    assert_eq!(session.tabs[0].id, "tab-1");
    assert_eq!(session.tabs[0].project_root, repo);

    let workspace = load_restored_workspace_state(&repo).expect("load persisted workspace");
    assert_eq!(workspace.viewport.x, 12.0);
    assert_eq!(workspace.viewport.y, 34.0);
    assert_eq!(workspace.viewport.zoom, 1.25);
    let window = workspace
        .windows
        .iter()
        .find(|window| window.id == "shell-1")
        .expect("persisted window");
    assert_eq!(window.geometry.x, 56.0);
    assert_eq!(window.geometry.y, 78.0);
    assert_eq!(window.geometry.width, 720.0);
    assert_eq!(window.geometry.height, 480.0);
}

#[test]
fn app_runtime_duplicate_viewport_update_skips_workspace_broadcast_and_persist() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo,
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let viewport = gwt::CanvasViewport {
        x: 12.0,
        y: 34.0,
        zoom: 1.25,
    };

    assert_eq!(
        runtime
            .update_viewport_events(&runtime.test_context(), viewport.clone())
            .len(),
        1
    );
    assert_eq!(runtime.persist_dispatcher.enqueued_count(), 1);

    assert!(
        runtime
            .update_viewport_events(&runtime.test_context(), viewport)
            .is_empty(),
        "duplicate viewport payload should not broadcast a workspace_state",
    );
    assert_eq!(
        runtime.persist_dispatcher.enqueued_count(),
        1,
        "duplicate viewport payload should not enqueue another persist snapshot",
    );

    assert_eq!(
        runtime
            .update_viewport_events(
                &runtime.test_context(),
                gwt::CanvasViewport {
                    x: 12.0,
                    y: 34.0,
                    zoom: 1.5,
                }
            )
            .len(),
        1,
        "changed zoom must still broadcast workspace_state",
    );
    assert_eq!(
        runtime.persist_dispatcher.enqueued_count(),
        2,
        "changed viewport must still enqueue persistence",
    );
}

#[test]
fn app_runtime_geometry_update_rejects_stale_base_revision() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "shell-1",
        repo.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");

    assert_eq!(
        runtime
            .update_window_geometry_events(
                &window_id,
                WindowGeometry {
                    x: 56.0,
                    y: 78.0,
                    width: 720.0,
                    height: 480.0,
                },
                100,
                30,
                Some(0),
            )
            .len(),
        1
    );

    assert_eq!(
        runtime
            .update_window_geometry_events(
                &window_id,
                WindowGeometry {
                    x: 90.0,
                    y: 120.0,
                    width: 960.0,
                    height: 640.0,
                },
                120,
                40,
                Some(0),
            )
            .len(),
        1,
        "stale updates should return the current workspace state so the frontend can resync"
    );

    assert!(
        runtime
            .persist_dispatcher
            .wait_idle(std::time::Duration::from_secs(5)),
        "persist dispatcher should drain before disk readback",
    );
    let workspace = load_restored_workspace_state(&repo).expect("load persisted workspace");
    let window = workspace
        .windows
        .iter()
        .find(|window| window.id == "shell-1")
        .expect("persisted window");
    assert_eq!(window.geometry.x, 56.0);
    assert_eq!(window.geometry.y, 78.0);
    assert_eq!(window.geometry.width, 720.0);
    assert_eq!(window.geometry.height, 480.0);
    assert_eq!(window.geometry_revision, 1);
}

#[test]
fn account_switch_releases_pane_quota_without_relatching_the_old_notice() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "codex");
    let mut old = codex_usage_account(100.0, true);
    old.account_id = Some("account-a".into());
    runtime.set_provider_usage_accounts(vec![old]);
    runtime.observe_provider_quota_notice(
        &window_id,
        Some(CODEX_USAGE_LIMIT_SCREEN),
        instant("2026-09-02T09:00:00Z"),
    );
    assert!(runtime.provider_quota_holds.contains_key(&window_id));
    let mut new = gwt_core::usage::ProviderUsage::degraded(
        gwt_core::usage::UsageProvider::Codex,
        gwt_core::usage::UsageState::NoData,
    );
    new.account_id = Some("account-b".into());
    runtime.handle_provider_usage_snapshot(vec![new], instant("2026-09-02T09:02:00Z"));
    assert!(!runtime.provider_quota_holds.contains_key(&window_id));
    runtime.observe_provider_quota_notice(
        &window_id,
        Some(CODEX_USAGE_LIMIT_SCREEN),
        instant("2026-09-02T09:03:00Z"),
    );
    runtime.observe_provider_quota_notice(
        &window_id,
        Some(CODEX_USAGE_LIMIT_SCREEN),
        instant("2026-09-02T09:05:00Z"),
    );
    assert!(!runtime.provider_quota_holds.contains_key(&window_id));
    runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Stopped,
        Some(CODEX_USAGE_LIMIT_SCREEN.to_string()),
        true,
    );
    assert!(!runtime.provider_quota_holds.contains_key(&window_id));
}

/// Issue #4908: a settled refusal forms a hold even while telemetry reads healthy.
#[test]
fn a_settled_screen_refusal_holds_even_while_usage_reads_healthy() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "codex");
    runtime.set_provider_usage_accounts(vec![codex_usage_account(26.0, false)]);

    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CODEX_USAGE_LIMIT_SCREEN),
        instant("2026-09-02T09:00:00Z"),
    );
    assert!(!runtime.provider_quota_holds.contains_key(&window_id));
    assert!(runtime.provider_quota_candidates.contains_key(&window_id));
    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CODEX_USAGE_LIMIT_SCREEN),
        instant("2026-09-02T09:05:00Z"),
    );
    assert!(
        runtime.provider_quota_holds.contains_key(&window_id),
        "the settled refusal overrides usage telemetry on the first launch"
    );
    assert!(!runtime.provider_quota_candidates.contains_key(&window_id));
}
