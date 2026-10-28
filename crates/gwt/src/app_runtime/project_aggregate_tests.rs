use super::ClientScope;

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

    let gap = runtime.handle_project_owner_event(
        &a,
        2,
        &identity,
        gwt::project_runtime::events::OwnerEvent::Output {
            id: "owner-pane-a".into(),
            incarnation: 7,
            seq: 8,
            data: b"delta after a dropped chunk".to_vec(),
            screen: "full repaired screen".into(),
            screen_ansi: b"full repaired ANSI".to_vec(),
            screen_seq: 8,
        },
    );
    assert!(gap.iter().any(|event| matches!(&event.event,
        BackendEvent::TerminalSnapshot { data_base64, .. }
        if base64::engine::general_purpose::STANDARD.decode(data_base64).unwrap() == b"full repaired ANSI")
        && event.terminal_stream_seq == Some(8)),
        "a conflated owner stream must repair from its authoritative ANSI screen");
    assert!(
        !gap.iter()
            .any(|event| matches!(event.event, BackendEvent::TerminalOutput { .. })),
        "a stream gap cannot forward the surviving delta as a contiguous raw chunk"
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
