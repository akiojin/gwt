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
    record_legacy_workspace_work_event_for_test(&repo, {
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
    record_legacy_workspace_work_event_for_test(&repo, {
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
    record_legacy_workspace_work_event_for_test(&repo, {
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
    let _home = ScopedGwtHome::set(temp.path());
    let (owner, probe, mut runtime, mut source) =
        super::managed_launch_tests::managed_exit_fixture(temp.path(), 0, "", false);
    let repo = runtime.tabs[0].project_root.clone();
    let session = probe
        .active_session
        .as_ref()
        .expect("actual owner AgentRecord");
    save_start_work_workspace_projection(
        &repo,
        session,
        "origin/main",
        None,
        None,
        None,
        Some(&std::collections::HashSet::new()),
    )
    .expect("save projection");

    let events =
        super::managed_launch_tests::finish_managed_exit(&owner, &probe, &mut runtime, &mut source);

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
    owner_fixture_block_on(owner.shutdown());
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
        AppEventProxy::Real(_)
        | AppEventProxy::Project { .. }
        | AppEventProxy::Notifying { .. } => {
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
    let proxy = runtime.proxy.clone();
    let status_thread = gwt::pane_runtime::spawn_status_thread(
        window_id.clone(),
        incarnation,
        pane.clone(),
        move |event| {
            if let gwt::pane_runtime::PaneRuntimeEvent::Status {
                id,
                incarnation,
                status,
                detail,
                exit_confirmed,
            } = event
            {
                proxy.send(UserEvent::RuntimeStatus {
                    id,
                    incarnation,
                    status,
                    detail,
                    exit_confirmed,
                });
            }
        },
    );

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

#[cfg(windows)]
fn wait_for_native_conpty_output(
    runtime: &mut AppRuntime,
    window_id: &str,
    receiver: &mut tokio::sync::broadcast::Receiver<gwt::project_runtime::events::OwnerEvent>,
    marker: &str,
    expected_state: Option<WindowProcessStatus>,
    timeout: Duration,
) -> Vec<OutboundEvent> {
    use gwt::project_runtime::events::OwnerEvent;

    // test-hygiene: allow-wall-clock-deadline Real ConPTY IO must reach the observable push within the acceptance deadline.
    let deadline = Instant::now() + timeout;
    let context = runtime
        .project_context("tab-1")
        .expect("fixture project context");
    let identity = runtime.runtimes[window_id].owner_identity.clone();
    let probe = owner_runtime_probe(runtime, window_id);
    let mut output = probe
        .pane
        .lock()
        .expect("actual owner ConPTY")
        .screen()
        .contents();
    let mut pushed = Vec::new();
    let mut process_exited = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "ConPTY timed out: {output:?}");
        let received = owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline Bound kernel IO and service the production output batching deadline between reads.
            tokio::time::timeout(remaining.min(Duration::from_millis(100)), receiver.recv()).await
        });
        match received {
            Ok(Ok(event)) => {
                match &event {
                    OwnerEvent::Output {
                        id,
                        incarnation,
                        data,
                        ..
                    } if id == &probe.id && *incarnation == probe.incarnation => {
                        output.push_str(&String::from_utf8_lossy(data));
                    }
                    OwnerEvent::Status {
                        id,
                        incarnation,
                        exit_confirmed,
                        exit_code,
                        detail,
                        ..
                    } if id == &probe.id && *incarnation == probe.incarnation => {
                        process_exited |= *exit_confirmed;
                        if *exit_confirmed {
                            assert_eq!(
                                *exit_code,
                                Some(7),
                                "actual fixture exit status; marker={marker:?}; detail={detail:?}; output={output:?}; screen={:?}",
                                probe.pane.lock().expect("actual owner ConPTY").screen().contents()
                            );
                        }
                    }
                    _ => {}
                }
                pushed.extend(runtime.handle_project_owner_event(&context, 1, &identity, event));
            }
            Err(_) => {}
            Ok(Err(error)) => panic!("ConPTY owner event stream disconnected: {error}"),
        }
        pushed.extend(runtime.flush_runtime_output_batches(Instant::now()));
        if output.contains(marker)
            && (process_exited
                || !matches!(
                    expected_state,
                    Some(WindowProcessStatus::Stopped | WindowProcessStatus::Error)
                ))
            && expected_state.is_none_or(|expected| {
                pushed.iter().any(|event| {
                    matches!(
                        event.event,
                        BackendEvent::WindowState { state, .. } if state == expected
                    )
                })
            })
        {
            return pushed;
        }
    }
}

#[cfg(windows)]
#[test]
fn app_runtime_native_conpty_pushes_running_waiting_idle_and_process_exit() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("native-conpty-project");
    fs::create_dir_all(&repo).expect("native ConPTY project root");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab("tab-1", "Native ConPTY", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    // Each real stdin read is an observable barrier, so startup time is not
    // charged to the one-second state-push acceptance requirement.
    let fixture = concat!(
        "@echo off & echo GWT_NATIVE_READY & set /p phase= & ",
        "echo GWT_NATIVE_TOKEN & set /p phase= & ",
        "echo Would you like to run the following command? & echo. & ",
        "echo cargo test -p gwt & echo. & echo ^> 1. Yes, proceed & ",
        "echo   2. No, and tell Codex what to do differently & echo. & ",
        "echo Press enter to confirm or esc to cancel & set /p phase= & exit /B 7"
    );
    let runner = temp.path().join("native-conpty-codex.cmd");
    fs::write(&runner, format!(
        "@echo off\r\nif \"%~1\"==\"--version\" (echo 1.2.3& exit /b 0)\r\nif \"%~1\"==\"features\" exit /b 0\r\n{fixture}\r\n"
    )).expect("actual provider with ordinary runner probes");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/native-conpty")
        .build();
    config.command = runner.display().to_string();
    config.args.clear();
    let window_id = spawn_owner_codex_fixture(&mut runtime, "tab-1", config, None);
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let probe = owner_runtime_probe(&runtime, &window_id);
    let context = runtime.project_context("tab-1").unwrap();
    let identity = owner.client().identity().unwrap().clone();
    let mut receiver = owner.subscribe();
    wait_for_native_conpty_output(
        &mut runtime,
        &window_id,
        &mut receiver,
        "GWT_NATIVE_READY",
        None,
        Duration::from_secs(5),
    );
    let event = owner_runtime_status_event(&owner, &probe, "waiting", "SessionStart");
    runtime.handle_project_owner_event(&context, 1, &identity, event);
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Idle)
    );

    for (source, status, expected) in [
        ("UserPromptSubmit", "running", WindowProcessStatus::Running),
        ("Stop", "waiting", WindowProcessStatus::Idle),
    ] {
        let started = Instant::now();
        let event = owner_runtime_status_event(&owner, &probe, status, source);
        let events = runtime.handle_project_owner_event(&context, 1, &identity, event);
        assert!(events.iter().any(|event| matches!(
            event.event,
            BackendEvent::WindowState { state, .. } if state == expected
        )));
        // test-hygiene: allow-wall-clock-deadline Issue #3809 requires each runtime hook state push within one second.
        assert!(started.elapsed() <= Duration::from_secs(1));
        if expected == WindowProcessStatus::Running {
            probe
                .pty
                .write_input(b"tokens\r")
                .expect("start token output");
            wait_for_native_conpty_output(
                &mut runtime,
                &window_id,
                &mut receiver,
                "GWT_NATIVE_TOKEN",
                None,
                Duration::from_secs(5),
            );
            assert_eq!(runtime.window_status(&window_id), Some(expected));
            probe.pty.write_input(b"approval\r").expect("emit approval");
            wait_for_native_conpty_output(
                &mut runtime,
                &window_id,
                &mut receiver,
                "Press enter to confirm or esc to cancel",
                Some(WindowProcessStatus::Waiting),
                Duration::from_secs(1),
            );
        }
    }
    probe
        .pty
        .write_input(b"exit\r")
        .expect("exit provider fixture");
    wait_for_native_conpty_output(
        &mut runtime,
        &window_id,
        &mut receiver,
        "",
        Some(WindowProcessStatus::Error),
        Duration::from_secs(1),
    );
    assert_eq!(
        probe
            .pane
            .lock()
            .unwrap()
            .last_exit()
            .map(|exit| exit.exit_code),
        Some(7)
    );
    close_test_owner_runtime(&mut runtime, &window_id);
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
    let pty = owner_runtime_probe(&runtime, &window_id).pty.clone();
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
    // A Stop legitimately queues blocking work: Issue #3777 AC-3 moves the
    // Active Work projection rebuild off the Tao loop, and Issue #4411 moves
    // heartbeat publication off it too. Counting queued tasks
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
        BackendEvent::WorkspaceState { .. }
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
fn app_runtime_pm_status_changed_refreshes_only_the_exact_registered_pm_without_state_changes() {
    use gwt::project_runtime::events::OwnerCommand;
    for (current_claim, registered_pm, should_refresh) in [
        (true, true, true),
        (false, true, false),
        (true, false, false),
    ] {
        let temp = tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let root = temp.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        init_repo_with_initial_commit(&root);
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "agent-1",
            root.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Waiting,
        );
        let (mut runtime, recorded_events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        let id = combined_window_id("tab-1", "agent-1");
        let _pane = attach_live_pm_pane(&mut runtime, &id);
        // Finish the real launch's background projection before observing only
        // work caused by this notification.
        runtime.blocking_tasks = BlockingTaskSpawner::thread();
        runtime.active_work_projection_broadcast_for_tab("tab-1");
        wait_for_active_work_projection(&mut runtime);
        runtime_state_hook_events(&mut runtime, &id, "Running", 1);
        assert_eq!(
            owner_runtime_probe(&runtime, &id)
                .active_session
                .unwrap()
                .agent_id,
            "claude"
        );
        let prompt = b"\x1b[2J\x1b[HBash command\r\n\r\n cargo test -p gwt\r\n\r\n\
            Do you want to proceed?\r\n > 1. Yes\r\n\
            2. No, and tell Claude what to do differently\r\n\r\n\
            Enter to confirm \xc2\xb7 Esc to cancel\r\n";
        approval_owner_screen(&mut runtime, &id, prompt, approval_owner_future_anchor());
        let owner = project_owner_fixture(&mut runtime, &id);
        let before = owner_runtime_probe(&runtime, &id);
        let session_id = &before.session.as_ref().unwrap().id;
        let latch = before.policy.approval.expect("actual PM approval latch");
        if !registered_pm {
            assert!(
                gwt::pm_registry::deregister_pm(
                    &gwt::pm_registry::pm_prefs_path_for_repo_path(&root),
                    session_id,
                )
                .unwrap()
                .1
            );
        }
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;
        recorded_events.lock().unwrap().clear();
        let status_before = runtime.window_status(&id);
        let gui_latch_before = runtime.window_approval_waiting.get(&id).copied();
        let mut event = runtime_hook_coordination_event(if current_claim {
            session_id
        } else {
            "retired-session"
        });
        event.source_event = Some("PmStatusChanged".into());
        event.message = Some("coordination:PmStatusChanged".into());
        event.project_root = Some(root.display().to_string());
        // A provider-id fallback match must never grant PM notification authority.
        event.agent_session_id = Some(session_id.clone());

        let mut source = owner.subscribe();
        owner_fixture_block_on(async {
            assert_eq!(
                before.runtime_hook(&event).await.unwrap(),
                if current_claim {
                    reqwest::StatusCode::NO_CONTENT
                } else {
                    reqwest::StatusCode::UNAUTHORIZED
                }
            );
            owner.command(OwnerCommand::Health).await.unwrap();
        });
        let events = relay_approval_owner_events(&mut runtime, &id, &mut source);
        let after = owner_runtime_probe(&runtime, &id);
        let status_after = runtime.window_status(&id);
        let gui_latch_after = runtime.window_approval_waiting.get(&id).copied();
        for task in std::mem::take(&mut *tasks.lock().unwrap()) {
            task();
        }
        let completed = recorded_events.lock().unwrap();
        let census = completed.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::RemoteOwnerInventoryComplete(completion)
                    if completion.context.project_root == root && completion.result.is_ok()
            )
        });
        let prepared = completed.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::ActiveWorkProjectionPrepared(completion)
                    if completion.project_root == root && completion.result.is_ok()
            )
        });
        drop(completed);
        close_test_owner_runtime(&mut runtime, &id);

        assert_eq!(status_after, status_before);
        assert_eq!(gui_latch_after, gui_latch_before);
        let after_latch = after
            .policy
            .approval
            .expect("PM notification keeps approval");
        assert_eq!(after_latch.active_fingerprint, latch.active_fingerprint);
        assert_eq!(
            after_latch.resolving_fingerprint,
            latch.resolving_fingerprint
        );
        assert_eq!(after_latch.resolution_started, latch.resolution_started);
        assert_eq!(
            after.policy.approval_settle_at,
            before.policy.approval_settle_at
        );
        assert!(!events.iter().any(|event| matches!(
            event.event,
            BackendEvent::TerminalStatus { .. } | BackendEvent::WindowState { .. }
        )));
        assert_eq!(
            census, should_refresh,
            "only the exact registered PM refreshes the owner census"
        );
        assert_eq!(
            prepared, should_refresh,
            "only the exact registered PM prepares Work"
        );
    }
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

