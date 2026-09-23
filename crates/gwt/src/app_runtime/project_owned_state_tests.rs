#[test]
fn project_owned_caches_and_pending_work_survive_other_project_reopen_and_close() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let mut runtime = sample_runtime(temp.path(), vec![
        sample_project_tab("tab-a", "A", a.clone(), ProjectKind::Git, &[]),
        sample_project_tab("tab-b", "B", b.clone(), ProjectKind::Git, &[]),
    ], Some("tab-a"));
    for (tab, root) in [("tab-a", &a), ("tab-b", &b)] {
        let context = runtime.project_context(tab).unwrap();
        let mut wizard = sample_launch_wizard_session(tab, root);
        wizard.project_context = context;
        wizard.wizard_id = format!("wizard-{tab}");
        let state = runtime.project_state_for_tab_mut(tab).unwrap();
        state.pending_launch_wizard_materializations.insert(wizard.wizard_id.clone(), wizard);
        state.issue_monitor_scheduled_scans_in_flight.insert(gwt::issue_monitor_prefs_path_for_repo_path(root));
        state.active_work_projection_payload_cache.borrow_mut().insert(tab.into(), Arc::from(tab));
        state.active_work_projection_cache.borrow_mut().insert(tab.into(), active_work_projection_from_saved(
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(root),
        ));
    }
    runtime.apply_work_merge_status(&a, HashMap::from([("merged-a".into(), chrono::Utc::now())]), HashMap::new(), HashSet::new(), HashSet::new(), None);
    let cache_a = runtime.project_state_for_tab("tab-a").unwrap().work_items_cache.clone();
    let cache_b = runtime.project_state_for_tab("tab-b").unwrap().work_items_cache.clone();
    assert!(!Arc::ptr_eq(&cache_a, &cache_b), "projects must not share a mutable parse cache");
    assert!(runtime.project_state_for_tab("tab-b").unwrap().work_merged_branches.is_empty());
    runtime.project_tab_incarnations.get_mut("tab-a").unwrap().generation += 1;
    runtime.refresh_project_state("tab-a");
    let fresh_a = runtime.project_state_for_tab("tab-a").unwrap();
    assert!(fresh_a.work_merged_branches.is_empty());
    assert!(fresh_a.active_work_projection_cache.borrow().is_empty());
    assert!(fresh_a.pending_launch_wizard_materializations.is_empty());
    assert!(fresh_a.issue_monitor_scheduled_scans_in_flight.is_empty());
    assert!(!Arc::ptr_eq(&cache_a, &fresh_a.work_items_cache));
    runtime.close_project_tab_events("tab-a");
    let state_b = runtime.project_state_for_tab("tab-b").unwrap();
    assert!(Arc::ptr_eq(&cache_b, &state_b.work_items_cache));
    assert!(state_b.pending_launch_wizard_materializations.contains_key("wizard-tab-b"));
    assert!(state_b.issue_monitor_scheduled_scans_in_flight.contains(&gwt::issue_monitor_prefs_path_for_repo_path(&b)));
    assert!(state_b.active_work_projection_cache.borrow().contains_key("tab-b"));
    assert_eq!(state_b.active_work_projection_payload_cache.borrow().get("tab-b").map(AsRef::as_ref), Some("tab-b"));
}
