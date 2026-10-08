use super::*;

#[test]
fn project_index_bootstrap_runs_in_background_without_blocking_launch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (proxy, events) = AppEventProxy::stub();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let service = crate::project_index_bootstrap::ProjectIndexBootstrapService::new_for_test();
    let project_root = temp.path().to_path_buf();
    let (spawn_returned_tx, spawn_returned_rx) = mpsc::sync_channel(1);

    let spawn_caller = std::thread::spawn(move || {
        let spawned = service.spawn_with(
            proxy,
            project_root,
            move |_project_root| {
                started_tx.send(()).expect("signal bootstrap start");
                release_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release bootstrap");
                Ok(())
            },
            |_project_root| {
                gwt::ProjectIndexStatusView::new(
                    gwt::ProjectIndexStatusState::Ready,
                    "test bootstrap complete",
                )
            },
        );
        spawn_returned_tx
            .send(spawned)
            .expect("report bootstrap spawn return");
    });

    // The bootstrap body cannot complete until `release_tx` fires below.
    // Receiving the return value first proves launch is independent of the
    // background bootstrap duration. This timeout only bounds a regression.
    let spawned = match spawn_returned_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(spawned) => {
            spawn_caller.join().expect("join bootstrap spawn caller");
            spawned
        }
        Err(error) => {
            let _ = release_tx.send(());
            let _ = spawn_caller.join();
            panic!("bootstrap spawn did not return before body release: {error}");
        }
    };

    assert_eq!(
        spawned,
        crate::project_index_bootstrap::ProjectIndexBootstrapRequest::Spawned
    );
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("background bootstrap should start promptly");
    assert!(
        events.lock().expect("event log").is_empty(),
        "no status should be emitted before the slow bootstrap completes"
    );

    release_tx.send(()).expect("release bootstrap");
    wait_for_recorded_event("project index status", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::ProjectIndexStatus {
                    project_root,
                    status,
                } if project_root == &dunce::canonicalize(temp.path())
                        .unwrap_or_else(|_| temp.path().to_path_buf())
                        .display()
                        .to_string()
                    && status.state == gwt::ProjectIndexStatusState::Ready
                    && status.detail == "test bootstrap complete"
            )
        })
    });
}

#[test]
fn agent_launch_success_dispatches_launch_complete_before_project_index_status() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (proxy, events) = AppEventProxy::stub();
    let completion: AgentLaunchCompletion = (
        ProcessLaunch {
            initial_prompt_file: None,
            command: "agent".to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(temp.path().to_path_buf()),
            resource_policy: None,
        },
        "session-1".to_string(),
        "feature/test".to_string(),
        "Codex".to_string(),
        temp.path().to_path_buf(),
        gwt_agent::AgentId::Codex,
        None,
        None,
        gwt_agent::LaunchRuntimeTarget::Host,
        gwt_agent::SessionMode::Normal,
        false,
        temp.path().display().to_string().into(),
    );

    dispatch_agent_launch_success(
        proxy,
        "tab-1::agent-1".to_string(),
        completion,
        |proxy, project_root| {
            proxy.send(UserEvent::ProjectIndexStatus {
                project_root: project_root.display().to_string(),
                status: Box::new(gwt::ProjectIndexStatusView::new(
                    gwt::ProjectIndexStatusState::Ready,
                    "ready",
                )),
            });
        },
    );

    let recorded = events.lock().expect("events");
    assert!(
        matches!(recorded.first(), Some(UserEvent::LaunchComplete { .. })),
        "LaunchComplete must be emitted first"
    );
    assert!(
        matches!(
            recorded.get(1),
            Some(UserEvent::ProjectIndexStatus {
                project_root,
                status,
            }) if project_root == &temp.path().display().to_string()
                && status.state == gwt::ProjectIndexStatusState::Ready
        ),
        "ProjectIndexStatus must follow LaunchComplete and carry project root"
    );
}

#[test]
fn manual_launch_live_local_holder_requires_typed_decision_before_materialization() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-holder", Some("manual-live-holder".to_string())));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let holder_window_id = combined_window_id("tab-1", "agent-holder");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-live-holder",
        gwt_agent::AgentStatus::Running,
        Some(&holder_window_id),
    );
    insert_test_pane_runtime(&mut runtime, &holder_window_id);
    install_manual_holder_capability(&mut runtime, &repo, &holder_window_id, &holder);
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );

    let wizard = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("holder decision keeps wizard open")
        .wizard
        .view();
    let decision = wizard.holder_decision.expect("exact live holder decision");
    assert_eq!(decision.holder_session_id, holder.session_id);
    assert_eq!(
        decision.holder_window_id.as_deref(),
        Some(holder_window_id.as_str())
    );
    assert!(decision.stop_available);
    assert!(decision.move_available);
    assert!(!wizard.launch_materialization_pending);
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::LaunchWizardState { wizard: Some(view) }
            if view.holder_decision.is_some()
    )));
    assert!(!recorded_events
        .lock()
        .expect("event log")
        .iter()
        .any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
            )
        }));
}

#[test]
fn manual_launch_stop_action_proves_the_exact_local_runtime_terminal_before_materialization() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-holder", Some("manual-stop-holder".to_string())));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let holder_window_id = combined_window_id("tab-1", "agent-holder");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-stop-holder",
        gwt_agent::AgentStatus::Running,
        Some(&holder_window_id),
    );
    insert_test_pane_runtime(&mut runtime, &holder_window_id);
    install_manual_holder_capability(&mut runtime, &repo, &holder_window_id, &holder);
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    let decision = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("holder decision wizard")
        .wizard
        .view()
        .holder_decision
        .expect("local holder decision");

    settle_test_pane_child(&runtime, &holder_window_id);
    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::StopAndStartSuccessor {
            fingerprint: decision.fingerprint,
            window_id: holder_window_id.clone(),
        },
        Some(canvas_bounds()),
    );

    assert!(!runtime.runtimes.contains_key(&holder_window_id));
    assert!(!runtime
        .active_agent_sessions
        .contains_key(&holder_window_id));
    let persisted = gwt_agent::Session::load(
        &runtime
            .sessions_dir
            .join(format!("{}.toml", holder.session_id)),
    )
    .expect("load stopped holder");
    assert_eq!(persisted.status, gwt_agent::AgentStatus::Stopped);
    assert_eq!(
        gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(
            &runtime.sessions_dir,
            &holder.session_id,
        ))
        .expect("exact stopped runtime sidecar")
        .status,
        gwt_agent::AgentStatus::Stopped,
    );
    let handoff_path = gwt_agent::manual_handoff_path(&runtime.sessions_dir, &holder.session_id);
    assert!(
        handoff_path.exists(),
        "Stop must durably fence Active relaunch until successor preparation"
    );
    assert!(
        gwt::cli::execution_state::begin_active_session_launch_handshake(
            &runtime.sessions_dir,
            &holder,
        )
        .expect("relaunch fence check")
        .is_none(),
        "a durable manual handoff must exclude a late Active relaunch"
    );
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::LaunchWizardState { wizard: Some(view) }
            if view.launch_materialization_pending && view.holder_decision.is_none()
    )));
    wait_for_recorded_event(
        "stop-and-start materialization",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. }
                        if matches!(
                            config.as_ref(),
                            gwt::LaunchWizardLaunchRequest::Agent(config)
                                if matches!(
                                    &config.execution_intent,
                                    gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
                                        expected_predecessor,
                                        ..
                                    } if expected_predecessor.as_deref() == Some(&holder)
                                )
                        )
                )
            })
        },
    );
    let mut successor_config = recorded_events
        .lock()
        .expect("event log")
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. } => {
                match config.as_ref().clone() {
                    gwt::LaunchWizardLaunchRequest::Agent(config) => Some(config),
                    gwt::LaunchWizardLaunchRequest::Shell(_) => None,
                }
            }
            _ => None,
        })
        .expect("stop-and-start Agent materialization request");
    runtime
        .prepare_manual_successor_before_pane(&repo, &mut successor_config)
        .expect("the committed stop handoff must be consumed by successor preflight");
    assert!(
        !handoff_path.exists(),
        "Prepared successor commit consumes the exact durable handoff fence"
    );
    assert!(matches!(
        successor_config.execution_intent,
        gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(_)
    ));
}

