//! Issue #7869: `FrontendReady` and `GetExecutionEnvironments` must not wait on
//! locks another process holds, and resends must not pile up worker reads.

use super::*;

/// The locks a daemon worker holds while it commits PM or Agent registry state.
fn hold_daemon_locks(root: &Path) -> Vec<gwt_core::operation_deadline::NamedFileLock> {
    let registry = gwt::agent_registry::path().with_extension("lock");
    let pm_prefs = gwt::pm_registry::pm_prefs_path_for_repo_path(root).with_extension("lock");
    [
        (
            registry.with_extension("core.lock"),
            "shared Agent registry core",
        ),
        (registry, "shared Agent registry"),
        (pm_prefs, "pm_prefs"),
    ]
    .into_iter()
    .map(|(path, operation)| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        gwt_core::operation_deadline::NamedFileLock::acquire_quiet(&path, operation).unwrap()
    })
    .collect()
}

fn hydration_runtime(
    temp: &Path,
) -> (
    AppRuntime,
    Arc<Mutex<Vec<UserEvent>>>,
    BlockingTestTaskQueue,
) {
    let root = temp.join("repo");
    fs::create_dir_all(&root).unwrap();
    let tab = sample_project_tab("tab-a", "A", root, ProjectKind::Git, &[]);
    let (mut runtime, recorded) = sample_runtime_with_events(temp, vec![tab], Some("tab-a"));
    let (spawner, queued) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    (runtime, recorded, queued)
}

fn pm_status_replies(recorded: &Arc<Mutex<Vec<UserEvent>>>) -> usize {
    recorded
        .lock()
        .unwrap()
        .iter()
        .filter(|event| match event {
            UserEvent::AgentRosterDispatchPrepared { events, .. } => events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::PmStatus { .. })),
            _ => false,
        })
        .count()
}

fn execution_environment_replies(recorded: &Arc<Mutex<Vec<UserEvent>>>) -> usize {
    recorded
        .lock()
        .unwrap()
        .iter()
        .filter(|event| match event {
            UserEvent::Dispatch(events) => events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::ExecutionEnvironments { .. })),
            _ => false,
        })
        .count()
}

#[test]
fn frontend_ready_returns_while_daemon_holds_pm_and_registry_locks() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, recorded, queued) = hydration_runtime(temp.path());
    let context = runtime.test_context();
    let scope = ClientScope::Project(context.project_key.clone());
    let locks = hold_daemon_locks(&context.project_root);

    let started = std::time::Instant::now();
    let events = runtime.handle_frontend_event_in_scope(
        "client-a".into(),
        FrontendEvent::FrontendReady,
        &scope,
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(100),
        "FrontendReady waited {elapsed:?} on locks another process holds"
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event.event, BackendEvent::PmStatus { .. })),
        "the PM status is read off the event loop: {events:?}"
    );

    // AC-2: resends while that read runs join it instead of queueing more.
    let queued_after_first = queued.lock().unwrap().len();
    for _ in 0..50 {
        runtime.handle_frontend_event_in_scope(
            "client-a".into(),
            FrontendEvent::FrontendReady,
            &scope,
        );
    }
    drop(locks);
    drain_queued_blocking_tasks(&queued);
    assert_eq!(
        pm_status_replies(&recorded),
        2,
        "one read plus one rerun covers every resend (first ready queued {queued_after_first} tasks)"
    );
}

#[test]
fn get_execution_environments_returns_while_daemon_holds_locks_and_coalesces_resends() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (mut runtime, recorded, queued) = hydration_runtime(temp.path());
    let context = runtime.test_context();
    let scope = ClientScope::Project(context.project_key.clone());
    let locks = hold_daemon_locks(&context.project_root);

    let started = std::time::Instant::now();
    let events = runtime.handle_frontend_event_in_scope(
        "client-a".into(),
        FrontendEvent::GetExecutionEnvironments,
        &scope,
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(100),
        "GetExecutionEnvironments waited {elapsed:?} on the event loop"
    );
    assert!(
        events.is_empty(),
        "the view is built off the event loop: {events:?}"
    );
    assert_eq!(queued.lock().unwrap().len(), 1);

    for _ in 0..50 {
        runtime.handle_frontend_event_in_scope(
            "client-a".into(),
            FrontendEvent::GetExecutionEnvironments,
            &scope,
        );
    }
    assert_eq!(
        queued.lock().unwrap().len(),
        1,
        "resends join the read in flight instead of queueing workers"
    );
    drop(locks);
    drain_queued_blocking_tasks(&queued);
    assert_eq!(execution_environment_replies(&recorded), 2);

    // A request after the read finished starts a fresh one.
    runtime.handle_frontend_event_in_scope(
        "client-a".into(),
        FrontendEvent::GetExecutionEnvironments,
        &scope,
    );
    assert_eq!(queued.lock().unwrap().len(), 1);
}
