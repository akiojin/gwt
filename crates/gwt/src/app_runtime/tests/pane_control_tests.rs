use super::*;

#[test]
fn agent_pane_input_with_equal_session_ids_targets_authenticated_project_only() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let foreign_project = temp.path().join("foreign-project");
    let authenticated_project = temp.path().join("authenticated-project");
    fs::create_dir_all(&foreign_project).expect("foreign project");
    fs::create_dir_all(&authenticated_project).expect("authenticated project");

    let mut foreign_tab = sample_project_tab_with_window_at(
        "tab-foreign",
        "agent-foreign",
        foreign_project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(foreign_tab
        .workspace
        .set_session_id("agent-foreign", Some("shared-session".to_string())));
    let mut authenticated_tab = sample_project_tab_with_window_at(
        "tab-authenticated",
        "agent-authenticated",
        authenticated_project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(authenticated_tab
        .workspace
        .set_session_id("agent-authenticated", Some("shared-session".to_string())));
    // Keep the foreign tab first so a process-global session lookup would
    // deterministically select the wrong pane.
    let (mut runtime, _) = sample_runtime_with_events(
        temp.path(),
        vec![foreign_tab, authenticated_tab],
        Some("tab-foreign"),
    );
    let principal = AgentSessionPrincipal::for_test(&authenticated_project, "shared-session")
        .expect("authenticated principal");

    let events = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal,
        AgentFrontendRequest::SendInput {
            text: "status\r".to_string(),
        },
    );

    assert!(matches!(
        events.as_slice(),
        [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::PaneSendResult {
                ok: false,
                window_id: Some(window_id),
                error: Some(error),
            },
            ..
        }] if client_id == "pane-client"
            && window_id == "tab-authenticated::agent-authenticated"
            && error.contains("no live runtime")
    ));
}

#[test]
fn agent_pane_close_rejects_window_from_foreign_project() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let foreign_project = temp.path().join("foreign-project");
    let authenticated_project = temp.path().join("authenticated-project");
    fs::create_dir_all(&foreign_project).expect("foreign project");
    fs::create_dir_all(&authenticated_project).expect("authenticated project");
    let foreign_tab = sample_project_tab_with_window_at(
        "tab-foreign",
        "agent-foreign",
        foreign_project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let authenticated_tab = sample_project_tab_with_window_at(
        "tab-authenticated",
        "agent-authenticated",
        authenticated_project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _) = sample_runtime_with_events(
        temp.path(),
        vec![foreign_tab, authenticated_tab],
        Some("tab-authenticated"),
    );
    let principal =
        AgentSessionPrincipal::for_test(&authenticated_project, "session-authenticated")
            .expect("authenticated principal");

    let denied = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal.clone(),
        AgentFrontendRequest::CloseWindow {
            id: "tab-foreign::agent-foreign".to_string(),
            request_id: None,
            responder: None,
        },
    );

    // Issue #3629 AC-12: the refusal answers explicitly instead of silence;
    // the foreign window itself must survive untouched.
    assert!(denied.iter().all(|outbound| matches!(
        outbound.event,
        BackendEvent::PaneCloseResult { ok: false, .. }
    )));
    assert!(runtime
        .window_lookup
        .contains_key("tab-foreign::agent-foreign"));

    let _accepted = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal,
        AgentFrontendRequest::CloseWindow {
            id: "tab-authenticated::agent-authenticated".to_string(),
            request_id: None,
            responder: None,
        },
    );
    assert!(!runtime
        .window_lookup
        .contains_key("tab-authenticated::agent-authenticated"));
    assert!(runtime
        .window_lookup
        .contains_key("tab-foreign::agent-foreign"));
}

/// Issue #3629 AC-10/AC-12: every agent-route close outcome must answer the
/// requesting client with an explicit `pane_close_result` instead of the
/// silent empty event list that made `pane.close` undiagnosable.
#[test]
fn agent_pane_close_replies_with_pane_close_result() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let foreign_project = temp.path().join("foreign-project");
    let authenticated_project = temp.path().join("authenticated-project");
    fs::create_dir_all(&foreign_project).expect("foreign project");
    fs::create_dir_all(&authenticated_project).expect("authenticated project");
    let foreign_tab = sample_project_tab_with_window_at(
        "tab-foreign",
        "agent-foreign",
        foreign_project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let authenticated_tab = sample_project_tab_with_window_at(
        "tab-authenticated",
        "agent-authenticated",
        authenticated_project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _) = sample_runtime_with_events(
        temp.path(),
        vec![foreign_tab, authenticated_tab],
        Some("tab-authenticated"),
    );
    let principal =
        AgentSessionPrincipal::for_test(&authenticated_project, "session-authenticated")
            .expect("authenticated principal");

    let denied = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal.clone(),
        AgentFrontendRequest::CloseWindow {
            id: "tab-foreign::agent-foreign".to_string(),
            request_id: None,
            responder: None,
        },
    );
    assert!(
        matches!(
            denied.as_slice(),
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::PaneCloseResult {
                    ok: false,
                    window_id,
                    reason: Some(_),
                },
                ..
            }] if client_id == "pane-client" && window_id == "tab-foreign::agent-foreign"
        ),
        "a cross-project close refusal must answer explicitly, got: {denied:?}"
    );
    assert!(runtime
        .window_lookup
        .contains_key("tab-foreign::agent-foreign"));

    let accepted = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal,
        AgentFrontendRequest::CloseWindow {
            id: "tab-authenticated::agent-authenticated".to_string(),
            request_id: None,
            responder: None,
        },
    );
    assert!(
        matches!(
            accepted.first(),
            Some(OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::PaneCloseResult {
                    ok: true,
                    window_id,
                    reason: None,
                },
                ..
            }) if client_id == "pane-client" && window_id == "tab-authenticated::agent-authenticated"
        ),
        "a successful close must answer the requesting client first, got: {accepted:?}"
    );
    assert!(!runtime
        .window_lookup
        .contains_key("tab-authenticated::agent-authenticated"));
}

#[test]
fn recover_restored_window_protects_replacement_and_normal_launches() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut session = gwt_agent::Session::new(&project, "work/test", gwt_agent::AgentId::Codex);
    tab.workspace
        .set_session_id("agent-1", Some(session.id.clone()));
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    session.save(&runtime.sessions_dir).expect("save session");
    let principal = AgentSessionPrincipal::for_test(&project, "pm-session").expect("principal");
    let window_id = "tab-1::agent-1";
    insert_test_pane_runtime(&mut runtime, window_id);
    let child_pid = runtime.runtimes[window_id]
        .pane
        .lock()
        .expect("pane")
        .pty()
        .process_id()
        .expect("child pid");
    let child_started_at = gwt::process::host_process_start_time(child_pid).expect("child start");
    let request = |session_id: &str| AgentFrontendRequest::RecoverRestoredWindow {
        id: window_id.to_string(),
        session_id: session_id.to_string(),
        child_pid,
        child_started_at,
    };
    let assert_refused = |events: Vec<OutboundEvent>, expected: &str| {
        assert!(
            matches!(events.as_slice(), [OutboundEvent {
            target: DispatchTarget::Client(client_id),
            event: BackendEvent::PaneCloseResult { ok: false, reason: Some(reason), .. },
            ..
        }] if client_id == "pane-client" && reason.contains(expected)),
            "{events:?}"
        );
    };

    // A window id may now identify a successor of the snapshotted Session.
    assert_refused(
        runtime.handle_agent_frontend_event(
            "pane-client".to_string(),
            principal.clone(),
            request("replaced-session"),
        ),
        "session changed",
    );
    for origin in [
        gwt_agent::SessionLaunchOrigin::Launch,
        gwt_agent::SessionLaunchOrigin::UserRestart,
        gwt_agent::SessionLaunchOrigin::Unknown,
    ] {
        session.launch_origin = origin;
        session
            .save(&runtime.sessions_dir)
            .expect("save non-restore origin");
        assert_refused(
            runtime.handle_agent_frontend_event(
                "pane-client".to_string(),
                principal.clone(),
                request(&session.id),
            ),
            "automatic restore",
        );
    }
    assert!(runtime.window_lookup.contains_key(window_id));

    session.launch_origin = gwt_agent::SessionLaunchOrigin::AutomaticRestore;
    session
        .save(&runtime.sessions_dir)
        .expect("save incomplete restore provenance");
    assert_refused(
        runtime.handle_agent_frontend_event(
            "pane-client".to_string(),
            principal.clone(),
            request(&session.id),
        ),
        "automatic restore",
    );
    session.restore_source_session_id = Some("source-session".to_string());
    session
        .save(&runtime.sessions_dir)
        .expect("save restore provenance");
    assert_refused(
        runtime.handle_agent_frontend_event(
            "pane-client".to_string(),
            principal.clone(),
            AgentFrontendRequest::RecoverRestoredWindow {
                id: window_id.to_string(),
                session_id: session.id.clone(),
                child_pid,
                child_started_at: child_started_at + 1,
            },
        ),
        "process changed",
    );
    let foreign = temp.path().join("foreign");
    fs::create_dir_all(&foreign).expect("foreign project");
    assert_refused(
        runtime.handle_agent_frontend_event(
            "pane-client".to_string(),
            AgentSessionPrincipal::for_test(&foreign, "foreign-pm").expect("foreign principal"),
            request(&session.id),
        ),
        "project scope",
    );
    assert_refused(
        runtime.handle_agent_frontend_event(
            "pane-client".to_string(),
            AgentSessionPrincipal::for_test(&project, &session.id).expect("self principal"),
            request(&session.id),
        ),
        "correlated acceptance",
    );
    assert!(runtime.window_lookup.contains_key(window_id));

    let events = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal,
        request(&session.id),
    );
    assert!(
        matches!(
            events.first(),
            Some(OutboundEvent {
                event: BackendEvent::PaneCloseResult {
                    ok: true,
                    reason: None,
                    ..
                },
                ..
            })
        ),
        "{events:?}"
    );
    assert!(!runtime.window_lookup.contains_key(window_id));
}

/// Issue #3705 AC-1/AC-2: consecutive close of live-PTY panes must keep the
/// agent pane route answering. The hang was the GUI event loop joining PTY
/// reader threads and waiting for child exit on each close; `pane.list` then
/// sat behind that queue until the websocket timed out.
#[test]
fn consecutive_live_pty_pane_closes_keep_agent_route_responsive() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let tab = sample_project_tab(
        "tab-project",
        "Repo",
        project.clone(),
        ProjectKind::Git,
        &[
            WindowPreset::Agent,
            WindowPreset::Agent,
            WindowPreset::Agent,
            WindowPreset::Agent,
        ],
    );
    let window_ids: Vec<String> = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| combined_window_id("tab-project", &window.id))
        .collect();
    assert_eq!(window_ids.len(), 4, "AC-1 requires four live panes");
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let mut children = Vec::new();
    let mut worker_releases = Vec::new();
    let (worker_done, workers_done) = mpsc::channel();
    for window_id in &window_ids {
        let pane = long_running_test_pane(window_id);
        children.push(TestPaneGuard::new(&pane));
        let pane = Arc::new(Mutex::new(pane));
        let mut window_runtime =
            WindowRuntime::new(super::super::next_window_runtime_incarnation(), pane);
        for worker in [
            &mut window_runtime.output_thread,
            &mut window_runtime.status_thread,
        ] {
            let (release, released) = mpsc::channel::<()>();
            worker_releases.push(release);
            let done = worker_done.clone();
            *worker = Some(thread::spawn(move || {
                // Stay blocked during close, but release on both success and panic.
                let _ = released.recv();
                let _ = done.send(());
            }));
        }
        runtime.runtimes.insert(window_id.clone(), window_runtime);
    }
    let principal = AgentSessionPrincipal::for_test(&project, "session-pm").expect("pm principal");

    let started = Instant::now();
    for window_id in &window_ids {
        let closed = runtime.handle_agent_frontend_event(
            "pane-client".to_string(),
            principal.clone(),
            AgentFrontendRequest::CloseWindow {
                id: window_id.clone(),
                request_id: None,
                responder: None,
            },
        );
        assert!(
            matches!(
                closed.first(),
                Some(OutboundEvent {
                    event: BackendEvent::PaneCloseResult { ok: true, .. },
                    ..
                })
            ),
            "live PTY close must answer ok, got: {closed:?}"
        );
        assert!(
            !runtime.window_lookup.contains_key(window_id),
            "closed window {window_id} must leave the lookup"
        );
    }
    let listed = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal,
        AgentFrontendRequest::ListWindows,
    );
    assert!(
        !listed.is_empty(),
        "pane.list must still answer after consecutive live PTY closes"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(400),
        "four live-PTY closes plus pane.list blocked the agent route for {elapsed:?}"
    );
    drop(worker_releases);
    for _ in 0..window_ids.len() * 2 {
        workers_done
            .recv_timeout(Duration::from_secs(20))
            .expect("close fixture worker exited");
    }
    drop(children);
}

