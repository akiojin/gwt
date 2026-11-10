use super::ClientScope;

#[test]
fn remote_operation_failure_preserves_lifecycle_and_recovers_after_reattach() {
    use super::{RemoteRuntimeCommandCompletion, RemoteRuntimeCommandKind};
    use gwt::project_runtime::events::{OwnerCommand, OwnerEvent};

    let _env = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, probe, id) = owner_browser_input_fixture(temp.path());
    let context = runtime.test_context();
    let identity = runtime.runtimes[&id].owner_identity.clone();
    let original_status = runtime.runtimes[&id].status;
    let completion = |kind| RemoteRuntimeCommandCompletion {
        context: context.clone(),
        connection_id: 1,
        identity: identity.clone(),
        id: id.clone(),
        incarnation: probe.incarnation,
        kind,
        approval_resolution_started: false,
        input_command_id: None,
        result: Err("project command timed out".into()),
    };
    let mut wire = Vec::new();
    for (kind, operation) in [
        (
            RemoteRuntimeCommandKind::Input { reply_client: None },
            "input",
        ),
        (RemoteRuntimeCommandKind::Resize, "resize"),
        (RemoteRuntimeCommandKind::Stop, "stop"),
    ] {
        let events = runtime.handle_remote_runtime_command_complete(completion(kind));
        assert_eq!(events.len(), 1, "one operation error, no lifecycle frames");
        assert!(
            matches!(&events[0].target, DispatchTarget::Project(key) if key == &context.project_key)
        );
        let value = serde_json::to_value(&events[0].event).unwrap();
        assert_eq!(value["kind"], "window_command_error");
        assert_eq!(value["operation"], operation);
        assert_eq!(value["message"], "project command timed out");
        assert_eq!(
            events[0].event.delivery_policy().delivery,
            gwt::protocol::BackendEventDeliveryClass::Error
        );
        assert_eq!(runtime.runtimes[&id].status, original_status);
        assert!(!runtime.runtimes[&id].exit_confirmed);
        wire.push(value);
    }
    let ack = runtime.handle_remote_runtime_command_complete(completion(
        RemoteRuntimeCommandKind::Input {
            reply_client: Some("origin".into()),
        },
    ));
    assert!(matches!(&ack[0].target, DispatchTarget::Client(client) if client == "origin"));
    assert!(matches!(
        &ack[0].event,
        BackendEvent::PaneSendResult {
            ok: false,
            error: Some(_),
            ..
        }
    ));
    let mut stale = completion(RemoteRuntimeCommandKind::Resize);
    stale.incarnation += 1;
    assert!(runtime
        .handle_remote_runtime_command_complete(stale)
        .is_empty());

    // Both pre-send fences defer delivery; no worker may send to the old pane.
    let pane_id = runtime.runtimes[&id].owner_pane_id.clone();
    runtime
        .project_state_mut(&context)
        .unwrap()
        .project_owner_identity = None;
    for empty_pane in [false, true] {
        if empty_pane {
            runtime
                .project_state_mut(&context)
                .unwrap()
                .project_owner_identity = Some(identity.clone());
            runtime.runtimes.get_mut(&id).unwrap().owner_pane_id.clear();
        }
        let events = runtime.queue_remote_runtime_command(
            &id,
            OwnerCommand::Resize {
                id: pane_id.clone(),
                incarnation: probe.incarnation,
                cols: 90,
                rows: 30,
            },
            RemoteRuntimeCommandKind::Resize,
        );
        assert!(
            events.is_empty(),
            "transient owner drift must recover automatically"
        );
    }

    // Use normal health -> inventory -> attach, never repair the stale fields by hand.
    runtime.handle_project_owner_health(
        &context,
        2,
        Some(identity.clone()),
        gwt::protocol::ProjectOwnerHealthView {
            project_key: context.project_key.to_string(),
            state: gwt::protocol::ProjectOwnerHealthState::Healthy,
            detail: None,
        },
    );
    let BlockingTaskSpawner::Queued(tasks) = &runtime.blocking_tasks else {
        unreachable!()
    };
    drain_queued_blocking_tasks(&Arc::clone(tasks));
    let AppEventProxy::Stub(recorded) = &runtime.proxy else {
        unreachable!()
    };
    let completed = std::mem::take(&mut *recorded.lock().unwrap());
    for event in completed {
        if let Some(UserEvent::RemoteOwnerInventoryComplete(completion)) =
            runtime.accept_project_completion(event)
        {
            runtime.handle_remote_owner_inventory_complete(*completion);
        }
    }
    assert_eq!(runtime.runtimes[&id].owner_pane_id, pane_id);
    assert_eq!(runtime.runtimes[&id].owner_identity, identity);
    assert!(runtime
        .pane_send_input_to_window_events("origin".into(), &id, "reattached\n")
        .is_empty());
    let ack = drain_browser_owner_input(&mut runtime);
    assert!(ack
        .iter()
        .any(|event| matches!(event.event, BackendEvent::PaneSendResult { ok: true, .. })));
    assert!(probe.pty.try_wait().unwrap().is_none());

    // A real owner/PTY error is authoritative even without confirmed process exit.
    let owner = project_owner_fixture(&mut runtime, &id);
    let mut source = owner.subscribe();
    owner_fixture_block_on(
        owner.pane_event(gwt::pane_runtime::PaneRuntimeEvent::Status {
            id: probe.id.clone(),
            incarnation: probe.incarnation,
            status: WindowProcessStatus::Error,
            detail: Some("actual PTY read failure".into()),
            exit_confirmed: false,
        }),
    )
    .unwrap();
    while let Ok(event) = source.try_recv() {
        if matches!(event, OwnerEvent::Status { .. }) {
            wire.extend(
                runtime
                    .handle_project_owner_event(&context, 2, &identity, event)
                    .into_iter()
                    .map(|event| serde_json::to_value(event.event).unwrap()),
            );
        }
    }
    assert_eq!(runtime.runtimes[&id].status, WindowProcessStatus::Error);
    assert!(runtime.runtimes[&id]
        .detail
        .as_deref()
        .unwrap()
        .starts_with("actual PTY read failure"));
    assert!(!runtime.runtimes[&id].exit_confirmed);
    println!("ISSUE3340_EVENTS={}", serde_json::to_string(&wire).unwrap());
}