pub(super) fn runtime_state_hook_fixture(temp: &Path) -> (AppRuntime, String) {
    let repo = temp.join("hook-project");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab("tab-1", "Hook project", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp, vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/runtime-hook-fixture")
        .build();
    let id = spawn_owner_codex_fixture(&mut runtime, "tab-1", config, None);
    let owner = project_owner_fixture(&mut runtime, &id);
    let probe = owner_runtime_probe(&runtime, &id);
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline startup hang guard; the real provider's marker proves the reader has parsed its only startup output before injected screens
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let snapshot = owner
                    .command(gwt::project_runtime::events::OwnerCommand::Snapshot {
                        id: probe.id.clone(),
                        incarnation: probe.incarnation,
                    })
                    .await
                    .unwrap();
                if snapshot["screen"]
                    .as_str()
                    .is_some_and(|screen| screen.contains("GWT_PM_FIXTURE_READY"))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actual provider startup output");
    });
    (runtime, id)
}

fn runtime_state_hook_events(
    runtime: &mut AppRuntime,
    window_id: &str,
    status: &str,
    count: usize,
) -> Vec<OutboundEvent> {
    use gwt::project_runtime::events::OwnerCommand;
    let owner = project_owner_fixture(runtime, window_id);
    let probe = owner_runtime_probe(runtime, window_id);
    let session = probe.session.as_ref().unwrap();
    let mut hook = runtime_hook_state(status, &session.id);
    hook.project_root = Some(
        probe
            .grant
            .as_ref()
            .unwrap()
            .principal()
            .canonical_project_root()
            .display()
            .to_string(),
    );
    hook.branch = Some(session.branch.clone());
    hook.agent_session_id.clone_from(&session.agent_session_id);
    let mut source = owner.subscribe();
    owner_fixture_block_on(async {
        for _ in 0..count {
            hook.occurred_at = Utc::now().to_rfc3339();
            assert_eq!(
                probe.runtime_hook(&hook).await.unwrap(),
                reqwest::StatusCode::NO_CONTENT
            );
        }
        // The listener has accepted every hook before this ordinary mailbox ACK.
        owner.command(OwnerCommand::Health).await.unwrap();
    });
    relay_approval_owner_events(runtime, window_id, &mut source)
}

