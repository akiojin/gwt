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

#[test]
fn hub_catalog_can_close_a_project_without_selecting_it() {
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
    let request = FrontendEvent::CloseProjectTab { tab_id: "b".into() };
    assert!(crate::hub_frontend_event_allowed(&request));
    let events =
        runtime.handle_frontend_event_in_scope("hub".into(), request, &super::ClientScope::Hub);
    assert!(runtime.project_context("b").is_none());
    assert!(runtime.project_context("a").is_some());
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::HubState { .. })));
}