#[test]
fn remote_command_recovers_after_owner_identity_switch_and_health_recovery() {
    use gwt::protocol::{ProjectOwnerHealthState, ProjectOwnerHealthView};
    let _env = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for identity_switch in [true, false] {
        let temp = tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let (mut runtime, probe, id) = owner_browser_input_fixture(temp.path());
        let context = runtime.test_context();
        let identity = runtime.runtimes[&id].owner_identity.clone();
        if identity_switch {
            // The authenticated fixture owner replaces the GUI's stale owner.
            runtime
                .runtimes
                .get_mut(&id)
                .unwrap()
                .owner_identity
                .host_instance_id = "previous-owner-instance".into();
        } else {
            runtime.handle_project_owner_health(
                &context,
                2,
                None,
                ProjectOwnerHealthView {
                    project_key: context.project_key.to_string(),
                    state: ProjectOwnerHealthState::Degraded,
                    detail: Some("fixture reconnect".into()),
                },
            );
        }
        let events = runtime.pane_send_input_to_window_events("origin".into(), &id, "recover\n");
        assert!(
            events.is_empty(),
            "owner drift must defer, not refuse: {events:?}"
        );
        runtime.handle_project_owner_health(
            &context,
            3,
            Some(identity),
            ProjectOwnerHealthView {
                project_key: context.project_key.to_string(),
                state: ProjectOwnerHealthState::Healthy,
                detail: None,
            },
        );
        let events = finish_remote_command_recovery(&mut runtime);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.event,
                    BackendEvent::PaneSendResult { ok: true, .. }
                ))
                .count(),
            1
        );
        assert!(!events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::PaneSendResult { ok: false, .. })));
        assert!(probe.pty.try_wait().unwrap().is_none());
        assert!(
            finish_remote_command_recovery(&mut runtime).is_empty(),
            "never replay a completed command"
        );
    }
}

#[test]
fn remote_command_owner_recovery_expires_with_reason_and_attempts() {
    let _env = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, _probe, id) = owner_browser_input_fixture(temp.path());
    let context = runtime.test_context();
    runtime.runtimes.get_mut(&id).unwrap().owner_pane_id.clear();
    runtime.handle_project_owner_health(
        &context,
        2,
        None,
        gwt::protocol::ProjectOwnerHealthView {
            project_key: context.project_key.to_string(),
            state: gwt::protocol::ProjectOwnerHealthState::Degraded,
            detail: None,
        },
    );
    assert!(runtime
        .pane_send_input_to_window_events("origin".into(), &id, "expire\n")
        .is_empty());
    let start = Instant::now();
    assert!(runtime.next_runtime_output_deadline().is_some());
    // Drive the existing event-loop clock directly; no wall-clock sleep.
    let events = runtime.flush_runtime_output_batches(start + Duration::from_secs(11));
    let error = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::PaneSendResult {
                ok: false, error, ..
            } => error.as_ref(),
            _ => None,
        })
        .expect("expired command outcome");
    assert!(error.contains("Degraded"), "{error}");
    assert!(error.contains("identity mismatch"), "{error}");
    assert!(error.contains("owner pane unset"), "{error}");
    assert!(error.contains("Automatic recovery"), "{error}");
    assert!(error.contains("ms") && error.contains("attempt"), "{error}");
    assert!(
        !error.contains("Refresh") && !error.contains("try again"),
        "{error}"
    );
    assert!(
        !runtime.pane_has_unsent_user_input(&id),
        "expired input must clear its speculative busy hint"
    );
}