#[test]
fn app_runtime_runtime_state_bursts_emit_no_browser_hook_events() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());

    let browser_hook_events = runtime_state_hook_events(&mut runtime, &window_id, "Waiting", 1_000)
        .into_iter()
        .filter(|event| matches!(event.event, BackendEvent::RuntimeHookEvent { .. }))
        .count();

    assert_eq!(browser_hook_events, 0);
    // This existing Stop + Waiting payload is normalized to Idle by the owner.
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Idle)
    );
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn app_runtime_duplicate_runtime_state_hooks_emit_status_events_only_once() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());

    let events = runtime_state_hook_events(&mut runtime, &window_id, "Waiting", 1_000);

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
    close_test_owner_runtime(&mut runtime, &window_id);
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

    let restored = WindowRegistry::from_persisted(persisted);
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
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());
    runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);
    let prompt = b"\x1b[2J\x1b[HWould you like to run the following command?\r\n\r\n\
        cargo test -p gwt window_state\r\n\r\n\
        > 1. Yes, proceed\r\n\
          2. No, and tell Codex what to do differently\r\n\r\n\
        Press enter to confirm or esc to cancel\r\n";
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let probe = owner_runtime_probe(&runtime, &window_id);
    let mut source = owner.subscribe();
    let seq = {
        let mut pane = probe.pane.lock().unwrap();
        pane.process_bytes(prompt);
        pane.output_seq()
    };
    owner_fixture_block_on(
        owner.pane_event(gwt::pane_runtime::PaneRuntimeEvent::Output {
            id: probe.id,
            incarnation: probe.incarnation,
            data: prompt.to_vec(),
            seq,
            screen: None,
        }),
    )
    .unwrap();
    let events = relay_approval_owner_events(&mut runtime, &window_id, &mut source);

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Waiting,
            ..
        }
    )));
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_some());
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn app_runtime_provider_auth_notice_parks_the_live_owner_from_pty_output() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id, repo) = monitor_owned_codex_output_fixture(temp.path());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let output = b"\x1b[2J\x1b[H\xe2\x96\xa0 Your access token could not be refreshed because your refresh token has expired. Please log out and sign in again.\r\n";
    process_test_owner_output(&mut runtime, &window_id, output);
    let persisted = owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Observe the real Monitor failure transaction; elapsed time cannot establish handoff ownership.
        tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
            loop {
                let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                if prefs.autonomous_records.iter().any(|record| {
                    record.issue_number == 42 && record.phase == gwt::AutonomousPhase::NeedsHuman
                }) {
                    break prefs;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actual provider-auth NeedsHuman handoff")
    });
    assert!(
        persisted.launched_issues.is_empty(),
        "reauthentication releases the slot"
    );
    let record = persisted
        .autonomous_records
        .iter()
        .find(|record| record.issue_number == 42)
        .unwrap();
    assert_eq!(record.phase, gwt::AutonomousPhase::NeedsHuman);
    assert_eq!(record.retry_not_before, None);
    assert_eq!(record.retry_hold_provider, None);
    assert!(persisted.failed_issues[0]
        .message
        .contains("Codex reauthentication required"));
    assert!(
        persisted.provider_quota_holds.is_empty(),
        "auth never creates a quota hold"
    );
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .quota_hold
        .is_none());
    close_test_owner_runtime(&mut runtime, &window_id);
}

fn monitor_owned_codex_output_fixture(temp: &Path) -> (AppRuntime, String, PathBuf) {
    use gwt::project_runtime::events::OwnerCommand;
    let (root, owner, delivery) = owner_monitor_fixture(temp, 42, true, &["codex"]);
    configure_owner_fixture_tools(&root, &["codex"]);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        root.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp, vec![tab], Some("tab-1"));
    pin_live_pm_fixture_agents(&runtime, temp);
    let selected = pane_control_tests::control_launch_config(&root, 42);
    let worktree = selected.working_dir.unwrap();
    run_git(&worktree, &["branch", "-m", "work/issue-42"]);
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree)
        .branch("work/issue-42")
        .base_branch(selected.base_branch.unwrap())
        .build();
    config.linked_issue_number = Some(42);
    config.explicit_follow_up = true;
    let codex_home = temp.join(".codex");
    fs::create_dir_all(&codex_home).unwrap();
    config
        .env_vars
        .insert("CODEX_HOME".into(), codex_home.display().to_string());
    let probe = owner_monitor_launch_with_config(&root, &owner, 42, "agent-1", delivery, config);
    let snapshot = owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline The actual native provider's READY output orders later screen injection; this only bounds an OS/IPC hang.
        tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
            loop {
                let snapshot = owner
                    .command(OwnerCommand::Snapshot {
                        id: probe.id.clone(),
                        incarnation: probe.incarnation,
                    })
                    .await
                    .unwrap();
                if snapshot["screen"]
                    .as_str()
                    .is_some_and(|screen| screen.contains("GWT_PM_FIXTURE_READY"))
                {
                    break snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actual Monitor provider readiness")
    });
    assert_eq!(owner_monitor_read(&root, 42).active_count(), 1);
    assert!(owner_monitor_read(&root, 42).live_claim_id(42).is_some());
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
    let context = runtime.test_context();
    let client = owner.client();
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = client.identity().cloned();
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.install_remote_owner_projection(
        &context,
        &window_id,
        client,
        serde_json::from_value(snapshot).unwrap(),
        probe.session.clone(),
    );
    runtime
        .test_project_owners
        .insert(root.clone(), Arc::new(owner));
    (runtime, window_id, root)
}

