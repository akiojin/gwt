// Issue #4538 AC-1 / AC-2 / AC-4: per-project URL routing. A browser tab bound
// to `/p/<hash>` announces itself with `frontend_ready` on a Project-scoped
// connection; the runtime resolves that hash against open Projects and Recent
// entries without touching the filesystem on the tao thread.

fn take_recorded_event<T>(
    label: &str,
    recorded_events: &Arc<Mutex<Vec<UserEvent>>>,
    extract: impl Fn(&UserEvent) -> Option<T>,
) -> T {
    wait_for_recorded_event(label, recorded_events, |events| {
        events
            .iter()
            .any(|event| extract(recorded_project_payload(event)).is_some())
    });
    let mut events = recorded_events.lock().expect("event log");
    let index = events
        .iter()
        .position(|event| extract(recorded_project_payload(event)).is_some())
        .expect(label);
    let event = events.remove(index);
    extract(recorded_project_payload(&event)).expect(label)
}

fn take_recent_project_keys_resolution(
    recorded_events: &Arc<Mutex<Vec<UserEvent>>>,
) -> RecentProjectKeysResolved {
    take_recorded_event(
        "recent project key resolution",
        recorded_events,
        |event| match event {
            UserEvent::RecentProjectKeysResolved(resolved) => Some(resolved.clone()),
            _ => None,
        },
    )
}

fn route_test_runtime(
    temp: &Path,
    recent: &[&Path],
) -> (AppRuntime, Arc<Mutex<Vec<UserEvent>>>, BlockingTestTaskQueue) {
    let (mut runtime, recorded_events) = sample_runtime_with_events(temp, Vec::new(), None);
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = blocking_tasks;
    runtime.recent_projects = recent
        .iter()
        .map(|path| gwt::RecentProjectEntry {
            path: path.to_path_buf(),
            title: gwt::project_title_from_path(path),
            kind: ProjectKind::NonRepo,
        })
        .collect();
    (runtime, recorded_events, queued_tasks)
}

#[test]
fn project_route_for_a_known_recent_project_auto_opens_it_for_the_bound_client() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let recent = temp.path().join("recent-project");
    fs::create_dir_all(&recent).expect("recent project");
    let key = gwt_core::paths::resolve_project_scope(&recent).hash;
    let (mut runtime, recorded_events, queued_tasks) =
        route_test_runtime(temp.path(), &[recent.as_path()]);

    let ready = runtime.handle_frontend_event_in_scope(
        "client-a".into(),
        FrontendEvent::FrontendReady,
        &crate::app_runtime::ClientScope::Project(key.clone()),
    );
    assert!(
        ready.is_empty(),
        "hash resolution runs off the tao thread: {ready:?}"
    );
    assert!(runtime.tabs.is_empty());

    drain_queued_blocking_tasks(&queued_tasks);
    let resolved = take_recent_project_keys_resolution(&recorded_events);
    runtime.handle_recent_project_keys_resolved(resolved);
    let committed = commit_pending_project_navigation(&mut runtime, &queued_tasks, &recorded_events);

    assert_eq!(runtime.tabs.len(), 1, "the known Recent project opens");
    assert!(
        committed.iter().any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })
            && event.target == DispatchTarget::Project(key.clone())),
        "the waiting /p/<hash> client receives the Project full sync: {committed:?}"
    );
    assert!(committed
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::ProjectNotFound { .. })));
}

#[test]
fn project_route_for_an_unknown_hash_replies_not_found_to_that_client_only() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let recent = temp.path().join("recent-project");
    fs::create_dir_all(&recent).expect("recent project");
    let unknown = gwt_core::repo_hash::ProjectKey::parse("cccccccccccccccc").expect("valid unresolved hash");
    let (mut runtime, recorded_events, queued_tasks) =
        route_test_runtime(temp.path(), &[recent.as_path()]);

    assert!(runtime
        .handle_frontend_event_in_scope(
            "client-a".into(),
            FrontendEvent::FrontendReady,
            &crate::app_runtime::ClientScope::Project(unknown.clone()),
        )
        .is_empty());
    drain_queued_blocking_tasks(&queued_tasks);
    let resolved = take_recent_project_keys_resolution(&recorded_events);
    let events = runtime.handle_recent_project_keys_resolved(resolved);

    let not_found: Vec<_> = events
        .iter()
        .filter(|event| matches!(event.event, BackendEvent::ProjectNotFound { .. }))
        .collect();
    assert_eq!(not_found.len(), 1, "{events:?}");
    assert_eq!(
        not_found[0].target,
        DispatchTarget::Client("client-a".to_string())
    );
    let payload = serde_json::to_string(&not_found[0].event).expect("serialize");
    assert!(payload.contains("cccccccccccccccc"));
    assert!(
        !payload.contains(&recent.display().to_string()),
        "not-found never discloses a filesystem path: {payload}"
    );
    assert!(runtime.tabs.is_empty());
    assert!(queued_tasks.lock().expect("task queue").is_empty());

    // With every Recent key cached, the answer needs no further worker.
    let again = runtime.handle_frontend_event_in_scope(
        "client-b".into(),
        FrontendEvent::FrontendReady,
        &crate::app_runtime::ClientScope::Project(unknown),
    );
    assert!(again.iter().any(|event| matches!(event.event, BackendEvent::ProjectNotFound { .. })
        && event.target == DispatchTarget::Client("client-b".to_string())));
}

