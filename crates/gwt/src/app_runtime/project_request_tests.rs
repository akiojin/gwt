#[test]
fn project_request_cannot_close_another_projects_window() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(
        temp.path(),
        vec![
            sample_project_tab_with_window_at(
                "a",
                "window-a",
                temp.path().join("a"),
                WindowPreset::Shell,
                WindowProcessStatus::Stopped,
            ),
            sample_project_tab_with_window_at(
                "b",
                "window-b",
                temp.path().join("b"),
                WindowPreset::Shell,
                WindowProcessStatus::Stopped,
            ),
        ],
        Some("a"),
    );
    let context = runtime.project_context("a").unwrap();
    let window_b = runtime
        .window_lookup
        .keys()
        .find(|id| runtime.project_key_for_window(id) != Some(&context.project_key))
        .unwrap()
        .clone();
    let events = runtime.handle_frontend_event_for_project(
        &context,
        "client-a".into(),
        FrontendEvent::CloseWindow {
            id: window_b.clone(),
            request_id: None,
        },
    );
    assert!(
        events.is_empty(),
        "cross-project requests must be rejected before mutation"
    );
    assert!(runtime.window_lookup.contains_key(&window_b));
    let window_a = runtime
        .window_lookup
        .keys()
        .find(|id| runtime.project_key_for_window(id) == Some(&context.project_key))
        .unwrap()
        .clone();
    let own_events = runtime.handle_frontend_event_for_project(
        &context,
        "client-a".into(),
        FrontendEvent::CloseWindow {
            id: window_a.clone(),
            request_id: None,
        },
    );
    assert!(!own_events.is_empty());
    assert!(!runtime.window_lookup.contains_key(&window_a));
    assert!(runtime.window_lookup.contains_key(&window_b));
}

#[test]
fn project_generation_change_discards_only_its_cached_work_summaries() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let root_a = temp.path().join("a");
    let root_b = temp.path().join("b");
    let mut runtime = sample_runtime(
        temp.path(),
        vec![
            sample_project_tab("a", "A", root_a.clone(), ProjectKind::Git, &[]),
            sample_project_tab("b", "B", root_b.clone(), ProjectKind::Git, &[]),
        ],
        Some("a"),
    );
    runtime.work_tip_subjects.insert(
        root_a.clone(),
        HashMap::from([("main".into(), "old A".into())]),
    );
    runtime
        .work_tip_subjects
        .insert(root_b.clone(), HashMap::from([("main".into(), "B".into())]));
    runtime.refresh_project_tab_incarnation("a");
    assert!(!runtime.work_tip_subjects.contains_key(&root_a));
    assert_eq!(runtime.work_tip_subjects[&root_b]["main"], "B");
}

fn close_request(value: serde_json::Value) -> FrontendEvent {
    serde_json::from_value(value).expect("Close Project protocol")
}

#[test]
fn close_project_authorization_is_client_scoped_single_use_and_generation_bound() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(
        temp.path(),
        vec![
            sample_project_tab("a", "A", temp.path().join("a"), ProjectKind::Git, &[]),
            sample_project_tab("b", "B", temp.path().join("b"), ProjectKind::Git, &[]),
        ],
        Some("a"),
    );
    let a = runtime.project_context("a").unwrap();
    let b = runtime.project_context("b").unwrap();
    let preview = serde_json::json!({"kind":"preview_close_project", "project_key":a.project_key.to_string()});
    let events = runtime.handle_frontend_event_in_scope(
        "hub".into(),
        close_request(preview.clone()),
        &super::ClientScope::Hub,
    );
    let body = serde_json::to_value(&events[0].event).unwrap();
    assert_eq!(body["kind"], "close_project_preview");
    assert!(matches!(&events[0].target, DispatchTarget::Client(id) if id == "hub"));
    assert_eq!(body["running_agents"], serde_json::json!([]));
    let token = body["token"].clone();
    let confirm = serde_json::json!({"kind":"confirm_close_project", "token":token});
    let mut forged = token.clone();
    forged["nonce"] = "forged".into();
    for (client, scope, request) in [
        ("other", super::ClientScope::Hub, confirm.clone()),
        (
            "hub",
            super::ClientScope::Project(b.project_key.clone()),
            confirm.clone(),
        ),
        (
            "hub",
            super::ClientScope::Project(b.project_key.clone()),
            preview.clone(),
        ),
        (
            "hub",
            super::ClientScope::Hub,
            serde_json::json!({"kind":"confirm_close_project", "token": forged}),
        ),
    ] {
        let errors =
            runtime.handle_frontend_event_in_scope(client.into(), close_request(request), &scope);
        assert_eq!(
            serde_json::to_value(&errors[0].event).unwrap()["kind"],
            "close_project_error"
        );
        assert!(matches!(&errors[0].target, DispatchTarget::Client(id) if id == client));
        assert!(runtime.project_context("a").is_some());
    }
    let cancel = serde_json::json!({"kind":"cancel_close_project", "token":token});
    assert!(runtime
        .handle_frontend_event_in_scope(
            "hub".into(),
            close_request(cancel),
            &super::ClientScope::Hub
        )
        .is_empty());
    let errors = runtime.handle_frontend_event_in_scope(
        "hub".into(),
        close_request(confirm),
        &super::ClientScope::Hub,
    );
    assert_eq!(
        serde_json::to_value(&errors[0].event).unwrap()["kind"],
        "close_project_error"
    );
    let events = runtime.handle_frontend_event_in_scope(
        "hub".into(),
        close_request(preview.clone()),
        &super::ClientScope::Hub,
    );
    let stale_token = serde_json::to_value(&events[0].event).unwrap()["token"].clone();
    runtime.close_project_tab_events("a");
    runtime.tabs.push(sample_project_tab(
        "a",
        "A",
        temp.path().join("a"),
        ProjectKind::Git,
        &[],
    ));
    runtime.refresh_project_tab_incarnation("a");
    let errors = runtime.handle_frontend_event_in_scope(
        "hub".into(),
        close_request(serde_json::json!({"kind":"confirm_close_project", "token":stale_token})),
        &super::ClientScope::Hub,
    );
    assert_eq!(
        serde_json::to_value(&errors[0].event).unwrap()["kind"],
        "close_project_error"
    );
    let events = runtime.handle_frontend_event_in_scope(
        "hub".into(),
        close_request(preview),
        &super::ClientScope::Hub,
    );
    let token = serde_json::to_value(&events[0].event).unwrap()["token"].clone();
    let confirm = serde_json::json!({"kind":"confirm_close_project", "token":token});
    let events = runtime.handle_frontend_event_in_scope(
        "hub".into(),
        close_request(confirm.clone()),
        &super::ClientScope::Hub,
    );
    assert!(runtime.project_context("a").is_none());
    assert_eq!(runtime.project_context("b"), Some(b));
    assert!(events.iter().any(
        |event| matches!(&event.target, DispatchTarget::Project(key) if key == &a.project_key)
            && serde_json::to_value(&event.event).unwrap()["kind"] == "project_closed"
    ));
    assert!(events
        .iter()
        .any(|event| matches!(event.target, DispatchTarget::Hub)
            && matches!(event.event, BackendEvent::HubState { .. })));
    let errors = runtime.handle_frontend_event_in_scope(
        "hub".into(),
        close_request(confirm),
        &super::ClientScope::Hub,
    );
    assert_eq!(
        serde_json::to_value(&errors[0].event).unwrap()["kind"],
        "close_project_error"
    );
}

