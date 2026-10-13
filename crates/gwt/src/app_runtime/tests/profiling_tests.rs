use super::*;

#[test]
fn issue_3777_runtime_hook_returns_before_full_projection_build() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "codex-1",
        repo,
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    super::super::workspace_views::reset_full_active_work_projection_builds();
    let events = runtime.handle_runtime_hook_event(runtime_hook_state("Stopped", "session-1"));

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
        "one heartbeat worker and one background projection worker must be queued",
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
    let (runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
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
    assert!(runtime
        .active_work_projection_reply("client-1", "tab-1")
        .is_some());
    assert!(
        recorded_events.lock().expect("recorded events").is_empty(),
        "FrontendReady must not replay a payload serialized before the watcher patch"
    );
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
        1,
        "a refresh burst must keep at most one worker in flight",
    );

    let first_task = tasks.lock().expect("queued tasks").remove(0);
    first_task();
    let first_completion = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("first projection completion");
    let UserEvent::ActiveWorkProjectionPrepared(first_completion) =
        into_recorded_project_payload(first_completion)
    else {
        panic!("expected ActiveWorkProjectionPrepared");
    };
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

    let latest_task = tasks.lock().expect("queued tasks").remove(0);
    latest_task();
    let latest_completion = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("latest projection completion");
    let UserEvent::ActiveWorkProjectionPrepared(latest_completion) =
        into_recorded_project_payload(latest_completion)
    else {
        panic!("expected ActiveWorkProjectionPrepared");
    };
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
    assert_eq!(tasks.lock().expect("queued tasks").len(), 1);
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

    tasks.lock().expect("queued tasks").remove(0)();
    let completion_index = recorded_events
        .lock()
        .expect("recorded events")
        .iter()
        .position(|event| {
            matches!(
                recorded_project_payload(event),
                UserEvent::ActiveWorkProjectionPrepared(_)
            )
        })
        .expect("stale projection completion");
    let completion = recorded_events
        .lock()
        .expect("recorded events")
        .remove(completion_index);
    let UserEvent::ActiveWorkProjectionPrepared(completion) =
        into_recorded_project_payload(completion)
    else {
        panic!("expected ActiveWorkProjectionPrepared");
    };
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
    tasks.lock().expect("queued tasks").remove(0)();
    let completion = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("first projection completion");
    let UserEvent::ActiveWorkProjectionPrepared(completion) =
        into_recorded_project_payload(completion)
    else {
        panic!("expected ActiveWorkProjectionPrepared");
    };
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

    tasks.lock().expect("queued tasks").remove(0)();
    let first = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("first completion");
    let UserEvent::ActiveWorkProjectionPrepared(first) = into_recorded_project_payload(first)
    else {
        panic!("expected ActiveWorkProjectionPrepared");
    };
    runtime.handle_active_work_projection_prepared(*first);
    tasks.lock().expect("queued tasks").remove(0)();
    let latest = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("latest completion");
    let UserEvent::ActiveWorkProjectionPrepared(latest) = into_recorded_project_payload(latest)
    else {
        panic!("expected ActiveWorkProjectionPrepared");
    };

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
    tasks.lock().expect("queued tasks").remove(0)();
    let completion = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("failed projection completion");
    let UserEvent::ActiveWorkProjectionPrepared(completion) =
        into_recorded_project_payload(completion)
    else {
        panic!("expected ActiveWorkProjectionPrepared");
    };
    let commit = runtime.handle_active_work_projection_prepared(*completion);

    assert!(commit.prepared_dispatch.is_none());
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
    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(120));
        lease.unlock().expect("release Work lease");
    });

    runtime.refresh_active_work_projection_for_project_root(&repo);
    tasks.lock().expect("queued tasks").remove(0)();
    release.join().expect("join Work lease holder");
    let completion = recorded_events
        .lock()
        .expect("recorded events")
        .pop()
        .expect("projection completion");
    let UserEvent::ActiveWorkProjectionPrepared(completion) =
        into_recorded_project_payload(completion)
    else {
        panic!("expected ActiveWorkProjectionPrepared");
    };

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