/// Issue #3783 AC-2/AC-3/AC-4: accepting a pane close is an in-memory
/// lifecycle transition. PTY termination and durable Work/Session cleanup
/// belong to independent background finalizers, so neither a large projection
/// nor two consecutive closes can delay the pane bridge acknowledgement.
#[test]
fn consecutive_agent_pane_closes_queue_finalizers_before_disk_projection_work() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab(
        "tab-project",
        "Repo",
        project.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent, WindowPreset::Agent],
    );
    let window_ids = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| combined_window_id("tab-project", &window.id))
        .collect::<Vec<_>>();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let mut ptys = Vec::new();
    for (index, window_id) in window_ids.iter().enumerate() {
        insert_test_pane_runtime(&mut runtime, window_id);
        ptys.push(
            runtime
                .runtimes
                .get(window_id)
                .expect("live runtime")
                .pty
                .clone(),
        );
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: format!("session-close-finalizer-{index}"),
                agent_id: "codex".to_string(),
                branch_name: format!("work/close-finalizer-{index}"),
                display_name: "Codex".to_string(),
                worktree_path: project.clone(),
                agent_project_root: project.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-project".to_string(),
            },
        );
    }
    let principal = AgentSessionPrincipal::for_test(&project, "session-pm").expect("pm principal");
    // Issue #3777 AC-3: `active_work_projection_for_tab` no longer decodes the
    // projection inline — it schedules the background prepare and returns
    // `None`, so the cache this test needs is only populated once that task has
    // run and its completion has been committed. The spawner here is queued, so
    // drive both explicitly rather than expecting a synchronous value.
    assert!(
        runtime
            .active_work_projection_for_tab("tab-project", &runtime.tabs[0])
            .is_none(),
        "the tab accessor must schedule the rebuild instead of decoding inline"
    );
    let queued = std::mem::take(
        &mut *tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    for task in queued {
        task();
    }
    let initial_projection = wait_for_active_work_projection(&mut runtime);
    assert_eq!(initial_projection.active_agents, window_ids.len());

    super::super::workspace_views::reset_full_active_work_projection_builds();
    let started = Instant::now();
    let mut final_projection = None;
    for window_id in &window_ids {
        let events = runtime.handle_agent_frontend_event(
            "pane-client".to_string(),
            principal.clone(),
            AgentFrontendRequest::CloseWindow {
                id: window_id.clone(),
                request_id: None,
                responder: None,
            },
        );
        assert!(matches!(
            events.first(),
            Some(OutboundEvent {
                event: BackendEvent::PaneCloseResult { ok: true, .. },
                ..
            })
        ));
        final_projection = events.iter().find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjectionPatch { projection } => Some(projection.clone()),
            _ => None,
        });
        assert!(!runtime.window_lookup.contains_key(window_id));
        assert!(!runtime.runtimes.contains_key(window_id));
    }
    assert!(!runtime
        .handle_agent_frontend_event(
            "pane-client".to_string(),
            principal,
            AgentFrontendRequest::ListWindows,
        )
        .is_empty());
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "two close acknowledgements plus pane.list must stay below the bridge budget"
    );
    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0,
        "the close hot path must never enter the disk-backed projection builder"
    );
    assert_eq!(
        final_projection
            .as_deref()
            .expect("cache-only close projection")
            .active_agents,
        0,
        "the cache-only acknowledgement must not replay a closed agent as active"
    );
    assert_eq!(
        tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        window_ids.len(),
        "each accepted close owns one independent background finalizer"
    );
    assert!(
        ptys.iter()
            .all(|pty| pty.try_wait().expect("probe child").is_none()),
        "PTY teardown must not run before the queued finalizer"
    );

    let queued = {
        let mut tasks = tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks.drain(..).collect::<Vec<_>>()
    };
    for task in queued {
        task();
    }
    assert!(
        ptys.iter()
            .all(|pty| pty.try_wait().expect("probe reaped child").is_some()),
        "every finalizer must terminate and reap its detached PTY"
    );
}

/// Issue #3783: closing a whole project tab is the same accepted lifecycle
/// transition as closing its panes one by one. The Tao thread removes the tab
/// and queues one finalizer per live runtime without killing or waiting on a
/// PTY inline.
#[test]
fn close_project_tab_queues_all_window_finalizers_before_pty_teardown() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab(
        "tab-project",
        "Repo",
        project.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent, WindowPreset::Agent],
    );
    let raw_window_ids = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| window.id.clone())
        .collect::<Vec<_>>();
    let window_ids = raw_window_ids
        .iter()
        .map(|raw_id| combined_window_id("tab-project", raw_id))
        .collect::<Vec<_>>();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let mut ptys = Vec::new();
    for (index, (raw_id, window_id)) in raw_window_ids.iter().zip(&window_ids).enumerate() {
        runtime.register_window("tab-project", raw_id);
        insert_test_pane_runtime(&mut runtime, window_id);
        ptys.push(
            runtime
                .runtimes
                .get(window_id)
                .expect("live runtime")
                .pty
                .clone(),
        );
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: format!("session-project-close-{index}"),
                agent_id: "codex".to_string(),
                branch_name: format!("work/project-close-{index}"),
                display_name: "Codex".to_string(),
                worktree_path: project.clone(),
                agent_project_root: project.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-project".to_string(),
            },
        );
    }

    let events = runtime.close_project_tab_events("tab-project");

    assert!(
        !events.is_empty(),
        "the accepted tab close is broadcast immediately"
    );
    assert!(
        runtime.tabs.is_empty(),
        "the tab is removed at the ACK boundary"
    );
    assert!(
        runtime.runtimes.is_empty(),
        "runtime ownership moves into finalizers"
    );
    assert_eq!(
        finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        window_ids.len(),
        "each live pane owns one queued finalizer"
    );
    assert!(
        ptys.iter()
            .all(|pty| pty.try_wait().expect("probe child").is_none()),
        "project close must not kill PTYs on the Tao thread"
    );

    let queued = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .drain(..)
        .collect::<Vec<_>>();
    for task in queued {
        task();
    }
    assert!(
        ptys.iter()
            .all(|pty| pty.try_wait().expect("probe reaped child").is_some()),
        "queued project-close finalizers terminate and reap every PTY"
    );
    assert!(
        runtime
            .window_lifecycle_generations
            .lock()
            .expect("window lifecycle generations")
            .is_empty(),
        "completed finalizers settle every closed window generation"
    );
}

/// Issue #4234 AC-3: closing a pane must release every buffer keyed by that
/// pane. Before this, an ordinary close dropped the runtime and the session but
/// left ~20 window-keyed maps holding an entry per window for the life of the
/// process — and because window ids are reassigned lowest-free, a later window
/// with the same id would read that residue as its own state.
#[test]
fn closing_a_window_releases_every_window_scoped_buffer() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-1",
        project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let window_id = "tab-project::agent-1".to_string();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let (spawner, _finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    insert_test_pane_runtime(&mut runtime, &window_id);
    seed_window_scoped_state(&mut runtime, &window_id);
    assert!(
        !runtime.window_scoped_state_residue(&window_id).is_empty(),
        "the fixture must actually populate window-scoped state"
    );

    assert!(runtime.close_window_outcome(&window_id).closed);

    let residue = runtime.window_scoped_state_residue(&window_id);
    assert!(
        residue.is_empty(),
        "closing {window_id} left window-scoped state behind: {residue:?}"
    );
}

/// Issue #4234 AC-4: opening and closing panes N times returns the window-keyed
/// state to its starting level instead of growing once per window.
#[test]
fn opening_and_closing_windows_returns_window_scoped_state_to_baseline() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-1",
        project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let window_id = "tab-project::agent-1".to_string();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let (spawner, _finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    // The fixture tab already carries `agent-1`; close it so the loop below
    // starts from an empty canvas and every round opens its own window.
    assert!(runtime.close_window_outcome(&window_id).closed);

    let baseline = runtime.window_scoped_state_entry_count();
    let mut opened = Vec::new();
    for _ in 0..8 {
        let raw_id = runtime
            .tab_mut("tab-project")
            .expect("tab")
            .workspace
            .add_window(WindowPreset::Agent, canvas_bounds())
            .id;
        let id = combined_window_id("tab-project", &raw_id);
        runtime.register_window("tab-project", &raw_id);
        insert_test_pane_runtime(&mut runtime, &id);
        seed_window_scoped_state(&mut runtime, &id);
        assert!(runtime.close_window_outcome(&id).closed);
        opened.push(id);
    }

    let residue = opened
        .iter()
        .flat_map(|id| {
            runtime
                .window_scoped_state_residue(id)
                .into_iter()
                .map(move |field| format!("{id}:{field}"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        runtime.window_scoped_state_entry_count(),
        baseline,
        "window-scoped state grew across 8 open/close rounds; residue: {residue:?}"
    );
}

/// Issue #3783 generation fence: a queued predecessor close may finish after
/// the canvas has reused the same public window id for a successor. The old
/// finalizer owns only its captured PTY and must not deregister or revoke the
/// successor writer generation.
#[test]
fn queued_close_finalizer_preserves_same_window_successor_writer_generation() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-1",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let window_id = "tab-project::agent-1".to_string();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    insert_test_pane_runtime(&mut runtime, &window_id);
    let predecessor = runtime
        .runtimes
        .get(&window_id)
        .expect("predecessor runtime")
        .pty
        .clone();
    let predecessor_pane = runtime
        .runtimes
        .get(&window_id)
        .expect("predecessor runtime")
        .pane
        .clone();
    runtime.register_pty_writer(&window_id, &predecessor_pane);

    assert!(runtime.close_window_outcome(&window_id).closed);
    assert_eq!(
        finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        1
    );

    runtime.register_window("tab-project", "agent-1");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let successor_runtime = runtime.runtimes.get(&window_id).expect("successor runtime");
    let successor_incarnation = successor_runtime.incarnation;
    let successor = successor_runtime.pty.clone();
    let successor_pane = successor_runtime.pane.clone();
    runtime.register_pty_writer(&window_id, &successor_pane);

    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued predecessor finalizer");
    finalizer();

    let registered = runtime
        .pty_writers
        .read()
        .expect("PTY writer registry")
        .get(&window_id)
        .cloned()
        .expect("successor writer remains registered");
    assert!(Arc::ptr_eq(&registered.handle, &successor));
    assert!(
        successor.reserve_input_transaction().is_ok(),
        "the predecessor finalizer must not revoke successor input"
    );
    assert_eq!(
        runtime
            .runtimes
            .get(&window_id)
            .map(|runtime| runtime.incarnation),
        Some(successor_incarnation)
    );
    assert!(
        runtime
            .window_lifecycle_generations
            .lock()
            .expect("window lifecycle generations")
            .contains_key(&window_id),
        "the predecessor finalizer must not settle the successor generation"
    );
    assert!(
        predecessor
            .try_wait()
            .expect("predecessor exit probe")
            .is_some(),
        "the predecessor PTY is still reaped"
    );
    successor.kill().expect("stop successor test PTY");
}

/// Issue #3783 authority fence: an unreadable predecessor Session is not
/// evidence that the Session id is unbound. A later same-id successor must
/// remain Running when the old local PTY finalizer eventually executes.
#[test]
fn queued_close_finalizer_with_unavailable_predecessor_session_preserves_same_id_successor() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-1",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let window_id = "tab-project::agent-1".to_string();
    let session_id = "session-reused-after-unreadable-close";
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: session_id.to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/predecessor".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: project.clone(),
            agent_project_root: project.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-project".to_string(),
        },
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    fs::write(
        runtime.sessions_dir.join(format!("{session_id}.toml")),
        "not valid session toml",
    )
    .expect("seed unreadable predecessor Session");

    assert!(runtime.close_window_outcome(&window_id).closed);

    let mut successor =
        gwt_agent::Session::new(&project, "work/successor", gwt_agent::AgentId::Codex);
    successor.id = session_id.to_string();
    successor.status = gwt_agent::AgentStatus::Running;
    successor.restore_window_on_startup = true;
    successor
        .save(&runtime.sessions_dir)
        .expect("persist same-id successor Session");

    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued predecessor finalizer");
    finalizer();

    let durable =
        gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{session_id}.toml")))
            .expect("reload successor Session");
    assert_eq!(durable.status, gwt_agent::AgentStatus::Running);
    assert!(
        durable.restore_window_on_startup,
        "an unavailable predecessor identity must not clear successor restore state"
    );
}

