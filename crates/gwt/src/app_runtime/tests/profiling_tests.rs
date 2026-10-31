use super::*;

fn run_queued_projection_prepare(
    tasks: &BlockingTestTaskQueue,
    events: &Arc<Mutex<Vec<UserEvent>>>,
) -> Box<super::super::workspace_views::ActiveWorkProjectionPrepared> {
    // Run the independent census before the initially queued Work task. Later
    // generations may queue only Work, or another census after an in-flight Work.
    let queued = std::mem::take(&mut *tasks.lock().expect("queued tasks"));
    for task in queued {
        task();
    }
    let mut events = events.lock().expect("recorded events");
    let index = events
        .iter()
        .position(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::ActiveWorkProjectionPrepared(_)
            )
        })
        .expect("projection completion");
    let UserEvent::ActiveWorkProjectionPrepared(prepared) =
        into_recorded_project_payload(events.remove(index))
    else {
        unreachable!();
    };
    prepared
}

#[test]
fn issue_3777_runtime_hook_returns_before_full_projection_build() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let (mut runtime, probe, id) = owner_browser_input_fixture(temp.path());
    let owner = runtime.test_project_owners.values().next().unwrap().clone();
    let status = owner_runtime_status_event(&owner, &probe, "Error", "Stop");
    let context = runtime.test_context();
    let identity = owner.client().identity().unwrap().clone();
    // Commit launch-time preparation before measuring this status request.
    // test-hygiene: allow-wall-clock-deadline Bounds the existing setup; actual observer completion establishes ordering.
    let preparation_deadline = Instant::now() + Duration::from_secs(5);
    runtime.blocking_tasks = BlockingTaskSpawner::thread();
    runtime.refresh_active_work_projection_for_project_root(&context.project_root);
    wait_for_active_work_projection(&mut runtime);
    // Work and census finish independently. A running census would correctly
    // coalesce this next request instead of enqueueing another worker.
    while super::super::workspace_views::local_agent_activity_refresh_is_running(
        &runtime,
        &context.project_root,
    ) {
        assert!(
            // test-hygiene: allow-wall-clock-deadline Same setup hang guard; the observer's running state supplies ordering.
            Instant::now() < preparation_deadline,
            "launch-time census did not finish before measuring the next hook"
        );
        thread::sleep(Duration::from_millis(100));
    }
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    super::super::workspace_views::reset_full_active_work_projection_builds();
    let events = runtime.handle_project_owner_event(&context, 1, &identity, status);
    assert_eq!(runtime.runtimes[&id].status, WindowProcessStatus::Error);

    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        0,
        "RuntimeHook must only enqueue projection preparation on the tao callback",
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
        "the projection is dispatched only after background preparation commits",
    );
    assert_eq!(
        tasks.lock().expect("queued tasks").len(),
        2,
        "the owner handled heartbeat; the GUI queues census and projection workers",
    );
}

#[test]
fn issue_3777_tab_change_reuses_background_serialized_projection() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let payload: Arc<str> = Arc::from("x".repeat(4 * 1024 * 1024));
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_payload_cache
        .borrow_mut()
        .insert("tab-1".to_string(), payload.clone());

    let structured = runtime.active_work_projection_broadcast_on_tab_change("tab-1");

    assert!(structured.is_none());
    let event = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("prepared cache dispatch");
    let UserEvent::PreparedActiveWorkDispatch {
        tab_id,
        target,
        payload: dispatched,
        context,
    } = event
    else {
        panic!("expected PreparedActiveWorkDispatch");
    };
    assert_eq!(tab_id, "tab-1");
    assert_eq!(context, runtime.project_context("tab-1").unwrap());
    assert!(
        matches!(target, DispatchTarget::Project(key) if Some(&key) == runtime.project_key_for_tab("tab-1"))
    );
    assert!(Arc::ptr_eq(&payload, &dispatched));
}

#[test]
fn issue_3777_frontend_ready_reuses_background_serialized_projection() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let payload: Arc<str> = Arc::from("x".repeat(4 * 1024 * 1024));
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_payload_cache
        .borrow_mut()
        .insert("tab-1".to_string(), payload.clone());

    let structured = runtime.active_work_projection_reply("client-1", "tab-1");

    assert!(structured.is_none());
    let event = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("prepared cache reply");
    let UserEvent::PreparedActiveWorkDispatch {
        target,
        payload: dispatched,
        ..
    } = event
    else {
        panic!("expected PreparedActiveWorkDispatch");
    };
    assert!(matches!(target, DispatchTarget::Client(id) if id == "client-1"));
    assert!(Arc::ptr_eq(&payload, &dispatched));
}