#[test]
fn manual_launch_stop_materialization_survives_wizard_replacement_exactly_once() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab.workspace.set_session_id(
        "agent-holder",
        Some("manual-replaced-wizard-holder".to_string())
    ));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let holder_window_id = combined_window_id("tab-1", "agent-holder");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-replaced-wizard-holder",
        gwt_agent::AgentStatus::Running,
        Some(&holder_window_id),
    );
    insert_test_pane_runtime(&mut runtime, &holder_window_id);
    install_manual_holder_capability(&mut runtime, &repo, &holder_window_id, &holder);
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    let decision = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.wizard.view().holder_decision)
        .expect("holder decision");
    settle_test_pane_child(&runtime, &holder_window_id);
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::StopAndStartSuccessor {
            fingerprint: decision.fingerprint,
            window_id: holder_window_id,
        },
        Some(canvas_bounds()),
    );
    wait_for_recorded_event(
        "replacement-safe holder materialization",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
                )
            })
        },
    );
    let request = {
        let mut events = recorded_events.lock().expect("event log");
        let index = events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
                )
            })
            .expect("materialization request");
        events.remove(index)
    };
    let UserEvent::LaunchWizardLaunchMaterializationRequested {
        wizard_id,
        client_id,
        config,
        bounds,
    } = request
    else {
        unreachable!("matched above")
    };
    let mut replacement = sample_ready_agent_launch_wizard_session("tab-1", &repo);
    replacement.wizard_id = "replacement-wizard".to_string();
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(replacement);
    let windows_before = runtime.tabs[0].workspace.persisted().windows.len();

    let first = runtime.handle_launch_wizard_launch_materialization_requested(
        wizard_id.clone(),
        client_id.clone(),
        config.as_ref().clone(),
        bounds.clone(),
    );

    assert!(
        !first.is_empty(),
        "the original request must still materialize"
    );
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .as_ref()
            .expect("replacement remains visible")
            .wizard_id,
        "replacement-wizard"
    );
    let windows_after_first = runtime.tabs[0].workspace.persisted().windows.len();
    assert_eq!(windows_after_first, windows_before + 1);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 42,
    };
    let attempts_after_first = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read ledger")
        .expect("ledger")
        .continuation_attempts
        .len();

    let duplicate = runtime.handle_launch_wizard_launch_materialization_requested(
        wizard_id,
        client_id,
        config.as_ref().clone(),
        bounds,
    );

    assert!(duplicate.is_empty());
    assert_eq!(
        runtime.tabs[0].workspace.persisted().windows.len(),
        windows_after_first
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read ledger after duplicate")
            .expect("ledger after duplicate")
            .continuation_attempts
            .len(),
        attempts_after_first
    );
    wait_for_recorded_event(
        "replacement-safe launch completion",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchComplete { .. }
                )
            })
        },
    );
}

#[test]
fn manual_launch_stop_rejects_intent_after_same_window_runtime_is_replaced() {
    assert_manual_launch_action_rejects_replaced_runtime(StaleManualHolderAction::Stop);
}

#[test]
fn manual_launch_move_rejects_intent_after_same_window_runtime_is_replaced() {
    assert_manual_launch_action_rejects_replaced_runtime(StaleManualHolderAction::Move);
}

#[test]
fn manual_launch_stop_rejects_replaced_durable_session_before_killing_pane() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab.workspace.set_session_id(
        "agent-holder",
        Some("manual-durable-replacement".to_string())
    ));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-holder");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-durable-replacement",
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    install_manual_holder_capability(&mut runtime, &repo, &window_id, &holder);
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    let decision = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.wizard.view().holder_decision)
        .expect("holder decision");
    let expected_incarnation = runtime
        .runtimes
        .get(&window_id)
        .expect("holder runtime")
        .incarnation;
    let session_path = runtime
        .sessions_dir
        .join(format!("{}.toml", holder.session_id));
    let mut replacement = gwt_agent::Session::load(&session_path).expect("load holder");
    replacement.agent_id = gwt_agent::AgentId::Custom("replacement".to_string());
    replacement
        .save(&runtime.sessions_dir)
        .expect("replace holder");
    let replacement_bytes = fs::read(&session_path).expect("replacement bytes");

    let logs = capture_tracing_events(|| {
        runtime.handle_launch_wizard_action(
            &runtime.test_context(),
            LaunchWizardAction::StopAndStartSuccessor {
                fingerprint: decision.fingerprint.clone(),
                window_id: window_id.clone(),
            },
            Some(canvas_bounds()),
        );
    });

    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(matches!(
        runtime
            .runtimes
            .get(&window_id)
            .expect("preserved pane")
            .pane
            .lock()
            .expect("pane")
            .check_status()
            .expect("status"),
        gwt_terminal::PaneStatus::Running
    ));
    assert_eq!(
        fs::read(&session_path).expect("session bytes"),
        replacement_bytes
    );
    assert!(recorded_events.lock().expect("events").is_empty());
    let log = logs
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("stop_and_start_successor")
        })
        .expect("holder rejection log");
    assert_eq!(
        log.fields.get("holder_session_id").map(String::as_str),
        Some(holder.session_id.as_str())
    );
    assert_eq!(
        log.fields.get("holder_window_id").map(String::as_str),
        Some(window_id.as_str())
    );
    assert_eq!(
        log.fields
            .get("holder_runtime_incarnation")
            .map(String::as_str),
        Some(expected_incarnation.to_string().as_str())
    );
    let digest = log
        .fields
        .get("holder_fingerprint_digest")
        .expect("holder fingerprint digest");
    assert_eq!(digest.len(), 16);
    assert_ne!(digest, &decision.fingerprint);
    runtime.active_agent_sessions.remove(&window_id);
    runtime.stop_window_runtime_without_session_projection(&window_id);
}

