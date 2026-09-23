#[test]
fn project_window_list_and_stop_all_do_not_cross_project_owners() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tabs = vec![
        sample_project_tab(
            "tab-a",
            "A",
            temp.path().join("a"),
            ProjectKind::NonRepo,
            &[WindowPreset::Shell],
        ),
        sample_project_tab(
            "tab-b",
            "B",
            temp.path().join("b"),
            ProjectKind::NonRepo,
            &[WindowPreset::Shell],
        ),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-a"));
    let a = runtime.project_context("tab-a").unwrap();
    let a_window = combined_window_id(
        "tab-a",
        &runtime.tabs[0].workspace.persisted().windows[0].id,
    );
    let b_window = combined_window_id(
        "tab-b",
        &runtime.tabs[1].workspace.persisted().windows[0].id,
    );
    insert_test_pane_runtime(&mut runtime, &a_window);
    insert_test_pane_runtime(&mut runtime, &b_window);

    let listed = runtime.handle_frontend_event_for_project(
        &a,
        "client-a".to_string(),
        FrontendEvent::ListWindows,
    );
    let windows = listed
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::WindowList { windows } => Some(windows),
            _ => None,
        })
        .expect("window list");
    assert_eq!(windows.len(), 1, "A must not receive B's window metadata");
    assert_eq!(windows[0].id, a_window);

    runtime.handle_frontend_event_for_project(
        &a,
        "client-a".to_string(),
        FrontendEvent::StopAllWindows {},
    );
    assert!(!runtime.runtimes.contains_key(&a_window));
    assert!(
        runtime.runtimes.contains_key(&b_window),
        "A's stop-all must leave B running"
    );
}

#[test]
fn project_root_requests_reject_other_owner_before_side_effects() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let a_root = temp.path().join("a");
    let b_root = temp.path().join("b");
    fs::create_dir_all(&a_root).unwrap();
    fs::create_dir_all(&b_root).unwrap();
    let mut runtime = sample_runtime(temp.path(), vec![
        sample_project_tab("tab-a", "A", a_root.clone(), ProjectKind::NonRepo, &[]),
        sample_project_tab("tab-b", "B", b_root.clone(), ProjectKind::NonRepo, &[]),
    ], Some("tab-a"));
    let context = runtime.project_context("tab-a").unwrap();
    let request = |root: &Path| FrontendEvent::UpdateProjectBoardConfig {
        project_root: root.display().to_string(),
        provider: Some("local".to_string()), channel: None, tenant: None,
    };
    let rejected = runtime.handle_frontend_event_for_project(&context, "client-a".to_string(), request(&b_root));
    assert!(rejected.is_empty(), "A must not update B's board configuration");
    assert!(!b_root.join(".gwt/work/board.toml").exists());
    let accepted = runtime.handle_frontend_event_for_project(&context, "client-a".to_string(), request(&a_root));
    assert!(!accepted.is_empty(), "the owner must retain access to its own settings");
    for event in [
        FrontendEvent::GetProjectBoardConfig { project_root: b_root.display().to_string() },
        FrontendEvent::RefreshIndexStatus { project_root: b_root.display().to_string() },
        FrontendEvent::RebuildIndexCell { project_root: b_root.display().to_string(), scope: gwt::IndexRebuildScope::Files, worktree_hash: None },
    ] {
        assert!(!runtime.project_owns_frontend_request(&context, &event));
    }
}
