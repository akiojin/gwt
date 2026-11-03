#[test]
fn project_worker_completion_rejects_reopened_generation() {
    let temp = tempfile::tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let project_root = temp.path().join("project");
    std::fs::create_dir_all(&project_root).unwrap();
    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab-a",
            "A",
            project_root.clone(),
            ProjectKind::Git,
            &[],
        )],
        Some("tab-a"),
    );
    let context = runtime.project_context("tab-a").unwrap();
    let (proxy, queued) = AppEventProxy::stub();
    let worker = proxy.for_project(context.clone());
    worker.send(UserEvent::WorkTipSubjects {
        project_root: project_root.clone(),
        tip_subjects: HashMap::new(),
    });
    let current = queued.lock().unwrap().pop().unwrap();
    assert!(matches!(
        runtime.accept_project_completion(current),
        Some(UserEvent::WorkTipSubjects { .. })
    ));

    // Reopening the same root and tab allocates a fresh incarnation.
    runtime
        .project_tab_incarnations
        .get_mut("tab-a")
        .unwrap()
        .generation += 1;
    runtime.refresh_project_state("tab-a");
    worker.send(UserEvent::WorkTipSubjects {
        project_root,
        tip_subjects: HashMap::from([("feature/stale".to_string(), "old result".to_string())]),
    });
    let stale = queued.lock().unwrap().pop().unwrap();
    assert!(
        runtime.accept_project_completion(stale).is_none(),
        "a worker from a closed project generation must not mutate the reopened project"
    );
}