#[test]
fn issue_3777_cache_only_patch_invalidates_stale_serialized_projection() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let stale_payload: Arc<str> = Arc::from("stale-before-watcher-patch");
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_payload_cache
        .borrow_mut()
        .insert("tab-1".to_string(), stale_payload);
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow_mut()
        .insert(
            "tab-1".to_string(),
            active_work_projection_from_saved(
                gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
            ),
        );
    let mut fresh = gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    fresh.title = "Fresh watcher state".to_string();

    runtime.merge_workspace_projection_into_cached_active_work(&repo, &fresh);

    assert!(
        !runtime
            .project_state_for_tab("tab-1")
            .unwrap()
            .active_work_projection_payload_cache
            .borrow()
            .contains_key("tab-1"),
        "a cache-only structured mutation must invalidate the older wire payload"
    );
    assert!(
        runtime
            .active_work_projection_reply("client-1", "tab-1")
            .is_none(),
        "a materialized Work must not be replaced by the empty live-session fallback"
    );
    assert_eq!(
        tasks.lock().unwrap().len(),
        2,
        "the census and full wire payload are prepared independently off the GUI thread"
    );
    assert!(
        recorded_events.lock().expect("recorded events").is_empty(),
        "FrontendReady must not replay a payload serialized before the watcher patch"
    );
}

#[test]
fn local_agent_activity_does_not_wait_for_work_history_lock() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (sent, received) = std::sync::mpsc::channel();
    runtime.proxy = AppEventProxy::Notifying {
        inner: Box::new(runtime.proxy.clone()),
        sent,
    };
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.active_work_projection_reply("client", "tab-1");
    let cache = runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .work_items_cache
        .clone();
    let history_lock = cache.lock().unwrap();
    let workers = tasks
        .lock()
        .unwrap()
        .drain(..)
        .map(std::thread::spawn)
        .collect::<Vec<_>>();
    // This timeout only guards a deadlock. The held mutex, not elapsed time,
    // proves that the census arrives before Work history can be materialized.
    // test-hygiene: allow-wall-clock-deadline Deadlock guard: the Work-history mutex stays locked until an independent census event arrives.
    let observed = received.recv_timeout(Duration::from_secs(30));
    drop(history_lock);
    for worker in workers {
        worker.join().unwrap();
    }
    assert!(
        observed.is_ok(),
        "local census must not wait for Work history"
    );
    let event = events.lock().unwrap().remove(0);
    let UserEvent::ProjectDispatch { events, .. } = into_recorded_project_payload(event) else {
        panic!("expected an independent local census dispatch");
    };
    assert!(matches!(
        events[0].event,
        BackendEvent::LocalAgentActivity { .. }
    ));
}

#[test]
fn local_agent_activity_hydrates_while_work_refreshes_keep_superseding() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.merge_workspace_projection_into_cached_active_work(
        &repo,
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    );
    assert!(runtime
        .active_work_projection_reply("client", "tab-1")
        .is_none());

    for _ in 0..2 {
        runtime.schedule_runtime_hook_active_work_projection_refresh(
            &repo,
            "posttooluse",
            "running",
        );
        let prepared = run_queued_projection_prepare(&tasks, &events);
        let commit = runtime.handle_active_work_projection_prepared(*prepared);
        assert!(
            commit.prepared_dispatch.is_none(),
            "stale Work stays withheld"
        );
        let event = commit
            .local_activity
            .expect("current census must reach hydration");
        let wire = serde_json::to_value(event.event).unwrap();
        assert_eq!(wire["kind"], "local_agent_activity");
        assert_eq!(wire["activity"]["agents"], serde_json::json!([]));
        assert_eq!(tasks.lock().unwrap().len(), 1, "one rerun remains queued");
    }

    let mut prepared = run_queued_projection_prepare(&tasks, &events);
    prepared.context.generation += 1;
    let commit = runtime.handle_active_work_projection_prepared(*prepared);
    assert!(
        commit.local_activity.is_none(),
        "old project contexts stay fenced"
    );
    assert!(commit.prepared_dispatch.is_none());
}