#[test]
fn remote_command_recovery_rejects_a_successor_session() {
    let _env = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, _probe, id) = owner_browser_input_fixture(temp.path());
    let context = runtime.test_context();
    let identity = runtime.runtimes[&id].owner_identity.clone();
    runtime
        .project_state_mut(&context)
        .unwrap()
        .project_owner_identity = None;
    assert!(runtime
        .pane_send_input_to_window_events("origin".into(), &id, "old session input\n")
        .is_empty());
    // An owner restart may reuse a pane ID and incarnation for another Session.
    runtime
        .tab_mut(&context.tab_id)
        .unwrap()
        .workspace
        .set_session_id("agent", Some("successor-session".into()));
    runtime
        .project_state_mut(&context)
        .unwrap()
        .project_owner_identity = Some(identity);
    let events = runtime.retry_remote_runtime_commands(&id, Instant::now());
    assert!(events.iter().any(|event| matches!(&event.event,
        BackendEvent::PaneSendResult { ok: false, error: Some(error), .. } if error.contains("Session"))));
    let BlockingTaskSpawner::Queued(tasks) = &runtime.blocking_tasks else {
        panic!("queued fixture")
    };
    assert!(
        tasks.lock().unwrap().is_empty(),
        "never send old input to a successor Session"
    );
}

fn finish_remote_command_recovery(runtime: &mut AppRuntime) -> Vec<OutboundEvent> {
    let BlockingTaskSpawner::Queued(tasks) = &runtime.blocking_tasks else {
        panic!("queued fixture")
    };
    let tasks = Arc::clone(tasks);
    let AppEventProxy::Stub(completions) = &runtime.proxy else {
        panic!("stub fixture")
    };
    let completions = Arc::clone(completions);
    let mut events = Vec::new();
    loop {
        let queued = std::mem::take(&mut *tasks.lock().unwrap());
        let had_tasks = !queued.is_empty();
        for task in queued {
            task();
        }
        let completed = std::mem::take(&mut *completions.lock().unwrap());
        let had_completions = !completed.is_empty();
        for event in completed {
            match runtime.accept_project_completion(event) {
                Some(UserEvent::RemoteOwnerInventoryComplete(completion)) => {
                    events.extend(runtime.handle_remote_owner_inventory_complete(*completion))
                }
                Some(UserEvent::RemoteRuntimeCommandComplete(completion)) => {
                    events.extend(runtime.handle_remote_runtime_command_complete(*completion))
                }
                Some(UserEvent::ProjectOwnerSnapshot(completion)) => {
                    events.extend(runtime.handle_project_owner_snapshot(*completion))
                }
                _ => {}
            }
        }
        if !had_tasks && !had_completions {
            break;
        }
    }
    events
}

fn aggregate_test_runtime(root: &Path) -> AppRuntime {
    let tabs = ["a", "b"]
        .into_iter()
        .map(|id| {
            let mut tab = sample_project_tab_with_window_at(
                id,
                "agent",
                root.join(id),
                WindowPreset::Agent,
                WindowProcessStatus::Running,
            );
            let mut persisted = tab.workspace.persisted().clone();
            persisted.windows[0].placement = WindowPlacement::IssuePreview {
                issue_window_id: "issue".into(),
                issue_number: 4540,
            };
            persisted.windows.push(sample_window(
                "shell",
                WindowPreset::Shell,
                WindowProcessStatus::Error,
            ));
            tab.workspace = WindowRegistry::from_persisted(persisted);
            tab
        })
        .collect();
    let mut runtime = sample_runtime(root, tabs, Some("a"));
    for id in ["a::agent", "b::agent"] {
        runtime
            .window_hook_states
            .insert(id.into(), WindowProcessStatus::Running);
    }
    runtime
}

fn aggregate_payload(
    events: &[OutboundEvent],
    key: &gwt_core::repo_hash::ProjectKey,
) -> gwt::protocol::ProjectAgentAggregate {
    events
        .iter()
        .find_map(|event| match (&event.target, &event.event) {
            (
                DispatchTarget::Project(target),
                BackendEvent::ProjectAgentAggregate { aggregate },
            ) if target == key => Some(aggregate.clone()),
            _ => None,
        })
        .expect("project aggregate")
}

#[test]
fn project_aggregate_counts_hidden_agents_and_scopes_changes() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = aggregate_test_runtime(temp.path());
    let a = runtime.project_context("a").unwrap();
    let b = runtime.project_context("b").unwrap();
    let initial = runtime.refresh_project_aggregates();
    assert_eq!(aggregate_payload(&initial, &a.project_key).running_count, 1);
    assert!(!aggregate_payload(&initial, &a.project_key).unread);
    runtime
        .window_pty_statuses
        .insert("b::agent".into(), WindowProcessStatus::Error);
    let changed = runtime.refresh_project_aggregates();
    assert_eq!(changed.len(), 1);
    let aggregate = aggregate_payload(&changed, &b.project_key);
    assert_eq!(
        (
            aggregate.running_count,
            aggregate.block_count,
            aggregate.error_count
        ),
        (0, 1, 1)
    );
    assert!(aggregate.unread);
    assert!(runtime.refresh_project_aggregates().is_empty());
    let tray = runtime.tray_snapshot();
    assert_eq!(
        tray.projects
            .iter()
            .find(|entry| entry.project_key == b.project_key)
            .unwrap()
            .error_count,
        1
    );
}