#[test]
fn hub_catalog_carries_recent_project_keys_once_they_are_resolved() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let recent = temp.path().join("recent-project");
    fs::create_dir_all(&recent).expect("recent project");
    let key = gwt_core::paths::resolve_project_scope(&recent).hash;
    let (mut runtime, recorded_events, queued_tasks) =
        route_test_runtime(temp.path(), &[recent.as_path()]);

    let hub = runtime.handle_frontend_event_in_scope(
        "hub".into(),
        FrontendEvent::FrontendReady,
        &crate::app_runtime::ClientScope::Hub,
    );
    assert!(hub.iter().any(|event| matches!(&event.event, BackendEvent::HubState { hub }
        if hub.recent_projects[0].project_key.is_none())));

    drain_queued_blocking_tasks(&queued_tasks);
    let resolved = take_recent_project_keys_resolution(&recorded_events);
    let events = runtime.handle_recent_project_keys_resolved(resolved);

    assert!(
        events.iter().any(|event| event.target == DispatchTarget::Hub
            && matches!(&event.event, BackendEvent::HubState { hub }
                if hub.recent_projects[0].project_key.as_deref() == Some(key.as_str()))),
        "Hub clients receive path-free Recent links after resolution: {events:?}"
    );
}

#[test]
fn control_project_open_replies_with_the_committed_project_key() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let target = temp.path().join("cli-project");
    fs::create_dir_all(&target).expect("cli project");
    let key = gwt_core::paths::resolve_project_scope(&target).hash;
    let (mut runtime, recorded_events, queued_tasks) = route_test_runtime(temp.path(), &[]);
    let (reply, mut outcome) = ProjectOpenReply::channel();

    assert!(runtime.control_project_open_events(target, reply).is_empty());
    assert!(
        outcome.try_recv().is_err(),
        "the reply waits for the committed open"
    );
    commit_pending_project_navigation(&mut runtime, &queued_tasks, &recorded_events);

    assert_eq!(outcome.try_recv().expect("reply sent"), Ok(key));
    assert_eq!(runtime.tabs.len(), 1);
}

#[test]
fn control_project_open_rejects_a_path_that_cannot_be_opened() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let missing = temp.path().join("missing-project");
    let (mut runtime, recorded_events, queued_tasks) = route_test_runtime(temp.path(), &[]);
    let (reply, mut outcome) = ProjectOpenReply::channel();

    runtime.control_project_open_events(missing, reply);
    commit_pending_project_navigation(&mut runtime, &queued_tasks, &recorded_events);

    assert!(matches!(
        outcome.try_recv().expect("reply sent"),
        Err(ProjectOpenControlFailure::Rejected(_))
    ));
    assert!(runtime.tabs.is_empty());
}

#[test]
fn control_project_open_reports_a_superseded_request_as_unavailable() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir_all(&first).expect("first project");
    fs::create_dir_all(&second).expect("second project");
    let (mut runtime, recorded_events, queued_tasks) = route_test_runtime(temp.path(), &[]);
    let (reply, mut outcome) = ProjectOpenReply::channel();

    runtime.control_project_open_events(first, reply);
    runtime.open_project_path_events(second);
    drain_queued_blocking_tasks(&queued_tasks);
    for _ in 0..2 {
        let prepared = take_project_navigation_completion(&recorded_events);
        runtime.handle_project_navigation_prepared(prepared);
    }

    assert!(matches!(
        outcome.try_recv().expect("reply sent"),
        Err(ProjectOpenControlFailure::Unavailable(_))
    ));
}