#[test]
fn close_project_preview_reports_running_agents_and_project_confirmation_is_allowed() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab_with_window_at(
            "a",
            "agent",
            temp.path().join("a"),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        )],
        Some("a"),
    );
    let context = runtime.project_context("a").unwrap();
    let scope = super::ClientScope::Project(context.project_key.clone());
    let preview = close_request(
        serde_json::json!({"kind":"preview_close_project", "project_key":context.project_key.to_string()}),
    );
    assert!(crate::hub_frontend_event_allowed(&preview));
    let events = runtime.handle_frontend_event_in_scope("a-client".into(), preview, &scope);
    let body = serde_json::to_value(&events[0].event).unwrap();
    assert_eq!(body["running_agents"].as_array().unwrap().len(), 1);
    let confirm =
        close_request(serde_json::json!({"kind":"confirm_close_project", "token":body["token"]}));
    assert!(crate::hub_frontend_event_allowed(&confirm));
    runtime.handle_frontend_event_in_scope("a-client".into(), confirm, &scope);
    assert!(runtime.project_context("a").is_none());
}

#[test]
fn close_project_reopen_registers_restored_windows_without_changing_other_project_generations() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let root_a = temp.path().join("a");
    let root_b = temp.path().join("b");
    fs::create_dir_all(&root_a).unwrap();
    fs::create_dir_all(&root_b).unwrap();
    let tabs = vec![
        sample_project_tab_with_window_at(
            "a",
            "window-a",
            root_a.clone(),
            WindowPreset::FileTree,
            WindowProcessStatus::Stopped,
        ),
        sample_project_tab_with_window_at(
            "b",
            "window-b",
            root_b,
            WindowPreset::FileTree,
            WindowProcessStatus::Stopped,
        ),
    ];
    let (mut runtime, recorded_events) = sample_runtime_with_events(temp.path(), tabs, Some("a"));
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;
    let old_context = runtime.project_context("a").unwrap();
    let other_id = combined_window_id("b", "window-b");
    let other_generation = runtime.window_lifecycle_generations.lock().unwrap()[&other_id];
    runtime.persist().unwrap();
    assert!(runtime.persist_dispatcher.wait_idle(Duration::from_secs(5)));
    runtime.close_project_tab_events("a");
    assert!(!runtime
        .window_lookup
        .contains_key(&combined_window_id("a", "window-a")));

    runtime.open_project_path_events(root_a);
    drain_queued_blocking_tasks(&queued_tasks);
    let prepared = take_project_navigation_completion(&recorded_events);
    runtime.handle_project_navigation_prepared(prepared);
    let reopened = runtime
        .project_contexts()
        .into_iter()
        .find(|context| context.project_key == old_context.project_key)
        .unwrap();
    assert_ne!(reopened.generation, old_context.generation);
    let reopened_window = combined_window_id(&reopened.tab_id, "window-a");
    assert!(
        runtime.window_lookup.contains_key(&reopened_window),
        "restored windows must remain addressable after explicit close/reopen"
    );
    assert_eq!(
        runtime.project_key_for_window(&reopened_window),
        Some(&reopened.project_key)
    );
    assert_eq!(
        runtime.window_lifecycle_generations.lock().unwrap()[&other_id],
        other_generation
    );
}