#[test]
fn close_finalizer_falls_back_when_blocking_spawner_fails() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-1",
        project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let window_id = "tab-project::agent-1".to_string();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    runtime.blocking_tasks = BlockingTaskSpawner::failing("injected close scheduler failure");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let pty = runtime
        .runtimes
        .get(&window_id)
        .expect("live runtime")
        .pty
        .clone();

    assert!(!runtime
        .close_window_after_issue_monitor_finalize_events(&window_id)
        .is_empty());

    let deadline = Instant::now() + Duration::from_secs(3);
    while pty.try_wait().expect("probe fallback child").is_none() {
        assert!(
            Instant::now() < deadline,
            "fallback finalizer must terminate and reap the detached PTY"
        );
        // test-hygiene: allow-short-duration bounded OS child-exit polling; the detached child has no exit notification in this fixture
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn close_finalizer_queues_on_process_worker_when_both_schedulers_fail() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-1",
        project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let window_id = "tab-project::agent-1".to_string();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    runtime.blocking_tasks = BlockingTaskSpawner::failing("injected close scheduler failure");
    insert_test_pane_runtime(&mut runtime, &window_id);
    let pty = runtime
        .runtimes
        .get(&window_id)
        .expect("live runtime")
        .pty
        .clone();
    let release_process_worker =
        super::super::pty_io::hold_process_close_finalizer_worker_for_test();
    let _raw_spawn_failure =
        super::super::pty_io::force_close_finalizer_thread_spawn_failure_for_test();

    assert!(!runtime
        .close_window_after_issue_monitor_finalize_events(&window_id)
        .is_empty());
    assert!(
        pty.try_wait().expect("probe queued child").is_none(),
        "an accepted close must not run its finalizer inline when both per-close schedulers fail"
    );

    drop(release_process_worker);
    let deadline = Instant::now() + Duration::from_secs(3);
    while pty
        .try_wait()
        .expect("probe process-worker child")
        .is_none()
    {
        assert!(
            Instant::now() < deadline,
            "the process-owned worker must eventually terminate and reap the detached PTY"
        );
        // test-hygiene: allow-short-duration bounded OS child-exit polling after explicit worker release; state observation establishes completion
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn exact_close_finalizer_holds_handoff_through_generation_gated_cleanup() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let session_id = "session-close-cleanup-successor";
    assert!(tab
        .workspace
        .set_session_id("agent-holder", Some(session_id.to_string())));
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    fs::create_dir_all(&runtime.sessions_dir).expect("create Session store");
    let window_id = "tab-1::agent-holder".to_string();
    let holder = install_manual_launch_holder(
        &mut runtime,
        &project,
        session_id,
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    let issuer = install_manual_holder_capability(&mut runtime, &project, &window_id, &holder);
    let active = runtime
        .active_agent_sessions
        .get(&window_id)
        .expect("active exact holder")
        .clone();
    save_workspace_launch_projection(
        &project,
        &active,
        Some("develop"),
        Some(42),
        None,
        None,
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([session_id.to_string()])),
    )
    .expect("seed predecessor Workspace projection");
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let generations = Arc::clone(&runtime.window_lifecycle_generations);
    let hook_issuer = issuer.clone();
    let hook_project = project.clone();
    let hook_window_id = window_id.clone();
    let hook_holder = holder.clone();
    let hook_active = active.clone();
    let successor_issue = Arc::new(Mutex::new(None));
    let hook_successor_issue = Arc::clone(&successor_issue);
    super::super::pty_io::set_close_finalizer_before_durable_cleanup_test_hook(move || {
        let result = hook_issuer.issue_bound(
            &hook_project,
            session_id,
            hook_holder.execution_binding.clone(),
        );
        *hook_successor_issue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
        generations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                hook_window_id.clone(),
                super::super::next_window_runtime_incarnation(),
            );
        save_workspace_launch_projection(
            &hook_project,
            &hook_active,
            Some("develop"),
            Some(42),
            None,
            None,
            WorkspaceLaunchProjectionKind::StartWork,
            Some(&HashSet::from([session_id.to_string()])),
        )
        .expect("materialize same Session/window successor projection");
    });

    assert!(runtime.close_window_outcome(&window_id).closed);
    finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued exact close finalizer")();

    assert!(
        successor_issue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .expect("successor issue attempt")
            .is_err(),
        "the exact handoff fence must remain held through durable cleanup"
    );
    let projection = gwt_core::workspace_projection::load_workspace_projection(&project)
        .expect("load successor projection")
        .expect("successor projection");
    assert!(
        projection.agents.iter().any(|agent| {
            agent.session_id == session_id && agent.window_id.as_deref() == Some(window_id.as_str())
        }),
        "a stale predecessor finalizer must not remove the same Session/window successor"
    );
    assert!(
        issuer
            .issue_bound(&project, session_id, holder.execution_binding.clone(),)
            .is_ok(),
        "the exact handoff fence must release after generation-gated cleanup"
    );
}

/// Issue #3755 AC-1/AC-2: one pane whose output/status worker currently owns
/// the Pane mutex must not hold the Tao event loop hostage. The authenticated
/// read sync still has to return an unrelated launch-error pane and finish the
/// request with an explicit availability receipt.
#[test]
fn agent_pane_sync_skips_a_contended_runtime_without_hiding_error_panes() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let mut tab = sample_project_tab(
        "tab-project",
        "Repo",
        project.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent, WindowPreset::Agent],
    );
    let raw_window_ids = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| window.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(raw_window_ids.len(), 2);
    let busy_window_id = combined_window_id("tab-project", &raw_window_ids[0]);
    let error_window_id = combined_window_id("tab-project", &raw_window_ids[1]);
    assert!(tab
        .workspace
        .set_status(&raw_window_ids[1], WindowProcessStatus::Error));
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let busy_pane = Arc::new(Mutex::new(long_running_test_pane(&busy_window_id)));
    runtime.runtimes.insert(
        busy_window_id.clone(),
        WindowRuntime::new(
            super::super::next_window_runtime_incarnation(),
            Arc::clone(&busy_pane),
        ),
    );
    runtime.launch_error_terminal_details.insert(
        error_window_id.clone(),
        "agent bootstrap failed before PTY start".to_string(),
    );
    let principal = AgentSessionPrincipal::for_test(&project, "session-reader")
        .expect("authenticated principal");
    let (locked_tx, locked_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let holder_pane = Arc::clone(&busy_pane);
    let holder = thread::spawn(move || {
        let _guard = holder_pane.lock().expect("hold pane mutex");
        locked_tx.send(()).expect("signal held mutex");
        gwt_core::test_support::recv_event(&release_rx, "pane sync request finished");
    });
    locked_rx.recv().expect("pane mutex acquired");

    let mut events = None;
    let mut elapsed = Duration::MAX;
    let logs = capture_tracing_events(|| {
        let started = Instant::now();
        events = Some(runtime.handle_agent_frontend_event(
            "pane-client".to_string(),
            principal,
            AgentFrontendRequest::Ready,
        ));
        elapsed = started.elapsed();
    });
    let events = events.expect("pane sync events");
    release_tx.send(()).expect("release pane mutex");
    holder.join().expect("pane mutex holder");

    assert!(
        elapsed < Duration::from_millis(300),
        "pane.read sync waited {elapsed:?} for an unrelated pane mutex"
    );
    assert!(
        events.iter().any(|outbound| matches!(
            &outbound.event,
            BackendEvent::TerminalSnapshot { id, .. } if id == &error_window_id
        )),
        "the launch-error pane must remain readable while another pane is busy: {events:?}"
    );
    assert!(
        events.iter().any(|outbound| {
            serde_json::to_value(&outbound.event)
                .ok()
                .and_then(|value| {
                    value
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .as_deref()
                == Some("pane_sync_complete")
        }),
        "pane.read needs an explicit completion receipt: {events:?}"
    );
    assert!(events.iter().any(|outbound| matches!(
        &outbound.event,
        BackendEvent::PaneSyncComplete {
            busy_window_ids,
            empty_window_ids,
            unavailable_window_ids,
            failed_window_ids,
        } if busy_window_ids == &vec![busy_window_id.clone()]
            && empty_window_ids.is_empty()
            && unavailable_window_ids.is_empty()
            && failed_window_ids.is_empty()
    )));
    let busy_log = logs
        .iter()
        .find(|event| {
            event.target == "gwt.pane.sync"
                && event.fields.get("window_id").map(String::as_str)
                    == Some(busy_window_id.as_str())
                && event.fields.get("stage").map(String::as_str) == Some("pane_snapshot_try_lock")
        })
        .expect("busy snapshot diagnostic");
    assert_eq!(
        busy_log.fields.get("outcome").map(String::as_str),
        Some("busy")
    );
    assert!(busy_log.fields.contains_key("elapsed_ms"));
}

/// Issue #3755 AC-2: completion outcomes are disjoint. A blank live terminal
/// still has a replayable vt100 snapshot, a missing runtime is unavailable,
/// and a poisoned parser mutex is an internal failure rather than busy.
#[test]
fn agent_pane_sync_completion_distinguishes_snapshot_unavailable_and_poisoned() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let tab = sample_project_tab(
        "tab-project",
        "Repo",
        project.clone(),
        ProjectKind::Git,
        &[
            WindowPreset::Agent,
            WindowPreset::Agent,
            WindowPreset::Agent,
        ],
    );
    let window_ids = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| combined_window_id("tab-project", &window.id))
        .collect::<Vec<_>>();
    let empty_window_id = window_ids[0].clone();
    let unavailable_window_id = window_ids[1].clone();
    let failed_window_id = window_ids[2].clone();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    runtime.window_lookup.remove(&unavailable_window_id);
    let empty_pane = Arc::new(Mutex::new(long_running_test_pane(&empty_window_id)));
    runtime.runtimes.insert(
        empty_window_id.clone(),
        WindowRuntime::new(
            super::super::next_window_runtime_incarnation(),
            Arc::clone(&empty_pane),
        ),
    );
    let failed_pane = Arc::new(Mutex::new(long_running_test_pane(&failed_window_id)));
    runtime.runtimes.insert(
        failed_window_id.clone(),
        WindowRuntime::new(
            super::super::next_window_runtime_incarnation(),
            Arc::clone(&failed_pane),
        ),
    );
    let poison_pane = Arc::clone(&failed_pane);
    let _ = thread::spawn(move || {
        let _guard = poison_pane.lock().expect("poison target pane");
        panic!("intentional pane mutex poison");
    })
    .join();
    let principal = AgentSessionPrincipal::for_test(&project, "session-reader")
        .expect("authenticated principal");

    let mut events = None;
    let logs = capture_tracing_events(|| {
        events = Some(runtime.handle_agent_frontend_event(
            "pane-client".to_string(),
            principal,
            AgentFrontendRequest::Ready,
        ));
    });
    let events = events.expect("pane sync events");

    assert!(events.iter().any(|outbound| matches!(
        &outbound.event,
        BackendEvent::TerminalSnapshot { id, .. } if id == &empty_window_id
    )));
    assert!(events.iter().any(|outbound| matches!(
        &outbound.event,
        BackendEvent::PaneSyncComplete {
            empty_window_ids,
            busy_window_ids,
            unavailable_window_ids,
            failed_window_ids,
        } if empty_window_ids.is_empty()
            && busy_window_ids.is_empty()
            && unavailable_window_ids == &vec![unavailable_window_id.clone()]
            && failed_window_ids == &vec![failed_window_id.clone()]
    )));
    let failed_log = logs
        .iter()
        .find(|event| {
            event.target == "gwt.pane.sync"
                && event.fields.get("window_id").map(String::as_str)
                    == Some(failed_window_id.as_str())
                && event.fields.get("stage").map(String::as_str) == Some("pane_snapshot_try_lock")
        })
        .expect("poisoned snapshot diagnostic");
    assert_eq!(
        failed_log.fields.get("outcome").map(String::as_str),
        Some("poisoned")
    );
    assert!(failed_log.fields.contains_key("elapsed_ms"));
}