#[test]
fn app_runtime_codex_directory_trust_prompt_escalates_only_monitor_owned_live_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id, repo) = monitor_owned_codex_output_fixture(temp.path());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let prompt = b"\x1b[2J\x1b[HYou are in /tmp/managed-worktree\r\n\r\n\
        Do you trust the contents of this directory? Working with untrusted contents comes with higher\r\n\
        risk of prompt injection. Trusting the directory allows project-local config, hooks, and exec\r\n\
        policies to load.\r\n\r\n\
        > 1. Yes, continue\r\n  2. No, quit\r\n\r\n  Press enter to continue\r\n";
    process_test_owner_output(&mut runtime, &window_id, prompt);
    let persisted = owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Observe the actual Monitor handoff transaction, not a guessed scheduling delay.
        tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
            loop {
                let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                if prefs.autonomous_records.iter().any(|record| {
                    record.issue_number == 42 && record.phase == gwt::AutonomousPhase::NeedsHuman
                }) {
                    break prefs;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actual directory-trust NeedsHuman handoff")
    });
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
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_none());
    close_test_owner_runtime(&mut runtime, &window_id);
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
    owner_runtime_probe(&runtime, &window_id)
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
    let root = temp.path().join("claude-project");
    fs::create_dir_all(&root).unwrap();
    init_repo_with_initial_commit(&root);
    let tab = sample_project_tab("tab-1", "Claude project", root, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
        .branch("work/approval-claude-fixture")
        .build();
    let window_id = spawn_owner_codex_fixture(&mut runtime, "tab-1", config, None);
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let probe = owner_runtime_probe(&runtime, &window_id);
    assert_eq!(probe.active_session.as_ref().unwrap().agent_id, "claude");
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline The native provider's real READY output orders the later injected screen; this only bounds an OS/IPC hang.
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let snapshot = owner
                    .command(gwt::project_runtime::events::OwnerCommand::Snapshot {
                        id: probe.id.clone(),
                        incarnation: probe.incarnation,
                    })
                    .await
                    .unwrap();
                if snapshot["screen"]
                    .as_str()
                    .is_some_and(|screen| screen.contains("GWT_PM_FIXTURE_READY"))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actual Claude startup output");
    });
    runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);
    let address = runtime.window_lookup[&window_id].clone();
    let saved = runtime
        .tab(&address.tab_id)
        .unwrap()
        .workspace
        .window(&address.raw_id)
        .unwrap();
    assert_eq!(saved.preset, WindowPreset::Agent);
    assert_eq!(saved.agent_id.as_deref(), Some("claude"));
    // The GUI's optional live-session cache is not the provider authority.
    runtime.active_agent_sessions.remove(&window_id);
    let prompt = b"\x1b[2J\x1b[HBash command\r\n\r\n  cargo test -p gwt\r\n\r\n\
        Do you want to proceed?\r\n\
        > 1. Yes\r\n\
          2. No, and tell Claude what to do differently\r\n\r\n\
        Enter to confirm \xc2\xb7 Esc to cancel\r\n";
    let mut source = owner.subscribe();
    let seq = {
        let mut pane = probe.pane.lock().unwrap();
        pane.process_bytes(prompt);
        pane.output_seq()
    };
    owner_fixture_block_on(
        owner.pane_event(gwt::pane_runtime::PaneRuntimeEvent::Output {
            id: probe.id,
            incarnation: probe.incarnation,
            data: prompt.to_vec(),
            seq,
            screen: None,
        }),
    )
    .unwrap();
    let events = relay_approval_owner_events(&mut runtime, &window_id, &mut source);

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Waiting,
            ..
        }
    )));
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_some());
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn app_runtime_terminal_navigation_and_submit_keep_wait_until_output_resolves_it() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());
    runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);
    let anchor = approval_owner_future_anchor();
    let prompt = b"Would you like to run the following command?\r\n\r\n\
        cargo test -p gwt\r\n\r\n\
        > 1. Yes, proceed\r\n\
          2. No, and tell Codex what to do differently\r\n\r\n\
        Press enter to confirm or esc to cancel\r\n";
    approval_owner_screen(&mut runtime, &window_id, prompt, anchor);

    assert!(runtime
        .terminal_input_events(&window_id, "\u{1b}[A")
        .is_empty());
    assert!(drain_browser_owner_input(&mut runtime).is_empty());
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_some());
    let partial_redraw = approval_owner_screen(&mut runtime, &window_id, b"\x1b[2J\x1b[H", anchor);
    assert!(partial_redraw.is_empty());
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_some());

    let submitted = runtime.terminal_input_events(&window_id, "1\r");

    assert!(submitted.is_empty());
    assert!(drain_browser_owner_input(&mut runtime).is_empty());
    let latch = owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .expect("approval remains latched until rendered output changes");
    assert_eq!(latch.resolving_fingerprint, latch.active_fingerprint);

    let pending = approval_owner_screen(&mut runtime, &window_id, b"", anchor);
    assert!(pending.is_empty());
    let due = owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval_settle_at
        .expect("owner settle deadline");
    approval_owner_screen(
        &mut runtime,
        &window_id,
        b"\x1b[2J\x1b[HWorking\r\n",
        anchor,
    );
    let cleared = approval_owner_tick(&mut runtime, &window_id, due);
    assert!(cleared.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_none());
    close_test_owner_runtime(&mut runtime, &window_id);
}

fn approval_owner_future_anchor() -> Instant {
    // test-hygiene: allow-wall-clock-deadline Inject a future monotonic anchor so the real actor cannot race the explicit deadline tick; no assertion waits for wall time.
    Instant::now() + Duration::from_secs(3600)
}