#[test]
fn issue_5404_unchanged_projection_inputs_skip_rebuild_and_changed_work_refreshes() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    // Hook discovery is fingerprinted from HOME as well as GWT HOME.
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    // Keep the first audit independent of the user's installed plugin inventory.
    fs::create_dir_all(repo.join(".gwt")).expect("fixture settings");
    fs::write(repo.join(".gwt/external-skill-roots.json"), "[]").expect("isolated skill roots");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    super::super::workspace_views::reset_full_active_work_projection_builds();

    let refresh = |runtime: &mut AppRuntime| {
        runtime.refresh_active_work_projection_for_project_root(&repo);
        let completion = run_queued_projection_prepare(&tasks, &events);
        runtime.handle_active_work_projection_prepared(*completion)
    };
    assert!(refresh(&mut runtime).prepared_dispatch.is_some());
    let unchanged = refresh(&mut runtime);
    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        1,
        "unchanged input fingerprints must reuse the prepared projection",
    );
    assert!(
        unchanged.prepared_dispatch.is_none(),
        "unchanged inputs do not broadcast"
    );
    assert!(
        unchanged.local_activity.is_some(),
        "memo hits still carry their census"
    );

    let mut projection =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
    projection.title = "Changed canonical Work".into();
    gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
        .expect("save changed Current");
    assert!(refresh(&mut runtime).prepared_dispatch.is_some());
    assert_eq!(
        super::super::workspace_views::full_active_work_projection_builds(),
        2
    );

    fs::create_dir_all(repo.join(".claude")).expect("create hook surface");
    assert!(
        refresh(&mut runtime).prepared_dispatch.is_some(),
        "hook surface presence invalidates the projection memo"
    );

    let discussion = repo.join(".gwt/discussion.md");
    fs::create_dir_all(discussion.parent().unwrap()).unwrap();
    fs::write(
        discussion,
        "## Discussion TODO\n\n### Proposal A - Goal handoff [chosen]\n\
         - Goal Condition: finish projection verification\n- Goal State: pending\n",
    )
    .unwrap();
    assert!(
        refresh(&mut runtime).prepared_dispatch.is_some(),
        "pending goal changes invalidate the projection memo"
    );
}

#[test]
#[ignore = "Issue #5404 same-fixture GUI occupancy measurement"]
fn issue_5404_gui_occupancy_measurement() {
    let temp = tempdir().expect("fixture");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    fs::create_dir_all(&runtime.sessions_dir).unwrap();
    let mut session =
        gwt_agent::Session::new(repo.clone(), "work/fixture", gwt_agent::AgentId::Codex);
    session.launch_args = vec!["x".repeat(21 * 1024)];
    for index in 0..2500 {
        session.id = format!("occupancy-session-{index:04}");
        session.save(&runtime.sessions_dir).unwrap();
    }
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let before = Instant::now();
    runtime
        .build_active_work_projection_for_tab_for_test("tab-1", &runtime.tabs[0])
        .unwrap();
    let synchronous_ms = before.elapsed().as_secs_f64() * 1000.0;
    let after = Instant::now();
    for _ in 0..20 {
        runtime.refresh_active_work_projection_for_project_root(&repo);
    }
    let gui_ms_per_call = after.elapsed().as_secs_f64() * 1000.0 / 20.0;
    assert_eq!(tasks.lock().unwrap().len(), 2);
    let completion = run_queued_projection_prepare(&tasks, &events);
    runtime.handle_active_work_projection_prepared(*completion);
    let completion = run_queued_projection_prepare(&tasks, &events);
    runtime.handle_active_work_projection_prepared(*completion);
    let builds = super::super::workspace_views::full_active_work_projection_builds();
    let loads = super::super::workspace_views::active_work_projection_shared_loads();
    let memo_started = Instant::now();
    runtime.refresh_active_work_projection_for_project_root(&repo);
    run_queued_projection_prepare(&tasks, &events);
    let memo_refresh_ms = memo_started.elapsed().as_secs_f64() * 1000.0;
    let extra_builds = super::super::workspace_views::full_active_work_projection_builds() - builds;
    let extra_loads = super::super::workspace_views::active_work_projection_shared_loads();
    let extra_loads = std::array::from_fn::<_, 3, _>(|index| extra_loads[index] - loads[index]);
    println!("Issue #5404: sessions=2500, synchronous_full_build_ms={synchronous_ms:.3}, gui_schedule_ms_per_call={gui_ms_per_call:.3}, burst_requests=20, workers=2, memo_refresh_ms={memo_refresh_ms:.3}, memo_full_builds={extra_builds}, memo_hook_journal_ledger_loads={extra_loads:?}");
}