/// Issue #3755 AC-3 / Issue #3988: removing mutex waits must not hide a new
/// multi-pane serialization stall. The original guard was a 300ms wall-clock
/// budget, which turned every saturated CI runner red without saying anything
/// about the invariant, so the guard is structural now. Two properties make a
/// stall impossible and both are decidable without a clock: a pane whose mutex
/// is held is reported busy instead of awaited, and every snapshot the dispatch
/// emits stays clamped to the scrollback replay cap however much the pane
/// printed. The only timeout left is a deadlock guard on the holder thread, so
/// a regression that waits on the mutex fails an assertion instead of hanging
/// the test binary.
#[test]
fn agent_pane_sync_with_full_scrollback_stays_bounded() {
    // Rows of `long_running_test_pane`, which the snapshot replays on top of
    // the capped scrollback.
    const PANE_ROWS: usize = 24;
    // The holder releases as soon as the dispatch returns, so this only bounds
    // the failing path where the dispatch waits on the mutex forever.
    const HOLDER_DEADLOCK_GUARD: Duration = Duration::from_secs(60);
    // Print well past the cap so the clamp is exercised rather than assumed.
    let printed_lines = SNAPSHOT_SCROLLBACK_REPLAY_LIMIT + PANE_ROWS + 1_000;

    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let tab = sample_project_tab(
        "tab-project",
        "Repo",
        project.clone(),
        ProjectKind::Git,
        &[
            WindowPreset::Agent,
            WindowPreset::Agent,
            WindowPreset::Agent,
            WindowPreset::Agent,
        ],
    );
    let window_ids = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| combined_window_id("tab-project", &window.id))
        .collect::<Vec<_>>();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let replay = (0..printed_lines)
        .map(|line| format!("snapshot-line-{line:06}\n"))
        .collect::<String>();
    let contended_window_id = window_ids[0].clone();
    let mut contended_pane = None;
    for window_id in &window_ids {
        let mut pane = long_running_test_pane(window_id);
        pane.process_bytes(replay.as_bytes());
        let pane = Arc::new(Mutex::new(pane));
        if window_id == &contended_window_id {
            contended_pane = Some(Arc::clone(&pane));
        }
        runtime.runtimes.insert(
            window_id.clone(),
            WindowRuntime::new(super::super::next_window_runtime_incarnation(), pane),
        );
    }
    let contended_pane = contended_pane.expect("contended pane runtime");

    // Explicit sync points replace the old sleep: the holder owns the mutex
    // before the dispatch starts and releases it only after the dispatch has
    // returned, so contention covers the whole dispatch on any host speed.
    let (held_tx, held_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::channel();
    let holder = thread::spawn(move || {
        let _guard = contended_pane.lock().expect("hold contended pane mutex");
        held_tx.send(()).expect("signal held mutex");
        let _ = release_rx.recv_timeout(HOLDER_DEADLOCK_GUARD);
    });
    held_rx.recv().expect("contended pane mutex acquired");

    let principal = AgentSessionPrincipal::for_test(&project, "session-reader")
        .expect("authenticated principal");
    let events = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal,
        AgentFrontendRequest::Ready,
    );
    let _ = release_tx.send(());
    holder.join().expect("contended pane holder");

    assert!(
        events.iter().any(|outbound| matches!(
            &outbound.event,
            BackendEvent::PaneSyncComplete {
                empty_window_ids,
                busy_window_ids,
                unavailable_window_ids,
                failed_window_ids,
            } if busy_window_ids == &vec![contended_window_id.clone()]
                && empty_window_ids.is_empty()
                && unavailable_window_ids.is_empty()
                && failed_window_ids.is_empty()
        )),
        "a full-scrollback pane holding its mutex must be reported busy, never awaited: {events:?}"
    );

    let snapshots = events
        .iter()
        .filter_map(|outbound| match &outbound.event {
            BackendEvent::TerminalSnapshot { id, data_base64 } => {
                Some((id.clone(), data_base64.clone()))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut expected_snapshot_ids = window_ids
        .iter()
        .filter(|id| *id != &contended_window_id)
        .cloned()
        .collect::<Vec<_>>();
    expected_snapshot_ids.sort();
    assert_eq!(
        snapshots.keys().cloned().collect::<Vec<_>>(),
        expected_snapshot_ids,
        "every uncontended full-scrollback pane must still be snapshotted in the same dispatch"
    );

    let newest_line = format!("snapshot-line-{:06}", printed_lines - 1);
    for (window_id, data_base64) in &snapshots {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data_base64)
            .expect("snapshot base64");
        let snapshot = String::from_utf8_lossy(&decoded);
        let replayed_lines = snapshot.matches("snapshot-line-").count();
        assert!(
            replayed_lines <= SNAPSHOT_SCROLLBACK_REPLAY_LIMIT + PANE_ROWS,
            "{window_id} replayed {replayed_lines} lines, past the \
             {SNAPSHOT_SCROLLBACK_REPLAY_LIMIT}-row scrollback replay cap"
        );
        assert!(
            snapshot.contains(&newest_line),
            "{window_id} dropped the newest line {newest_line} from its snapshot"
        );
        assert!(
            !snapshot.contains("snapshot-line-000000"),
            "{window_id} replayed a line older than the scrollback replay cap"
        );
    }
}

/// Issue #3755 AC-2/AC-3: close removes the runtime and window synchronously,
/// but a contended Pane mutex is cleanup work and must never be awaited by the
/// GUI event loop.
#[test]
fn agent_pane_close_bypasses_a_contended_pane_lock() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let mut tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-project",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-project", Some("session-holder".to_string())));
    let window_id = "tab-project::agent-project".to_string();
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let holder_identity = install_manual_launch_holder(
        &mut runtime,
        &project,
        "session-holder",
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    install_manual_holder_capability(&mut runtime, &project, &window_id, &holder_identity);
    let exact_runtime = runtime
        .runtimes
        .get(&window_id)
        .expect("exact holder runtime");
    let busy_pane = exact_runtime.pane.clone();
    let pty = exact_runtime.pty.clone();
    let incarnation = exact_runtime.incarnation;
    let runtime_path =
        gwt_agent::runtime_state_path(&runtime.sessions_dir, &holder_identity.session_id);
    let principal = AgentSessionPrincipal::for_test(&project, "session-closer")
        .expect("authenticated principal");
    let (locked_tx, locked_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let holder_pane = Arc::clone(&busy_pane);
    let holder = thread::spawn(move || {
        let _guard = holder_pane.lock().expect("hold pane mutex");
        locked_tx.send(()).expect("signal held mutex");
        gwt_core::test_support::recv_event(&release_rx, "pane close request finished");
    });
    locked_rx.recv().expect("pane mutex acquired");

    let mut events = None;
    let mut elapsed = Duration::MAX;
    let logs = capture_tracing_events(|| {
        let started = Instant::now();
        events = Some(runtime.handle_agent_frontend_event(
            "pane-client".to_string(),
            principal,
            AgentFrontendRequest::CloseWindow {
                id: window_id.clone(),
                request_id: None,
                responder: None,
            },
        ));
        elapsed = started.elapsed();
        release_tx.send(()).expect("release pane mutex");
        let queued = {
            let mut finalizers = finalizers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(finalizers.len(), 1, "close must queue one finalizer");
            finalizers.drain(..).collect::<Vec<_>>()
        };
        for finalizer in queued {
            finalizer();
        }
    });
    let events = events.expect("pane close events");
    holder.join().expect("pane mutex holder");

    let proof_deadline = Instant::now() + Duration::from_secs(5);
    let terminal_proof = loop {
        let child_exited = pty
            .try_wait()
            .expect("probe exact holder child exit")
            .is_some();
        let proof = gwt_agent::SessionRuntimeState::load(&runtime_path).ok();
        if child_exited
            && proof
                .as_ref()
                .is_some_and(|proof| proof.status == gwt_agent::AgentStatus::Stopped)
        {
            break proof.expect("terminal runtime proof");
        }
        assert!(
            Instant::now() < proof_deadline,
            "exact holder child exit and terminal proof were not observed before the deadline"
        );
        // test-hygiene: allow-short-duration bounded OS child-exit and durable stop-proof polling; ordering comes from observed state, not this interval
        thread::sleep(Duration::from_millis(10));
    };

    assert!(
        elapsed < Duration::from_millis(300),
        "pane.close waited {elapsed:?} for the pane mutex"
    );
    assert!(matches!(
        events.first(),
        Some(OutboundEvent {
            event: BackendEvent::PaneCloseResult { ok: true, .. },
            ..
        })
    ));
    assert!(!runtime.runtimes.contains_key(&window_id));
    assert!(!runtime.window_lookup.contains_key(&window_id));
    let kill_log = logs
        .iter()
        .find(|event| {
            event.target == "gwt.pane.teardown"
                && event.fields.get("stage").map(String::as_str) == Some("pty_kill")
                && event.fields.get("outcome").map(String::as_str) == Some("completed")
        })
        .expect("pty_kill teardown diagnostic");
    assert_eq!(
        kill_log.fields.get("window_id").map(String::as_str),
        Some(window_id.as_str())
    );
    assert!(kill_log.fields.contains_key("elapsed_ms"));
    assert_eq!(kill_log.fields.get("ok").map(String::as_str), Some("true"));
    assert_eq!(
        terminal_proof.execution_identity.as_ref(),
        Some(&holder_identity)
    );
    assert_eq!(terminal_proof.runtime_incarnation, Some(incarnation));
}

/// Issue #3629 AC-12: an uncorrelated self-close stays refused, but the
/// refusal must be explicit so the CLI stops guessing at session correlation.
#[test]
fn agent_pane_close_reports_uncorrelated_self_close_refusal() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let mut tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-project",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-project", Some("session-project".to_string())));
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let principal =
        AgentSessionPrincipal::for_test(&project, "session-project").expect("self-owned principal");

    let refused = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal,
        AgentFrontendRequest::CloseWindow {
            id: "tab-project::agent-project".to_string(),
            request_id: None,
            responder: None,
        },
    );

    assert!(
        matches!(
            refused.as_slice(),
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::PaneCloseResult {
                    ok: false,
                    window_id,
                    reason: Some(reason),
                },
                ..
            }] if client_id == "pane-client"
                && window_id == "tab-project::agent-project"
                && reason.contains("correlated")
        ),
        "an uncorrelated self-close must name the correlation requirement, got: {refused:?}"
    );
    assert!(runtime
        .window_lookup
        .contains_key("tab-project::agent-project"));
}