#[test]
fn app_runtime_approval_settle_same_prompt_cancels_resolution_without_frames() {
    let (_temp, _home, mut runtime, window_id, prompt) = approval_settle_runtime();
    let anchor = approval_owner_future_anchor();
    assert!(approval_owner_screen(&mut runtime, &window_id, b"\x1b[2J\x1b[H", anchor).is_empty());
    let due = owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval_settle_at
        .unwrap();
    let mut redraw = b"\x1b[2J\x1b[H".to_vec();
    redraw.extend(prompt);
    assert!(approval_owner_screen(&mut runtime, &window_id, &redraw, anchor).is_empty());
    let events = approval_owner_tick(&mut runtime, &window_id, due);
    let actual = owner_runtime_probe(&runtime, &window_id);
    close_test_owner_runtime(&mut runtime, &window_id);
    assert!(events.is_empty());
    assert!(!actual.policy.approval.unwrap().resolution_started);
    assert!(actual.policy.approval_settle_at.is_none());
    assert_eq!(actual.status, WindowProcessStatus::Waiting);
}

#[test]
fn app_runtime_approval_settle_stable_progress_clears_waiting() {
    let (_temp, _home, mut runtime, window_id, _prompt) = approval_settle_runtime();
    approval_owner_screen(
        &mut runtime,
        &window_id,
        b"\x1b[2J\x1b[HWorking\r\n",
        approval_owner_future_anchor(),
    );
    let due = owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval_settle_at
        .unwrap();
    let events = approval_owner_tick(&mut runtime, &window_id, due);
    let actual = owner_runtime_probe(&runtime, &window_id);
    assert_eq!(
        runtime.runtimes[&window_id].status,
        WindowProcessStatus::Running
    );
    close_test_owner_runtime(&mut runtime, &window_id);
    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(actual.policy.approval.is_none());
    assert!(actual.policy.approval_settle_at.is_none());
    assert_eq!(actual.status, WindowProcessStatus::Running);
}

#[test]
fn app_runtime_approval_settle_partial_provider_evidence_keeps_waiting() {
    let (_temp, _home, mut runtime, window_id, _prompt) = approval_settle_runtime();
    approval_owner_screen(
        &mut runtime,
        &window_id,
        b"\x1b[2J\x1b[HWould you like to run the following command?\r\n",
        approval_owner_future_anchor(),
    );
    let due = owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval_settle_at
        .unwrap();
    let events = approval_owner_tick(&mut runtime, &window_id, due);
    let actual = owner_runtime_probe(&runtime, &window_id);
    close_test_owner_runtime(&mut runtime, &window_id);
    assert!(events.is_empty());
    assert_eq!(actual.status, WindowProcessStatus::Waiting);
    assert!(actual.policy.approval.unwrap().resolution_started);
    assert!(
        actual.policy.approval_settle_at.is_none(),
        "partial evidence retains Waiting without continuously rearming an idle timer"
    );
}

#[test]
fn app_runtime_approval_settle_ignores_complete_prompt_in_scrollback_above_progress() {
    let (_temp, _home, mut runtime, window_id, _prompt) = approval_settle_runtime();
    approval_owner_screen(
        &mut runtime,
        &window_id,
        b"\r\nRunning tests...\r\n",
        approval_owner_future_anchor(),
    );
    let pane = owner_runtime_probe(&runtime, &window_id);
    assert!(pane
        .pane
        .lock()
        .unwrap()
        .screen()
        .contents()
        .contains("Would you like to run the following command?"));
    let due = pane.policy.approval_settle_at.unwrap();
    let events = approval_owner_tick(&mut runtime, &window_id, due);
    let actual = owner_runtime_probe(&runtime, &window_id);
    close_test_owner_runtime(&mut runtime, &window_id);
    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(actual.policy.approval.is_none());
    assert!(actual.policy.approval_settle_at.is_none());
}

#[test]
fn app_runtime_approval_settle_stale_token_is_noop() {
    let (_temp, _home, mut runtime, window_id, _prompt) = approval_settle_runtime();
    let anchor = approval_owner_future_anchor();
    approval_owner_screen(
        &mut runtime,
        &window_id,
        b"\x1b[2J\x1b[HWorking\r\n",
        anchor,
    );
    let before = owner_runtime_probe(&runtime, &window_id);
    assert!(runtime
        .handle_runtime_approval_settle(&window_id, u64::MAX)
        .is_empty());
    let owner = project_owner_fixture(&mut runtime, &window_id);
    assert!(owner_fixture_block_on(owner.observe_runtime_policy(
        &before.id,
        before.incarnation + 1,
        Utc::now(),
        anchor
    ))
    .is_err());
    let after = owner_runtime_probe(&runtime, &window_id);
    close_test_owner_runtime(&mut runtime, &window_id);
    assert_eq!(
        after.policy.approval_settle_at,
        before.policy.approval_settle_at
    );
    assert_eq!(
        after.policy.approval.unwrap().active_fingerprint,
        before.policy.approval.unwrap().active_fingerprint
    );
    assert!(after.policy.approval.unwrap().resolution_started);
}

#[test]
fn app_runtime_approval_settle_uses_owner_deadline_without_gui_token_event() {
    let (_temp, _home, mut runtime, window_id, _prompt) = approval_settle_runtime();
    approval_owner_screen(
        &mut runtime,
        &window_id,
        b"\x1b[2J\x1b[HWorking\r\n",
        approval_owner_future_anchor(),
    );
    let due = owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval_settle_at
        .unwrap();
    let events = approval_owner_tick(&mut runtime, &window_id, due);
    let actual = owner_runtime_probe(&runtime, &window_id);
    let AppEventProxy::Stub(recorded) = &runtime.proxy else {
        panic!("recording owner projection fixture")
    };
    assert!(!recorded.lock().unwrap().iter().any(|event| matches!(
        recorded_project_payload(event),
        UserEvent::RuntimeApprovalSettle { .. }
    )));
    close_test_owner_runtime(&mut runtime, &window_id);
    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert_eq!(actual.status, WindowProcessStatus::Running);
    assert!(actual.policy.approval.is_none());
}

