#[test]
fn project_worker_completion_rejects_reopened_generation() {
    let temp = tempfile::tempdir().unwrap();
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
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-pty-failure";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize genesis execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize genesis ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read genesis binding")
        .expect("genesis binding");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(repo.clone());
    session.linked_issue_number = Some(owner.number);
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    session
        .set_execution_binding(Some(binding.clone()))
        .expect("bind genesis Session");
    session
        .save(&runtime.sessions_dir)
        .expect("save genesis Session");
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::Genesis,
        session_id,
        &repo,
        &repo,
        owner,
        Some(&binding),
        Some(&gwt_agent::AgentId::Codex),
    )
    .expect("persist exact genesis recovery receipt");
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Running);

    let context = runtime.project_context("tab-1").unwrap();
    let (proxy, queued) = AppEventProxy::stub();
    let worker = proxy.for_project(context);
    worker.send(UserEvent::LaunchComplete {
        window_id: window_id.clone(),
        result: Box::new(Ok((
            ProcessLaunch {
                command: "/definitely/missing/gwt-agent".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(repo.clone()),
                pending_tool_runtime_migration: None,
                resource_policy: None,
            },
            session_id.to_string(),
            "work/issue-2359".to_string(),
            "Codex".to_string(),
            repo.clone(),
            gwt_agent::AgentId::Codex,
            Some(owner.number),
            Some("origin/develop".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            gwt_agent::SessionMode::Normal,
            false,
            super::launch::AgentLaunchRuntimeContext {
                agent_project_root: repo.display().to_string(),
                expected_execution_identity: gwt_agent::SessionExecutionIdentity::from_session(
                    &session,
                )
                .unwrap(),
                active_launch_handshake: None,
            },
        ))),
    });

    runtime
        .project_tab_incarnations
        .get_mut("tab-1")
        .unwrap()
        .generation += 1;
    runtime.refresh_project_state("tab-1");
    runtime
        .window_details
        .insert(window_id.clone(), "new pane state".to_string());
    let completion = queued.lock().unwrap().pop().unwrap();
    assert!(runtime.accept_project_completion(completion).is_none());
    assert_eq!(
        runtime.window_details.get(&window_id).map(String::as_str),
        Some("new pane state")
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .unwrap()
        .unwrap();
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
        "discarded launch must release its exact genesis authority"
    );
    assert!(!runtime
        .sessions_dir
        .join(format!("{session_id}.toml"))
        .exists());
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id
    ));
}