/// Issue #3503 / #3552 AC-1: the pane the PM could never clean up had failed
/// *before* its PTY started, so it carries no session binding at all. The
/// self-session protection compares the window's real binding, so `None` is
/// not "this may be you" — it is a peer pane and it closes. Locking both
/// halves in one test keeps the guard identity-shaped, so it cannot drift back
/// into the state- or authority-shaped gate that made every PM cleanup close
/// fail regardless of the target pane's state. `pane.stop` resolves to this
/// same command, so it is covered here too.
#[test]
fn agent_pane_close_removes_a_launch_failed_peer_pane_without_session_binding() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let mut tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-caller",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-caller", Some("caller-session".to_string())));
    let launch_failed = tab
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds());
    let launch_failed_window_id = combined_window_id("tab-project", &launch_failed.id);
    assert!(
        tab.workspace
            .window(&launch_failed.id)
            .expect("launch-failed window")
            .session_id
            .is_none(),
        "a pane that failed before PTY start carries no session binding"
    );
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    // The PM is a conversational role with no linked owner, so its principal
    // never carries an active execution binding.
    let owner_less_principal =
        AgentSessionPrincipal::for_test(&project, "caller-session").expect("owner-less principal");
    assert!(!owner_less_principal.authorizes_producing_mutation());

    let closed = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        owner_less_principal.clone(),
        AgentFrontendRequest::CloseWindow {
            id: launch_failed_window_id.clone(),
            request_id: None,
            responder: None,
        },
    );

    assert!(!closed.is_empty());
    assert!(!runtime.window_lookup.contains_key(&launch_failed_window_id));

    let refused = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        owner_less_principal,
        AgentFrontendRequest::CloseWindow {
            id: "tab-project::agent-caller".to_string(),
            request_id: None,
            responder: None,
        },
    );

    assert!(
        matches!(
            refused.as_slice(),
            [OutboundEvent {
                event: BackendEvent::PaneCloseResult { ok: false, .. },
                ..
            }]
        ),
        "self-session protection still refuses an uncorrelated close of the caller's own pane, got: {refused:?}"
    );
    assert!(runtime
        .window_lookup
        .contains_key("tab-project::agent-caller"));
}

/// Issue #3629 AC-9: a husk window (workspace record without a lookup entry,
/// left behind by an app restart) must still close through the agent route.
#[test]
fn agent_pane_close_removes_husk_window_missing_from_lookup() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-husk",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    runtime.window_lookup.remove("tab-project::agent-husk");
    let principal = AgentSessionPrincipal::for_test(&project, "session-pm").expect("pm principal");

    let events = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal,
        AgentFrontendRequest::CloseWindow {
            id: "tab-project::agent-husk".to_string(),
            request_id: None,
            responder: None,
        },
    );

    assert!(
        matches!(
            events.first(),
            Some(OutboundEvent {
                event: BackendEvent::PaneCloseResult { ok: true, .. },
                ..
            })
        ),
        "husk close must succeed via the workspace fallback, got: {events:?}"
    );
    assert!(runtime
        .tab("tab-project")
        .expect("project tab")
        .workspace
        .window("agent-husk")
        .is_none());
}

#[test]
fn queued_agent_pane_request_rechecks_generation_before_runtime_dispatch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let mut tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-project",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-project", Some("session-project".to_string())));
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    runtime.agent_capability_issuer = Some(issuer.clone());
    let original = issuer
        .issue(&project, "session-project")
        .expect("original capability");
    let queued_grant = issuer
        .grant_for_test(&original.token)
        .expect("queued authenticated grant");

    let current = issuer
        .issue(&project, "session-project")
        .expect("rotate capability before tao dispatch");
    let stale_outcome = runtime.handle_agent_frontend_event_if_current(
        "pane-client".to_string(),
        queued_grant,
        AgentFrontendRequest::Ready,
    );
    assert!(
        matches!(
            stale_outcome,
            super::super::AgentFrontendDispatchOutcome::StaleCapability
        ),
        "a queued request whose generation was revoked must be rejected as stale"
    );

    let current_grant = issuer
        .grant_for_test(&current.token)
        .expect("current authenticated grant");
    let current_events = match runtime.handle_agent_frontend_event_if_current(
        "pane-client".to_string(),
        current_grant,
        AgentFrontendRequest::Ready,
    ) {
        super::super::AgentFrontendDispatchOutcome::Dispatched(events) => events,
        super::super::AgentFrontendDispatchOutcome::StaleCapability => {
            panic!("current grant must dispatch")
        }
        super::super::AgentFrontendDispatchOutcome::ExecutionAuthorityUnavailable => {
            panic!("current inspection grant does not require durable authority")
        }
    };
    assert!(
        current_events
            .iter()
            .any(|event| matches!(&event.event, BackendEvent::WindowCanvasState { .. })),
        "the current observation grant must still receive its scoped snapshot"
    );
}

/// Issue #3816: the Tao dispatch path must release the caller capability
/// registry read lock before close teardown fences the target capability.
/// Otherwise `pane.close` self-deadlocks before its acknowledgement and every
/// later request on the same event loop stalls behind it.
#[test]
fn current_peer_pane_close_releases_caller_grant_lock_before_target_handoff() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-target",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let target_session_id = "session-close-target";
    assert!(tab
        .workspace
        .set_session_id("agent-target", Some(target_session_id.to_string())));
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-target");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &project,
        target_session_id,
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    let issuer = install_manual_holder_capability(&mut runtime, &project, &window_id, &holder);
    let stale_caller = issuer
        .issue(&project, "session-close-caller")
        .expect("issue stale peer caller capability");
    let stale_caller_grant = issuer
        .grant_for_test(&stale_caller.token)
        .expect("authenticate stale peer caller capability");
    let caller = issuer
        .issue(&project, "session-close-caller")
        .expect("issue peer caller capability");
    let caller_grant = issuer
        .grant_for_test(&caller.token)
        .expect("authenticate peer caller capability");
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    let stale_close = runtime.handle_agent_frontend_event_if_current(
        "pane-client".to_string(),
        stale_caller_grant,
        AgentFrontendRequest::CloseWindow {
            id: window_id.clone(),
            request_id: None,
            responder: None,
        },
    );
    assert!(matches!(
        stale_close,
        super::super::AgentFrontendDispatchOutcome::StaleCapability
    ));
    assert!(
        runtime.tracked_window_exists(&window_id),
        "rotation before acceptance must reject the close without touching its target"
    );
    assert!(
        finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty(),
        "a close rejected before acceptance must not queue teardown"
    );

    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let thread_home = temp.path().to_path_buf();
    let worker_window_id = window_id.clone();
    let worker = thread::spawn(move || {
        let _gwt_home = ScopedGwtHome::set(&thread_home);
        let started = Instant::now();
        let close = runtime.handle_agent_frontend_event_if_current(
            "pane-client".to_string(),
            caller_grant.clone(),
            AgentFrontendRequest::CloseWindow {
                id: worker_window_id.clone(),
                request_id: None,
                responder: None,
            },
        );
        let close_elapsed = started.elapsed();
        let reclose = runtime.handle_agent_frontend_event_if_current(
            "pane-client".to_string(),
            caller_grant.clone(),
            AgentFrontendRequest::CloseWindow {
                id: worker_window_id,
                request_id: None,
                responder: None,
            },
        );
        let list_started = Instant::now();
        let listed = runtime.handle_agent_frontend_event_if_current(
            "pane-client".to_string(),
            caller_grant,
            AgentFrontendRequest::ListWindows,
        );
        result_tx
            .send((
                close,
                reclose,
                listed,
                close_elapsed,
                list_started.elapsed(),
            ))
            .expect("report peer close outcomes");
    });

    let (close, reclose, listed, close_elapsed, list_elapsed) = result_rx
        .recv_timeout(Duration::from_millis(400))
        .expect("peer close and the following bridge requests must not deadlock");
    worker.join().expect("join peer close dispatch");
    assert!(matches!(
        close,
        super::super::AgentFrontendDispatchOutcome::Dispatched(ref events)
            if matches!(
                events.first(),
                Some(OutboundEvent {
                    event: BackendEvent::PaneCloseResult { ok: true, .. },
                    ..
                })
            )
    ));
    assert!(matches!(
        reclose,
        super::super::AgentFrontendDispatchOutcome::Dispatched(ref events)
            if matches!(
                events.first(),
                Some(OutboundEvent {
                    event: BackendEvent::PaneCloseResult { ok: false, .. },
                    ..
                })
            )
    ));
    assert!(matches!(
        listed,
        super::super::AgentFrontendDispatchOutcome::Dispatched(ref events) if !events.is_empty()
    ));
    assert!(
        close_elapsed < Duration::from_millis(400),
        "peer close acknowledgement took {close_elapsed:?}"
    );
    assert!(
        list_elapsed < Duration::from_secs(1),
        "pane.list after a failed re-close took {list_elapsed:?}"
    );
    assert_eq!(
        finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        1,
        "only the accepted close may queue background teardown"
    );
    let queued = {
        let mut finalizers = finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        finalizers.drain(..).collect::<Vec<_>>()
    };
    for finalizer in queued {
        finalizer();
    }
}

/// Issue #3816: once the caller grant has crossed the acceptance snapshot,
/// rotating that caller must not cancel the already accepted peer operation.
#[test]
fn accepted_peer_close_snapshot_survives_caller_rotation() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-target",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Stopped,
    );
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-target");
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    runtime.agent_capability_issuer = Some(issuer.clone());
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let caller = issuer
        .issue(&project, "session-close-caller")
        .expect("issue peer caller capability");
    let caller_grant = issuer
        .grant_for_test(&caller.token)
        .expect("authenticate peer caller capability");
    let asserted_grant = caller_grant.clone();
    let (accepted_tx, accepted_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let thread_home = temp.path().to_path_buf();
    let worker_window_id = window_id.clone();
    let worker = thread::spawn(move || {
        let _gwt_home = ScopedGwtHome::set(&thread_home);
        super::super::set_agent_peer_close_after_acceptance_test_hook(move || {
            accepted_tx
                .send(())
                .expect("signal peer close acceptance snapshot");
            resume_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("resume accepted peer close");
        });
        let outcome = runtime.handle_agent_frontend_event_if_current(
            "pane-client".to_string(),
            caller_grant,
            AgentFrontendRequest::CloseWindow {
                id: worker_window_id,
                request_id: None,
                responder: None,
            },
        );
        (runtime, outcome)
    });

    accepted_rx
        .recv_timeout(Duration::from_millis(400))
        .expect("production dispatch reached the post-acceptance barrier");
    let rotation_issuer = issuer.clone();
    let rotation_project = project.clone();
    let (rotation_tx, rotation_rx) = mpsc::sync_channel(1);
    let rotation = thread::spawn(move || {
        rotation_tx
            .send(rotation_issuer.issue(&rotation_project, "session-close-caller"))
            .expect("report caller rotation");
    });
    rotation_rx
        .recv_timeout(Duration::from_millis(400))
        .expect("caller rotation must not wait on the accepted dispatch")
        .expect("rotate caller after acceptance");
    rotation.join().expect("join caller rotation");
    assert!(
        !issuer.grant_is_current(&asserted_grant),
        "the accepted grant must now be stale in the registry"
    );
    resume_tx.send(()).expect("resume accepted peer close");
    let (runtime, outcome) = worker.join().expect("join accepted peer close dispatch");
    assert!(matches!(
        outcome,
        super::super::AgentFrontendDispatchOutcome::Dispatched(ref events)
            if matches!(
                events.first(),
                Some(OutboundEvent {
                    event: BackendEvent::PaneCloseResult { ok: true, .. },
                    ..
                })
            )
    ));
    assert!(
        !runtime.tracked_window_exists(&window_id),
        "rotation after acceptance must not cancel the accepted close"
    );
    let queued = {
        let mut finalizers = finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        finalizers.drain(..).collect::<Vec<_>>()
    };
    for finalizer in queued {
        finalizer();
    }
}