#[test]
fn project_aggregate_ack_requires_current_visible_focused_project_revision() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = aggregate_test_runtime(temp.path());
    let context = runtime.project_context("a").unwrap();
    let initial = aggregate_payload(&runtime.refresh_project_aggregates(), &context.project_key);
    runtime
        .window_pty_statuses
        .insert("a::agent".into(), WindowProcessStatus::Waiting);
    runtime
        .window_hook_states
        .insert("a::agent".into(), WindowProcessStatus::Waiting);
    let current = aggregate_payload(&runtime.refresh_project_aggregates(), &context.project_key);
    for (revision, visible, focused, scope) in [
        (
            initial.revision,
            true,
            true,
            ClientScope::Project(context.project_key.clone()),
        ),
        (
            current.revision + 1,
            true,
            true,
            ClientScope::Project(context.project_key.clone()),
        ),
        (
            current.revision,
            true,
            true,
            ClientScope::Project(runtime.project_context("b").unwrap().project_key),
        ),
        (
            current.revision,
            false,
            true,
            ClientScope::Project(context.project_key.clone()),
        ),
        (
            current.revision,
            true,
            false,
            ClientScope::Project(context.project_key.clone()),
        ),
        (current.revision, true, true, ClientScope::Hub),
    ] {
        let events = runtime.handle_frontend_event_in_scope(
            "client".into(),
            FrontendEvent::ProjectAggregateAck {
                revision,
                visible,
                focused,
            },
            &scope,
        );
        assert!(events.iter().all(|event| !matches!(&event.event, BackendEvent::ProjectAgentAggregate { aggregate } if !aggregate.unread)));
    }
    let events = runtime.handle_frontend_event_in_scope(
        "client".into(),
        FrontendEvent::ProjectAggregateAck {
            revision: current.revision,
            visible: true,
            focused: true,
        },
        &ClientScope::Project(context.project_key.clone()),
    );
    let cleared = aggregate_payload(&events, &context.project_key);
    assert!(!cleared.unread);
    assert!(cleared.revision > current.revision);
}

#[test]
fn project_aggregate_reopen_discards_unread_and_fences_old_ack() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = aggregate_test_runtime(temp.path());
    let context = runtime.project_context("a").unwrap();
    runtime.refresh_project_aggregates();
    runtime
        .window_pty_statuses
        .insert("a::agent".into(), WindowProcessStatus::Stopped);
    let old = aggregate_payload(&runtime.refresh_project_aggregates(), &context.project_key);
    runtime
        .project_tab_incarnations
        .get_mut("a")
        .unwrap()
        .generation += 1;
    runtime.refresh_project_state("a");
    let reopened = aggregate_payload(&runtime.refresh_project_aggregates(), &context.project_key);
    assert!(
        !reopened.unread,
        "restored stopped panes are a baseline, not new attention"
    );
    assert!(reopened.revision > old.revision);
    runtime
        .window_pty_statuses
        .insert("a::agent".into(), WindowProcessStatus::Error);
    let current = aggregate_payload(&runtime.refresh_project_aggregates(), &context.project_key);
    assert!(current.unread);
    let events = runtime.handle_frontend_event_in_scope(
        "client".into(),
        FrontendEvent::ProjectAggregateAck {
            revision: old.revision,
            visible: true,
            focused: true,
        },
        &ClientScope::Project(context.project_key),
    );
    assert!(events.is_empty());
}

