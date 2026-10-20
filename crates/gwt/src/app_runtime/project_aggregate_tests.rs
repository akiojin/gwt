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
            tab.workspace = WindowCanvasState::from_persisted(persisted);
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
    runtime.stop_window_events("a::agent");
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
    runtime.install_process_window(
        id,
        super::next_window_runtime_incarnation(),
        long_running_test_pane(id),
        None,
        None,
        None,
    );
    assert_eq!(runtime.window_status(id), Some(WindowProcessStatus::Starting));
    runtime.observe_notification_transition_at(
        id,
        WindowProcessStatus::Stopped,
        Instant::now() + Duration::from_secs(300),
    );
    let intents = runtime.notification_coordinator.drain_intents();
    runtime.stop_window_runtime(id);
    assert!(intents.is_empty(), "PTY liveness is not Agent Running");
}