/// Issue #3816 AC-3/AC-5/AC-6: exercise the real TCP WebSocket bridge, its
/// capability gate, the Tao-shaped UserEvent queue, and AppRuntime dispatch as
/// one loop. Wire-level close (also used by the parser-proven `pane.stop`
/// alias), list/read, and self pane send must remain responsive on the same
/// authenticated connection.
#[test]
fn real_agent_pane_websocket_stays_responsive_after_peer_close() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("isolated HOME");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project = temp.path().join("project");
    let caller_session_id = "session-ws-caller";
    let target_session_id = "session-ws-target";
    let caller_binding = materialize_active_agent_pane_binding(&project, caller_session_id);

    let mut tab = sample_project_tab(
        "tab-project",
        "Repo",
        project.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent, WindowPreset::Agent],
    );
    let raw_window_ids = tab
        .workspace
        .persisted()
        .windows
        .iter()
        .map(|window| window.id.clone())
        .collect::<Vec<_>>();
    let caller_window_id = combined_window_id("tab-project", &raw_window_ids[0]);
    let target_window_id = combined_window_id("tab-project", &raw_window_ids[1]);
    assert!(tab
        .workspace
        .set_session_id(&raw_window_ids[0], Some(caller_session_id.to_string())));
    assert!(tab
        .workspace
        .set_session_id(&raw_window_ids[1], Some(target_session_id.to_string())));

    let (mut app, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    for window_id in [&caller_window_id, &target_window_id] {
        insert_test_pane_runtime(&mut app, window_id);
    }
    let caller_pane = app
        .runtimes
        .get(&caller_window_id)
        .expect("caller runtime")
        .pane
        .clone();
    let mut caller_reader = caller_pane
        .lock()
        .expect("caller pane")
        .reader()
        .expect("caller PTY reader");
    let caller_output_thread = thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while std::io::Read::read(&mut caller_reader, &mut buffer).is_ok_and(|read| read > 0) {}
    });
    caller_pane
        .lock()
        .expect("caller pane")
        .process_bytes(b"caller snapshot remains readable after peer close\n");
    let caller_child_pid = caller_pane
        .lock()
        .expect("caller pane")
        .pty()
        .process_id()
        .expect("caller child pid");
    let caller_child_started_at = gwt::process::host_process_start_time(caller_child_pid)
        .expect("caller child process start time");
    app.register_pty_writer(&caller_window_id, &caller_pane);

    let mut caller_session = sample_active_agent_session("tab-project", &caller_window_id);
    caller_session.session_id = caller_session_id.to_string();
    caller_session.worktree_path = project.clone();
    caller_session.agent_project_root = project.display().to_string();
    let mut target_session = sample_active_agent_session("tab-project", &target_window_id);
    target_session.session_id = target_session_id.to_string();
    target_session.worktree_path = project.clone();
    target_session.agent_project_root = project.display().to_string();
    app.active_agent_sessions
        .insert(caller_window_id.clone(), caller_session);
    app.active_agent_sessions
        .insert(target_window_id.clone(), target_session);
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    app.blocking_tasks = spawner;

    let tokio = TokioRuntime::new().expect("Tokio runtime");
    let (proxy, recorded_events) = AppEventProxy::stub();
    app.proxy = proxy.clone();
    let clients = crate::embedded_server::ClientHub::default();
    let mut server = crate::embedded_server::EmbeddedServer::start(
        &tokio,
        proxy,
        clients.clone(),
        Arc::clone(&app.pty_writers),
        AttachmentUploadStore::in_system_temp(),
    )
    .expect("embedded server");
    let issuer = server.agent_capability_issuer();
    let caller = issuer
        .issue_bound(&project, caller_session_id, caller_binding)
        .expect("active caller capability");
    let target = issuer
        .issue(&project, target_session_id)
        .expect("peer target capability");
    app.agent_capability_issuer = Some(issuer.clone());
    app.agent_capability_tokens
        .insert(caller_window_id.clone(), caller.token.clone());
    app.agent_capability_tokens
        .insert(target_window_id.clone(), target.token);

    let (stop_tx, stop_rx) = mpsc::sync_channel(1);
    let driver_home = temp.path().to_path_buf();
    let driver_events = Arc::clone(&recorded_events);
    let driver_clients = clients.clone();
    let driver = thread::spawn(move || {
        let _gwt_home = ScopedGwtHome::set(&driver_home);
        loop {
            match stop_rx.try_recv() {
                Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            let event = {
                let mut events = driver_events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                events
                    .iter()
                    .position(|event| {
                        matches!(
                            recorded_project_payload(event),
                            UserEvent::AgentFrontend { .. }
                        )
                    })
                    .map(|position| events.remove(position))
            };
            let Some(UserEvent::AgentFrontend {
                client_id,
                grant,
                request,
            }) = event
            else {
                // test-hygiene: allow-short-duration bounded event-driver and OS process-exit polling; observed event/state establishes ordering, not this interval
                thread::sleep(Duration::from_millis(1));
                continue;
            };
            let outcome =
                app.handle_agent_frontend_event_if_current(client_id.clone(), grant, request);
            crate::apply_agent_frontend_dispatch_outcome(&driver_clients, &client_id, outcome);
        }
        app
    });

    let pane_url = issuer.agent_pane_websocket_url().to_string();
    tokio.block_on(async {
        let mut request = pane_url
            .as_str()
            .into_client_request()
            .expect("agent pane WebSocket request");
        request.headers_mut().insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", caller.token)
                .parse()
                .expect("bearer header"),
        );
        let (mut socket, _) = connect_async(request)
            .await
            .expect("real agent pane WebSocket");

        socket
            .send(WebSocketMessage::Text(
                r#"{"kind":"list_windows"}"#.to_string().into(),
            ))
            .await
            .expect("seed authenticated pane scope");
        let _ =
            next_test_agent_websocket_event(&mut socket, "workspace_state", Duration::from_secs(1))
                .await;

        let close_started = Instant::now();
        socket
            .send(WebSocketMessage::Text(
                serde_json::json!({"kind": "close_window", "id": target_window_id})
                    .to_string()
                    .into(),
            ))
            .await
            .expect("send peer close");
        let close = next_test_agent_websocket_event(
            &mut socket,
            "pane_close_result",
            Duration::from_millis(400),
        )
        .await;
        assert_eq!(close["ok"], true);
        assert_eq!(close["window_id"], target_window_id);
        assert!(
            close_started.elapsed() < Duration::from_millis(400),
            "real WebSocket peer close exceeded the bridge budget"
        );

        let reclose_started = Instant::now();
        socket
            .send(WebSocketMessage::Text(
                serde_json::json!({"kind": "close_window", "id": target_window_id})
                    .to_string()
                    .into(),
            ))
            .await
            .expect("send failed peer re-close");
        let reclose = next_test_agent_websocket_event(
            &mut socket,
            "pane_close_result",
            Duration::from_millis(400),
        )
        .await;
        assert_eq!(reclose["ok"], false);
        assert!(
            reclose_started.elapsed() < Duration::from_millis(400),
            "real WebSocket failed peer re-close exceeded the bridge budget"
        );

        socket
            .send(WebSocketMessage::Text(
                r#"{"kind":"list_windows"}"#.to_string().into(),
            ))
            .await
            .expect("send list after failed close");
        let listed =
            next_test_agent_websocket_event(&mut socket, "workspace_state", Duration::from_secs(1))
                .await;
        let listed_windows = listed["workspace"]["tabs"][0]["workspace"]["windows"]
            .as_array()
            .expect("listed windows");
        assert!(listed_windows
            .iter()
            .any(|window| window["id"] == caller_window_id));
        assert!(listed_windows
            .iter()
            .all(|window| window["id"] != target_window_id));

        socket
            .send(WebSocketMessage::Text(
                r#"{"kind":"frontend_ready"}"#.to_string().into(),
            ))
            .await
            .expect("send read sync after peer close");
        let snapshot = next_test_agent_websocket_event(
            &mut socket,
            "terminal_snapshot",
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(snapshot["id"], caller_window_id);
        let snapshot_bytes = base64::engine::general_purpose::STANDARD
            .decode(snapshot["data_base64"].as_str().expect("snapshot base64"))
            .expect("decode caller snapshot");
        assert!(String::from_utf8_lossy(&snapshot_bytes)
            .contains("caller snapshot remains readable after peer close"));

        socket
            .send(WebSocketMessage::Text(
                serde_json::json!({
                    "kind": "pane_send_input",
                    "session_id": caller_session_id,
                    "text": "status after peer close"
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send caller pane input after peer close");
        let sent = next_test_agent_websocket_event(
            &mut socket,
            "pane_send_result",
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(sent["ok"], true);
        assert_eq!(sent["window_id"], caller_window_id);
        socket.close(None).await.expect("close test WebSocket");
    });

    stop_tx.send(()).expect("stop Tao-shaped event driver");
    let mut app = driver.join().expect("join Tao-shaped event driver");
    server.shutdown();
    assert_eq!(
        finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        1,
        "the accepted peer close owns one background finalizer"
    );
    let queued = {
        let mut finalizers = finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        finalizers.drain(..).collect::<Vec<_>>()
    };
    for finalizer in queued {
        finalizer();
    }
    app.stop_window_runtime_without_session_projection(&caller_window_id);
    let cleanup_deadline = Instant::now() + Duration::from_secs(5);
    while gwt::process::exact_pty_process_tree_is_alive(caller_child_pid, caller_child_started_at) {
        assert!(
            Instant::now() < cleanup_deadline,
            "caller PTY process tree survived explicit test cleanup"
        );
        // test-hygiene: allow-short-duration bounded event-driver and OS process-exit polling; observed event/state establishes ordering, not this interval
        thread::sleep(Duration::from_millis(10));
    }
    // Close the retained ConPTY master before waiting for the reader's EOF.
    drop(caller_pane);
    caller_output_thread
        .join()
        .expect("join caller PTY output drain");
}

/// Issue #3667 AC-1/AC-3 at the runtime dispatch layer: a settled grant (an
/// in-memory Active binding whose durable record is stale) still observes its
/// scoped state while producing mutation stays refused.
#[test]
fn settled_grant_observes_scoped_state_but_mutation_is_refused() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let mut tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-project",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-project", Some("session-settled".to_string())));
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    runtime.agent_capability_issuer = Some(issuer.clone());
    // No durable session file exists under the scoped home, so the durable
    // authority for this binding resolves Stale — the settled shape.
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: "session-settled".to_string(),
        repo_hash: "repo-3667".to_string(),
        owner_kind: "issue".to_string(),
        owner_number: 3667,
        identity: gwt_agent::ExecutionBindingIdentity {
            generation_id: "generation-3667".to_string(),
            binding_id: "binding-3667".to_string(),
            ledger_head_hash: "head-3667".to_string(),
        },
        capability_generation: 1,
    };
    let target = issuer
        .issue_bound(&project, "session-settled", binding)
        .expect("settled capability");
    let grant = issuer
        .grant_for_test(&target.token)
        .expect("authenticated settled grant");

    let observed = match runtime.handle_agent_frontend_event_if_current(
        "pane-client".to_string(),
        grant.clone(),
        AgentFrontendRequest::Ready,
    ) {
        super::super::AgentFrontendDispatchOutcome::Dispatched(events) => events,
        super::super::AgentFrontendDispatchOutcome::StaleCapability => {
            panic!("settled observation must dispatch instead of reading as stale")
        }
        super::super::AgentFrontendDispatchOutcome::ExecutionAuthorityUnavailable => {
            panic!("settled observation must not require durable authority")
        }
    };
    assert!(
        observed
            .iter()
            .any(|event| matches!(&event.event, BackendEvent::WindowCanvasState { .. })),
        "the settled grant must still receive its scoped snapshot"
    );

    let refused = runtime.handle_agent_frontend_event_if_current(
        "pane-client".to_string(),
        grant,
        AgentFrontendRequest::SendInput {
            text: "must-not-dispatch".to_string(),
        },
    );
    assert!(
        matches!(
            refused,
            super::super::AgentFrontendDispatchOutcome::StaleCapability
        ),
        "settled producing mutation must stay refused at runtime dispatch"
    );
}

#[test]
fn agent_pane_list_windows_returns_scoped_state_without_terminal_snapshots() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let foreign_project = temp.path().join("foreign-project");
    let authenticated_project = temp.path().join("authenticated-project");
    fs::create_dir_all(&foreign_project).expect("foreign project");
    fs::create_dir_all(&authenticated_project).expect("authenticated project");
    let foreign_tab = sample_project_tab_with_window_at(
        "tab-foreign",
        "agent-foreign",
        foreign_project,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let authenticated_tab = sample_project_tab_with_window_at(
        "tab-authenticated",
        "agent-authenticated",
        authenticated_project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, _) = sample_runtime_with_events(
        temp.path(),
        vec![foreign_tab, authenticated_tab],
        Some("tab-authenticated"),
    );
    let window_id = "tab-authenticated::agent-authenticated";
    let mut pane = long_running_test_pane(window_id);
    pane.process_bytes(b"pane list must not serialize this snapshot\n");
    let pane = Arc::new(Mutex::new(pane));
    runtime.runtimes.insert(
        window_id.to_string(),
        WindowRuntime::new(
            super::super::next_window_runtime_incarnation(),
            pane.clone(),
        ),
    );
    let principal =
        AgentSessionPrincipal::for_test(&authenticated_project, "session-authenticated")
            .expect("authenticated principal");
    let intake_worktree = temp.path().join(".intake-pane-list");
    fs::create_dir(&intake_worktree).expect("intake-shaped worktree");
    let mut active_session = sample_active_agent_session("tab-authenticated", window_id);
    active_session.worktree_path = intake_worktree;
    runtime
        .active_agent_sessions
        .insert(window_id.to_string(), active_session);
    let (held_tx, held_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let holder = thread::spawn(move || {
        let _pane = pane.lock().expect("hold terminal snapshot mutex");
        held_tx.send(()).expect("announce held snapshot mutex");
        release_rx.recv_timeout(Duration::from_secs(2)).is_ok()
    });
    held_rx.recv().expect("snapshot mutex must be held");

    let events = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        principal,
        AgentFrontendRequest::ListWindows,
    );
    release_tx
        .send(())
        .expect("list response must not wait for the snapshot mutex");
    assert!(
        holder.join().expect("snapshot mutex holder"),
        "list response waited for the terminal snapshot mutex"
    );

    let [OutboundEvent {
        target: DispatchTarget::Client(client_id),
        event: BackendEvent::WindowCanvasState { workspace },
        ..
    }] = events.as_slice()
    else {
        panic!("pane list must return exactly one workspace_state reply");
    };
    assert_eq!(client_id, "pane-client");
    assert_eq!(workspace.tabs.len(), 1);
    assert_eq!(
        workspace.tabs[0].project_root,
        authenticated_project.to_string_lossy()
    );
    assert_eq!(workspace.tabs[0].workspace.windows.len(), 1);
    assert_eq!(workspace.tabs[0].workspace.windows[0].id, window_id);
    assert_eq!(
        workspace.tabs[0].workspace.windows[0].worktree_form,
        gwt::WindowWorktreeForm::Unknown,
        "pane list must use the persisted no-I/O projection instead of resolving Git worktree form"
    );

    let ready_events = runtime.handle_agent_frontend_event(
        "pane-client".to_string(),
        AgentSessionPrincipal::for_test(&authenticated_project, "session-authenticated")
            .expect("authenticated principal"),
        AgentFrontendRequest::Ready,
    );
    let ready_workspace = ready_events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::WindowCanvasState { workspace } => Some(workspace),
            _ => None,
        })
        .expect("frontend_ready workspace state");
    assert_eq!(
        ready_workspace.tabs[0].workspace.windows[0].worktree_form,
        gwt::WindowWorktreeForm::BranchBacked,
        "existing frontend_ready payload must retain resolved worktree-form semantics"
    );
}