#[test]
fn project_notification_routes_one_client_and_updates_unread() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = aggregate_test_runtime(temp.path());
    let context = runtime.project_context("a").unwrap();
    let other = runtime.project_context("b").unwrap();
    let clients = crate::ClientHub::default();
    for (id, key) in [
        ("first", &context.project_key),
        ("latest", &context.project_key),
        ("foreign", &other.project_key),
    ] {
        clients.register_scoped(id.into(), ClientScope::Project(key.clone()));
        let attention = serde_json::from_value(serde_json::json!({
            "kind": "project_client_attention", "visible": false, "focused": false,
            "notification_permission": "granted"
        }))
        .unwrap();
        runtime.handle_frontend_event_in_scope(
            id.into(),
            attention,
            &ClientScope::Project(key.clone()),
        );
    }
    runtime.refresh_project_aggregates();
    let started = Instant::now() - Duration::from_secs(300);
    runtime.observe_notification_transition_at("a::agent", WindowProcessStatus::Running, started);
    runtime.apply_owner_status_projection("a::agent", WindowProcessStatus::Stopped, None);
    let (events, native) = runtime.collect_project_notifications(&clients);
    assert!(native.is_empty());
    let recipients: Vec<_> = events
        .iter()
        .filter_map(|event| match (&event.target, &event.event) {
            (DispatchTarget::Client(id), BackendEvent::ProjectNotification { .. }) => {
                Some(id.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(recipients, ["latest"]);
    assert!(aggregate_payload(&events, &context.project_key).unread);
    assert!(runtime.collect_project_notifications(&clients).0.is_empty());
}

#[test]
fn project_notification_monitor_replay_is_suppressed_and_close_revokes_native() {
    use std::sync::atomic::Ordering;
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = aggregate_test_runtime(temp.path());
    let context = runtime.project_context("a").unwrap();
    let clients = crate::ClientHub::default();
    // Establish the baseline that frontend_ready sends before attention reports.
    runtime.refresh_project_aggregates();
    let attention = serde_json::from_value(serde_json::json!({
        "kind": "project_client_attention", "visible": false, "focused": false,
        "notification_permission": "granted"
    }))
    .unwrap();
    runtime.handle_frontend_event_in_scope(
        "gone".into(),
        attention,
        &ClientScope::Project(context.project_key.clone()),
    );
    let event = OutboundEvent::project(
        context.project_key.clone(),
        BackendEvent::IssueMonitorToast {
            notification_transition: Some(
                gwt::issue_monitor::MonitorNotificationTransition::NeedsHuman,
            ),
            notification_transition_id: Some("source-transition".into()),
            level: "error".into(),
            message: "private/path/error".into(),
            issue_number: Some(3287),
        },
    );
    runtime.observe_notification_toasts(std::slice::from_ref(&event));
    let (events, native) = runtime.collect_project_notifications(&clients);
    assert!(aggregate_payload(&events, &context.project_key).unread);
    assert_eq!(native.len(), 1);
    assert!(!native[0].intent.body.contains("private/path"));
    runtime.observe_notification_toasts(&[event]);
    assert!(runtime.collect_project_notifications(&clients).1.is_empty());
    runtime.project_tab_incarnations.remove("a");
    runtime.collect_project_notifications(&clients);
    assert!(!native[0].validity.load(Ordering::Acquire));
}

#[test]
fn project_notification_silent_process_install_does_not_infer_agent_running() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = aggregate_test_runtime(temp.path());
    let id = "a::agent";
    let context = runtime.project_context("a").unwrap();
    let owner = owner_fixture_block_on(production_owner_fixture(&context.project_root));
    let receipt = owner_fixture_block_on(owner.command(owner_fixture_agent_command(
        &owner,
        &context.project_root,
        "silent-agent",
        "silent-agent-launch",
    )))
    .expect("actual silent Agent launch");
    let snapshot: super::RemoteRuntimeSnapshot = serde_json::from_value(
        owner_fixture_block_on(owner.command(
            gwt::project_runtime::events::OwnerCommand::Snapshot {
                id: owner.pane_id("silent-agent"),
                incarnation: receipt["incarnation"].as_u64().unwrap(),
            },
        ))
        .expect("actual silent owner snapshot"),
    )
    .unwrap();
    assert_eq!(
        snapshot.status,
        WindowProcessStatus::Starting,
        "physical liveness without a provider hook must remain Starting at the owner"
    );
    let session = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir()
            .join(format!("{}.toml", snapshot.session_id.as_deref().unwrap())),
    )
    .unwrap();
    runtime.install_remote_owner_projection(&context, id, owner.client(), snapshot, Some(session));
    assert_eq!(
        runtime.window_status(id),
        Some(WindowProcessStatus::Starting)
    );
    runtime.observe_notification_transition_at(
        id,
        WindowProcessStatus::Stopped,
        Instant::now() + Duration::from_secs(300),
    );
    let intents = runtime.notification_coordinator.drain_intents();
    owner_fixture_block_on(owner.shutdown());
    assert!(intents.is_empty(), "PTY liveness is not Agent Running");
}

#[test]
fn owner_output_interleaving_preserves_contiguous_pane_deltas() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut runtime = aggregate_test_runtime(temp.path());
    let context = runtime.project_context("a").unwrap();
    let identity = gwt::daemon_subscriber::ProjectOwnerIdentity {
        pid: 42,
        host_started_at: 100,
        host_instance_id: "interleaved-owner".into(),
    };
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = Some(identity.clone());
    let mut endpoint = gwt_core::daemon::DaemonEndpoint::new(
        gwt_core::daemon::RuntimeScope::new(
            context.project_key.as_str(),
            "fixture",
            context.project_root.clone(),
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .unwrap(),
        identity.pid,
        "fixture".into(),
        "private-fixture".into(),
        "fixture".into(),
    );
    endpoint.project_transport_url = Some("http://127.0.0.1:1".into());
    for (alias, owner_id) in [("a::agent", "owner-a"), ("a::shell", "owner-b")] {
        runtime.runtimes.insert(
            alias.into(),
            WindowRuntime::remote(
                owner_id.into(),
                7,
                100,
                gwt::project_runtime::client::ProjectRuntimeClient::new(&endpoint).unwrap(),
                identity.clone(),
            ),
        );
    }
    let mut events = Vec::new();
    // Global stream positions interleave; each pane's own predecessor remains exact.
    for (id, seq, previous_seq) in [("owner-a", 1, 0), ("owner-b", 2, 0), ("owner-a", 3, 1)] {
        let output = serde_json::from_value(serde_json::json!({
            "type": "output", "id": id, "incarnation": 7,
            "seq": seq, "previous_seq": previous_seq, "screen_seq": seq,
            "data": [b'x'], "screen": "x", "screen_ansi": [b'x'],
        }))
        .unwrap();
        events.extend(runtime.handle_project_owner_event(&context, 1, &identity, output));
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.event, BackendEvent::TerminalOutput { .. }))
            .count(),
        3,
        "another pane's output must not replace a continuous delta with a snapshot"
    );
    assert!(!events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::TerminalSnapshot { .. })));
}