#[test]
fn manual_launch_stop_loses_to_an_existing_cross_process_active_launch_fence() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-holder", Some("manual-fenced-stop".to_string())));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-holder");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-fenced-stop",
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    let handshake = gwt::cli::execution_state::begin_active_session_launch_handshake(
        &runtime.sessions_dir,
        &holder,
    )
    .expect("begin Active relaunch")
    .expect("Active relaunch fence");
    insert_test_pane_runtime(&mut runtime, &window_id);
    install_manual_holder_capability(&mut runtime, &repo, &window_id, &holder);
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    let decision = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.wizard.view().holder_decision)
        .expect("holder decision");
    let session_path = runtime
        .sessions_dir
        .join(format!("{}.toml", holder.session_id));
    let session_before = fs::read(&session_path).expect("Session bytes before losing Stop");

    settle_test_pane_child(&runtime, &window_id);
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::StopAndStartSuccessor {
            fingerprint: decision.fingerprint,
            window_id: window_id.clone(),
        },
        Some(canvas_bounds()),
    );

    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    assert_eq!(
        fs::read(&session_path).expect("Session bytes"),
        session_before
    );
    assert!(recorded_events.lock().expect("events").is_empty());
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.wizard.error.as_deref())
        .is_some_and(|error| error.contains("fenced")));

    assert!(
        gwt::cli::execution_state::finish_active_session_launch_handshake(
            &runtime.sessions_dir,
            &handshake,
        )
        .expect("clear Active relaunch fence")
    );
    runtime.active_agent_sessions.remove(&window_id);
    runtime.stop_window_runtime_without_session_projection(&window_id);
}

#[test]
fn manual_successor_preflight_failure_after_stop_allows_normal_retry() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-holder", Some("manual-retry-after-stop".to_string())));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-holder");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-retry-after-stop",
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    install_manual_holder_capability(&mut runtime, &repo, &window_id, &holder);
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    let decision = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.wizard.view().holder_decision)
        .expect("holder decision");
    settle_test_pane_child(&runtime, &window_id);
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::StopAndStartSuccessor {
            fingerprint: decision.fingerprint,
            window_id,
        },
        Some(canvas_bounds()),
    );
    let materialization = recorded_events
        .lock()
        .expect("events")
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchWizardLaunchMaterializationRequested {
                wizard_id,
                client_id,
                config,
                bounds,
            } => Some((
                wizard_id.clone(),
                client_id.clone(),
                config.as_ref().clone(),
                bounds.clone(),
            )),
            _ => None,
        })
        .expect("materialization request");
    runtime.agent_capability_issuer = None;
    runtime.handle_launch_wizard_launch_materialization_requested(
        materialization.0,
        materialization.1,
        materialization.2,
        materialization.3,
    );
    let session = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("retry wizard");
    assert!(session.wizard.error.is_some());
    assert!(session.wizard.holder_decision.is_none());
    assert!(session.manual_holder_intent.is_some());

    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SetModel {
            model: "default".to_string(),
        },
        Some(canvas_bounds()),
    );
    assert!(!runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("mutable retry wizard")
        .wizard
        .error
        .as_deref()
        .is_some_and(|error| error.contains("Resolve or cancel")));
}

#[test]
fn manual_successor_async_failure_after_prepare_replays_the_exact_operation() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab.workspace.set_session_id(
        "agent-holder",
        Some("manual-async-failure-holder".to_string())
    ));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-holder");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-async-failure-holder",
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    install_manual_holder_capability(&mut runtime, &repo, &window_id, &holder);
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    let decision = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.wizard.view().holder_decision)
        .expect("holder decision");
    settle_test_pane_child(&runtime, &window_id);
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::StopAndStartSuccessor {
            fingerprint: decision.fingerprint,
            window_id,
        },
        Some(canvas_bounds()),
    );
    wait_for_recorded_event(
        "initial manual successor request",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
                )
            })
        },
    );
    let initial_request = {
        let mut events = recorded_events.lock().expect("events");
        let index = events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
                )
            })
            .expect("initial materialization request");
        events.remove(index)
    };
    let UserEvent::LaunchWizardLaunchMaterializationRequested {
        wizard_id,
        client_id,
        config,
        bounds,
    } = initial_request
    else {
        unreachable!("matched above")
    };
    let operation_id = match config.as_ref() {
        gwt::LaunchWizardLaunchRequest::Agent(config) => match &config.execution_intent {
            gwt_agent::ExecutionLaunchIntent::ManualSuccessor { operation_id, .. } => {
                operation_id.clone()
            }
            intent => panic!("expected manual successor intent, got {intent:?}"),
        },
        gwt::LaunchWizardLaunchRequest::Shell(_) => panic!("expected Agent request"),
    };
    let invalid_profile = temp.path().join("invalid-profile.toml");
    fs::write(&invalid_profile, "this is not valid TOML = [").expect("write invalid profile");
    runtime.profile_config_path = Some(invalid_profile);
    runtime.handle_launch_wizard_launch_materialization_requested(
        wizard_id,
        client_id,
        config.as_ref().clone(),
        bounds,
    );
    wait_for_recorded_event(
        "manual successor async failure",
        &recorded_events,
        |events| {
            events
                .iter()
                .any(|event| matches!(recorded_project_payload(event), UserEvent::LaunchComplete { result, .. } if result.is_err()))
        },
    );
    let (failed_window_id, failed_result) = {
        let mut events = recorded_events.lock().expect("events");
        let index = events
            .iter()
            .position(|event| matches!(recorded_project_payload(event), UserEvent::LaunchComplete { result, .. } if result.is_err()))
            .expect("failed completion");
        match into_recorded_project_payload(events.remove(index)) {
            UserEvent::LaunchComplete { window_id, result } => (window_id, result),
            _ => unreachable!("matched above"),
        }
    };
    runtime.handle_launch_complete_and_drain(failed_window_id, *failed_result);
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &repo,
            gwt::cli::execution_state::ExecutionOwnerKey {
                kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 42,
            },
            &operation_id,
        )
        .expect("read prepared attempt")
        .expect("prepared attempt")
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
    );

    runtime.profile_config_path = Some(temp.path().join("missing-default-profile.toml"));
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );

    wait_for_recorded_event("manual successor exact retry", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. }
                    if matches!(
                        config.as_ref(),
                        gwt::LaunchWizardLaunchRequest::Agent(config)
                            if matches!(
                                &config.execution_intent,
                                gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
                                    operation_id: replayed,
                                    ..
                                } if replayed == &operation_id
                            )
                    )
            )
        })
    });
    let mut replay = recorded_events
        .lock()
        .expect("events")
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. } => {
                match config.as_ref().clone() {
                    gwt::LaunchWizardLaunchRequest::Agent(config) => Some(config),
                    gwt::LaunchWizardLaunchRequest::Shell(_) => None,
                }
            }
            _ => None,
        })
        .expect("replayed Agent request");
    runtime
        .prepare_manual_successor_before_pane(&repo, &mut replay)
        .expect("same operation must remain exactly replayable");
    assert!(matches!(
        replay.execution_intent,
        gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(_)
    ));
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(
            &repo,
            gwt::cli::execution_state::ExecutionOwnerKey {
                kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 42,
            },
        )
        .expect("read ledger")
        .expect("ledger")
        .continuation_attempts
        .len(),
        1,
    );
}

