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