#[test]
fn issue_3777_runtime_hook_refresh_burst_keeps_one_worker_and_latest_generation() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    assert!(runtime
        .refresh_active_work_projection_for_project_root(&repo)
        .is_empty());
    assert!(runtime
        .refresh_active_work_projection_for_project_root(&repo)
        .is_empty());
    assert_eq!(
        tasks.lock().expect("queued tasks").len(),
        2,
        "a refresh burst keeps one census and one Work worker in flight",
    );

    let first_completion = run_queued_projection_prepare(&tasks, &recorded_events);
    let first_commit = runtime.handle_active_work_projection_prepared(*first_completion);
    assert!(
        first_commit.prepared_dispatch.is_none(),
        "a stale generation must not dispatch",
    );
    assert!(
        first_commit.profile.is_some(),
        "a stale generation still emits its content-free timing profile",
    );
    assert_eq!(
        tasks.lock().expect("queued tasks").len(),
        1,
        "the latest dirty generation starts only after the first worker completes",
    );

    let latest_completion = run_queued_projection_prepare(&tasks, &recorded_events);
    let latest_commit = runtime.handle_active_work_projection_prepared(*latest_completion);
    assert!(
        latest_commit.prepared_dispatch.is_some(),
        "only the latest generation may commit and dispatch",
    );
}

#[test]
fn issue_3777_close_project_tab_discards_cached_and_pending_projection_work() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&other).expect("other dir");
    let tabs = vec![
        sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]),
        sample_project_tab("tab-2", "Other", other, ProjectKind::Git, &[]),
    ];
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), tabs, Some("tab-2"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow_mut()
        .insert(
            "tab-1".to_string(),
            active_work_projection_from_saved(
                gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
            ),
        );
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_payload_cache
        .borrow_mut()
        .insert("tab-1".to_string(), Arc::from("closed-project-payload"));
    runtime
        .project_state_for_tab("tab-2")
        .unwrap()
        .active_work_projection_payload_cache
        .borrow_mut()
        .insert("tab-2".to_string(), Arc::from("active-project-payload"));
    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    fs::create_dir_all(work_items_path.parent().expect("project state dir"))
        .expect("project state dir");
    gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
        &work_items_path,
        &gwt_core::workspace_projection::WorkItemsProjection::empty(Utc::now()),
    )
    .expect("seed cached Work projection");
    let cached_work_items = runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .work_items_cache
        .lock()
        .expect("Work cache")
        .load_or_synthesize_shared(&repo)
        .expect("cache Work projection")
        .0;
    let retained_work_items = Arc::downgrade(&cached_work_items);
    drop(cached_work_items);

    runtime.refresh_active_work_projection_for_project_root(&repo);
    runtime.refresh_active_work_projection_for_project_root(&repo);
    assert_eq!(tasks.lock().expect("queued tasks").len(), 2);
    let cache_owner = runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .work_items_cache
        .clone();
    let cache_lease = cache_owner.lock().expect("hold Work cache lease");

    runtime.close_project_tab_events("tab-1");

    assert!(runtime.project_state_for_tab("tab-1").is_none());
    assert!(runtime
        .project_state_for_tab("tab-2")
        .unwrap()
        .active_work_projection_payload_cache
        .borrow()
        .contains_key("tab-2"));
    assert!(
        retained_work_items.upgrade().is_some(),
        "an outstanding worker retains its own project cache until it completes"
    );
    drop(cache_lease);
    drop(cache_owner);

    let completion = run_queued_projection_prepare(&tasks, &recorded_events);
    let commit = runtime.handle_active_work_projection_prepared(*completion);
    assert!(commit.prepared_dispatch.is_none());
    assert!(
        retained_work_items.upgrade().is_none(),
        "worker completion releases the closed project's final cache owner"
    );
    assert!(
        tasks.lock().expect("queued tasks").is_empty(),
        "closing a project must discard its pending generation"
    );
}