#[test]
fn manual_successor_sync_spawn_failure_retains_exact_recovery_for_retry() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab.workspace.set_session_id(
        "agent-holder",
        Some("manual-sync-failure-holder".to_string())
    ));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-holder");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-sync-failure-holder",
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    install_manual_holder_capability(&mut runtime, &repo, &window_id, &holder);
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    let decision = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.wizard.view().holder_decision)
        .expect("holder decision");
    settle_test_pane_child(&runtime, &window_id);
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::StopAndStartSuccessor {
            fingerprint: decision.fingerprint,
            window_id,
        },
        Some(canvas_bounds()),
    );
    wait_for_recorded_event("sync failure request", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
            )
        })
    });
    let request = {
        let mut events = recorded_events.lock().expect("events");
        let index = events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
                )
            })
            .expect("materialization request");
        events.remove(index)
    };
    let UserEvent::LaunchWizardLaunchMaterializationRequested {
        wizard_id,
        client_id: _client_id,
        config,
        bounds,
    } = request
    else {
        unreachable!("matched above")
    };
    let agent_config = match config.as_ref().clone() {
        gwt::LaunchWizardLaunchRequest::Agent(config) => config,
        gwt::LaunchWizardLaunchRequest::Shell(_) => panic!("expected Agent request"),
    };
    let operation_id = match &agent_config.execution_intent {
        gwt_agent::ExecutionLaunchIntent::ManualSuccessor { operation_id, .. } => {
            operation_id.clone()
        }
        intent => panic!("expected manual successor, got {intent:?}"),
    };
    let session = runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .take()
        .expect("materializing wizard remains visible");
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pending_launch_wizard_materializations
        .remove(&wizard_id)
        .expect("consume exact pending materialization snapshot");

    let mut failure = None;
    let logs = capture_tracing_events(|| {
        failure = Some(runtime.materialize_launch_wizard_agent_with(
            session,
            true,
            agent_config,
            |_runtime, _session, _config| Err("injected synchronous spawn failure".to_string()),
        ));
    });
    let failure = failure.expect("synchronous failure events");

    assert!(failure.iter().any(|event| matches!(
        &event.event,
        BackendEvent::LaunchWizardState { wizard: Some(view) }
            if view.error.as_deref() == Some("injected synchronous spawn failure")
    )));
    let recovery = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.manual_holder_intent.as_ref())
        .expect("exact holder recovery survives synchronous spawn failure");
    assert_eq!(recovery.operation_id, operation_id);
    assert_eq!(recovery.predecessor, holder);
    let log = logs
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("spawn_agent_window")
        })
        .unwrap_or_else(|| panic!("structured synchronous spawn failure log: {logs:#?}"));
    assert_eq!(
        log.fields.get("holder_session_id").map(String::as_str),
        Some(holder.session_id.as_str())
    );
    assert_ne!(
        log.fields
            .get("holder_fingerprint_digest")
            .map(String::as_str),
        Some("none")
    );

    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(bounds),
    );
    wait_for_recorded_event("exact sync failure retry", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
            )
        })
    });
    let replay = recorded_events
        .lock()
        .expect("events")
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. } => {
                match config.as_ref().clone() {
                    gwt::LaunchWizardLaunchRequest::Agent(config) => Some(config),
                    gwt::LaunchWizardLaunchRequest::Shell(_) => None,
                }
            }
            _ => None,
        })
        .expect("retry Agent request");
    assert!(matches!(
        &replay.execution_intent,
        gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
            operation_id: replay_operation_id,
            ..
        } if replay_operation_id == &operation_id
    ));
}

#[test]
fn manual_holder_decision_rejects_draft_mutation_and_missing_bounds_before_stop() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let mut tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-holder",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent-holder", Some("manual-fixed-intent".to_string())));
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-holder");
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-fixed-intent",
        gwt_agent::AgentStatus::Running,
        Some(&window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    install_manual_holder_capability(&mut runtime, &repo, &window_id, &holder);
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    let decision = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.wizard.view().holder_decision)
        .expect("holder decision");

    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SetLaunchTarget {
            target: gwt::LaunchTargetKind::Shell,
        },
        Some(canvas_bounds()),
    );
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .as_ref()
            .expect("wizard")
            .wizard
            .launch_target,
        gwt::LaunchTargetKind::Agent
    );
    settle_test_pane_child(&runtime, &window_id);
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::StopAndStartSuccessor {
            fingerprint: decision.fingerprint,
            window_id: window_id.clone(),
        },
        None,
    );
    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(recorded_events.lock().expect("events").is_empty());
    runtime.active_agent_sessions.remove(&window_id);
    runtime.stop_window_runtime_without_session_projection(&window_id);
}

#[test]
fn manual_launch_exact_terminal_holder_materializes_only_the_terminal_successor_intent() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-terminal-holder",
        gwt_agent::AgentStatus::Stopped,
        None,
    );
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::LaunchWizardState { wizard: Some(view) }
            if view.launch_materialization_pending && view.holder_decision.is_none()
    )));
    wait_for_recorded_event(
        "manual terminal successor materialization",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. }
                        if matches!(
                            config.as_ref(),
                            gwt::LaunchWizardLaunchRequest::Agent(config)
                                if matches!(
                                    &config.execution_intent,
                                    gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
                                        expected_predecessor,
                                        ..
                                    } if expected_predecessor.as_deref() == Some(&holder)
                                )
                        )
                )
            })
        },
    );
}

#[test]
fn manual_launch_defunct_exact_holder_replays_through_typed_successor_preflight() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.agent_capability_issuer =
        Some(crate::embedded_server::AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45155/internal/hook-live",
            "ws://127.0.0.1:46255/ws",
            "ws://127.0.0.1:45155/internal/pane-ws",
        ));
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-defunct-holder",
        gwt_agent::AgentStatus::Running,
        None,
    );
    let proof = gwt_agent::ManualLaunchRuntimeProof {
        host_pid: u32::MAX - 1,
        runtime_incarnation: 73,
    };
    gwt_agent::SessionRuntimeState::for_execution_process(
        gwt_agent::AgentStatus::Running,
        &holder,
        proof.runtime_incarnation,
        1,
        u32::MAX - 2,
        1,
    )
    .save(&gwt_agent::runtime_state_path_for_pid(
        &runtime.sessions_dir,
        proof.host_pid,
        &holder.session_id,
    ))
    .expect("persist defunct exact runtime proof");
    let mut fence =
        gwt_agent::with_session_lease(&runtime.sessions_dir, &holder.session_id, |_| {
            gwt_agent::begin_session_manual_handoff_under_lease(
                &runtime.sessions_dir,
                &holder,
                "manual-defunct-holder-fence",
                1,
            )
        })
        .expect("lock exact holder")
        .expect("create exact manual handoff fence");
    fence.host_pid = proof.host_pid;
    fence.host_started_at = 1;
    fs::write(
        gwt_agent::manual_handoff_path(&runtime.sessions_dir, &holder.session_id),
        serde_json::to_vec_pretty(&fence).expect("encode manual handoff fence"),
    )
    .expect("persist exact manual handoff fence");
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );

    wait_for_recorded_event("defunct holder successor", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. }
                    if matches!(
                        config.as_ref(),
                        gwt::LaunchWizardLaunchRequest::Agent(config)
                            if matches!(
                                &config.execution_intent,
                                gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
                                    expected_predecessor,
                                    expected_runtime: Some(runtime),
                                    predecessor_kind:
                                        gwt_agent::ManualLaunchSuccessorPredecessor::ExactTerminalActive,
                                    ..
                                } if expected_predecessor.as_deref() == Some(&holder)
                                    && runtime
                                        == &gwt_agent::ManualLaunchRuntimeEvidence::Proof(proof)
                            )
                    )
            )
        })
    });
    let mut successor = recorded_events
        .lock()
        .expect("events")
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. } => {
                match config.as_ref().clone() {
                    gwt::LaunchWizardLaunchRequest::Agent(config) => Some(config),
                    gwt::LaunchWizardLaunchRequest::Shell(_) => None,
                }
            }
            _ => None,
        })
        .expect("defunct successor request");
    runtime
        .prepare_manual_successor_before_pane(&repo, &mut successor)
        .expect("coordinator revalidates the exact defunct runtime proof");
    assert!(matches!(
        successor.execution_intent,
        gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(_)
    ));
}