#[test]
fn stale_project_launch_completion_cleans_exact_genesis_without_touching_reopened_pane() {
    use gwt::project_runtime::{events::OwnerCommand, test_support::OwnerLaunchStage};
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    seed_owner_fixture_roster(&repo, 8);
    configure_owner_fixture_tools(&repo, &["pane-contract-fixture"]);
    let executor = tokio::runtime::Runtime::new().unwrap();
    let owner = executor.block_on(production_owner_fixture(&repo));
    executor
        .block_on(owner.set_pty_start_gate(temp.path().join("missing-pty-gate"), Vec::new()))
        .unwrap();
    let authority = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let window_id = combined_window_id("tab-1", "agent-1");
    let context = runtime.project_context("tab-1").unwrap();
    let generation = runtime.window_lifecycle_generations.lock().unwrap()[&window_id];
    let checkpoint = executor
        .block_on(owner.pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall))
        .unwrap();
    runtime
        .queue_project_owner_launch(
            &context,
            window_id.clone(),
            Some(generation),
            Box::new(pane_control_tests::control_launch_config(&repo, authority.number)),
            Box::default(),
        )
        .unwrap();
    let captured = executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bounds the real Genesis materialization checkpoint, whose receipt establishes ordering.
        tokio::time::timeout(Duration::from_secs(30), checkpoint.wait())
            .await
            .expect("actual Genesis materialization")
    });
    let source = captured.session.as_ref().unwrap();
    assert!(captured.pty.is_none());
    assert!(runtime.sessions_dir.join(format!("{}.toml", source.id)).exists());
    assert!(durable_launch_recovery_exists(&runtime.sessions_dir, &source.id));
    // A missing executable is rejected before spawn on Windows. On other
    // platforms an exec attempt is ambiguous, so cancel at the existing
    // materialization barrier to establish a real pre-child rollback instead.
    #[cfg(not(windows))]
    captured.current.store(false, std::sync::atomic::Ordering::Release);
    checkpoint.release();
    let completion = take_remote_owner_launch_completion(match &runtime.proxy {
        AppEventProxy::Stub(events) => events,
        _ => unreachable!("fixture event proxy"),
    });
    let failure = match &completion.result {
        Ok(super::RemoteOwnerLaunchOutcome::CompensatedFailure(failure)) => failure,
        _ => panic!("actual owner must return confirmed Genesis compensation"),
    };
    assert_eq!(
        failure.removed_genesis_session,
        gwt_agent::SessionExecutionIdentity::from_session(source).unwrap().unwrap()
    );

    // The canonical handshake/cleanup races remain covered by
    // active_launch_handshake_and_terminal_successor_are_cross_process_exclusive
    // and startup_reaper_loses_prepared_handshake_and_session_replacement_races_cleanly.
    // This GUI completion carries only the old owner's exact operation proof.
    runtime.project_tab_incarnations.get_mut("tab-1").unwrap().generation += 1;
    runtime.refresh_project_state("tab-1");
    runtime.pending_launch_completions.remove(&window_id);
    let reopened = runtime.project_context("tab-1").unwrap();
    configure_owner_fixture_start_gate(&owner);
    let launched = executor
        .block_on(owner.command(OwnerCommand::LaunchAgent {
            id: captured.id.clone(),
            operation_id: "reopened-project-agent".into(),
            tab_id: reopened.project_key.to_string(),
            config: Box::new(pane_control_tests::control_launch_config(&repo, 2360)),
            metadata: Box::default(),
            cols: 80,
            rows: 24,
        }))
        .unwrap();
    let replacement = executor
        .block_on(owner.probe(&captured.id, launched["incarnation"].as_u64().unwrap()))
        .unwrap();
    let verified = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    let replacement_session = replacement.session.as_ref().unwrap();
    let replacement_pid = replacement.pty.process_id().unwrap();
    assert_ne!(replacement_session.id, source.id);
    assert_ne!(replacement.incarnation, captured.incarnation);
    assert!(replacement.pty.try_wait().unwrap().is_none());
    let snapshot = executor.block_on(owner.command(OwnerCommand::Snapshot {
        id: replacement.id.clone(),
        incarnation: replacement.incarnation,
    })).unwrap();
    runtime.install_remote_owner_projection(
        &reopened,
        &window_id,
        owner.client(),
        serde_json::from_value(snapshot).unwrap(),
        Some(replacement_session.clone()),
    );
    runtime.window_details.insert(window_id.clone(), "new pane state".into());
    let replacement_path = runtime.sessions_dir.join(format!("{}.toml", replacement_session.id));
    let replacement_bytes = fs::read(&replacement_path).unwrap();
    let ledger_before =
        gwt::cli::execution_state::load_generation_ledger(&source.worktree_path, authority)
            .unwrap().unwrap();
    let (proxy, queued) = AppEventProxy::stub();
    let worker = proxy.for_project(context);
    let cancel_ack = completion.cancel_ack.clone();
    worker.send(UserEvent::RemoteOwnerLaunchComplete(Box::new(completion)));
    let (spawner, cleanup_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let completion = queued.lock().unwrap().pop().unwrap();
    let retry = completion.clone();
    assert!(runtime.accept_project_completion(completion).is_none());
    assert_eq!(
        cleanup_tasks.lock().unwrap().len(),
        1,
        "stale owner completion must enqueue cleanup instead of doing ledger I/O on the GUI"
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(
            &source.worktree_path, authority,
        ).unwrap().unwrap(),
        ledger_before,
        "discarding a GUI completion must leave canonical authority unchanged"
    );
    assert!(executor.block_on(async { cancel_ack.lock().await.acknowledgement.is_none() }));
    drain_queued_blocking_tasks(&cleanup_tasks);
    let acknowledgement = executor.block_on(async { cancel_ack.lock().await.acknowledgement.clone() });
    assert_eq!(acknowledgement, Some(Ok(())), "exact old-operation cleanup ACK");
    assert!(runtime.accept_project_completion(retry).is_none());
    drain_queued_blocking_tasks(&cleanup_tasks);
    assert_eq!(
        runtime.window_details.get(&window_id).map(String::as_str),
        Some("new pane state")
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&source.worktree_path, authority)
        .unwrap()
        .unwrap();
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
        "discarded launch must release its exact genesis authority"
    );
    assert_eq!(ledger, ledger_before);
    assert!(!runtime
        .sessions_dir
        .join(format!("{}.toml", source.id))
        .exists());
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        &source.id
    ));
    assert_eq!(fs::read(replacement_path).unwrap(), replacement_bytes);
    let current = executor.block_on(owner.probe(&replacement.id, replacement.incarnation)).unwrap();
    assert_eq!(current.session.as_ref().unwrap().id, replacement_session.id);
    assert_eq!(current.pty.process_id(), Some(replacement_pid));
    assert!(current.pty.try_wait().unwrap().is_none());
    assert_eq!(runtime.runtimes[&window_id].incarnation, replacement.incarnation);
    assert_eq!(runtime.runtimes[&window_id].child_pid, Some(replacement_pid));
    }));
    let closed = executor.block_on(owner.command(OwnerCommand::Close {
        id: replacement.id.clone(),
        incarnation: replacement.incarnation,
    }));
    let reaped = replacement.pty.try_wait();
    drop(runtime);
    executor.block_on(owner.shutdown());
    closed.expect("close the exact replacement after all stale completion checks");
    assert!(reaped.unwrap().is_some());
    verified.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}