#[test]
fn app_runtime_different_prompt_after_submit_emits_clear_then_waiting_reentry() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());
    runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);
    let anchor = approval_owner_future_anchor();
    let first = b"Would you like to run the following command?\r\n first-command\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    approval_owner_screen(&mut runtime, &window_id, first, anchor);
    let first_fingerprint = owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .unwrap()
        .active_fingerprint;
    runtime.terminal_input_events(&window_id, "1\r");
    drain_browser_owner_input(&mut runtime);
    assert!(
        owner_runtime_probe(&runtime, &window_id)
            .policy
            .approval
            .unwrap()
            .resolution_started
    );

    let second =
        b"\x1b[2J\x1b[HWould you like to run the following command?\r\n second-command\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    let events = approval_owner_screen(&mut runtime, &window_id, second, anchor);
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
    let next = owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .unwrap();
    assert_ne!(next.active_fingerprint, first_fingerprint);
    assert!(!next.resolution_started);
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn app_runtime_pane_send_marks_resolution_before_delayed_submit() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());
    runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);
    let prompt = b"\x1b[2J\x1b[HWould you like to run the following command?\r\n command\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    approval_owner_screen(
        &mut runtime,
        &window_id,
        prompt,
        approval_owner_future_anchor(),
    );
    let pane = owner_runtime_probe(&runtime, &window_id);
    let fingerprint = pane.policy.approval.unwrap().active_fingerprint;

    let queued =
        runtime.pane_send_input_to_window_events("client-1".to_string(), &window_id, "1\r");
    assert!(queued.is_empty());
    // The owner marks the exact prompt before dispatching the physical worker;
    // its typed ACK proves that the delayed submit also completed successfully.
    let events = drain_browser_owner_input(&mut runtime);

    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::PaneSendResult { ok: true, .. })));
    assert_eq!(
        owner_runtime_probe(&runtime, &window_id)
            .policy
            .approval
            .unwrap()
            .resolving_fingerprint,
        fingerprint
    );
    assert!(
        owner_runtime_probe(&runtime, &window_id)
            .policy
            .approval
            .unwrap()
            .resolution_started
    );
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn app_runtime_failed_input_paths_do_not_mark_approval_resolution() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());
    runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);
    let prompt = b"\x1b[2J\x1b[HWould you like to run the following command?\r\n command\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    approval_owner_screen(&mut runtime, &window_id, prompt, Instant::now());
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let before = owner_runtime_probe(&runtime, &window_id);
    let approval = before.policy.approval.expect("actual owner approval latch");
    assert!(!approval.resolution_started);
    let writer = before.writer.clone().expect("actual current writer");
    let input = before
        .pty
        .ordinary_input_observation()
        .map(|(revision, unsent, _)| (revision, unsent));
    let session = before.session.as_ref().unwrap();
    let session_path = runtime.sessions_dir.join(format!("{}.toml", session.id));
    let session_bytes = fs::read(&session_path).unwrap();
    // A stale GUI intent must be refused before touching the successor's
    // actual approval policy, writer, input or durable Session.
    runtime.runtimes.get_mut(&window_id).unwrap().incarnation = before.incarnation + 1;

    let terminal_events = runtime.terminal_input_events(&window_id, "1\r");
    let mut pane_events =
        runtime.pane_send_input_to_window_events("client-1".to_string(), &window_id, "1\r");

    assert!(terminal_events.is_empty());
    assert!(
        pane_events.is_empty(),
        "the RPC reply arrives through the typed completion"
    );
    pane_events.extend(drain_browser_owner_input(&mut runtime));
    assert!(pane_events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::PaneSendResult { ok: false, .. })));
    let after = owner_fixture_block_on(owner.probe(&before.id, before.incarnation)).unwrap();
    let latch = after
        .policy
        .approval
        .expect("actual owner approval remains");
    assert!(!latch.resolution_started);
    assert!(latch.resolving_fingerprint.is_none());
    assert_eq!(latch.active_fingerprint, approval.active_fingerprint);
    assert_eq!(
        after.policy.approval_settle_at,
        before.policy.approval_settle_at
    );
    assert_eq!(
        after
            .pty
            .ordinary_input_observation()
            .map(|(revision, unsent, _)| (revision, unsent)),
        input
    );
    assert!(after.current.load(std::sync::atomic::Ordering::Acquire));
    assert!(Arc::ptr_eq(after.writer.as_ref().unwrap(), &writer));
    assert_eq!(after.session.as_ref().unwrap().id, session.id);
    assert_eq!(fs::read(&session_path).unwrap(), session_bytes);
    assert!(after.pty.try_wait().unwrap().is_none());
    runtime.runtimes.get_mut(&window_id).unwrap().incarnation = before.incarnation;

    // Exercise only the current-writer guard on this still-live fixture. No
    // cancellation or lifecycle transition has occurred before restoration.
    writer
        .current
        .store(false, std::sync::atomic::Ordering::Release);
    assert!(runtime.terminal_input_events(&window_id, "1\r").is_empty());
    assert!(runtime
        .pane_send_input_to_window_events("client-1".to_string(), &window_id, "1\r")
        .is_empty());
    let expired_events = drain_browser_owner_input(&mut runtime);
    let expired = owner_fixture_block_on(owner.probe(&before.id, before.incarnation)).unwrap();
    let writer_was_current = expired.current.load(std::sync::atomic::Ordering::Acquire);
    writer
        .current
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(expired_events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::PaneSendResult { ok: false, .. })));
    let latch = expired
        .policy
        .approval
        .expect("approval remains after refused writer");
    assert!(!latch.resolution_started);
    assert!(latch.resolving_fingerprint.is_none());
    assert!(!writer_was_current);
    assert_eq!(latch.active_fingerprint, approval.active_fingerprint);
    assert_eq!(
        expired.policy.approval_settle_at,
        before.policy.approval_settle_at
    );
    assert!(Arc::ptr_eq(expired.writer.as_ref().unwrap(), &writer));
    assert_eq!(
        expired
            .pty
            .ordinary_input_observation()
            .map(|(revision, unsent, _)| (revision, unsent)),
        input
    );
    assert_eq!(expired.session.as_ref().unwrap().id, session.id);
    assert_eq!(fs::read(&session_path).unwrap(), session_bytes);
    assert!(expired.pty.try_wait().unwrap().is_none());
    close_test_owner_runtime(&mut runtime, &window_id);
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
    owner_runtime_probe(&runtime, &window_id)
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
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());
    runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
    let prompt = b"\x1b[2J\x1b[HWould you like to run the following command?\r\n command\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    approval_owner_screen(&mut runtime, &window_id, prompt, Instant::now());
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let probe = owner_runtime_probe(&runtime, &window_id);
    assert!(probe.policy.approval.is_some());
    probe
        .pane
        .lock()
        .expect("pane")
        .process_bytes(b"SECRET_TOKEN=/private/path command --token sentinel\r\n");

    let mut source = owner.subscribe();
    owner_fixture_block_on(
        owner.pane_event(gwt::pane_runtime::PaneRuntimeEvent::Status {
            id: probe.id.clone(),
            incarnation: probe.incarnation,
            status: WindowProcessStatus::Error,
            detail: Some("failed SECRET_TOKEN=/private/path".into()),
            exit_confirmed: false,
        }),
    )
    .unwrap();
    let events = relay_approval_owner_events(&mut runtime, &window_id, &mut source);
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
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_none());
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn app_runtime_approval_error_redacts_screen_before_output_event_sets_latch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let probe = owner_runtime_probe(&runtime, &window_id);
    let prompt = b"Would you like to run the following command?\r\n\
        SECRET_TOKEN=/private/path command --token sentinel\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    probe.pane.lock().expect("pane").process_bytes(prompt);
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_none());

    let mut source = owner.subscribe();
    owner_fixture_block_on(
        owner.pane_event(gwt::pane_runtime::PaneRuntimeEvent::Status {
            id: probe.id.clone(),
            incarnation: probe.incarnation,
            status: WindowProcessStatus::Error,
            detail: Some("failed SECRET_TOKEN=/private/path".into()),
            exit_confirmed: false,
        }),
    )
    .unwrap();
    let events = relay_approval_owner_events(&mut runtime, &window_id, &mut source);
    let serialized = format!("{events:?}");

    assert!(!serialized.contains("SECRET_TOKEN"));
    assert!(!serialized.contains("/private/path"));
    assert!(!serialized.contains("--token"));
    assert_eq!(
        runtime.window_details.get(&window_id).map(String::as_str),
        Some("Agent approval prompt ended unexpectedly")
    );
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_none());
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn app_runtime_progress_hook_clears_approval_wait_even_when_hook_state_is_unchanged() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());
    runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);
    let prompt = b"\x1b[2J\x1b[HWould you like to run the following command?\r\n command\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    approval_owner_screen(
        &mut runtime,
        &window_id,
        prompt,
        approval_owner_future_anchor(),
    );
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_some());

    let cleared = runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);

    assert!(cleared.iter().any(|event| matches!(
        event.event,
        BackendEvent::WindowState {
            state: WindowProcessStatus::Running,
            ..
        }
    )));
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_none());
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn app_runtime_terminal_state_clears_approval_wait_tracking() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());
    runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);
    let prompt = b"\x1b[2J\x1b[HWould you like to run the following command?\r\n command\r\n\
        > 1. Yes, proceed\r\n 2. No, and tell Codex what to do differently\r\n\
        Press enter to confirm or esc to cancel\r\n";
    approval_owner_screen(
        &mut runtime,
        &window_id,
        prompt,
        approval_owner_future_anchor(),
    );
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let probe = owner_runtime_probe(&runtime, &window_id);
    assert!(probe.policy.approval.is_some());

    let mut source = owner.subscribe();
    owner_fixture_block_on(
        owner.pane_event(gwt::pane_runtime::PaneRuntimeEvent::Status {
            id: probe.id.clone(),
            incarnation: probe.incarnation,
            status: WindowProcessStatus::Stopped,
            detail: None,
            exit_confirmed: false,
        }),
    )
    .unwrap();
    let events = relay_approval_owner_events(&mut runtime, &window_id, &mut source);

    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Stopped,
            ..
        }
    )));
    assert!(!runtime.window_approval_waiting.contains_key(&window_id));
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .approval
        .is_none());
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn app_runtime_runtime_state_change_after_duplicate_burst_emits_status_events() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());

    let first_events = runtime_state_hook_events(&mut runtime, &window_id, "Waiting", 1);
    let duplicate_events = runtime_state_hook_events(&mut runtime, &window_id, "Waiting", 1);
    let changed_events = runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);

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
    close_test_owner_runtime(&mut runtime, &window_id);
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
    runtime.register_window("tab-1", &window.id);
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let context = runtime.test_context();
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = owner.client().identity().cloned();
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.start_window("tab-1", &window.id, window.preset);
    let events = drain_owner_launch_fixtures(&mut runtime, &tasks);

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::WindowState { window_id: id, state }
            if id == &window_id && *state == WindowProcessStatus::Running
    )));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalStatus { id, status, detail, .. }
            if id == &window_id
                && *status == WindowProcessStatus::Running
                && detail.is_none()
    )));
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(
        owner_runtime_probe(&runtime, &window_id).writer.is_some(),
        "only the physical owner retains the current PTY writer"
    );
    assert!(owner_runtime_probe(&runtime, &window_id)
        .pty
        .try_wait()
        .unwrap()
        .is_none());

    close_test_owner_runtime(&mut runtime, &window_id);
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
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let before = owner_runtime_probe(&runtime, &window_id);
    let context = runtime.test_context();
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = owner.client().identity().cloned();
    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;

    let mut events = runtime.stop_window_events(&window_id);
    events.extend(drain_browser_owner_input(&mut runtime));

    // The physical runtime and writer are gone; the GUI retains its stopped cache.
    assert!(owner_fixture_block_on(owner.probe(&before.id, before.incarnation)).is_err());
    assert!(before.pty.tree_exit_confirmed());
    assert!(!before
        .writer
        .unwrap()
        .current
        .load(std::sync::atomic::Ordering::Acquire));
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

