fn refresh_test_context(
    project_root: PathBuf,
    generation: u64,
) -> crate::app_runtime::ProjectContext {
    crate::app_runtime::ProjectContext {
        tab_id: "tab-a".to_string(),
        project_key: gwt_core::repo_hash::ProjectKey::parse("0123456789abcdef").unwrap(),
        project_root,
        generation,
    }
}

#[test]
fn board_refresh_cache_discards_views_and_late_finish_from_previous_generation() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let old = refresh_test_context(root, 1);
    let new = crate::app_runtime::ProjectContext {
        generation: 2,
        ..old.clone()
    };
    let (view, _) = gwt_core::coordination::refresh_scoped_board_view(
        &old.project_root,
        &gwt_core::coordination::BoardAudienceScope::All,
        None,
    )
    .unwrap();
    let mut queue = crate::BoardRefreshQueue::default();
    queue.begin(&old).unwrap();
    queue.finish(
        &old,
        std::collections::HashMap::from([("tab-a::board-1".to_string(), view)]),
    );
    assert!(
        queue.begin(&new).unwrap().is_empty(),
        "a reopened Board with the same window ID must rebuild its view"
    );
    assert!(!queue.finish(&new, Default::default()));
    queue.begin(&new).unwrap();
    let reopened = crate::app_runtime::ProjectContext {
        generation: 3,
        ..new.clone()
    };
    assert!(
        queue.begin(&reopened).is_some(),
        "a new incarnation must not wait for the old job"
    );
    assert!(!queue.finish(&new, Default::default()));
    assert!(
        queue.begin(&reopened).is_none(),
        "an old completion must not clear the new refresh in flight"
    );
    assert!(queue.finish(&reopened, Default::default()));
}

#[test]
fn workspace_projection_reload_callback_rejects_closed_project_generation() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&root);
    projection.title = "captured worker snapshot".to_string();
    gwt_core::workspace_projection::save_workspace_projection(&root, &projection).unwrap();
    let (mut app, recorded) = sample_runtime_with_events(
        temp.path(),
        vec![sample_project_tab(
            "tab-a",
            "A",
            root,
            ProjectKind::Git,
            &[],
        )],
        Some("tab-a"),
    );
    let (spawner, tasks) = crate::app_runtime::BlockingTaskSpawner::queued();
    let old = app.project_context("tab-a").unwrap();
    super::spawn_workspace_projection_reload(&spawner, app.proxy.clone(), old);
    app.project_tab_incarnations
        .get_mut("tab-a")
        .unwrap()
        .generation += 1;
    app.refresh_project_state("tab-a");
    tasks.lock().unwrap().pop().unwrap()();
    let completion = recorded.lock().unwrap().pop().unwrap();
    assert!(
        app.accept_project_completion(completion).is_none(),
        "the actual reload worker must not update a reopened project"
    );
    super::spawn_workspace_projection_reload(
        &spawner,
        app.proxy.clone(),
        app.project_context("tab-a").unwrap(),
    );
    tasks.lock().unwrap().pop().unwrap()();
    assert!(
        matches!(app.accept_project_completion(recorded.lock().unwrap().pop().unwrap()), Some(UserEvent::WorkspaceProjectionLoaded { projection, .. }) if projection.title == "captured worker snapshot")
    );
}