#[test]
fn issue_3777_first_authoritative_projection_refuses_legacy_only_work() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let legacy_root = gwt_core::paths::gwt_project_dir_for_repo_path(&repo).join("workspace");
    let legacy_current = legacy_root.join("current.json");
    let legacy_works = legacy_root.join("work_items.json");
    let now = Utc::now();
    let current_bytes = serde_json::to_vec(
        &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
    )
    .unwrap();
    fs::create_dir_all(&legacy_root).unwrap();
    fs::write(&legacy_current, &current_bytes).expect("seed legacy current");
    let mut work_items = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
    work_items.apply_event(gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "work-3777-legacy-first",
        now,
    ));
    let works_bytes = serde_json::to_vec(&work_items).unwrap();
    fs::write(&legacy_works, &works_bytes).expect("seed legacy works");
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.refresh_active_work_projection_for_project_root(&repo);
    let completion = run_queued_projection_prepare(&tasks, &recorded_events);
    assert!(completion.result.is_err());
    let error = completion.load_error.as_ref().expect("legacy load error");
    assert!(error.message.contains("v9.106.0"));

    let committed = runtime.handle_active_work_projection_prepared(*completion);
    assert!(committed.prepared_dispatch.is_none());
    assert!(recorded_events.lock().unwrap().iter().any(|event| matches!(
        recorded_project_payload(event),
        UserEvent::WorkspaceStateLoadFailed { error, .. }
            if error.message.contains("v9.106.0")
    )));
    assert_eq!(fs::read(&legacy_current).unwrap(), current_bytes);
    assert_eq!(fs::read(&legacy_works).unwrap(), works_bytes);
    let canonical = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&repo);
    assert!(!canonical.exists());
    assert!(!canonical.with_file_name("works.json").exists());
}

#[test]
fn issue_3777_normal_refresh_does_not_erase_pending_runtime_hook_profile() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.refresh_active_work_projection_for_project_root(&repo);
    runtime.schedule_runtime_hook_active_work_projection_refresh(&repo, "stop", "stopped");
    runtime.refresh_active_work_projection_for_project_root(&repo);

    let first = run_queued_projection_prepare(&tasks, &recorded_events);
    runtime.handle_active_work_projection_prepared(*first);
    let latest = run_queued_projection_prepare(&tasks, &recorded_events);

    assert_eq!(latest.profile.source_event, "stop");
    assert_eq!(latest.profile.composed_state, "stopped");
}

#[test]
fn issue_3777_runtime_hook_failure_preserves_last_good_projection() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let last_good = gwt::ActiveWorkProjectionView {
        id: "last-good".to_string(),
        title: "Last good".to_string(),
        status_category: "idle".to_string(),
        status_text: "Paused".to_string(),
        summary: None,
        progress_summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: None,
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        pr_created_at: None,
        board_refs: Vec::new(),
        journal_entries: Vec::new(),
        works: Vec::new(),
        cleanup_candidate: None,
        managed_hook_health: None,
        local_agent_activity: None,
        active_work_count: 0,
        active_works: Vec::new(),
        agents: Vec::new(),
        unassigned_agents: Vec::new(),
    };
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_cache
        .borrow_mut()
        .insert("tab-1".to_string(), last_good);
    let last_good_payload: Arc<str> = Arc::from("last-good-payload");
    runtime
        .project_state_for_tab("tab-1")
        .unwrap()
        .active_work_projection_payload_cache
        .borrow_mut()
        .insert("tab-1".to_string(), last_good_payload.clone());
    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    fs::create_dir_all(work_items_path.parent().expect("project-state dir"))
        .expect("project-state dir");
    fs::write(&work_items_path, b"{not valid json").expect("corrupt works fixture");
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.refresh_active_work_projection_for_project_root(&repo);
    let completion = run_queued_projection_prepare(&tasks, &recorded_events);
    let commit = runtime.handle_active_work_projection_prepared(*completion);

    assert!(commit.prepared_dispatch.is_none());
    assert!(
        commit.local_activity.is_some(),
        "Work errors do not suppress the census"
    );
    assert!(
        commit.profile.is_some(),
        "a failed prepare still emits its content-free timing profile",
    );
    assert_eq!(
        runtime
            .project_state_for_tab("tab-1")
            .unwrap()
            .active_work_projection_cache
            .borrow()
            .get("tab-1")
            .map(|projection| projection.id.as_str()),
        Some("last-good"),
        "a failed prepare must preserve the last-good cache",
    );
    assert!(Arc::ptr_eq(
        runtime
            .project_state_for_tab("tab-1")
            .unwrap()
            .active_work_projection_payload_cache
            .borrow()
            .get("tab-1")
            .expect("last-good payload"),
        &last_good_payload,
    ));
}