#[test]
fn queued_bound_agent_pane_request_rechecks_durable_authority_before_runtime_dispatch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let mut tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-project",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-project", Some("session-project".to_string())));
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    runtime.agent_capability_issuer = Some(issuer.clone());
    let target = issuer
        .issue_bound(
            &project,
            "session-project",
            gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: "session-project".to_string(),
                repo_hash: "repo-stale".to_string(),
                owner_kind: "issue".to_string(),
                owner_number: 2359,
                identity: gwt_agent::ExecutionBindingIdentity {
                    generation_id: "generation-stale".to_string(),
                    binding_id: "binding-stale".to_string(),
                    ledger_head_hash: "head-stale".to_string(),
                },
                capability_generation: 1,
            },
        )
        .expect("bound capability");
    let queued_grant = issuer
        .grant_for_test(&target.token)
        .expect("queued bound grant");

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "pane-client".to_string(),
        queued_grant,
        AgentFrontendRequest::SendInput {
            text: "must-not-reach-pty\r".to_string(),
        },
    );

    assert!(
        matches!(
            outcome,
            super::super::AgentFrontendDispatchOutcome::StaleCapability
        ),
        "a process-local current token with stale durable authority must fail closed"
    );
}

#[test]
fn bound_agent_pane_rotation_after_precheck_rejects_before_pty_write() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("isolated HOME");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project = temp.path().join("project");
    let binding = materialize_active_agent_pane_binding(&project, "session-pane-race");
    let (mut runtime, _issuer, grant) = active_bound_agent_pane_runtime(
        &temp.path().join("runtime"),
        &project,
        "session-pane-race",
        &binding,
    );
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let (rotation_tx, rotation_rx) = std::sync::mpsc::sync_channel(1);
    super::super::set_agent_after_durable_check_test_hook(move || {
        let rotated =
            gwt_agent::rotate_session_execution_capability(&sessions_dir, "session-pane-race");
        rotation_tx.send(rotated).expect("report Host rotation");
    });

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "pane-client".to_string(),
        grant,
        AgentFrontendRequest::SendInput {
            text: "must-not-reach-pty\r".to_string(),
        },
    );

    rotation_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("durable-check race hook runs")
        .expect("rotate Host epoch");
    assert!(
        matches!(
            outcome,
            super::super::AgentFrontendDispatchOutcome::StaleCapability
        ),
        "a Host rotation between the preliminary check and leased dispatch must reject before PTY mutation"
    );
}

#[test]
fn bound_agent_pane_write_holds_epoch_lease_until_actual_pty_mutation_finishes() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("isolated HOME");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project = temp.path().join("project");
    let binding = materialize_active_agent_pane_binding(&project, "session-pane-write");
    let initial_epoch = binding.capability_generation;
    let (mut runtime, _issuer, grant) = active_bound_agent_pane_runtime(
        &temp.path().join("runtime"),
        &project,
        "session-pane-write",
        &binding,
    );
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let (rotation_started_tx, rotation_started_rx) = std::sync::mpsc::sync_channel(1);
    let (rotation_done_tx, rotation_done_rx) = std::sync::mpsc::sync_channel(1);
    let rotation_done_rx = Arc::new(Mutex::new(rotation_done_rx));
    let hook_rotation_done_rx = Arc::clone(&rotation_done_rx);
    super::super::set_agent_leased_mutation_test_hook(move || {
        std::thread::spawn(move || {
            rotation_started_tx
                .send(())
                .expect("signal Host rotation attempt");
            let rotated =
                gwt_agent::rotate_session_execution_capability(&sessions_dir, "session-pane-write");
            rotation_done_tx
                .send(rotated)
                .expect("report Host rotation result");
        });
        rotation_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("Host rotation attempted");
        assert!(
            hook_rotation_done_rx
                .lock()
                .expect("rotation result receiver")
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "Host rotation must wait until the leased PTY mutation returns"
        );
    });

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "pane-client".to_string(),
        grant,
        AgentFrontendRequest::SendInput {
            text: "linearized-input\r".to_string(),
        },
    );

    assert!(matches!(
        outcome,
        super::super::AgentFrontendDispatchOutcome::Dispatched(ref events)
            if matches!(
                events.as_slice(),
                [OutboundEvent {
                    event: BackendEvent::PaneSendResult {
                        ok: true,
                        window_id: Some(window_id),
                        error: None,
                    },
                    ..
                }] if window_id == "tab-project::agent-project"
            )
    ));
    let rotated = rotation_done_rx
        .lock()
        .expect("rotation result receiver")
        .recv_timeout(Duration::from_secs(1))
        .expect("Host rotation resumes after PTY mutation")
        .expect("rotate Host epoch");
    assert_eq!(rotated.capability_generation, initial_epoch + 1);
}

#[test]
fn bound_agent_pane_dispatch_does_not_wait_for_a_contended_session_lease_on_tao_thread() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("isolated HOME");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let project = temp.path().join("project");
    let binding = materialize_active_agent_pane_binding(&project, "session-pane-contended");
    let (mut runtime, _issuer, grant) = active_bound_agent_pane_runtime(
        &temp.path().join("runtime"),
        &project,
        "session-pane-contended",
        &binding,
    );
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let holder_sessions_dir = sessions_dir.clone();
    let (lease_acquired_tx, lease_acquired_rx) = std::sync::mpsc::sync_channel(1);
    let (release_lease_tx, release_lease_rx) = std::sync::mpsc::sync_channel(1);
    let holder = std::thread::spawn(move || {
        gwt_agent::with_session_path_lease_wait(
            &holder_sessions_dir,
            "session-pane-contended",
            Duration::from_secs(1),
            |_state| {
                lease_acquired_tx
                    .send(())
                    .expect("report contended Session lease");
                release_lease_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release contended Session lease");
                Ok(())
            },
        )
        .expect("hold contended Session lease");
    });
    lease_acquired_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("Session lease holder starts");

    let (dispatch_done_tx, dispatch_done_rx) = std::sync::mpsc::sync_channel(1);
    let dispatch = std::thread::spawn(move || {
        let outcome = runtime.handle_agent_frontend_event_if_current(
            "pane-client".to_string(),
            grant,
            AgentFrontendRequest::SendInput {
                text: "must-not-block-tao\r".to_string(),
            },
        );
        dispatch_done_tx
            .send(outcome)
            .expect("report tao dispatch outcome");
    });

    // The holder cannot release its Session lease until this receive finishes
    // and `release_lease_tx` fires below. A successful receive therefore proves
    // dispatch completed while the lease was still contended. The generous
    // timeout is only a deadlock guard; it is not the behavior contract.
    let outcome = dispatch_done_rx.recv_timeout(Duration::from_secs(2));
    release_lease_tx
        .send(())
        .expect("release contended Session lease");
    holder.join().expect("join Session lease holder");
    dispatch.join().expect("join tao dispatch");
    let outcome = outcome.expect("tao dispatch must finish before the contended lease is released");
    assert!(
        matches!(
            outcome,
            super::super::AgentFrontendDispatchOutcome::ExecutionAuthorityUnavailable
        ),
        "tao dispatch must fail closed when its pre-dispatch lease is no longer immediately available"
    );
}

#[test]
fn queued_correlated_self_close_rechecks_generation_before_acceptance() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, events, issuer, queued_grant, window_id) = self_close_runtime(temp.path());
    issuer
        .issue(temp.path().join("project").as_path(), "session-project")
        .expect("rotate capability before queued self-close dispatch");
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "stale-origin-pane".to_string(),
        queued_grant,
        AgentFrontendRequest::CloseWindow {
            id: window_id.clone(),
            request_id: Some("acb37331-72e8-493a-a411-12d060d5a33b".to_string()),
            responder: Some(responder),
        },
    );

    assert!(matches!(
        outcome,
        super::super::AgentFrontendDispatchOutcome::StaleCapability
    ));
    assert!(acceptance.try_recv().is_err());
    assert!(runtime.pending_agent_self_closes.is_empty());
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(events.lock().expect("event log").is_empty());
}