/// Issue #3457 retargeted this fixture. Deleting the sidecar outright no
/// longer means "no proof" — absence is now decisive evidence that no Host is
/// running the holder, and refusing there is the permanent-lockout bug this
/// Issue fixes. The safety property under test is unchanged and still worth
/// pinning: when the runtime evidence is genuinely *ambiguous*, the refusal
/// must land before any pane or authority mutation. So the holder now
/// publishes a sidecar the classifier cannot trust instead of none at all.
#[test]
fn manual_launch_ambiguous_terminal_proof_refuses_before_pane_and_authority_mutation() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.agent_capability_issuer =
        Some(crate::embedded_server::AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:45155/internal/hook-live",
            "ws://127.0.0.1:46255/ws",
            "ws://127.0.0.1:45155/internal/pane-ws",
        ));
    let holder = install_manual_launch_holder(
        &mut runtime,
        &repo,
        "manual-unknown-terminal-holder",
        gwt_agent::AgentStatus::Stopped,
        None,
    );
    // Republish the sidecar without an execution identity: the evidence
    // exists but cannot be tied to this holder, which is the ambiguous
    // `Unknown` case rather than the decisive `Absent` one.
    gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Stopped)
        .save(&gwt_agent::runtime_state_path(
            &runtime.sessions_dir,
            &holder.session_id,
        ))
        .expect("publish untrustworthy terminal proof");
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 42,
    };
    let ledger_before = serde_json::to_vec(
        &gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read ledger")
            .expect("ledger"),
    )
    .expect("serialize ledger");
    let projection_before = serde_json::to_vec(
        &gwt::cli::execution_state::load(&repo)
            .expect("read projection")
            .expect("projection"),
    )
    .expect("serialize projection");
    let windows_before = runtime.tabs[0].workspace.persisted().windows.len();
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );

    assert_eq!(
        runtime.tabs[0].workspace.persisted().windows.len(),
        windows_before
    );
    assert!(runtime.runtimes.is_empty());
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .and_then(|session| session.wizard.error.as_deref())
        .is_some_and(|error| error.contains("runtime exit proof")));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::LaunchWizardState { wizard: Some(view) }
            if !view.launch_materialization_pending && view.error.is_some()
    )));
    assert!(!recorded_events
        .lock()
        .expect("event log")
        .iter()
        .any(|event| matches!(
            recorded_project_payload(event),
            UserEvent::LaunchWizardLaunchMaterializationRequested { .. }
        )));
    assert_eq!(
        serde_json::to_vec(
            &gwt::cli::execution_state::load_generation_ledger(&repo, owner)
                .expect("read ledger after refusal")
                .expect("ledger after refusal")
        )
        .expect("serialize ledger after refusal"),
        ledger_before
    );
    assert_eq!(
        serde_json::to_vec(
            &gwt::cli::execution_state::load(&repo)
                .expect("read projection after refusal")
                .expect("projection after refusal")
        )
        .expect("serialize projection after refusal"),
        projection_before
    );
    assert!(
        fs::read_dir(runtime.sessions_dir.join("execution-launch-recovery"))
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true),
        "a rejected preflight must clear its pre-issuance recovery receipt"
    );
}

#[test]
fn manual_launch_completed_and_blocked_use_typed_successor_routes_without_holder_session() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempdir().expect("home tempdir");
    let _home = ScopedEnvVar::set("HOME", home.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
    for (suffix, settlement, expected_kind) in [
        (
            "completed",
            gwt::cli::execution_state::ExecutionSettlement::Completed,
            gwt_agent::ManualLaunchSuccessorPredecessor::Completed,
        ),
        (
            "blocked",
            gwt::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "manual recovery fixture".to_string(),
                missing_verification: None,
            },
            gwt_agent::ManualLaunchSuccessorPredecessor::Blocked,
        ),
    ] {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let (mut runtime, recorded_events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
        fs::create_dir_all(&runtime.sessions_dir).expect("create canonical sessions dir");
        let holder_id = format!("manual-{suffix}-holder");
        install_manual_launch_holder(
            &mut runtime,
            &repo,
            &holder_id,
            gwt_agent::AgentStatus::Stopped,
            None,
        );
        assert!(matches!(
            gwt::cli::execution_state::settle(&repo, &holder_id, settlement)
                .expect("settle manual predecessor"),
            gwt::cli::execution_state::SettleResult::Settled(_)
        ));
        fs::remove_file(runtime.sessions_dir.join(format!("{holder_id}.toml")))
            .expect("remove terminal holder Session fixture");
        runtime
            .project_state_mut(&runtime.test_context())
            .expect("test project state")
            .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

        runtime.handle_launch_wizard_action(
            &runtime.test_context(),
            LaunchWizardAction::Submit,
            Some(canvas_bounds()),
        );

        wait_for_recorded_event("terminal generation route", &recorded_events, |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. }
                        if matches!(
                            config.as_ref(),
                            gwt::LaunchWizardLaunchRequest::Agent(config)
                                if matches!(
                                    &config.execution_intent,
                                    gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
                                        expected_predecessor: None,
                                        predecessor_kind,
                                        ..
                                    } if *predecessor_kind == expected_kind
                                )
                        )
                )
            })
        });
    }
}

#[test]
fn manual_launch_replays_existing_prepared_owner_successor_after_response_loss() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    fs::create_dir_all(&runtime.sessions_dir).expect("create canonical sessions dir");
    let holder_id = "manual-existing-prepared-holder";
    install_manual_launch_holder(
        &mut runtime,
        &repo,
        holder_id,
        gwt_agent::AgentStatus::Stopped,
        None,
    );
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            holder_id,
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("complete predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 42,
    };
    let predecessor = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .unwrap()
        .unwrap();
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "manual-existing-prepared-operation".to_string(),
        principal_id: "gwt-host-manual-launch".to_string(),
        work_id: None,
        source: gwt::cli::execution_state::MANUAL_COMPLETED_OWNER_LAUNCH_SOURCE.to_string(),
        session_binding_id: "manual-existing-prepared-binding".to_string(),
        initial_session_id: "manual-existing-prepared-candidate".to_string(),
        entrypoint: "$gwt-execute #3547".to_string(),
        requested_at: Utc::now(),
    };
    gwt::cli::execution_state::prepare_exact_manual_launch_successor(
        &repo,
        owner,
        &request,
        gwt::cli::execution_state::ExactManualLaunchPredecessor {
            sessions_dir: &runtime.sessions_dir,
            session: None,
            runtime: None,
            binding: &predecessor,
            status: gwt::cli::execution_state::SuccessorPredecessorStatus::Completed,
            terminal_reason: "unused",
        },
    )
    .expect("prepare response-loss fixture");
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );

    wait_for_recorded_event("existing Prepared replay", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. }
                    if matches!(
                        config.as_ref(),
                        gwt::LaunchWizardLaunchRequest::Agent(config)
                            if matches!(
                                &config.execution_intent,
                                gwt_agent::ExecutionLaunchIntent::ManualSuccessor {
                                    operation_id,
                                    ..
                                } if operation_id == &request.operation_id
                            )
                    )
            )
        })
    });
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .unwrap()
        .unwrap();
    assert_eq!(ledger.continuation_attempts.len(), 1);
}