// Issue #3366 — `log_entry_appended` is consumed only by Logs window
// state. `LoadLogs` re-reads the log directory on mount, so suppressing
// the live stream while no Logs window exists loses nothing.
#[test]
fn log_entry_events_drop_stream_without_logs_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    // A Board window alone must not subscribe the tracing log stream.
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::Shell, WindowPreset::Board],
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
    let window_id = combined_window_id("tab-1", &raw_ids[0]);
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let before = raw_ids
        .iter()
        .map(|raw| owner_runtime_probe(&runtime, &combined_window_id("tab-1", raw)))
        .collect::<Vec<_>>();
    let context = runtime.test_context();
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = owner.client().identity().cloned();
    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;

    runtime.stop_all_windows_events(&runtime.test_context());
    drain_browser_owner_input(&mut runtime);

    for (raw, probe) in raw_ids.iter().zip(before) {
        let window_id = combined_window_id("tab-1", raw);
        assert!(
            owner_fixture_block_on(owner.probe(&probe.id, probe.incarnation)).is_err(),
            "every agent runtime must be torn down"
        );
        assert!(probe.pty.tree_exit_confirmed());
        assert!(!probe
            .writer
            .unwrap()
            .current
            .load(std::sync::atomic::Ordering::Acquire));
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
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let context = runtime.test_context();
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = owner.client().identity().cloned();
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    let mut events = runtime.restart_window_events(&window_id);
    events.extend(drain_owner_launch_fixtures(&mut runtime, &tasks));

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
    let actual = owner_runtime_probe(&runtime, &window_id);
    assert!(actual.pty.try_wait().unwrap().is_none());
    assert!(actual
        .writer
        .unwrap()
        .current
        .load(std::sync::atomic::Ordering::Acquire));

    close_test_owner_runtime(&mut runtime, &window_id);
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
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let mut source = owner.subscribe();
    let (command, args) = if cfg!(windows) {
        ("powershell".into(), vec!["-NoProfile".into(), "-Command".into(),
            "[Console]::WriteLine('GWT_MANAGED_EXIT_READY'); $null = [Console]::ReadLine(); exit 0".into()])
    } else {
        (
            "/bin/sh".into(),
            vec![
                "-c".into(),
                "stty -echo; printf 'GWT_MANAGED_EXIT_READY\\n'; IFS= read -r line; exit 0".into(),
            ],
        )
    };
    insert_test_pane_runtime_with_command(&mut runtime, &window_id, command, args, 80, 24);
    let before = owner_runtime_probe(&runtime, &window_id);
    let context = runtime.test_context();
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = owner.client().identity().cloned();
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    #[cfg(windows)]
    owner_fixture_block_on(async {
        use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};
        // This plain Shell has no frontend terminal to answer ConPTY's
        // startup query. Use the actual query to order the normal response.
        let mut output = Vec::new();
        // test-hygiene: allow-wall-clock-deadline Bounds the real terminal handshake; raw CPR and READY establish ordering.
        tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
            loop {
                match source.recv().await.expect("actual Shell startup event") {
                    OwnerEvent::Output {
                        id,
                        incarnation,
                        data,
                        ..
                    } if id == before.id && incarnation == before.incarnation => {
                        output.extend(data);
                        if output.windows(4).any(|bytes| bytes == b"\x1b[6n") {
                            break;
                        }
                    }
                    OwnerEvent::Status {
                        id,
                        incarnation,
                        exit_confirmed: true,
                        detail,
                        ..
                    } if id == before.id && incarnation == before.incarnation => {
                        panic!("actual Shell exited before its terminal handshake: {detail:?}");
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("actual Shell cursor-position query");
        owner
            .command(OwnerCommand::Input {
                id: before.id.clone(),
                incarnation: before.incarnation,
                data: b"\x1b[1;1R".to_vec(),
            })
            .await
            .expect("normal terminal cursor-position response");
    });
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline The real READY marker orders input after Shell startup; elapsed time only bounds an OS/IPC hang.
        tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
            loop {
                let snapshot = owner
                    .command(gwt::project_runtime::events::OwnerCommand::Snapshot {
                        id: before.id.clone(),
                        incarnation: before.incarnation,
                    })
                    .await
                    .unwrap();
                if snapshot["screen"]
                    .as_str()
                    .is_some_and(|screen| screen.contains("GWT_MANAGED_EXIT_READY"))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actual Shell startup output");
    });

    super::managed_launch_tests::finish_managed_exit(&owner, &before, &mut runtime, &mut source);
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Stopped),
        "natural teardown must leave the stored terminal status readable"
    );

    let mut events = runtime.restart_window_events(&window_id);
    events.extend(drain_owner_launch_fixtures(&mut runtime, &tasks));

    assert!(
        !events.is_empty(),
        "the restart guard must admit natural exits"
    );
    assert_eq!(
        runtime.window_status(&window_id),
        Some(WindowProcessStatus::Running)
    );
    let after = owner_runtime_probe(&runtime, &window_id);
    assert!(after.incarnation > before.incarnation);
    assert!(after.pty.try_wait().unwrap().is_none());
    close_test_owner_runtime(&mut runtime, &window_id);
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
    let original_pane = Some(Arc::as_ptr(&owner_runtime_probe(&runtime, &window_id).pane) as usize)
        .expect("runtime present");

    let events = runtime.restart_window_events(&window_id);

    assert!(events.is_empty(), "restarting a running window is a no-op");
    let after_pane = Some(Arc::as_ptr(&owner_runtime_probe(&runtime, &window_id).pane) as usize)
        .expect("runtime still present");
    assert_eq!(
        original_pane, after_pane,
        "running window's runtime must not be replaced"
    );

    runtime.stop_window_runtime(&window_id);
}