#[test]
fn accepted_agent_self_close_waits_for_direct_ack_then_commits_exactly_once() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, events, issuer, grant, window_id) = self_close_runtime(temp.path());
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "origin-pane-client".to_string(),
        grant.clone(),
        AgentFrontendRequest::CloseWindow {
            id: window_id.clone(),
            request_id: Some("d466de22-4a6b-4b5c-9c88-bdc2ab334f67".to_string()),
            responder: Some(responder),
        },
    );

    assert!(matches!(
        outcome,
        super::super::AgentFrontendDispatchOutcome::Dispatched(ref dispatched)
            if dispatched.is_empty()
    ));
    let accepted = acceptance.try_recv().expect("direct acceptance");
    assert!(!issuer.grant_is_current(&grant));
    assert!(
        issuer
            .issue(temp.path().join("project").as_path(), "session-project")
            .is_err(),
        "the same principal must not be reissued while its close is pending"
    );
    assert!(
        runtime.window_lookup.contains_key(&window_id),
        "acceptance must not remove the window before the direct ACK attempt"
    );
    assert!(runtime.has_pending_agent_self_closes());

    drop(accepted);
    let ticket = take_self_close_commit(&events);
    let replay = ticket.clone();
    let committed = runtime.commit_agent_self_close(ticket);
    assert!(!committed.is_empty());
    assert!(!runtime.window_lookup.contains_key(&window_id));
    assert!(!runtime.has_pending_agent_self_closes());
    assert!(runtime.commit_agent_self_close(replay).is_empty());
    assert!(
        issuer
            .issue(temp.path().join("project").as_path(), "session-project")
            .is_ok(),
        "finalization must release the Closing principal"
    );
}

/// Issue #3783: the correlated self-close ACK moves its bearer into a
/// self-close ticket before Tao commits the window removal. The background
/// close must transfer that ticket into the exact-holder handoff instead of
/// retrying the now-absent bearer token and skipping durable Session cleanup.
#[test]
fn accepted_bound_agent_self_close_persists_terminal_session_in_finalizer() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    init_repo(&project);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let session_id = "session-self-close-exact";
    assert!(tab
        .workspace
        .set_session_id("agent-holder", Some(session_id.to_string())));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    fs::create_dir_all(&runtime.sessions_dir).expect("create capability Session store");
    let window_id = "tab-1::agent-holder".to_string();
    let holder = install_manual_launch_holder(
        &mut runtime,
        &project,
        session_id,
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    let mut durable =
        gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{session_id}.toml")))
            .expect("load holder Session");
    durable.restore_window_on_startup = true;
    durable.save(&runtime.sessions_dir).expect("enable restore");
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load(&runtime.sessions_dir);
    insert_test_pane_runtime(&mut runtime, &window_id);
    let issuer = install_manual_holder_capability(&mut runtime, &project, &window_id, &holder);
    let token = runtime
        .agent_capability_tokens
        .get(&window_id)
        .expect("bound pane token")
        .clone();
    let grant = issuer
        .grant_for_test(&token)
        .expect("authenticated bound pane grant");
    assert_eq!(
        gwt_core::paths::gwt_sessions_dir(),
        runtime.sessions_dir,
        "the capability gate and runtime fixture must share the Session store"
    );
    let holder_session =
        gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{session_id}.toml")))
            .expect("reload exact holder before dispatch");
    assert_eq!(
        holder_session.execution_binding.as_ref(),
        Some(&holder.execution_binding),
        "runtime proof persistence must preserve the holder execution binding"
    );
    assert!(
        gwt::cli::execution_state::current_active_execution_binding_matches(
            &holder_session.worktree_path,
            gwt::cli::execution_state::ExecutionOwnerKey {
                kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 42,
            },
            session_id,
            &holder.execution_binding.identity,
        )
        .expect("probe holder execution authority"),
        "the generation ledger must still authorize the exact holder"
    );
    assert_eq!(
        issuer.durable_authority(&grant),
        crate::embedded_server::AgentDurableAuthority::Current,
        "the exact holder fixture must carry current durable authority"
    );
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "origin-pane-client".to_string(),
        grant,
        AgentFrontendRequest::CloseWindow {
            id: window_id.clone(),
            request_id: Some("4e7b2cc2-69f7-40a6-897a-32754482c970".to_string()),
            responder: Some(responder),
        },
    );
    assert!(
        matches!(
            outcome,
            super::super::AgentFrontendDispatchOutcome::Dispatched(ref events) if events.is_empty()
        ),
        "unexpected self-close dispatch outcome: {outcome:?}"
    );
    drop(acceptance.try_recv().expect("direct self-close acceptance"));
    let ticket = take_self_close_commit(&recorded_events);
    assert!(!runtime.commit_agent_self_close(ticket).is_empty());
    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued exact self-close finalizer");
    finalizer();

    let durable =
        gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{session_id}.toml")))
            .expect("reload terminal holder Session");
    assert_eq!(durable.status, gwt_agent::AgentStatus::Stopped);
    assert!(!durable.restore_window_on_startup);
}

#[test]
fn agent_self_close_disconnect_before_acceptance_rolls_back_the_grant() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, events, issuer, grant, window_id) = self_close_runtime(temp.path());
    let (responder, acceptance) = AgentSelfCloseResponder::channel();
    drop(acceptance);

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "disconnected-pane-client".to_string(),
        grant.clone(),
        AgentFrontendRequest::CloseWindow {
            id: window_id.clone(),
            request_id: Some("89d7c5fc-3894-48dc-a6f4-180af89fb6a3".to_string()),
            responder: Some(responder),
        },
    );

    assert!(matches!(
        outcome,
        super::super::AgentFrontendDispatchOutcome::Dispatched(ref dispatched)
            if dispatched.is_empty()
    ));
    assert!(issuer.grant_is_current(&grant));
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.pending_agent_self_closes.is_empty());
    assert!(events.lock().expect("event log").is_empty());
}

#[test]
fn correlated_agent_self_close_rejects_same_project_peer_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let mut tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-project",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-project", Some("session-project".to_string())));
    let peer = tab
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds());
    assert!(tab
        .workspace
        .set_session_id(&peer.id, Some("session-peer".to_string())));
    let peer_window_id = combined_window_id("tab-project", &peer.id);
    let (mut runtime, _events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    runtime.agent_capability_issuer = Some(issuer.clone());
    let target = issuer
        .issue(&project, "session-project")
        .expect("self capability");
    let grant = issuer
        .grant_for_test(&target.token)
        .expect("authenticated grant");
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "origin-pane-client".to_string(),
        grant.clone(),
        AgentFrontendRequest::CloseWindow {
            id: peer_window_id.clone(),
            request_id: Some("fb5c985e-d51a-419a-9fcf-a0ba1229273a".to_string()),
            responder: Some(responder),
        },
    );

    assert!(matches!(
        outcome,
        super::super::AgentFrontendDispatchOutcome::Dispatched(ref dispatched)
            if dispatched.is_empty()
    ));
    assert!(acceptance.try_recv().is_err());
    assert!(issuer.grant_is_current(&grant));
    assert!(runtime.window_lookup.contains_key(&peer_window_id));
}

#[test]
fn correlated_agent_self_close_fails_closed_for_ambiguous_session_windows() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("project");
    let mut tab = sample_project_tab_with_window_at(
        "tab-project",
        "agent-project",
        project.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-project", Some("session-project".to_string())));
    let duplicate = tab
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds());
    assert!(tab
        .workspace
        .set_session_id(&duplicate.id, Some("session-project".to_string())));
    let (mut runtime, _events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-project"));
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    runtime.agent_capability_issuer = Some(issuer.clone());
    let target = issuer
        .issue(&project, "session-project")
        .expect("self capability");
    let grant = issuer
        .grant_for_test(&target.token)
        .expect("authenticated grant");
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "origin-pane-client".to_string(),
        grant.clone(),
        AgentFrontendRequest::CloseWindow {
            id: "tab-project::agent-project".to_string(),
            request_id: Some("6c00f003-8b70-42b4-be2f-af6f03c62698".to_string()),
            responder: Some(responder),
        },
    );

    assert!(matches!(
        outcome,
        super::super::AgentFrontendDispatchOutcome::Dispatched(ref dispatched)
            if dispatched.is_empty()
    ));
    assert!(acceptance.try_recv().is_err());
    assert!(issuer.grant_is_current(&grant));
    assert_eq!(runtime.window_lookup.len(), 2);
}

#[test]
fn accepted_agent_self_close_survives_external_stop_before_commit() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, events, _issuer, grant, window_id) = self_close_runtime(temp.path());
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "origin-pane-client".to_string(),
        grant,
        AgentFrontendRequest::CloseWindow {
            id: window_id.clone(),
            request_id: Some("e3c14844-f500-477d-b794-ab0fccb003af".to_string()),
            responder: Some(responder),
        },
    );
    assert!(matches!(
        outcome,
        super::super::AgentFrontendDispatchOutcome::Dispatched(_)
    ));
    let accepted = acceptance.try_recv().expect("direct acceptance");

    let stopped = runtime.stop_window_events(&window_id);
    assert!(!stopped.is_empty());
    assert!(runtime.window_lookup.contains_key(&window_id));
    drop(accepted);

    let ticket = take_self_close_commit(&events);
    let committed = runtime.commit_agent_self_close(ticket);
    assert!(!committed.is_empty());
    assert!(!runtime.window_lookup.contains_key(&window_id));
}

#[test]
fn accepted_agent_self_close_does_not_remove_same_id_successor_session() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, events, _issuer, grant, window_id) = self_close_runtime(temp.path());
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "origin-pane-client".to_string(),
        grant,
        AgentFrontendRequest::CloseWindow {
            id: window_id.clone(),
            request_id: Some("0745685c-28cc-4b3f-888e-e11a11bd773c".to_string()),
            responder: Some(responder),
        },
    );
    assert!(matches!(
        outcome,
        super::super::AgentFrontendDispatchOutcome::Dispatched(_)
    ));
    let accepted = acceptance.try_recv().expect("direct acceptance");
    let address = runtime
        .window_lookup
        .get(&window_id)
        .expect("window address")
        .clone();
    assert!(runtime
        .tab_mut(&address.tab_id)
        .expect("tab")
        .workspace
        .set_session_id(&address.raw_id, Some("successor-session".to_string())));
    drop(accepted);

    let ticket = take_self_close_commit(&events);
    assert!(runtime.commit_agent_self_close(ticket).is_empty());
    assert!(
        runtime.window_lookup.contains_key(&window_id),
        "a same-id replacement with a new Session must survive stale close finalization"
    );
}

#[test]
fn accepted_agent_self_close_does_not_remove_same_address_same_session_successor() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, events, issuer, grant, window_id) = self_close_runtime(temp.path());
    let (responder, mut acceptance) = AgentSelfCloseResponder::channel();

    let outcome = runtime.handle_agent_frontend_event_if_current(
        "origin-pane-client".to_string(),
        grant,
        AgentFrontendRequest::CloseWindow {
            id: window_id.clone(),
            request_id: Some("8b99ac42-d645-48a3-a2ed-dcd1cf211d70".to_string()),
            responder: Some(responder),
        },
    );
    assert!(matches!(
        outcome,
        super::super::AgentFrontendDispatchOutcome::Dispatched(_)
    ));
    let accepted = acceptance.try_recv().expect("direct acceptance");
    let address = runtime
        .window_lookup
        .get(&window_id)
        .expect("accepted window address")
        .clone();
    let accepted_generation = runtime
        .window_lifecycle_generations
        .lock()
        .expect("window lifecycle generations")
        .get(&window_id)
        .copied()
        .expect("accepted window generation");

    runtime.register_window(&address.tab_id, &address.raw_id);
    let successor_generation = runtime
        .window_lifecycle_generations
        .lock()
        .expect("window lifecycle generations")
        .get(&window_id)
        .copied()
        .expect("successor window generation");
    assert_ne!(accepted_generation, successor_generation);
    assert_eq!(
        runtime
            .tab(&address.tab_id)
            .and_then(|tab| tab.workspace.window(&address.raw_id))
            .and_then(|window| window.session_id.as_deref()),
        Some("session-project"),
        "the successor intentionally reuses the same Session identity"
    );
    drop(accepted);

    let ticket = take_self_close_commit(&events);
    let replay = ticket.clone();
    assert!(runtime.commit_agent_self_close(ticket).is_empty());
    assert!(runtime.commit_agent_self_close(replay).is_empty());
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(!runtime.has_pending_agent_self_closes());
    assert!(
        issuer
            .issue(temp.path().join("project").as_path(), "session-project")
            .is_ok(),
        "the stale ticket must be irreversibly settled after its no-op commit"
    );
}