#[test]
fn manual_launch_origin_isolation_and_genesis_keep_automatic_intent() {
    let origins = [
        super::super::LaunchWizardOrigin::Knowledge,
        super::super::LaunchWizardOrigin::StartWork,
        super::super::LaunchWizardOrigin::IssueMonitor,
        super::super::LaunchWizardOrigin::WorkspaceResume,
    ];
    for origin in origins {
        let temp = tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let (mut runtime, recorded_events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        install_manual_launch_holder(
            &mut runtime,
            &repo,
            "origin-isolation-holder",
            gwt_agent::AgentStatus::Running,
            None,
        );
        let mut wizard = sample_ready_agent_launch_wizard_session("tab-1", &repo);
        wizard.origin = origin;
        runtime
            .project_state_mut(&runtime.test_context())
            .expect("test project state")
            .launch_wizard = Some(wizard);

        runtime.handle_launch_wizard_action(
            &runtime.test_context(),
            LaunchWizardAction::Submit,
            Some(canvas_bounds()),
        );

        wait_for_recorded_event(
            "origin-isolated automatic launch",
            &recorded_events,
            |events| {
                events.iter().any(|event| {
                    matches!(
                        recorded_project_payload(event),
                        UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. }
                            if matches!(
                                config.as_ref(),
                                gwt::LaunchWizardLaunchRequest::Agent(config)
                                    if matches!(
                                        config.execution_intent,
                                        gwt_agent::ExecutionLaunchIntent::Automatic
                                    )
                            )
                    )
                })
            },
        );
        assert!(runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .as_ref()
            .is_some_and(|session| session.wizard.holder_decision.is_none()));
    }

    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        Some(canvas_bounds()),
    );
    wait_for_recorded_event("manual genesis", &recorded_events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::LaunchWizardLaunchMaterializationRequested { config, .. }
                    if matches!(
                        config.as_ref(),
                        gwt::LaunchWizardLaunchRequest::Agent(config)
                            if matches!(
                                config.execution_intent,
                                gwt_agent::ExecutionLaunchIntent::Automatic
                            )
                    )
            )
        })
    });
}

#[test]
fn app_runtime_frontend_ready_replies_only_to_requesting_project_client_and_starts_with_workspace()
{
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    runtime
        .window_details
        .insert(window_id.clone(), "Shell ready".to_string());
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));
    runtime.pending_update = Some(gwt_core::update::UpdateState::UpToDate { checked_at: None });

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    assert!(matches!(
        events.get(1),
        Some(event)
            if matches!(&event.target, DispatchTarget::Client(client_id) if client_id == "client-1")
                && matches!(event.event, BackendEvent::WindowCanvasState { .. })
    ));
    assert!(events.iter().all(|event| matches!(
        &event.target,
        DispatchTarget::Client(client_id) if client_id == "client-1"
    )));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalStatus { id, status, detail, .. }
            if id == &window_id
                && *status == WindowProcessStatus::Ready
                && detail.as_deref() == Some("Shell ready")
    )));
    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::LaunchWizardState { wizard: Some(_) }
    )));
    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::UpdateState(gwt_core::update::UpdateState::UpToDate { .. })
    )));
}

#[test]
fn app_runtime_frontend_ready_replies_launch_wizard_tombstone_when_closed() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    let tombstone = events
        .iter()
        .find(|event| {
            matches!(
                event.event,
                BackendEvent::LaunchWizardState { wizard: None }
            )
        })
        .expect("FrontendReady must clear stale Launch Wizard state after reconnect");
    assert!(
        matches!(&tombstone.target, DispatchTarget::Client(client_id) if client_id == "client-1"),
        "Launch Wizard tombstone must be scoped to the reconnecting client"
    );
}

#[test]
fn app_runtime_apply_update_uses_pending_available_update_state() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().join("repo"),
        ProjectKind::Git,
        &[WindowPreset::Shell],
    );
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.pending_update = Some(gwt_core::update::UpdateState::Available {
        current: "9.20.1".to_string(),
        latest: "9.20.2".to_string(),
        release_url: "https://example.invalid/releases/v9.20.2".to_string(),
        asset_url: Some("https://example.invalid/gwt-macos-arm64.dmg".to_string()),
        checked_at: chrono::Utc::now(),
    });

    let outbound =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::ApplyUpdate);

    assert!(outbound.is_empty(), "apply worker dispatch is internal");
    wait_for_recorded_event("pending update apply", &events, |events| {
        events.iter().any(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::ApplyUpdate {
                    client_id,
                    state: gwt_core::update::UpdateState::Available {
                        latest,
                        asset_url: Some(asset_url),
                        ..
                    },
                } if client_id == "client-1"
                    && latest == "9.20.2"
                    && asset_url == "https://example.invalid/gwt-macos-arm64.dmg"
            )
        })
    });
}

#[test]
fn app_runtime_apply_update_without_applicable_pending_update_reports_error() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().join("repo"),
        ProjectKind::Git,
        &[WindowPreset::Shell],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.pending_update = Some(gwt_core::update::UpdateState::Available {
        current: "9.20.1".to_string(),
        latest: "9.20.2".to_string(),
        release_url: "https://example.invalid/releases/v9.20.2".to_string(),
        asset_url: None,
        checked_at: chrono::Utc::now(),
    });

    let outbound =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::ApplyUpdate);

    assert!(outbound.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::UpdateApplyError { message: Some(message), .. }
                if message.contains("No applicable update asset")
        )
    }));
}

#[test]
fn app_runtime_frontend_ready_replays_terminal_snapshot_only_to_requesting_client() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "ping -n 2 127.0.0.1 >nul & exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "sleep 0.2; exit 0".to_string()],
        )
    };
    let mut pane = Pane::new(
        window_id.clone(),
        command,
        args,
        80,
        24,
        HashMap::new(),
        test_pane_cwd(),
    )
    .expect("pane");
    pane.process_bytes(b"hello from frontend ready\n");
    runtime.runtimes.insert(
        window_id.clone(),
        WindowRuntime::new(
            super::super::next_window_runtime_incarnation(),
            Arc::new(Mutex::new(pane)),
        ),
    );

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    assert!(events.iter().all(|event| matches!(
        &event.target,
        DispatchTarget::Client(client_id) if client_id == "client-1"
    )));
    let snapshot = events.iter().find_map(|event| match &event.event {
        BackendEvent::TerminalSnapshot { id, data_base64 } if id == &window_id => Some(data_base64),
        _ => None,
    });
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(snapshot.expect("terminal snapshot event"))
        .expect("decode terminal snapshot");
    assert!(String::from_utf8_lossy(&decoded).contains("hello from frontend ready"));
}