#[test]
fn project_owner_health_is_project_scoped_and_rejects_stale_context() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut runtime = aggregate_test_runtime(temp.path());
    let a = runtime.project_context("a").unwrap();
    let b = runtime.project_context("b").unwrap();
    let health = gwt::protocol::ProjectOwnerHealthView {
        project_key: a.project_key.to_string(),
        state: gwt::protocol::ProjectOwnerHealthState::Degraded,
        detail: Some("Project connection lost".into()),
    };
    let events = runtime.handle_project_owner_health(&a, 1, None, health.clone());
    assert!(events.iter().any(
        |event| matches!(&event.target, DispatchTarget::Project(key) if key == &a.project_key)
    ));
    assert!(!events.iter().any(
        |event| matches!(&event.target, DispatchTarget::Project(key) if key == &b.project_key)
    ));
    let catalog = runtime.hub_state_view();
    let a_health = &catalog
        .projects
        .iter()
        .find(|p| p.project_key == a.project_key.as_str())
        .unwrap()
        .owner_health;
    assert_eq!(a_health.as_ref(), Some(&health));
    assert!(catalog
        .projects
        .iter()
        .find(|p| p.project_key == b.project_key.as_str())
        .unwrap()
        .owner_health
        .is_none());
    let now = chrono::Utc::now().to_rfc3339();
    assert!(runtime
        .issue_monitor_window_snapshot_for_tab("b", &now)
        .is_some());
    assert!(
        runtime
            .issue_monitor_window_snapshot_for_tab("a", &now)
            .is_none(),
        "a native-managed project's GUI must not publish a second physical canvas, even when degraded"
    );
    let mut stale_health = health.clone();
    stale_health.detail = Some("Stale connection failure".into());
    assert!(runtime
        .handle_project_owner_health(&a, 0, None, stale_health)
        .is_empty());
    assert_eq!(
        runtime
            .project_state(&a)
            .unwrap()
            .project_owner_health
            .as_ref(),
        Some(&health)
    );
    // Owner events are a remote read model; no physical pane is created in the GUI.
    let identity = gwt::daemon_subscriber::ProjectOwnerIdentity {
        pid: 42,
        host_started_at: 100,
        host_instance_id: "owner-a".into(),
    };
    // Seed an already-authenticated subscriber projection; no background network worker
    // or physical pane belongs to this read-model regression.
    let accepted = runtime.project_state_mut(&a).unwrap();
    accepted.project_owner_connection_id = 2;
    accepted.project_owner_identity = Some(identity.clone());
    let mut endpoint = gwt_core::daemon::DaemonEndpoint::new(
        gwt_core::daemon::RuntimeScope::new(
            a.project_key.as_str(),
            "fixture",
            a.project_root.clone(),
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .unwrap(),
        identity.pid,
        "fixture".into(),
        "private-fixture".into(),
        "fixture".into(),
    );
    endpoint.project_transport_url = Some("http://127.0.0.1:1".into());
    runtime.runtimes.insert(
        "a::agent".into(),
        WindowRuntime::remote(
            "owner-pane-a".into(),
            7,
            100,
            gwt::project_runtime::client::ProjectRuntimeClient::new(&endpoint).unwrap(),
            identity.clone(),
        ),
    );
    // The next event is a contiguous delta after this accepted read-model position.
    runtime.runtimes.get_mut("a::agent").unwrap().output_seq = 4;
    let output = |incarnation| gwt::project_runtime::events::OwnerEvent::Output {
        id: "owner-pane-a".into(),
        incarnation,
        data: b"owner bytes".to_vec(),
        seq: 5,
        previous_seq: Some(4),
        screen: "owner screen".into(),
        screen_ansi: b"owner ANSI".to_vec(),
        screen_seq: 5,
    };
    assert!(
        runtime
            .handle_project_owner_event(&a, 1, &identity, output(7))
            .is_empty(),
        "old connection"
    );
    let mut replaced_identity = identity.clone();
    replaced_identity.host_instance_id = "replacement".into();
    assert!(
        runtime
            .handle_project_owner_event(&a, 2, &replaced_identity, output(7))
            .is_empty(),
        "old owner"
    );
    assert!(
        runtime
            .handle_project_owner_event(&b, 2, &identity, output(7))
            .is_empty(),
        "other project"
    );
    assert!(
        runtime
            .handle_project_owner_event(&a, 2, &identity, output(6))
            .is_empty(),
        "old pane incarnation"
    );
    let events = runtime.handle_project_owner_event(&a, 2, &identity, output(7));
    assert!(events.iter().any(
        |event| matches!(&event.event, BackendEvent::TerminalOutput { .. })
            && event.terminal_stream_seq == Some(5)
    ));
    assert_eq!(runtime.runtimes["a::agent"].screen, "owner screen");
    assert_eq!(runtime.runtimes["a::agent"].screen_ansi, b"owner ANSI");
    assert!(
        runtime
            .handle_project_owner_event(&a, 2, &identity, output(7))
            .is_empty(),
        "duplicate sequence"
    );
    let deadline = runtime
        .next_runtime_output_deadline()
        .expect("existing preview batch");
    assert!(runtime.flush_runtime_output_batches(deadline).iter().any(|event| matches!(&event.event, BackendEvent::TerminalPreview { text, .. } if text == "owner screen")));
    let status = gwt::project_runtime::events::OwnerEvent::Status {
        id: "owner-pane-a".into(),
        incarnation: 7,
        raw_status: WindowProcessStatus::Stopped,
        status: WindowProcessStatus::Stopped,
        detail: Some("owner exit".into()),
        exit_confirmed: true,
        exit_code: Some(0),
    };
    assert!(!runtime
        .handle_project_owner_event(&a, 2, &identity, status)
        .is_empty());
    assert!(runtime.runtimes["a::agent"].exit_confirmed);
    assert_eq!(
        runtime.runtimes["a::agent"].status,
        WindowProcessStatus::Stopped
    );
    assert_eq!(runtime.runtimes["a::agent"].exit_code, Some(0));
    let snapshot = super::RemoteRuntimeSnapshot {
        has_unsent_user_input: false,
        input_observation: None,
        id: "owner-pane-a".into(),
        incarnation: 7,
        screen: "new snapshot".into(),
        screen_ansi: b"new ANSI".to_vec(),
        screen_seq: 6,
        child_pid: Some(43),
        started_at_ms: 100,
        raw_status: WindowProcessStatus::Running,
        status: WindowProcessStatus::Running,
        detail: None,
        exit_confirmed: false,
        exit_code: None,
        session_id: None,
        pm_registration: None,
        agent_id: None,
        agent_name: None,
        display_name: None,
        purpose_title: None,
        branch: None,
        worktree_path: None,
        closing: false,
        runtime_target: None,
    };
    let repaired = runtime.handle_project_owner_snapshot(super::RemoteOwnerSnapshotCompletion {
        context: a.clone(),
        connection_id: 2,
        identity: identity.clone(),
        id: "a::agent".into(),
        incarnation: 7,
        status_revision: 0,
        client_id: None,
        result: Ok(snapshot),
    });
    assert!(repaired.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalSnapshot { .. }
    ) && event.terminal_stream_seq == Some(6)));
    assert_eq!(runtime.runtimes["a::agent"].screen, "new snapshot");
    assert_eq!(
        runtime.runtimes["a::agent"].status,
        WindowProcessStatus::Stopped,
        "an older snapshot request cannot regress newer status evidence"
    );
    assert!(runtime.runtimes["a::agent"].exit_confirmed);

    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
    let gap = runtime.handle_project_owner_event(
        &a,
        2,
        &identity,
        gwt::project_runtime::events::OwnerEvent::Output {
            id: "owner-pane-a".into(),
            incarnation: 7,
            seq: 8,
            previous_seq: Some(7),
            data: b"delta after a dropped chunk".to_vec(),
            screen: "full repaired screen".into(),
            screen_ansi: b"full repaired ANSI".to_vec(),
            screen_seq: 8,
        },
    );
    assert!(
        !gap.iter()
            .any(|event| matches!(event.event, BackendEvent::TerminalSnapshot { .. })),
        "a visible-only frame cannot replace the terminal's retained history"
    );
    let BlockingTaskSpawner::Queued(tasks) = &runtime.blocking_tasks else {
        unreachable!()
    };
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "request the owner's semantic snapshot"
    );
    assert!(
        !gap.iter()
            .any(|event| matches!(event.event, BackendEvent::TerminalOutput { .. })),
        "a stream gap cannot forward the surviving delta as a contiguous raw chunk"
    );

    // The reader may parse its final bytes before retirement rejects its
    // queued Output event. The successful close ACK must retain that screen.
    let closed = |incarnation, error: Option<&str>, screen_seq| {
        serde_json::from_value(serde_json::json!({
            "type": "closed", "id": "owner-pane-a", "incarnation": incarnation,
            "error": error,
            "final_screen": {
                "screen": "final diagnostic screen", "screen_ansi": b"final ANSI",
                "screen_seq": screen_seq,
            },
        }))
        .unwrap()
    };
    assert!(runtime
        .handle_project_owner_event(&a, 1, &identity, closed(7, None, 9))
        .is_empty());
    assert!(runtime
        .handle_project_owner_event(&a, 2, &replaced_identity, closed(7, None, 9))
        .is_empty());
    assert!(runtime
        .handle_project_owner_event(&b, 2, &identity, closed(7, None, 9))
        .is_empty());
    assert!(runtime
        .handle_project_owner_event(&a, 2, &identity, closed(6, None, 9))
        .is_empty());
    runtime.handle_project_owner_event(&a, 2, &identity, closed(7, Some("close failed"), 9));
    assert_eq!(runtime.runtimes["a::agent"].screen, "full repaired screen");
    let finalized = runtime.handle_project_owner_event(&a, 2, &identity, closed(7, None, 9));
    assert_eq!(
        runtime.runtimes["a::agent"].screen,
        "final diagnostic screen"
    );
    assert_eq!(runtime.runtimes["a::agent"].screen_seq, 9);
    assert_eq!(runtime.runtimes["a::agent"].output_seq, 9);
    assert!(finalized.iter().any(|event| matches!(&event.event,
        BackendEvent::TerminalSnapshot { data_base64, .. }
        if base64::engine::general_purpose::STANDARD.decode(data_base64).unwrap() == b"final ANSI")
        && event.terminal_stream_seq == Some(9)));
    let restored = runtime.request_owner_snapshot("a::agent", Some("reconnected-client".into()));
    assert!(
        restored.iter().any(|event| matches!(&event.event,
        BackendEvent::TerminalSnapshot { data_base64, .. }
        if base64::engine::general_purpose::STANDARD.decode(data_base64).unwrap() == b"final ANSI")
            && event.terminal_stream_seq == Some(9)
            && event.target == DispatchTarget::Client("reconnected-client".into())),
        "closed diagnostic panes restore the final semantic snapshot without a live owner"
    );
    assert!(runtime
        .handle_project_owner_event(&a, 2, &identity, output(7))
        .is_empty());
    runtime.handle_project_owner_event(&a, 2, &identity, closed(7, None, 7));
    assert_eq!(runtime.runtimes["a::agent"].screen_seq, 9);

    let legacy = serde_json::from_value(serde_json::json!({
        "type": "output", "id": "owner-pane-a", "incarnation": 7,
        "seq": 10, "screen_seq": 10, "data": b"legacy delta",
        "screen": "legacy screen", "screen_ansi": b"legacy ANSI",
    }))
    .unwrap();
    let legacy = runtime.handle_project_owner_event(&a, 2, &identity, legacy);
    assert!(!legacy
        .iter()
        .any(|event| matches!(event.event, BackendEvent::TerminalSnapshot { .. })));
    assert!(
        !legacy
            .iter()
            .any(|event| matches!(event.event, BackendEvent::TerminalOutput { .. })),
        "an older sender without a predecessor witness requires snapshot repair"
    );

    assert!(
        runtime
            .handle_project_owner_event(
                &a,
                2,
                &identity,
                gwt::project_runtime::events::OwnerEvent::WindowRemoved {
                    id: "owner-pane-a".into(),
                    incarnation: 6,
                    session_id: None,
                }
            )
            .is_empty(),
        "stale removal cannot close the current alias"
    );
    assert!(runtime.runtimes.contains_key("a::agent"));
    let removed = runtime.handle_project_owner_event(
        &a,
        2,
        &identity,
        gwt::project_runtime::events::OwnerEvent::WindowRemoved {
            id: "owner-pane-a".into(),
            incarnation: 7,
            session_id: None,
        },
    );
    assert!(!runtime.runtimes.contains_key("a::agent"));
    assert!(!runtime.window_lookup.contains_key("a::agent"));
    assert!(removed
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })));
    assert!(removed
        .iter()
        .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjectionPatch { .. })));

    runtime
        .project_tab_incarnations
        .get_mut("a")
        .unwrap()
        .generation += 1;
    runtime.refresh_project_state("a");
    assert!(runtime
        .handle_project_owner_health(&a, 2, None, health)
        .is_empty());
    assert!(
        runtime
            .handle_project_owner_event(&a, 2, &identity, output(7))
            .is_empty(),
        "closed ProjectContext generation"
    );
}