#[test]
fn issue_3777_runtime_hook_profiles_work_lease_wait_separately_from_parse() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    fs::create_dir_all(work_items_path.parent().expect("project-state dir"))
        .expect("project-state dir");
    let now = Utc::now();
    let mut work_items = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
    work_items.apply_event(gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "work-3777-profile",
        now,
    ));
    gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
        &work_items_path,
        &work_items,
    )
    .expect("save works fixture");
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    let lease = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(work_items_path.with_extension("lock"))
        .expect("open Work lease");
    lease.lock_exclusive().expect("hold Work lease");
    let (contention_tx, contention_rx) = std::sync::mpsc::channel();
    let _contention = gwt_core::operation_deadline::ScopedNamedLockContentionObserver::set(
        &work_items_path.with_extension("lock"),
        move || {
            contention_tx
                .send(())
                .expect("report Work lease contention")
        },
    );
    let release = thread::spawn(move || {
        // Start the existing hold interval only after the loader is waiting.
        // Always unlock before reporting a missing notification to avoid a hang.
        // test-hygiene: allow-wall-clock-deadline Bounds a missing notification, not the measured lock wait; unlock before failing.
        let contention = contention_rx.recv_timeout(Duration::from_secs(30));
        if contention.is_ok() {
            thread::sleep(Duration::from_millis(120));
        }
        lease.unlock().expect("release Work lease");
        contention.expect("observe Work lease contention before release");
    });

    runtime.refresh_active_work_projection_for_project_root(&repo);
    let completion = run_queued_projection_prepare(&tasks, &recorded_events);
    release.join().expect("join Work lease holder");

    assert!(
        completion.profile.lock_wait_ms >= 75,
        "filesystem Work lease wait must be attributed to lock_wait_ms: {:?}",
        completion.profile
    );
    assert!(
        completion.profile.parse_ms < completion.profile.lock_wait_ms,
        "parse_ms must exclude the Work lease wait: {:?}",
        completion.profile
    );
}

#[test]
fn issue_3777_runtime_hook_profile_uses_content_free_exact_substage_allowlist() {
    let source = include_str!("../../main.rs");
    let marker = source
        .split_once("marker = \"issue_3777_runtime_hook_profile\"")
        .map(|(_, tail)| tail.split_once(");").map_or(tail, |(marker, _)| marker))
        .expect("Issue #3777 RuntimeHook profile marker");
    let mut fields = marker
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('"') {
                return None;
            }
            let field = line
                .split_once(" =")
                .map_or_else(|| line.strip_suffix(','), |(field, _)| Some(field))?;
            (!field.is_empty()
                && field
                    .chars()
                    .all(|character| character == '_' || character.is_ascii_alphanumeric()))
            .then_some(field)
        })
        .collect::<Vec<_>>();
    fields.sort_unstable();
    assert_eq!(
        fields,
        vec![
            "cache_hit",
            "clone_ms",
            "composed_state",
            "dispatch_ms",
            "lock_wait_ms",
            "parse_ms",
            "projection_ms",
            "serialization_ms",
            "source_event",
        ],
        "profiling fields are an exact content-free allowlist",
    );
    for forbidden in [
        "project_root",
        "window_id",
        "message",
        "tool_name",
        "path",
        "payload",
        "error",
        "raw",
    ] {
        assert!(!marker.contains(forbidden), "forbidden field {forbidden}");
    }
}

#[test]
fn issue_3777_serialization_profile_attributes_serializer_latency() {
    let event = BackendEvent::ActiveWorkProjection {
        projection: Box::new(active_work_projection_from_saved(
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project("/repo"),
        )),
    };

    let (_payload, serialization_ms) =
        super::super::workspace_views::serialize_active_work_projection_event_with(
            &event,
            |event| {
                // test-hygiene: allow-short-duration retained elapsed-time instrumentation probe; serialization_ms must include the injected callback latency
                thread::sleep(Duration::from_millis(40));
                serde_json::to_string(event).map_err(|error| error.to_string())
            },
        )
        .expect("serialize projection event");

    assert!(
        serialization_ms >= 30,
        "serializer latency must be attributed to serialization_ms: {serialization_ms}ms"
    );
}