#[test]
fn app_runtime_frontend_ready_replays_terminal_snapshot_with_sgr_attributes() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };
    let mut pane = Pane::new(
        window_id.clone(),
        command,
        args,
        80,
        24,
        HashMap::new(),
        test_pane_cwd(),
    )
    .expect("pane");
    // Write red foreground + bold "ALERT" then reset, then default-color text.
    pane.process_bytes(b"\x1b[31;1mALERT\x1b[0m normal\n");

    runtime.runtimes.insert(
        window_id.clone(),
        WindowRuntime::new(
            super::super::next_window_runtime_incarnation(),
            Arc::new(Mutex::new(pane)),
        ),
    );

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    let snapshot = events.iter().find_map(|event| match &event.event {
        BackendEvent::TerminalSnapshot { id, data_base64 } if id == &window_id => Some(data_base64),
        _ => None,
    });
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(snapshot.expect("terminal snapshot event"))
        .expect("decode terminal snapshot");
    // Visible text must be present.
    assert!(
        String::from_utf8_lossy(&decoded).contains("ALERT"),
        "expected ALERT text in snapshot bytes, got: {:?}",
        String::from_utf8_lossy(&decoded)
    );
    // SGR escape sequence introducing a styled run (CSI ... m) must be present
    // so that xterm.js can replay foreground / bold / etc. from the snapshot.
    let has_sgr = decoded.windows(2).enumerate().any(|(idx, win)| {
        win == [0x1b, b'['] && {
            let tail = &decoded[idx + 2..];
            tail.iter().take(16).any(|b| *b == b'm')
        }
    });
    assert!(
            has_sgr,
            "expected SGR escape (CSI ... m) in TerminalSnapshot bytes so xterm.js can replay color/style; raw snapshot bytes: {:?}",
            decoded
        );
}

#[test]
fn app_runtime_frontend_ready_replays_terminal_snapshot_with_scrollback_history() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };
    let mut pane = Pane::new(
        window_id.clone(),
        command,
        args,
        80,
        6,
        HashMap::new(),
        test_pane_cwd(),
    )
    .expect("pane");
    for line in 1..=18 {
        pane.process_bytes(format!("SCROLLBACK-LINE-{line:03}\r\n").as_bytes());
    }

    runtime.runtimes.insert(
        window_id.clone(),
        WindowRuntime::new(
            super::super::next_window_runtime_incarnation(),
            Arc::new(Mutex::new(pane)),
        ),
    );

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    let snapshot = events.iter().find_map(|event| match &event.event {
        BackendEvent::TerminalSnapshot { id, data_base64 } if id == &window_id => Some(data_base64),
        _ => None,
    });
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(snapshot.expect("terminal snapshot event"))
        .expect("decode terminal snapshot");
    let text = String::from_utf8_lossy(&decoded);
    assert!(
        text.contains("SCROLLBACK-LINE-001"),
        "expected frontend reconnect snapshot to include old scrollback history, got: {text:?}"
    );
    assert!(
        text.contains("SCROLLBACK-LINE-018"),
        "expected frontend reconnect snapshot to include current visible screen, got: {text:?}"
    );
}

#[test]
fn app_runtime_dock_window_tab_preserves_real_fit_pty_sizes() {
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
    for window_id in [&shell_id, &claude_id] {
        let pane = Pane::new(
            window_id.clone(),
            if cfg!(windows) { "cmd" } else { "/bin/sh" }.to_string(),
            if cfg!(windows) {
                vec![
                    "/d".to_string(),
                    "/s".to_string(),
                    "/c".to_string(),
                    "exit /b 0".to_string(),
                ]
            } else {
                vec!["-lc".to_string(), "exit 0".to_string()]
            },
            80,
            24,
            HashMap::new(),
            test_pane_cwd(),
        )
        .expect("pane");
        runtime.runtimes.insert(
            window_id.clone(),
            WindowRuntime::new(
                super::super::next_window_runtime_incarnation(),
                Arc::new(Mutex::new(pane)),
            ),
        );
    }

    const REAL_COLS: u16 = 151;
    const REAL_ROWS: u16 = 43;
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

    let target_geometry = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .window("claude-1")
        .expect("claude")
        .geometry
        .clone();
    assert_ne!(
        geometry_to_pty_size(&target_geometry),
        (REAL_COLS, REAL_ROWS),
        "sentinel must differ from the dock geometry approximation",
    );

    let events = runtime.dock_window_tab_events(&shell_id, &claude_id);

    assert_eq!(events.len(), 1);
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
            "dock must not clobber the frontend-fitted PTY size",
        );
    }
}

#[test]
fn app_runtime_detach_window_tab_preserves_real_fit_pty_size() {
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
        1,
    );
    insert_test_pane_runtime(&mut runtime, &shell_id);

    const REAL_COLS: u16 = 149;
    const REAL_ROWS: u16 = 37;
    runtime
        .runtimes
        .get(&shell_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane")
        .resize(REAL_COLS, REAL_ROWS)
        .expect("resize");

    let detached_geometry = WindowGeometry {
        x: 120.0,
        y: 80.0,
        width: 920.0,
        height: 580.0,
    };
    assert_ne!(
        geometry_to_pty_size(&detached_geometry),
        (REAL_COLS, REAL_ROWS),
        "sentinel must differ from the detached geometry approximation",
    );

    let events = runtime.detach_window_tab_events(&shell_id, detached_geometry.clone());

    assert_eq!(events.len(), 1);
    assert_eq!(runtime.active_tab_id.as_deref(), Some("tab-1"));
    let detached = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .window("shell-1")
        .expect("shell");
    assert_eq!(detached.geometry, detached_geometry);
    assert_eq!(detached.tab_group_id, None);
    let pane = runtime
        .runtimes
        .get(&shell_id)
        .expect("runtime")
        .pane
        .lock()
        .expect("pane");
    assert_eq!(
        pane.screen().size(),
        (REAL_ROWS, REAL_COLS),
        "detach must not clobber the frontend-fitted PTY size",
    );
}

#[test]
fn app_runtime_dock_to_issue_refusals_notify_without_detaching() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::Agent, WindowPreset::Claude],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let agent_id = combined_window_id("tab-1", "agent-1");
    runtime
        .tab_mut("tab-1")
        .unwrap()
        .workspace
        .dock_window_tab("agent-1", "claude-1");
    for (linked_issue, reason) in [
        (None, "no linked Issue"),
        (Some(4812), "no Issue or Issue Monitor"),
    ] {
        let workspace = &mut runtime.tab_mut("tab-1").unwrap().workspace;
        workspace.set_linked_issue_number("agent-1", linked_issue);
        let before = workspace.persisted().clone();
        let events = runtime.dock_agent_window_to_issue_events(&agent_id);
        assert_eq!(events.len(), 1, "refusal must reach the frontend");
        assert!(matches!(&events[0].event, BackendEvent::IssueMonitorToast {
            level, message, issue_number, ..
        } if level == "warn" && message.contains(reason) && *issue_number == linked_issue));
        assert!(matches!(events[0].target, DispatchTarget::Project(_)));
        assert_eq!(runtime.tab("tab-1").unwrap().workspace.persisted(), &before);
    }
}

#[test]
fn app_runtime_places_agent_window_in_kanban_from_frontend_event() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[WindowPreset::AgentKanban, WindowPreset::Agent],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let agent_id = combined_window_id("tab-1", "agent-1");
    let board_id = combined_window_id("tab-1", "agent-kanban-1");

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::PlaceAgentWindowInKanban {
            id: agent_id,
            board_id,
            lane_id: gwt::AgentKanbanLane::Active,
            order: None,
        },
    );

    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "Kanban placement must broadcast workspace state"
    );
    let agent = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .window("agent-1")
        .expect("agent");
    assert_eq!(
        agent.placement,
        WindowPlacement::AgentKanban {
            board_id: "agent-kanban-1".to_string(),
            lane_id: gwt::AgentKanbanLane::Active,
            order: 0,
            collapsed: false,
        }
    );
}