#[test]
fn account_switch_releases_pane_quota_without_relatching_the_old_notice() {
    use gwt::project_runtime::events::OwnerEvent;
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, window_id) = runtime_state_hook_fixture(temp.path());
    runtime_state_hook_events(&mut runtime, &window_id, "Running", 1);
    let owner = project_owner_fixture(&mut runtime, &window_id);
    let probe = owner_runtime_probe(&runtime, &window_id);
    let mut old = codex_usage_account(100.0, true);
    old.account_id = Some("account-a".into());
    runtime.set_provider_usage_accounts(vec![old]);
    drain_browser_owner_input(&mut runtime);
    probe
        .pane
        .lock()
        .unwrap()
        .process_bytes(format!("\x1b[2J\x1b[H{CODEX_USAGE_LIMIT_SCREEN}").as_bytes());
    owner_fixture_block_on(owner.observe_runtime_policy(
        &probe.id,
        probe.incarnation,
        instant("2026-09-02T09:00:00Z"),
        approval_owner_future_anchor(),
    ))
    .unwrap();
    assert!(owner_runtime_probe(&runtime, &window_id)
        .policy
        .quota_hold
        .is_some());
    let mut new = gwt_core::usage::ProviderUsage::degraded(
        gwt_core::usage::UsageProvider::Codex,
        gwt_core::usage::UsageState::NoData,
    );
    new.account_id = Some("account-b".into());
    runtime.handle_provider_usage_snapshot(vec![new], instant("2026-09-02T09:02:00Z"));
    drain_browser_owner_input(&mut runtime);
    let released = owner_runtime_probe(&runtime, &window_id).policy;
    assert!(released.quota_hold.is_none());
    assert!(released.released_quota_notice.is_some());
    for at in ["2026-09-02T09:03:00Z", "2026-09-02T09:05:00Z"] {
        owner_fixture_block_on(owner.observe_runtime_policy(
            &probe.id,
            probe.incarnation,
            instant(at),
            approval_owner_future_anchor(),
        ))
        .unwrap();
        assert!(owner_runtime_probe(&runtime, &window_id)
            .policy
            .quota_hold
            .is_none());
    }
    // Keep the old notice on the final screen and obtain a real OS exit
    // receipt before checking the exit classifier's no-relatch behavior.
    let mut source = owner.subscribe();
    probe.pty.kill().unwrap();
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bounds the actual child exit receipt; no elapsed time grants terminal authority.
        tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
            loop {
                match source.recv().await.unwrap() {
                    OwnerEvent::Status {
                        id,
                        incarnation,
                        exit_confirmed: true,
                        ..
                    }
                    | OwnerEvent::Closed {
                        id,
                        incarnation,
                        error: None,
                    } if id == probe.id && incarnation == probe.incarnation => break,
                    _ => {}
                }
            }
        })
        .await
        .expect("exact actual owner terminal receipt");
    });
    if let Ok(final_runtime) = owner_fixture_block_on(owner.probe(&probe.id, probe.incarnation)) {
        assert!(final_runtime.policy.quota_hold.is_none());
        close_test_owner_runtime(&mut runtime, &window_id);
    }
    assert!(probe.pty.tree_exit_confirmed());
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
