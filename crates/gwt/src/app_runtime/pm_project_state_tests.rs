#[test]
fn project_pm_pending_state_is_dropped_on_reopen_without_touching_other_project() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    let mut runtime = sample_runtime(
        temp.path(),
        vec![
            sample_project_tab("tab-a", "A", a.clone(), ProjectKind::Git, &[]),
            sample_project_tab("tab-b", "B", b.clone(), ProjectKind::Git, &[]),
        ],
        Some("tab-a"),
    );
    runtime.project_state_for_root_mut(&a).unwrap().pending_pm_launches.insert("tab-a:pm".into(), a.clone());
    runtime.project_state_for_root_mut(&b).unwrap().pending_pm_launches.insert("tab-b:pm".into(), b.clone());
    runtime.project_state_for_root_mut(&a).unwrap().pending_pm_closes.insert(a.clone(), 1);
    runtime.project_state_for_root_mut(&b).unwrap().pending_pm_closes.insert(b.clone(), 2);
    for root in [&a, &b] {
        let state = runtime.project_state_for_root_mut(root).unwrap();
        state.pm_sessions.insert(root.clone(), "resident-pm".into());
        state.pm_wake_seen.insert(root.clone(), Default::default());
        state.pending_pm_worktree_preparations.insert(root.clone());
    }
    runtime.project_tab_incarnations.get_mut("tab-a").unwrap().generation += 1;
    runtime.refresh_project_state("tab-a");
    assert!(!runtime.project_state_for_root(&a).unwrap().pending_pm_launches.contains_key("tab-a:pm"));
    assert!(!runtime.project_state_for_root(&a).unwrap().pending_pm_closes.contains_key(&a));
    assert_eq!(runtime.project_state_for_root(&b).unwrap().pending_pm_launches.get("tab-b:pm"), Some(&b));
    assert_eq!(runtime.project_state_for_root(&b).unwrap().pending_pm_closes.get(&b), Some(&2));
    let reset = runtime.project_state_for_root(&a).unwrap();
    assert!(reset.pm_sessions.is_empty());
    assert!(reset.pm_wake_seen.is_empty());
    assert!(reset.pending_pm_worktree_preparations.is_empty());
    let context_a = runtime.project_context("tab-a").unwrap();
    runtime.close_project_tab_events("tab-a");
    assert!(runtime.project_state(&context_a).is_none());
    let survivor = runtime.project_state_for_root(&b).unwrap();
    assert_eq!(survivor.pm_sessions.get(&b).map(String::as_str), Some("resident-pm"));
    assert!(survivor.pm_wake_seen.contains_key(&b));
    assert!(survivor.pending_pm_worktree_preparations.contains(&b));
    assert_eq!(runtime.project_state_for_root(&b).unwrap().pending_pm_launches.get("tab-b:pm"), Some(&b));
    assert_eq!(runtime.project_state_for_root(&b).unwrap().pending_pm_closes.get(&b), Some(&2));
}

#[test]
fn stale_pm_close_finalizer_cannot_clear_reopened_projects_close_fence() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let tab = sample_project_tab("tab-a", "A", root.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-a"));
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.queue_window_close_finalizer(
        "tab-a::old-pm", Some(root.clone()), None, true, false, None, None,
    );
    runtime.project_tab_incarnations.get_mut("tab-a").unwrap().generation += 1;
    runtime.refresh_project_state("tab-a");
    runtime.project_state_for_root_mut(&root).unwrap().pending_pm_closes.insert(root.clone(), 1);
    let finalizer = finalizers.lock().unwrap().pop().unwrap();
    finalizer();
    apply_recorded_window_close_finalized(&mut runtime, &recorded);
    assert_eq!(
        runtime.project_state_for_root(&root).unwrap().pending_pm_closes.get(&root),
        Some(&1),
        "an old generation's finalizer must not release the successor PM close fence",
    );
}