#[test]
fn app_runtime_open_agent_kanban_launch_wizard_records_launch_target() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::AgentKanban],
    );
    let other_tab = sample_project_tab(
        "tab-2",
        "Other Repo",
        temp.path().join("other"),
        ProjectKind::NonRepo,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab, other_tab], Some("tab-2"));
    let board_id = combined_window_id("tab-1", "agent-kanban-1");

    let events = runtime.handle_frontend_event_in_scope(
        "client-1".to_string(),
        FrontendEvent::OpenAgentKanbanLaunchWizard {
            board_id,
            lane_id: gwt::AgentKanbanLane::Blocked,
        },
        &super::super::ClientScope::Project(runtime.project_context("tab-1").unwrap().project_key),
    );

    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "Kanban Launch Agent must open the normal Launch Agent wizard"
    );
    assert_eq!(runtime.active_tab_id.as_deref(), Some("tab-2"));
    assert!(events.iter().all(|event| matches!(&event.target, DispatchTarget::Project(key) if key == &runtime.project_context("tab-1").unwrap().project_key)));
    let session = runtime
        .project_state(&runtime.project_context("tab-1").unwrap())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard");
    let view = session.wizard.view();
    assert_eq!(view.title, "Launch Agent");
    assert_ne!(
        view.mode,
        gwt::LaunchWizardMode::StartWork,
        "Kanban Launch Agent must not require Start Work branch materialization"
    );
    let target = session
        .agent_kanban_target
        .as_ref()
        .expect("agent kanban launch target");
    assert_eq!(target.board_id, "agent-kanban-1");
    assert_eq!(target.lane_id, gwt::AgentKanbanLane::Blocked);
}

#[test]
fn app_runtime_spawn_agent_window_in_agent_kanban_places_new_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::AgentKanban],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260619-kanban")
        .build();

    runtime
        .spawn_agent_window_in_agent_kanban(
            "tab-1",
            config,
            canvas_bounds(),
            None,
            None,
            AgentKanbanLaunchTarget {
                board_id: "agent-kanban-1".to_string(),
                lane_id: gwt::AgentKanbanLane::Active,
            },
        )
        .expect("spawn agent in kanban");

    let agent = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(
        agent.placement,
        WindowPlacement::AgentKanban {
            board_id: "agent-kanban-1".to_string(),
            lane_id: gwt::AgentKanbanLane::Active,
            order: 0,
            collapsed: false,
        }
    );
}

#[test]
fn app_runtime_spawn_agent_window_in_agent_kanban_falls_back_to_canvas_when_board_missing() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::AgentKanban],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch("work/20260619-kanban-fallback")
        .build();

    runtime
        .spawn_agent_window_in_agent_kanban(
            "tab-1",
            config,
            canvas_bounds(),
            None,
            None,
            AgentKanbanLaunchTarget {
                board_id: "missing-board".to_string(),
                lane_id: gwt::AgentKanbanLane::Active,
            },
        )
        .expect("spawn agent even when kanban placement is unavailable");

    let agent = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window");
    assert_eq!(agent.placement, WindowPlacement::Canvas);
}

// SPEC-3671 FR-002 / T-007: an Issue Monitor auto-launch is mirrored inside the Issue
// window instead of opening a canvas window.
#[test]
fn app_runtime_issue_monitor_launch_places_agent_window_in_issue_preview() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::Issue],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let issue_window_id = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Issue)
        .expect("issue window")
        .id
        .clone();
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
        .branch("work/issue-3671")
        .build();

    runtime
        .spawn_agent_window_with_feedback(
            "tab-1",
            config,
            canvas_bounds(),
            None,
            issue_monitor_feedback(3671),
        )
        .expect("issue monitor launch");

    assert_eq!(
        spawned_agent_placement(&runtime, "tab-1"),
        WindowPlacement::IssuePreview {
            issue_window_id,
            issue_number: 3671,
        }
    );
}

// SPEC-3671 FR-002: without an Issue window there is nothing to mirror into, so the
// launch keeps the canvas placement rather than becoming invisible.
#[test]
fn app_runtime_issue_monitor_launch_falls_back_to_canvas_without_issue_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::Board],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
        .branch("work/issue-3671-fallback")
        .build();

    runtime
        .spawn_agent_window_with_feedback(
            "tab-1",
            config,
            canvas_bounds(),
            None,
            issue_monitor_feedback(3671),
        )
        .expect("issue monitor launch without issue window");

    assert_eq!(
        spawned_agent_placement(&runtime, "tab-1"),
        WindowPlacement::Canvas
    );
}

// SPEC-3671 FR-003 / T-009: Start Work and Launch Agent keep opening canvas windows.
#[test]
fn app_runtime_manual_launch_keeps_canvas_placement_with_issue_window_open() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo,
        ProjectKind::Git,
        &[WindowPreset::Issue],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    runtime
        .spawn_agent_window(
            "tab-1",
            gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
                .branch("work/manual-launch")
                .build(),
            canvas_bounds(),
            None,
        )
        .expect("manual launch");

    assert_eq!(
        spawned_agent_placement(&runtime, "tab-1"),
        WindowPlacement::Canvas
    );

    // A Launch Agent launch carries feedback but no Issue Monitor issue number.
    runtime
        .spawn_agent_window_with_feedback(
            "tab-1",
            gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
                .branch("work/manual-launch-feedback")
                .build(),
            canvas_bounds(),
            None,
            LaunchFeedbackContext {
                client_id: "client-1".to_string(),
                title: "Launch Agent".to_string(),
                issue_monitor_issue_number: None,
                issue_monitor_delivery_id: None,
                issue_monitor_project_root: None,
                issue_monitor_session_mode: None,
                issue_monitor_autonomous_handoff: None,
                issue_monitor_autonomous_submit_started: false,
                issue_monitor_review_dispatch: false,
            },
        )
        .expect("manual launch with feedback");

    for window in &runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
    {
        if window.preset == WindowPreset::Agent {
            assert_eq!(
                window.placement,
                WindowPlacement::Canvas,
                "manual launches must stay on the canvas"
            );
        }
    }
}

// SPEC-3671 FR-006 / T-013: Issue Monitor delivery tracking keys off the window id, not
// the placement, so an off-canvas preview stays fully tracked.
#[test]
fn app_runtime_issue_monitor_tracks_launched_window_id_for_issue_preview() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Issue],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let delivery_id = "launch:issue-preview-tracking".to_string();
    let mut feedback = issue_monitor_feedback(3671);
    feedback.issue_monitor_delivery_id = Some(delivery_id.clone());
    feedback.issue_monitor_project_root = Some(repo.clone());
    let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
        .branch("work/issue-3671-tracking")
        .build();

    runtime
        .spawn_agent_window_with_feedback("tab-1", config, canvas_bounds(), None, feedback)
        .expect("issue monitor launch");

    let raw_id = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("agent window")
        .id
        .clone();
    let window_id = super::super::combined_window_id("tab-1", &raw_id);

    assert!(matches!(
        runtime.issue_monitor_launch_deliveries.get(&delivery_id),
        Some(super::super::IssueMonitorLaunchDeliveryState::Materializing { window_id: tracked, .. })
            if tracked == &window_id
    ));
    assert_eq!(
        runtime.issue_monitor_issue_number_for_window(&repo, &window_id),
        Some(3671),
        "an off-canvas preview must still resolve back to its Issue number"
    );
}
