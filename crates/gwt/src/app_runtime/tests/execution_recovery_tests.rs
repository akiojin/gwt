use super::*;

#[test]
fn fresh_execution_response_loss_recovery_rejects_same_id_session_replacement() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture =
        pending_fresh_execution_fixture(temp.path(), "fresh-response-loss-session-replacement");
    leave_fresh_execution_activated_before_projection_commit(&mut fixture);
    let workspace_snapshot = snapshot_optional_files(&[
        gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&fixture.repo),
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.repo),
    ]);
    let sessions_dir = fixture.runtime.sessions_dir.clone();
    let session_id = fixture.candidate_session_id.clone();
    set_fresh_execution_pre_work_commit_hook_for_test(Box::new(move || {
        replace_fresh_candidate_session_incarnation(&sessions_dir, &session_id);
    }));

    let removed = gwt::project_runtime::startup::reconcile_durable_launches(
        &fixture.repo,
        &fixture.runtime.sessions_dir,
    );

    assert!(
        removed.is_empty(),
        "a replaced Session must retain response-loss recovery for the exact owner"
    );
    assert_optional_files_unchanged(&workspace_snapshot);
    assert!(durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
}

#[test]
fn settled_fresh_execution_recovery_removes_only_receipt_without_republishing_authority() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-settled-no-write");
    leave_fresh_execution_activated_before_projection_commit(&mut fixture);
    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);
    assert!(!durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
    persist_durable_launch_recovery(
        &fixture.runtime.sessions_dir,
        DurableLaunchRecoveryKind::FreshSuccessor {
            operation_id: fixture.operation_id.clone(),
        },
        &fixture.candidate_session_id,
        &fixture.repo,
        &fixture.repo,
        fixture.owner,
        Some(&fixture.binding),
        Some(&gwt_agent::AgentId::Codex),
    )
    .expect("restore receipt after simulated unlink response loss");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &fixture.repo,
            &fixture.candidate_session_id,
            gwt::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "post-launch blocker after receipt unlink response loss".to_string(),
                missing_verification: Some("post-launch verification".to_string()),
            },
        )
        .expect("advance the activated generation lifecycle"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    assert_ne!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read lifecycle-advanced binding"),
        Some(fixture.binding.identity.clone()),
        "the fixture must prove receipt cleanup no longer depends on the activation-time ledger head",
    );

    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(&fixture.repo)
        .expect("trusted worktree directory");
    let owner_ledger = trusted_dir
        .parent()
        .expect("trusted repository directory")
        .join("execution-owners")
        .join(format!("owner-{}", fixture.owner.number))
        .join("generation-ledger.json");
    let artifact_paths = [
        owner_ledger,
        trusted_dir.join("execution-control.json"),
        trusted_dir.join("execution-generation-pointer.json"),
        fixture
            .repo
            .join(gwt::cli::execution_state::EXECUTION_CONTROL_STATE_RELATIVE),
        fixture
            .repo
            .join(gwt::cli::execution_state::EXECUTION_GENERATION_POINTER_STATE_RELATIVE),
    ];
    let before = artifact_paths
        .iter()
        .map(|path| fs::read(path).expect("read settled authority artifact"))
        .collect::<Vec<_>>();
    let probes = artifact_paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let probe = temp
                .path()
                .join(format!("authority-artifact-{index}.probe"));
            fs::hard_link(path, &probe).expect("link authority artifact identity probe");
            probe
        })
        .collect::<Vec<_>>();

    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);

    assert!(!durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
    for ((path, expected), probe) in artifact_paths.iter().zip(&before).zip(probes) {
        assert_eq!(
            fs::read(path).expect("read reconciled authority artifact"),
            *expected,
            "settled recovery must leave authority bytes unchanged: {}",
            path.display(),
        );
        let sentinel = b"\nsettled-recovery-same-file\n";
        OpenOptions::new()
            .append(true)
            .open(&probe)
            .expect("open authority identity probe")
            .write_all(sentinel)
            .expect("append authority identity probe");
        assert!(
            fs::read(path)
                .expect("read authority artifact after probe")
                .ends_with(sentinel),
            "settled recovery must not atomically replace authority artifact: {}",
            path.display(),
        );
    }
}

#[test]
fn historical_fresh_receipt_does_not_republish_over_a_different_current_owner() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-owner-superseded");
    leave_fresh_execution_activated_before_projection_commit(&mut fixture);
    let superseding_owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3327,
    };
    replace_current_generation_authority_with_owner(
        &fixture.repo,
        superseding_owner,
        "superseding-owner-session",
    );
    let artifacts = current_generation_authority_artifacts(&fixture.repo);
    let before = artifacts
        .iter()
        .map(|path| fs::read(path).expect("read superseding authority artifact"))
        .collect::<Vec<_>>();

    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);

    assert!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, superseding_owner)
            .expect("read superseding authority")
            .is_some()
    );
    for (path, expected) in artifacts.iter().zip(before) {
        assert_eq!(
            fs::read(path).expect("read authority after historical cleanup"),
            expected,
            "historical fresh cleanup must not replace the current owner's authority: {}",
            path.display()
        );
    }
    assert!(!durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
}

#[test]
fn historical_fresh_receipt_waits_for_a_foreign_owner_ledger_first_repair() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-owner-partial");
    leave_fresh_execution_activated_before_projection_commit(&mut fixture);
    let superseding_owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3327,
    };
    let superseding_session_id = "partial-superseding-owner-session";
    replace_current_generation_authority_with_owner(
        &fixture.repo,
        superseding_owner,
        superseding_session_id,
    );
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &fixture.repo,
            superseding_session_id,
            gwt::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "prepare a foreign-owner successor crash".to_string(),
                missing_verification: Some("foreign owner verification".to_string()),
            },
        )
        .expect("block superseding owner"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    let artifacts = current_generation_authority_artifacts(&fixture.repo);
    let blocked_authority = artifacts
        .iter()
        .map(|path| fs::read(path).expect("read blocked foreign authority"))
        .collect::<Vec<_>>();
    let foreign_successor = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "foreign-owner-ledger-first-successor".to_string(),
        principal_id: "gwt-host-launch".to_string(),
        work_id: None,
        source: gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
        session_binding_id: "foreign-owner-ledger-first-binding".to_string(),
        initial_session_id: "foreign-owner-ledger-first-session".to_string(),
        entrypoint: "$gwt-execute #3327".to_string(),
        requested_at: Utc::now(),
    };
    gwt::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
        &fixture.repo,
        superseding_owner,
        &foreign_successor,
    )
    .expect("prepare foreign owner successor");
    gwt::cli::execution_state::activate_successor(
        &fixture.repo,
        superseding_owner,
        &foreign_successor,
    )
    .expect("activate foreign owner successor");
    for (path, old_bytes) in artifacts.iter().zip(&blocked_authority) {
        fs::write(path, old_bytes).expect("restore pre-activation foreign projection/pointer");
    }

    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);

    assert!(durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
    for (path, expected) in artifacts.iter().zip(&blocked_authority) {
        assert_eq!(
            fs::read(path).expect("read partial foreign authority"),
            *expected,
            "old-owner recovery must retain and not overwrite a partial foreign authority: {}",
            path.display()
        );
    }

    gwt::cli::execution_state::activate_successor(
        &fixture.repo,
        superseding_owner,
        &foreign_successor,
    )
    .expect("repair foreign owner projection/pointer");
    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);
    assert!(!durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
    assert!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, superseding_owner)
            .expect("read repaired foreign owner")
            .is_some()
    );
}

#[test]
fn historical_genesis_receipt_does_not_terminalize_a_different_current_owner() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-owner-superseded");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let old_owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "superseded-genesis-session";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        old_owner.kind,
        old_owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize old genesis");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        old_owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize old genesis ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&repo, old_owner)
        .expect("read old genesis binding")
        .expect("old genesis binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: old_owner.kind.as_str().to_string(),
        owner_number: old_owner.number,
        identity,
        capability_generation: 1,
    };
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(repo.clone());
    session.linked_issue_number = Some(old_owner.number);
    session
        .set_execution_binding(Some(binding.clone()))
        .expect("bind old genesis Session");
    session
        .save(&runtime.sessions_dir)
        .expect("save old genesis Session");
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::Genesis,
        session_id,
        &repo,
        &repo,
        old_owner,
        Some(&binding),
        Some(&gwt_agent::AgentId::Codex),
    )
    .expect("persist old genesis receipt");
    let mut old_agent = sample_active_agent_session("tab-old-owner", "window-old-owner");
    old_agent.session_id = session_id.to_string();
    old_agent.branch_name = "work/issue-2359".to_string();
    old_agent.worktree_path = repo.clone();
    old_agent.agent_project_root = repo.display().to_string();
    let old_work_context = WorkspaceResumeContext {
        title: Some("Superseded genesis Work".to_string()),
        owner: Some("Issue #2359".to_string()),
        summary: Some("Must be discarded without touching the new owner".to_string()),
        next_action: Some("Recover exact old launch".to_string()),
    };
    save_workspace_launch_projection(
        &repo,
        &old_agent,
        Some("develop"),
        Some(old_owner.number),
        None,
        Some(&old_work_context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([session_id.to_string()])),
    )
    .expect("publish old genesis Work");
    let old_work_id = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read old WorkItems")
        .expect("old WorkItems")
        .work_items
        .into_iter()
        .find(|item| {
            item.events
                .iter()
                .any(|event| event.agent_session_id.as_deref() == Some(session_id))
        })
        .expect("old genesis Work")
        .id;
    let unrelated_work_id = "unrelated-work-after-owner-change";
    let mut unrelated = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        unrelated_work_id,
        Utc::now(),
    );
    unrelated.title = Some("Unrelated current owner Work".to_string());
    unrelated.agent_session_id = Some("superseding-genesis-owner-session".to_string());
    gwt_core::workspace_projection::record_workspace_work_event(&repo, unrelated)
        .expect("publish unrelated Work");
    let superseding_owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3327,
    };
    replace_current_generation_authority_with_owner(
        &repo,
        superseding_owner,
        "superseding-genesis-owner-session",
    );
    let artifacts = current_generation_authority_artifacts(&repo);
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            "superseding-genesis-owner-session",
            gwt::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "prepare partial foreign genesis-owner successor".to_string(),
                missing_verification: Some("foreign genesis verification".to_string()),
            },
        )
        .expect("block foreign genesis owner"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    let blocked_authority = artifacts
        .iter()
        .map(|path| fs::read(path).expect("read blocked foreign genesis authority"))
        .collect::<Vec<_>>();
    let foreign_successor = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "foreign-genesis-owner-ledger-first-successor".to_string(),
        principal_id: "gwt-host-launch".to_string(),
        work_id: None,
        source: gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
        session_binding_id: "foreign-genesis-owner-ledger-first-binding".to_string(),
        initial_session_id: "foreign-genesis-owner-ledger-first-session".to_string(),
        entrypoint: "$gwt-execute #3327".to_string(),
        requested_at: Utc::now(),
    };
    gwt::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
        &repo,
        superseding_owner,
        &foreign_successor,
    )
    .expect("prepare foreign genesis-owner successor");
    gwt::cli::execution_state::activate_successor(&repo, superseding_owner, &foreign_successor)
        .expect("activate foreign genesis-owner successor");
    for (path, old_bytes) in artifacts.iter().zip(&blocked_authority) {
        fs::write(path, old_bytes).expect("restore partial foreign genesis authority");
    }

    reconcile_fixture_launches(&mut runtime, &repo);

    assert!(durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id
    ));
    assert!(runtime
        .sessions_dir
        .join(format!("{session_id}.toml"))
        .exists());
    for (path, expected) in artifacts.iter().zip(&blocked_authority) {
        assert_eq!(
            fs::read(path).expect("read partial foreign genesis authority"),
            *expected,
            "old genesis recovery must not overwrite a partial foreign authority: {}",
            path.display()
        );
    }
    let pending_work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read WorkItems while foreign authority is partial")
        .expect("WorkItems while foreign authority is partial");
    assert!(
        pending_work_items
            .work_items
            .iter()
            .find(|item| item.id == old_work_id)
            .is_some_and(|item| !item.discarded),
        "cleanup must wait until the different owner's authority is recoverably valid"
    );

    gwt::cli::execution_state::activate_successor(&repo, superseding_owner, &foreign_successor)
        .expect("repair foreign genesis-owner projection/pointer");
    let before = artifacts
        .iter()
        .map(|path| fs::read(path).expect("read repaired superseding genesis artifact"))
        .collect::<Vec<_>>();
    reconcile_fixture_launches(&mut runtime, &repo);

    assert!(
        gwt::cli::execution_state::current_execution_binding(&repo, superseding_owner)
            .expect("read superseding genesis authority")
            .is_some()
    );
    for (path, expected) in artifacts.iter().zip(before) {
        assert_eq!(
            fs::read(path).expect("read authority after genesis cleanup"),
            expected,
            "historical genesis cleanup must not replace the current owner's authority: {}",
            path.display()
        );
    }
    assert!(!runtime
        .sessions_dir
        .join(format!("{session_id}.toml"))
        .exists());
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id
    ));
    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("read Workspace after historical genesis cleanup")
        .expect("Workspace after historical genesis cleanup");
    assert!(
        projection.latest_agent_for_session(session_id).is_none(),
        "the superseded genesis Agent must be removed from the current Workspace"
    );
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read WorkItems after historical genesis cleanup")
        .expect("WorkItems after historical genesis cleanup");
    assert!(
        work_items
            .work_items
            .iter()
            .find(|item| item.id == old_work_id)
            .is_some_and(|item| item.discarded),
        "the brand-new Work published only by the failed genesis must be discarded"
    );
    assert!(
        work_items
            .work_items
            .iter()
            .find(|item| item.id == unrelated_work_id)
            .is_some_and(|item| !item.discarded),
        "the different owner's Work must remain untouched"
    );
}

#[test]
fn startup_repairs_activated_fresh_execution_without_process_local_pending_state() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture =
        pending_fresh_execution_fixture(temp.path(), "fresh-activated-restart-repair");
    leave_fresh_execution_activated_before_projection_commit(&mut fixture);
    gwt_agent::update_session(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
        |session| {
            session.update_status(gwt_agent::AgentStatus::Stopped);
            session.restore_window_on_startup = false;
            Ok(())
        },
    )
    .expect("stop candidate before simulated Host restart");
    let runtime_root = temp.path().join(".gwt");
    drop(fixture.runtime);
    let tab = sample_project_tab(
        "tab-restarted",
        "Repo",
        fixture.repo.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut restarted = sample_runtime(&runtime_root, vec![tab], Some("tab-restarted"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    restarted.blocking_tasks = spawner;
    assert!(restarted.pending_fresh_execution_launches.is_empty());
    assert!(restarted.active_agent_sessions.is_empty());
    assert!(restarted.agent_capability_tokens.is_empty());

    reconcile_fixture_launches(&mut restarted, &fixture.repo);
    restarted.spawn_startup_generation_reaper(std::slice::from_ref(&fixture.repo));
    drain_queued_blocking_tasks(&tasks);

    let restart_binding =
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read restart-repaired current binding")
            .expect("restart-repaired current binding");
    assert_eq!(
        restart_binding.generation_id,
        fixture.binding.identity.generation_id
    );
    assert_eq!(
        restart_binding.binding_id,
        fixture.binding.identity.binding_id
    );
    assert_eq!(
        gwt_core::workspace_projection::resolve_workspace_state_external_commit(
            &fixture.repo,
            &fixture.operation_id,
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
        )
        .expect("read restart-committed Workspace transaction"),
        gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
    );
    let restart_ledger =
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.owner)
            .expect("read restart-repaired ledger")
            .expect("restart-repaired ledger");
    assert_eq!(
        restart_ledger.generations.len(),
        2,
        "restart repair must not append a duplicate generation",
    );
    assert_eq!(
        restart_ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
        "the W-37 reaper must terminalize the deliberately stopped recovered holder",
    );
    assert!(!durable_launch_recovery_exists(
        &restarted.sessions_dir,
        &fixture.candidate_session_id,
    ));
}

#[test]
fn startup_fresh_execution_recovery_rejects_same_id_session_replacement() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture =
        pending_fresh_execution_fixture(temp.path(), "fresh-startup-session-replacement");
    leave_fresh_execution_activated_before_projection_commit(&mut fixture);
    fixture.runtime.pending_fresh_execution_launches.clear();
    fixture.runtime.active_agent_sessions.clear();
    fixture.runtime.agent_capability_tokens.clear();
    let workspace_snapshot = snapshot_optional_files(&[
        gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&fixture.repo),
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&fixture.repo),
    ]);
    let sessions_dir = fixture.runtime.sessions_dir.clone();
    let session_id = fixture.candidate_session_id.clone();
    set_fresh_execution_pre_work_commit_hook_for_test(Box::new(move || {
        replace_fresh_candidate_session_incarnation(&sessions_dir, &session_id);
    }));

    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);

    assert_optional_files_unchanged(&workspace_snapshot);
    assert!(durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
}

#[test]
fn startup_fresh_recovery_ignores_unindexed_historical_sessions() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "fresh-unindexed-history");
    clear_durable_launch_recovery(&fixture.runtime.sessions_dir, &fixture.candidate_session_id)
        .expect("remove recovery receipt to model settled historical Session");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    assert!(candidate_path.exists());
    for index in 0..250 {
        fs::write(
            fixture
                .runtime
                .sessions_dir
                .join(format!("historical-{index}.toml")),
            "not = [valid",
        )
        .expect("write historical Session fixture");
    }

    let mut runtime = fixture.runtime;
    reconcile_fixture_launches(&mut runtime, &fixture.repo);

    assert!(
        candidate_path.exists(),
        "unindexed Session must not be recovered"
    );
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.owner,
            &fixture.operation_id,
        )
        .expect("read unindexed attempt")
        .expect("unindexed attempt")
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
        "startup recovery must be proportional to pending receipts, not Session history",
    );
}

#[cfg(unix)]
#[test]
fn durable_launch_recovery_persist_syncs_new_directory_and_receipt_entry() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "durable-receipt-persist");
    clear_durable_launch_recovery(&fixture.runtime.sessions_dir, &fixture.candidate_session_id)
        .expect("clear fixture receipt");
    let recovery_dir = fixture
        .runtime
        .sessions_dir
        .join("execution-launch-recovery");
    fs::remove_file(recovery_dir.join(format!("{}.lock", fixture.candidate_session_id)))
        .expect("remove durable receipt lock fixture");
    fs::remove_dir(&recovery_dir).expect("remove empty recovery directory");

    let synced_directories = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let observed = Arc::clone(&synced_directories);
    set_durable_launch_recovery_directory_sync_test_hook(Some(Box::new(move |directory| {
        observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(directory.to_path_buf());
        Ok(())
    })));
    let result = persist_durable_launch_recovery(
        &fixture.runtime.sessions_dir,
        DurableLaunchRecoveryKind::FreshSuccessor {
            operation_id: fixture.operation_id.clone(),
        },
        &fixture.candidate_session_id,
        &fixture.repo,
        &fixture.repo,
        fixture.owner,
        Some(&fixture.binding),
        Some(&gwt_agent::AgentId::Codex),
    );
    set_durable_launch_recovery_directory_sync_test_hook(None);

    result.expect("persist durably synchronized receipt");
    let synced_directories = synced_directories
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        synced_directories.contains(&fixture.runtime.sessions_dir),
        "creating the recovery directory must sync the sessions directory entry"
    );
    assert!(
        synced_directories.contains(
            &fixture
                .runtime
                .sessions_dir
                .join("execution-launch-recovery")
        ),
        "renaming the receipt into place must sync the recovery directory entry"
    );
}

#[cfg(unix)]
#[test]
fn durable_launch_recovery_clear_retries_directory_sync_after_unlink_response_loss() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "durable-receipt-clear");
    let sync_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = Arc::clone(&sync_attempts);
    set_durable_launch_recovery_directory_sync_test_hook(Some(Box::new(move |_directory| {
        let attempt = observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if attempt == 0 {
            return Err(std::io::Error::other(
                "simulated directory sync response loss",
            ));
        }
        Ok(())
    })));

    let first =
        clear_durable_launch_recovery(&fixture.runtime.sessions_dir, &fixture.candidate_session_id);
    let second =
        clear_durable_launch_recovery(&fixture.runtime.sessions_dir, &fixture.candidate_session_id);
    set_durable_launch_recovery_directory_sync_test_hook(None);

    assert!(first.is_err(), "the first directory sync must fail visibly");
    assert!(
        !durable_launch_recovery_exists(
            &fixture.runtime.sessions_dir,
            &fixture.candidate_session_id
        ),
        "the unlink completed before its durability acknowledgement was lost"
    );
    second.expect("NotFound retry must complete the pending directory sync barrier");
    assert_eq!(
        sync_attempts.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "NotFound retry must still synchronize the receipt directory"
    );
}

#[test]
fn startup_does_not_commit_foreign_same_operation_for_stale_activated_session() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-activated-foreign-root");
    leave_fresh_execution_activated_before_projection_commit(&mut fixture);

    let foreign = temp.path().join("foreign-project-state");
    fs::create_dir_all(&foreign).expect("create foreign repository");
    init_repo(&foreign);
    let foreign_transaction = gwt_core::workspace_projection::transact_workspace_state_with_commit(
        &foreign,
        &fixture.operation_id,
        |_projection, _work_items, _| Ok(((), Vec::new())),
        || {
            Err(gwt_core::error::GwtError::Other(
                "leave foreign same-operation marker Prepared".to_string(),
            ))
        },
    );
    assert!(foreign_transaction.is_err());
    let foreign_repo_hash = detect_repo_hash(&foreign)
        .expect("foreign repo hash")
        .to_string();
    gwt_agent::update_session(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
        |session| {
            session.project_state_root = Some(foreign.clone());
            session.repo_hash = Some(foreign_repo_hash.clone());
            session
                .execution_binding
                .as_mut()
                .expect("candidate execution binding")
                .repo_hash = foreign_repo_hash.clone();
            session.update_status(gwt_agent::AgentStatus::Stopped);
            session.restore_window_on_startup = false;
            Ok(())
        },
    )
    .expect("persist stale foreign Session identity");

    let runtime_root = temp.path().join(".gwt");
    drop(fixture.runtime);
    let tab = sample_project_tab(
        "tab-restarted",
        "Repo",
        fixture.repo.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut restarted = sample_runtime(&runtime_root, vec![tab], Some("tab-restarted"));

    restarted.bootstrap();

    assert!(durable_launch_recovery_exists(
        &restarted.sessions_dir,
        &fixture.candidate_session_id,
    ));

    let mut unrelated = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "work-after-foreign-activated-recovery",
        Utc::now(),
    );
    unrelated.title = Some("Foreign marker must remain unresolved".to_string());
    assert!(
        gwt_core::workspace_projection::record_workspace_work_event(&foreign, unrelated).is_err(),
        "startup must not commit a same-operation Workspace marker from a foreign Session root",
    );
}

#[test]
fn startup_does_not_delete_aborted_session_when_project_state_root_is_foreign() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "fresh-aborted-foreign-root");
    let prepared = gwt_core::workspace_projection::transact_workspace_state_with_commit(
        &fixture.repo,
        &fixture.operation_id,
        |_projection, _work_items, _| Ok(((), Vec::new())),
        || {
            gwt::cli::execution_state::abort_successor(
                &fixture.repo,
                fixture.owner,
                &fixture
                    .runtime
                    .pending_fresh_execution_launches
                    .get(&fixture.window_id)
                    .expect("pending fresh launch")
                    .request,
                "simulated crash after durable abort",
            )?;
            Err(gwt_core::error::GwtError::Other(
                "leave canonical Workspace marker Prepared".to_string(),
            ))
        },
    );
    assert!(prepared.is_err());

    let foreign = temp.path().join("foreign-aborted-project-state");
    fs::create_dir_all(&foreign).expect("create foreign repository");
    init_repo(&foreign);
    let foreign_repo_hash = detect_repo_hash(&foreign)
        .expect("foreign repo hash")
        .to_string();
    gwt_agent::update_session(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
        |session| {
            session.project_state_root = Some(foreign.clone());
            session.repo_hash = Some(foreign_repo_hash.clone());
            session
                .execution_binding
                .as_mut()
                .expect("candidate execution binding")
                .repo_hash = foreign_repo_hash.clone();
            Ok(())
        },
    )
    .expect("persist stale foreign Session identity");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));

    let runtime_root = temp.path().join(".gwt");
    drop(fixture.runtime);
    let tab = sample_project_tab(
        "tab-restarted",
        "Repo",
        fixture.repo.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut restarted = sample_runtime(&runtime_root, vec![tab], Some("tab-restarted"));

    restarted.bootstrap();

    assert!(
        candidate_path.exists(),
        "a foreign Missing result must not delete the sole Session recovery record",
    );
    assert!(durable_launch_recovery_exists(
        &restarted.sessions_dir,
        &fixture.candidate_session_id,
    ));
    let mut unrelated = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "work-after-foreign-aborted-recovery",
        Utc::now(),
    );
    unrelated.title = Some("Canonical marker must remain unresolved".to_string());
    assert!(
        gwt_core::workspace_projection::record_workspace_work_event(&fixture.repo, unrelated)
            .is_err(),
        "startup must retain the canonical Prepared marker when Session root identity is invalid",
    );
}

#[test]
fn aborted_fresh_execution_cleanup_rejects_agent_substitution_without_mutation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-cleanup-mismatch");
    gwt::cli::execution_state::abort_successor(
        &fixture.repo,
        fixture.owner,
        &fixture
            .runtime
            .pending_fresh_execution_launches
            .get(&fixture.window_id)
            .expect("pending fresh launch")
            .request,
        "simulated launch failure",
    )
    .expect("abort fresh candidate");
    gwt_agent::update_session(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
        |session| {
            session.agent_id = gwt_agent::AgentId::Custom("review-bot".to_string());
            Ok(())
        },
    )
    .expect("replace candidate Session Agent");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    let replacement_before = fs::read(&candidate_path).expect("read replacement Session");
    let authority_before = snapshot_optional_files(&exact_continue_authority_artifacts(
        &fixture.repo,
        fixture.owner,
    ));
    let work_before = tracked_workspace_work_store_snapshot(&fixture.repo);

    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);

    assert!(
        fixture
            .runtime
            .pending_fresh_execution_launches
            .contains_key(&fixture.window_id),
        "binding-mismatch cleanup must retain process-local retry evidence",
    );
    assert_eq!(
        fs::read(&candidate_path).expect("read retained replacement Session"),
        replacement_before,
        "strict cleanup must retain the replacement Session byte-identically",
    );
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&fixture.repo, &work_before);
    assert!(durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
}

#[test]
fn startup_aborted_cleanup_retains_same_binding_agent_with_replaced_branch() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture =
        pending_fresh_execution_fixture(temp.path(), "fresh-aborted-same-ids-replacement");
    gwt::cli::execution_state::abort_successor(
        &fixture.repo,
        fixture.owner,
        &fixture
            .runtime
            .pending_fresh_execution_launches
            .get(&fixture.window_id)
            .expect("pending fresh launch")
            .request,
        "simulated crash after durable abort",
    )
    .expect("abort fresh candidate");
    gwt_agent::update_session(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
        |session| {
            session.branch = "work/replaced-after-validation".to_string();
            Ok(())
        },
    )
    .expect("persist same-binding and same-Agent replacement Session branch");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    let replacement_before = fs::read(&candidate_path).expect("read replacement Session");
    let authority_before = snapshot_optional_files(&exact_continue_authority_artifacts(
        &fixture.repo,
        fixture.owner,
    ));
    let work_before = tracked_workspace_work_store_snapshot(&fixture.repo);
    let mut restarted = fixture.runtime;

    reconcile_fixture_launches(&mut restarted, &fixture.repo);

    assert!(
        candidate_path.exists(),
        "recovery must not delete a replacement Session using a binding read from that replacement",
    );
    assert!(durable_launch_recovery_exists(
        &restarted.sessions_dir,
        &fixture.candidate_session_id,
    ));
    assert_eq!(
        fs::read(&candidate_path).expect("read retained replacement Session"),
        replacement_before,
    );
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&fixture.repo, &work_before);
}

#[cfg(unix)]
#[test]
fn startup_retains_dangling_fresh_session_without_mutation() {
    use std::os::unix::fs::symlink;

    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-dangling-startup");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    fs::remove_file(&candidate_path).expect("remove materialized candidate Session");
    let missing_target = fixture
        .runtime
        .sessions_dir
        .join("missing-startup-candidate");
    symlink(&missing_target, &candidate_path).expect("create dangling startup Session");
    let receipt_path = fixture
        .runtime
        .sessions_dir
        .join("execution-launch-recovery")
        .join(format!("{}.json", fixture.candidate_session_id));
    let receipt_before = fs::read(&receipt_path).expect("read recovery receipt");
    let authority_before = snapshot_optional_files(&exact_continue_authority_artifacts(
        &fixture.repo,
        fixture.owner,
    ));
    let work_before = tracked_workspace_work_store_snapshot(&fixture.repo);

    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);

    assert!(fs::symlink_metadata(&candidate_path)
        .expect("dangling startup Session must remain")
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&candidate_path).unwrap(), missing_target);
    assert_eq!(fs::read(&receipt_path).unwrap(), receipt_before);
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&fixture.repo, &work_before);
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.owner,
            &fixture.operation_id,
        )
        .expect("read retained attempt")
        .expect("retained attempt")
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
    );
}

#[test]
fn startup_fresh_missing_cleanup_rechecks_same_id_materialization_before_mutation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-missing-race");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    let original_session = gwt_agent::Session::load(&candidate_path).expect("original Session");
    fs::remove_file(&candidate_path).expect("observe missing startup candidate Session");
    let authority_before = snapshot_optional_files(&exact_continue_authority_artifacts(
        &fixture.repo,
        fixture.owner,
    ));
    let work_before = tracked_workspace_work_store_snapshot(&fixture.repo);
    let sessions_dir = fixture.runtime.sessions_dir.clone();
    let repo = fixture.repo.clone();
    let owner = fixture.owner;
    let expected_session_id = fixture.candidate_session_id.clone();
    set_missing_session_cleanup_hook_for_test(Box::new(move |session_id| {
        assert_eq!(session_id, expected_session_id);
        original_session
            .save(&sessions_dir)
            .expect("materialize exact Session after startup Missing observation");
        gwt::cli::execution_state::claim_prepared_session_launch(
            &repo,
            owner,
            &sessions_dir,
            &original_session,
        )
        .expect("claim materialized launch")
        .expect("live Host launch handshake");
    }));

    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);

    assert!(
        candidate_path.exists(),
        "exact live Session must be retained"
    );
    assert!(durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&fixture.repo, &work_before);
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.owner,
            &fixture.operation_id,
        )
        .expect("read retained attempt")
        .expect("retained attempt")
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
    );
}

#[test]
fn startup_retains_fresh_recovery_receipt_with_blank_operation_id() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "fresh-blank-receipt-op");
    let receipt_path = fixture
        .runtime
        .sessions_dir
        .join("execution-launch-recovery")
        .join(format!("{}.json", fixture.candidate_session_id));
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_path).expect("read launch recovery receipt"))
            .expect("parse launch recovery receipt");
    receipt["kind"]["fresh_successor"]["operation_id"] =
        serde_json::Value::String("   ".to_string());
    fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).expect("serialize malformed receipt"),
    )
    .expect("persist malformed receipt");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    let mut restarted = fixture.runtime;

    reconcile_fixture_launches(&mut restarted, &fixture.repo);

    assert!(
        receipt_path.exists(),
        "invalid receipt must be retained for explicit repair instead of being mistaken for settled state",
    );
    assert!(candidate_path.exists());
}

#[test]
fn startup_retains_schema_v2_recovery_receipt_without_full_session_identity() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-schema-v2-receipt");
    let receipt_path = fixture
        .runtime
        .sessions_dir
        .join("execution-launch-recovery")
        .join(format!("{}.json", fixture.candidate_session_id));
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_path).expect("read recovery receipt"))
            .expect("parse recovery receipt");
    receipt["schema_version"] = serde_json::Value::from(2);
    receipt
        .as_object_mut()
        .expect("receipt object")
        .remove("expected_session_identity");
    let receipt_before = serde_json::to_vec_pretty(&receipt).expect("serialize v2 receipt");
    fs::write(&receipt_path, &receipt_before).expect("persist v2 receipt");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    let candidate_before = fs::read(&candidate_path).expect("read candidate Session");
    let authority_before = snapshot_optional_files(&exact_continue_authority_artifacts(
        &fixture.repo,
        fixture.owner,
    ));
    let work_before = tracked_workspace_work_store_snapshot(&fixture.repo);

    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);

    assert_eq!(fs::read(&receipt_path).unwrap(), receipt_before);
    assert_eq!(fs::read(&candidate_path).unwrap(), candidate_before);
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&fixture.repo, &work_before);
}

#[test]
fn startup_retains_bound_fresh_receipt_when_owner_ledger_is_missing() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-missing-ledger");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    fs::remove_file(&candidate_path).expect("remove candidate Session");
    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(&fixture.repo)
        .expect("trusted worktree directory");
    let ledger_path = trusted_dir
        .parent()
        .expect("trusted repository directory")
        .join("execution-owners")
        .join(format!("owner-{}", fixture.owner.number))
        .join("generation-ledger.json");
    fs::remove_file(&ledger_path).expect("remove owner ledger");

    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);

    assert!(durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
}

#[test]
fn startup_retains_missing_session_fresh_receipt_after_binding_identity_mismatch() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture =
        pending_fresh_execution_fixture(temp.path(), "fresh-receipt-binding-mismatch");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    fs::remove_file(&candidate_path).expect("remove candidate Session");
    let receipt_path = fixture
        .runtime
        .sessions_dir
        .join("execution-launch-recovery")
        .join(format!("{}.json", fixture.candidate_session_id));
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_path).expect("read recovery receipt"))
            .expect("parse recovery receipt");
    receipt["expected_binding"]["identity"]["ledger_head_hash"] =
        serde_json::Value::String("mismatched-ledger-head".to_string());
    fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).expect("serialize mismatched recovery receipt"),
    )
    .expect("persist mismatched recovery receipt");

    reconcile_fixture_launches(&mut fixture.runtime, &fixture.repo);

    assert!(receipt_path.exists());
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.owner,
            &fixture.operation_id,
        )
        .expect("read retained attempt")
        .expect("retained attempt")
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
        "a mismatched recovery binding must not authorize aborting the candidate",
    );
}

#[test]
fn startup_clears_unbound_fresh_receipt_only_when_no_attempt_exists() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-unbound-fresh-no-attempt");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::FreshSuccessor {
            operation_id: "unbound-fresh-no-attempt".to_string(),
        },
        "unbound-fresh-session",
        &repo,
        &repo,
        owner,
        None,
        None,
    )
    .expect("persist unbound pre-prepare receipt");

    reconcile_fixture_launches(&mut runtime, &repo);

    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        "unbound-fresh-session",
    ));
}

#[test]
fn startup_retains_bound_genesis_receipt_when_all_execution_authority_is_missing() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-bound-genesis-missing-authority");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "bound-genesis-missing-authority";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize genesis authority");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize genesis ledger");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: gwt::cli::execution_state::current_execution_binding(&repo, owner)
            .expect("read genesis binding")
            .expect("genesis binding"),
        capability_generation: 1,
    };
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
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
    .expect("persist bound genesis receipt");
    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(&repo)
        .expect("trusted worktree directory");
    fs::remove_file(
        trusted_dir
            .parent()
            .expect("trusted repository directory")
            .join("execution-owners")
            .join(format!("owner-{}", owner.number))
            .join("generation-ledger.json"),
    )
    .expect("remove owner ledger");
    for path in current_generation_authority_artifacts(&repo) {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("remove execution authority artifact: {error}"),
        }
    }

    reconcile_fixture_launches(&mut runtime, &repo);

    assert!(durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
}

#[test]
fn startup_retains_bound_genesis_receipt_when_ledger_is_missing_but_flat_projection_remains() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-bound-genesis-flat-only");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "bound-genesis-flat-only";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize genesis authority");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize genesis ledger");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: gwt::cli::execution_state::current_execution_binding(&repo, owner)
            .expect("read genesis binding")
            .expect("genesis binding"),
        capability_generation: 1,
    };
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
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
    .expect("persist bound genesis receipt");
    let authority_paths = current_generation_authority_artifacts(&repo);
    let authority_before = authority_paths
        .iter()
        .map(|path| fs::read(path).expect("read flat authority artifact"))
        .collect::<Vec<_>>();
    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(&repo)
        .expect("trusted worktree directory");
    let ledger_path = trusted_dir
        .parent()
        .expect("trusted repository directory")
        .join("execution-owners")
        .join(format!("owner-{}", owner.number))
        .join("generation-ledger.json");
    fs::remove_file(&ledger_path).expect("remove owner ledger");

    reconcile_fixture_launches(&mut runtime, &repo);

    assert!(durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
    assert!(
        !ledger_path.exists(),
        "bound recovery must not recreate the ledger"
    );
    for (path, expected) in authority_paths.iter().zip(authority_before) {
        assert_eq!(
            fs::read(path).expect("read retained flat authority artifact"),
            expected,
            "bound recovery must not rewrite flat authority: {}",
            path.display(),
        );
    }
}

#[test]
fn startup_retains_blocked_genesis_receipt_after_full_binding_head_mismatch() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-binding-head-mismatch");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-binding-head-mismatch";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize genesis authority");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize genesis ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read genesis binding")
        .expect("genesis binding");
    let mut receipt_binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: identity.clone(),
        capability_generation: 1,
    };
    receipt_binding.identity.ledger_head_hash = "mismatched-genesis-ledger-head".to_string();
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::Genesis,
        session_id,
        &repo,
        &repo,
        owner,
        Some(&receipt_binding),
        Some(&gwt_agent::AgentId::Codex),
    )
    .expect("persist mismatched bound genesis receipt");
    gwt::cli::execution_state::block_uncommitted_genesis_launch(
        &repo,
        owner,
        session_id,
        &identity,
        "simulated terminalization before Host restart",
    )
    .expect("terminalize exact genesis generation");
    let authority_paths = current_generation_authority_artifacts(&repo);
    let authority_before = authority_paths
        .iter()
        .map(|path| fs::read(path).expect("read blocked authority artifact"))
        .collect::<Vec<_>>();

    reconcile_fixture_launches(&mut runtime, &repo);

    assert!(durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
    for (path, expected) in authority_paths.iter().zip(authority_before) {
        assert_eq!(
            fs::read(path).expect("read retained blocked authority artifact"),
            expected,
            "binding mismatch must perform no authority repair: {}",
            path.display(),
        );
    }
}

#[test]
fn pending_launch_recovery_receipt_excludes_session_from_quick_start_cache_refresh() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-cache-exclusion");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    let persisted = gwt_agent::Session::load(&candidate_path).expect("load pending candidate");

    assert!(
        fixture
            .runtime
            .launch_wizard_cache
            .session_by_id(&fixture.candidate_session_id)
            .is_none(),
        "initial cache load must exclude Sessions with a pending launch-recovery receipt",
    );

    fixture
        .runtime
        .apply_refreshed_launch_wizard_sessions(vec![persisted]);

    assert!(
        fixture
            .runtime
            .launch_wizard_cache
            .session_by_id(&fixture.candidate_session_id)
            .is_none(),
        "off-thread cache refresh must not resurrect a pending failed launch",
    );
}

#[test]
fn startup_retries_aborted_fresh_execution_candidate_cleanup() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "fresh-aborted-restart-cleanup");
    gwt::cli::execution_state::abort_successor(
        &fixture.repo,
        fixture.owner,
        &fixture
            .runtime
            .pending_fresh_execution_launches
            .get(&fixture.window_id)
            .expect("pending fresh launch")
            .request,
        "simulated crash after durable abort",
    )
    .expect("abort fresh candidate");
    let runtime_root = temp.path().join(".gwt");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    assert!(candidate_path.exists());
    drop(fixture.runtime);
    let tab = sample_project_tab(
        "tab-restarted",
        "Repo",
        fixture.repo.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut restarted = sample_runtime(&runtime_root, vec![tab], Some("tab-restarted"));

    let foreign_repo = temp.path().join("foreign-project");
    fs::create_dir_all(&foreign_repo).expect("foreign project");
    init_repo_with_initial_commit(&foreign_repo);
    let foreign = pending_fresh_execution_fixture_in_worktree(
        temp.path(),
        "foreign-pending-startup",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        foreign_repo.clone(),
        2359,
    );
    let foreign_path = foreign
        .runtime
        .sessions_dir
        .join(format!("{}.toml", foreign.candidate_session_id));
    let foreign_before = fs::read(&foreign_path).expect("foreign candidate bytes");
    let foreign_authority =
        snapshot_optional_files(&current_generation_authority_artifacts(&foreign_repo));
    let executor = tokio::runtime::Runtime::new().expect("owner executor");
    let owner = executor.block_on(production_owner_fixture(&fixture.repo));
    restarted.bootstrap();

    assert!(
        !candidate_path.exists(),
        "startup must finish exact cleanup recorded by the Aborted fresh attempt",
    );
    assert!(!durable_launch_recovery_exists(
        &restarted.sessions_dir,
        &fixture.candidate_session_id,
    ));
    assert!(
        restarted
            .launch_wizard_cache
            .quick_start_entries(&fixture.repo, "work/issue-2359")
            .iter()
            .all(|entry| entry.session_id != fixture.candidate_session_id),
        "startup cleanup must evict the aborted candidate from the in-memory Quick Start cache",
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read preserved predecessor"),
        Some(fixture.predecessor_binding),
    );
    assert_eq!(
        fs::read(&foreign_path).expect("foreign Session retained"),
        foreign_before
    );
    assert!(durable_launch_recovery_exists(
        &foreign.runtime.sessions_dir,
        &foreign.candidate_session_id
    ));
    assert_optional_files_unchanged(&foreign_authority);
    executor.block_on(owner.shutdown());
}

#[test]
fn startup_retains_aborted_cleanup_after_io_failure_and_retries_successfully() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "fresh-aborted-io-retry");
    gwt::cli::execution_state::abort_successor(
        &fixture.repo,
        fixture.owner,
        &fixture
            .runtime
            .pending_fresh_execution_launches
            .get(&fixture.window_id)
            .expect("pending fresh launch")
            .request,
        "simulated crash after durable abort",
    )
    .expect("abort fresh candidate");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    let removal_blocker = fixture
        .runtime
        .sessions_dir
        .join("runtime")
        .join("424242")
        .join(format!("{}.json", fixture.candidate_session_id));
    fs::create_dir_all(&removal_blocker)
        .expect("create directory-shaped runtime sidecar removal blocker");
    let runtime_root = temp.path().join(".gwt");
    drop(fixture.runtime);
    let tab = sample_project_tab(
        "tab-restarted",
        "Repo",
        fixture.repo.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut restarted = sample_runtime(&runtime_root, vec![tab], Some("tab-restarted"));

    reconcile_fixture_launches(&mut restarted, &fixture.repo);

    assert!(
        candidate_path.exists(),
        "cleanup I/O failure must retain the Session as durable retry evidence",
    );
    assert!(durable_launch_recovery_exists(
        &restarted.sessions_dir,
        &fixture.candidate_session_id,
    ));
    fs::remove_dir(&removal_blocker).expect("clear cleanup I/O blocker");

    reconcile_fixture_launches(&mut restarted, &fixture.repo);

    assert!(
        !candidate_path.exists(),
        "a later startup reconciliation must finish cleanup after the I/O blocker clears",
    );
    assert!(!durable_launch_recovery_exists(
        &restarted.sessions_dir,
        &fixture.candidate_session_id,
    ));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read preserved predecessor after retry"),
        Some(fixture.predecessor_binding),
    );
}

#[test]
fn fresh_execution_launch_completion_recovers_prepared_receipt_and_defers_projection_until_ready() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = owner_fresh_launch_fixture(
        temp.path(),
        "fresh-recovered-physical-launch",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        false,
        None,
    );
    use gwt::project_runtime::events::OwnerCommand;
    let physical = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert_eq!(
        physical
            .session
            .as_ref()
            .unwrap()
            .execution_binding
            .as_ref(),
        Some(&fixture.pending.binding)
    );
    assert_eq!(
        fixture
            .captured
            .pending_fresh_execution
            .as_ref()
            .unwrap()
            .operation_id,
        fixture.pending.operation_id
    );
    assert_eq!(
        physical.readiness_watch.as_ref().unwrap().operation_id,
        fixture.pending.operation_id
    );
    assert!(physical
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("Waiting for authenticated SessionStart")));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.authority)
            .unwrap(),
        Some(fixture.predecessor_binding.clone()),
        "PTY spawn cannot activate the fresh generation"
    );
    assert!(load_tracked_work_events(&fixture.repo).is_empty());
    fixture
        .executor
        .block_on(fixture.owner.check_readiness_deadline(
            &fixture.captured.id,
            fixture.captured.incarnation,
            ContinueWorkReadinessWatch::new(fixture.pending.operation_id.clone()),
        ))
        .unwrap();
    let extended = fixture
        .executor
        .block_on(
            fixture
                .owner
                .probe(&fixture.captured.id, fixture.captured.incarnation),
        )
        .unwrap();
    assert!(extended.readiness_watch.as_ref().unwrap().extensions > 0);
    let handed_off = mature_owner_readiness_handoff(
        &fixture.owner,
        &fixture.captured.id,
        fixture.captured.incarnation,
        &fixture.pending.operation_id,
    );
    assert!(handed_off.readiness_watch.as_ref().unwrap().handed_off);
    assert!(Arc::ptr_eq(&handed_off.pty, &physical.pty));
    assert!(
        handed_off.pty.try_wait().unwrap().is_none(),
        "live exact launch survives terminal handoff"
    );
    gwt::session_finalizer::kill_and_reap(&physical.pty, &fixture.captured.id).unwrap();
    assert!(matches!(
        continue_work_readiness_decision(
            handed_off.readiness_watch.as_ref().unwrap(),
            ReadinessPaneEvidence::Dead,
            0
        ),
        ReadinessDeadlineDecision::Abort { .. }
    ));
    fixture.executor.block_on(async {
        let _ = fixture
            .owner
            .check_readiness_deadline(
                &fixture.captured.id,
                fixture.captured.incarnation,
                ContinueWorkReadinessWatch {
                    handed_off: true,
                    ..readiness_watch_at_last_extension(&fixture.pending.operation_id, 0)
                },
            )
            .await;
        // A dead candidate can retain its recovery diagnostic. The explicit
        // exact Close owns retirement and acknowledges its physical cleanup.
        fixture
            .owner
            .command(OwnerCommand::Close {
                id: fixture.captured.id.clone(),
                incarnation: fixture.captured.incarnation,
            })
            .await
            .expect("exact close ACK after terminal handoff");
        assert!(
            fixture
                .owner
                .check_readiness_deadline(
                    &fixture.captured.id,
                    fixture.captured.incarnation,
                    ContinueWorkReadinessWatch {
                        handed_off: true,
                        ..readiness_watch_at_last_extension(&fixture.pending.operation_id, 0)
                    }
                )
                .await
                .is_err(),
            "an old deadline cannot revive a candidate after exact physical cleanup"
        );
        fixture.owner.command(OwnerCommand::Health).await.unwrap();
    });
    assert_owner_fresh_launch_rolled_back(&fixture);
    fixture.shutdown();
}

#[test]
fn continue_work_pre_dispatch_abort_surfaces_durable_write_failure() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    init_repo(temp.path());
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "continue-op-abort-write-failure".to_string(),
        principal_id: "gwt-host-continuation".to_string(),
        work_id: Some("work-selected".to_string()),
        source: "continue-work:resume".to_string(),
        session_binding_id: "binding-candidate".to_string(),
        initial_session_id: "candidate-session".to_string(),
        entrypoint: "gwt-execute".to_string(),
        requested_at: Utc::now(),
    };
    let execution = PendingContinueWorkExecution::Successor(request);

    let error = super::super::continuation::abort_prepared_execution(
        temp.path(),
        owner,
        &execution,
        "prepared launch failed",
    )
    .expect_err("missing durable ledger must not be treated as a successful abort");

    assert!(
        error.to_string().contains("ledger") || error.to_string().contains("attempt"),
        "abort failure must retain an actionable durable-state reason: {error}",
    );
}

#[test]
fn continue_work_authenticated_session_start_commits_stale_takeover_without_new_generation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let project_root = temp.path().join("workspace");
    let (bare_repo, _develop_worktree) =
        init_managed_workspace_with_develop_worktree(&project_root);
    // The production owner canonicalizes its project root. Capture the same
    // path identity before constructing the exact legacy Prepared receipt.
    let project_root = project_root.canonicalize().unwrap();
    let repo = project_root.join("work").join("issue-2359");
    fs::create_dir_all(repo.parent().expect("worktree parent")).expect("create worktree parent");
    let output = gwt_core::process::hidden_command("git")
        .args(["worktree", "add", "-q", "-b", "work/issue-2359"])
        .arg(&repo)
        .current_dir(&bare_repo)
        .output()
        .expect("add issue worktree");
    assert!(
        output.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let predecessor_session_id = "stale-owner-session";
    let operation_id = "continue-op-takeover";
    let work_id = "work-active";
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        predecessor_session_id,
        "gwt-execute",
        false,
    )
    .expect("materialize active predecessor");
    let imported = gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("import active predecessor");
    let generation_id = imported.current_generation_id;
    let predecessor_binding = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read predecessor binding")
        .expect("predecessor binding");
    let now = chrono::Utc::now();
    let mut start = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        work_id,
        now,
    );
    start.title = Some("Active stale Work".to_string());
    start.owner = Some("Issue #2359".to_string());
    start.agent_session_id = Some(predecessor_session_id.to_string());
    start.agent_id = Some("Codex".to_string());
    start.display_name = Some("Codex".to_string());
    start.execution_container = Some(
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some("work/issue-2359".to_string()),
            worktree_path: Some(repo.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        },
    );
    let mut work_items = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
    assert_eq!(
        work_items.apply_event(start.clone()),
        gwt_core::workspace_projection::WorkEventApplyOutcome::Applied
    );
    let work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
    gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
        &work_items_path,
        &work_items,
    )
    .expect("record repo-global active Work");
    gwt_core::workspace_projection::append_workspace_work_event_to_path(
        &gwt_core::paths::gwt_repo_local_work_events_path(&repo),
        &start,
    )
    .expect("record worktree-local active Work event");
    let mut current =
        gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&project_root);
    current.id = work_id.to_string();
    current.title = "Active stale Work".to_string();
    current.owner = Some("Issue #2359".to_string());
    gwt_core::workspace_projection::save_workspace_projection(&project_root, &current)
        .expect("seed canonical project current projection");

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        project_root.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let runtime_root = temp.path().join(".gwt");
    let runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
    pin_runtime_agents(&runtime, temp.path(), &[]);
    fs::copy(
        runtime.profile_config_path.as_ref().expect("profile"),
        gwt_core::paths::gwt_config_path(),
    )
    .expect("canonical owner profile");
    let runner = temp
        .path()
        .join("fixture-runner-bin")
        .join(if cfg!(windows) { "codex.cmd" } else { "codex" });
    #[cfg(not(windows))]
    fs::write(&runner, "#!/bin/sh\ncase \"$*\" in *--version*) printf '1.2.3\\n'; exit 0;; *'features list'*) exit 0;; esac\nexec /bin/cat\n").expect("live provider");
    let codex_home = temp.path().join("codex-home");
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", &codex_home);
    persist_owner_continue_conversation(&repo, &codex_home, predecessor_session_id, &project_root);
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::{OwnerCommand,OwnerEvent};
        let host = Arc::new(production_owner_fixture(&project_root).await);
        // The original regression exercises an already Prepared legacy takeover,
        // while the current ContinueWork coordinator prepares a Successor.
        let candidate_session_id = "takeover-candidate-session";
        let request = gwt::cli::execution_state::GenerationTakeoverRequest {
            operation_id: operation_id.into(),
            principal_id: "gwt-host-continuation".into(),
            work_id: Some(work_id.into()),
            source: Some("continue-work:resume".into()),
            from_session_id: predecessor_session_id.into(),
            to_session_id: candidate_session_id.into(),
            reason: "continue-work-stale-takeover: all owning Host runtimes are dead".into(),
            requested_at: now,
        };
        gwt::cli::execution_state::prepare_generation_takeover(&repo, owner, &request)
            .expect("prepare the original legacy takeover");
        let planned_identity = gwt::cli::execution_state::prepared_generation_takeover_execution_binding(&repo, owner, &request)
            .expect("canonical Prepared takeover binding");
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: candidate_session_id.into(),
            repo_hash: detect_repo_hash(&repo).unwrap().to_string(),
            owner_kind: owner.kind.as_str().into(),
            owner_number: owner.number,
            identity: planned_identity,
            capability_generation: 1,
        };
        let source = gwt_agent::Session::load(&gwt_core::paths::gwt_sessions_dir().join(format!("{predecessor_session_id}.toml"))).unwrap();
        let (mut config, outcome) = gwt::project_runtime::continuation::continuation_launch_config(
            &gwt::project_runtime::continuation::ContinueWorkLaunchSeed::DurableSession(Box::new(source)), &repo, owner, None);
        let readiness_nonce = uuid::Uuid::new_v4().to_string();
        config.env_vars.insert(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV.into(), readiness_nonce.clone());
        config.execution_intent = gwt_agent::ExecutionLaunchIntent::PreparedContinuation(binding.clone());
        let pending = Box::new(PendingContinueWork {
            client_id: "client-1".into(), operation_id: operation_id.into(), work_id: work_id.into(),
            project_root: project_root.clone(), worktree_path: repo.clone(), owner,
            work_branch: "work/issue-2359".into(), work_agent_id: gwt_agent::AgentId::Codex,
            work_agent_session_id: Some(predecessor_session_id.into()),
            execution: PendingContinueWorkExecution::Takeover(request), binding, readiness_nonce, outcome,
            resume_context: WorkspaceResumeContext { title: Some("Active stale Work".into()), owner: Some("Issue #2359".into()), summary: None, next_action: None },
            predecessor_session_id: predecessor_session_id.into(), predecessor_binding: predecessor_binding.clone(),
        });
        let checkpoint = host.pause_next_agent_launch().await.expect("actual same-generation preparation");
        let launch_host = host.clone();
        let mut launch = tokio::spawn(async move {
            launch_host.launch_prepared_continuation(Box::new(config), pending).await
        });
        let mut failures = host.subscribe();
        // test-hygiene: allow-wall-clock-deadline hang guard for the correlated actual takeover preparation or truthful early refusal
        let captured = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::select! {
                captured = checkpoint.wait() => captured,
                result = &mut launch => {
                    let outcomes: Vec<_> = std::iter::from_fn(|| failures.try_recv().ok()).collect();
                    panic!("actual takeover refused before preparation: {result:?}; {outcomes:?}");
                }
            }
        }).await.expect("takeover reaches actual checkpoint");
        let pending = captured.pending_continue_work.as_ref().expect("actual takeover receipt");
        assert!(matches!(pending.execution, PendingContinueWorkExecution::Takeover(_)));
        let candidate_session_id = &pending.binding.session_id;
        let planned_identity = pending.binding.identity.clone();
        let grant = captured.grant.as_ref().expect("actual Prepared grant");
        assert!(captured.issuer.grant_is_current(grant));
        checkpoint.release();
        launch.await.expect("takeover worker").expect("physical ACK");
        let mut projected = host.subscribe();
        let mut readiness = runtime_hook_state_for_event("Waiting", "SessionStart", candidate_session_id);
        readiness.project_root = Some(project_root.display().to_string());
        readiness.branch = Some("work/issue-2359".into());
        readiness.continuation_readiness_nonce = Some(pending.readiness_nonce.clone());
        let physical = host.probe(&captured.id, captured.incarnation).await.unwrap();
        assert_eq!(
            gwt_agent::SessionExecutionIdentity::from_session(physical.session.as_ref().unwrap()).unwrap().unwrap(),
            gwt::project_runtime::launch_finalization::pending_continue_work_session_identity(pending).unwrap(),
            "legacy Prepared identity must exactly match the actual owner Session"
        );
        readiness.agent_session_id.clone_from(&physical.session.as_ref().unwrap().agent_session_id);
        readiness.occurred_at = chrono::Utc::now().to_rfc3339();
        assert_eq!(physical.runtime_hook(&readiness).await.unwrap(), reqwest::StatusCode::NO_CONTENT);
        // test-hygiene: allow-wall-clock-deadline hang guard for the exact owner outcome; accepted messages establish ordering
        let outcome = tokio::time::timeout(Duration::from_secs(30), async { loop {
            let event = projected.recv().await.expect("owner outcome");
            if matches!(&event, OwnerEvent::ContinueWorkOutcome { operation_id: operation, .. } if operation == operation_id) { break event; }
        }}).await.expect("takeover readiness outcome");
        if !matches!(outcome, OwnerEvent::ContinueWorkOutcome { outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation, .. }) {
            eprintln!("actual takeover finalization: {outcome:?}");
            let cleanup = host.command(OwnerCommand::Close { id: captured.id.clone(), incarnation: captured.incarnation }).await;
            eprintln!("exact failed takeover close: {cleanup:?}");
            Arc::try_unwrap(host).ok().expect("failed launch caller released").shutdown().await;
            panic!("actual takeover finalization: {outcome:?}");
        }
        let active = host.probe(&captured.id, captured.incarnation).await.expect("actual takeover runtime");
        assert!(active.has_capability_bearer(grant));
        assert!(active.issuer.grant_is_current(active.grant.as_ref().expect("promoted grant")));
        assert_eq!(gwt::cli::execution_state::current_execution_binding(&repo, owner).expect("takeover binding"), Some(planned_identity));
        assert!(!gwt::cli::execution_state::current_active_execution_binding_matches(&repo, owner,
            predecessor_session_id, &predecessor_binding).expect("predecessor fence"));
        let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner).expect("ledger").expect("ledger");
        assert_eq!(ledger.current_generation_id, generation_id);
        assert_eq!(ledger.generations.len(), 1);
        assert_eq!(ledger.takeovers.len(), 1);
        assert_eq!(&ledger.takeovers[0].to_session_id, candidate_session_id);
        let work = gwt_core::workspace_projection::load_workspace_work_items(&project_root).expect("Work").expect("Work")
            .work_items.into_iter().find(|item| item.id == work_id).expect("selected Work");
        assert_eq!(work.status_category, gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
        assert!(work.agents.iter().any(|agent| &agent.session_id == candidate_session_id));
        assert_eq!(materialized_project_state_sots("works.json"),
            vec![gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root)],
            "Continue Work cannot create a worktree-local WorkItems shadow");
        // The original replay check is a fresh coordinator with no local pane
        // view while the committed candidate/Work remain Active. Closing it
        // first would legitimately pause Work and test a different conflict.
        let retry = continue_work_domain_failure(&runtime, "client-retry", operation_id, work_id);
        assert_eq!(retry.outcome, gwt::ContinueWorkOutcomeKind::Failed);
        assert_eq!(retry.code, "continuation_reconciliation_required");
        assert!(retry.retryable);
        let retry_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner).expect("retry ledger").expect("ledger");
        assert_eq!(retry_ledger.generations.len(), 1);
        assert_eq!(retry_ledger.takeovers.len(), 1);
        host.command(OwnerCommand::Close { id: captured.id, incarnation: captured.incarnation }).await.expect("exact close ACK");
        Arc::try_unwrap(host).ok().expect("prepared launch caller released").shutdown().await;
    });
}

#[test]
fn app_runtime_stop_revokes_issue_time_capability_after_project_deletion() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("project directory");
    init_repo_with_initial_commit(&repo);
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::events::OwnerCommand;
        let owner = production_owner_fixture(&repo).await;
        let command = owner_fixture_agent_command(
            &owner,
            &repo,
            "deleted-project",
            "deleted-project-operation",
        );
        let receipt = owner
            .command(command)
            .await
            .expect("actual physical launch");
        let id = receipt["id"].as_str().expect("owner pane id").to_string();
        let incarnation = receipt["incarnation"].as_u64().expect("exact incarnation");
        let actual = owner
            .probe(&id, incarnation)
            .await
            .expect("actual owner handles");
        let grant = actual
            .grant
            .as_ref()
            .expect("actual issue-time Host capability");
        assert!(actual.issuer.grant_is_current(grant));
        let original_store = gwt_core::paths::gwt_project_dir_for_repo_path(&repo);
        assert!(
            !gwt_core::workspace_projection::load_workspace_work_items(&repo)
                .expect("actual launched Work")
                .expect("Work projection")
                .work_items
                .is_empty()
        );
        let original_work = snapshot_optional_files(&[
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo),
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&repo),
        ]);
        fs::remove_dir_all(&repo).expect("delete project after actual capability issue");
        let fallback_store = gwt_core::paths::gwt_project_dir_for_repo_path(&repo);
        assert_ne!(
            original_store, fallback_store,
            "directory loss changes path-derived identity and cannot change the captured owner"
        );
        owner
            .command(OwnerCommand::Close {
                id: id.clone(),
                incarnation,
            })
            .await
            .expect("captured fixed-project cleanup survives directory loss");
        assert!(!actual.issuer.grant_is_current(grant));
        assert!(
            actual.pty.tree_exit_confirmed(),
            "the exact captured process tree has exited"
        );
        assert!(
            owner.probe(&id, incarnation).await.is_err(),
            "no process-local authority entry survives exact close"
        );
        assert!(
            !fallback_store.exists(),
            "exact close cannot synthesize paused Work in a newly derived project store"
        );
        assert_optional_files_unchanged(&original_work);
        owner.shutdown().await;
    });
}

#[test]
fn app_runtime_antigravity_missing_binary_launch_error_is_actionable() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let raw_error = "PTY creation failed: Unable to spawn agy because: \
No viable candidates found in PATH \
\"/private/var/folders/tmp/node_modules/.bin:/opt/homebrew/bin:/Users/example/.local/bin\"";

    let events = runtime.apply_owner_terminal_launch_projection(
        &runtime.test_context(),
        &window_id,
        None,
        None,
        WindowProcessStatus::Error,
        Some(raw_error.to_string()),
    );

    let detail = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalStatus {
                id, status, detail, ..
            } if id == &window_id && *status == WindowProcessStatus::Error => detail.as_deref(),
            _ => None,
        })
        .expect("terminal status detail");
    assert!(detail.contains("Antigravity CLI (`agy`) was not found"));
    assert!(detail.contains("https://antigravity.google/cli/install.sh"));
    assert!(detail.contains("~/.local/bin"));
    assert!(!detail.contains("No viable candidates found in PATH"));
    assert!(!detail.contains("/private/var/folders"));

    let diagnostic = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalOutput { id, data_base64 } if id == &window_id => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data_base64)
                    .expect("decode terminal diagnostic");
                Some(String::from_utf8_lossy(&decoded).to_string())
            }
            _ => None,
        })
        .expect("launch failure diagnostic terminal output");
    assert!(diagnostic.contains("Antigravity CLI (`agy`) was not found"));
    assert!(diagnostic.contains("https://antigravity.google/cli/install.sh"));
    assert!(!diagnostic.contains("No viable candidates found in PATH"));
    assert!(!diagnostic.contains("node_modules/.bin"));
}

#[test]
fn app_runtime_antigravity_missing_binary_launch_wizard_error_is_actionable() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let raw_error = "PTY creation failed: Unable to spawn agy because: \
No viable candidates found in PATH \
\"/private/var/folders/tmp/node_modules/.bin:/opt/homebrew/bin:/Users/example/.local/bin\"";

    runtime.pending_launch_feedback_contexts.insert(
        "tab-1::agent-1".into(),
        LaunchFeedbackContext {
            client_id: "client-1".to_string(),
            title: "Launch failed".to_string(),
            issue_monitor_issue_number: None,
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: None,
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );
    let events = runtime.apply_owner_terminal_launch_projection(
        &runtime.test_context(),
        "tab-1::agent-1",
        None,
        None,
        WindowProcessStatus::Error,
        Some(raw_error.to_string()),
    );

    let message = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardOpenError { title, message } if title == "Launch failed" => {
                Some(message.as_str())
            }
            _ => None,
        })
        .expect("launch wizard open error");
    assert!(message.contains("Antigravity CLI (`agy`) was not found"));
    assert!(message.contains("https://antigravity.google/cli/install.sh"));
    assert!(message.contains("~/.local/bin"));
    assert!(!message.contains("No viable candidates found in PATH"));
    assert!(!message.contains("/private/var/folders"));
}

// A missing installed OpenCode binary receives an actionable install hint,
// matching the Antigravity treatment.
#[test]
fn app_runtime_opencode_missing_binary_launch_error_is_actionable() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "agent-1",
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let raw_error = "PTY creation failed: Unable to spawn opencode because: \
No viable candidates found in PATH \
\"/private/var/folders/tmp/node_modules/.bin:/opt/homebrew/bin:/Users/example/.local/bin\"";

    let events = runtime.apply_owner_terminal_launch_projection(
        &runtime.test_context(),
        &window_id,
        None,
        None,
        WindowProcessStatus::Error,
        Some(raw_error.to_string()),
    );

    let detail = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalStatus {
                id, status, detail, ..
            } if id == &window_id && *status == WindowProcessStatus::Error => detail.as_deref(),
            _ => None,
        })
        .expect("terminal status detail");
    assert!(detail.contains("OpenCode (`opencode`) was not found"));
    assert!(detail.contains("npm i -g opencode-ai"));
    assert!(!detail.contains("No viable candidates found in PATH"));
    assert!(!detail.contains("/private/var/folders"));
}

#[test]
fn app_runtime_opencode_missing_binary_launch_wizard_error_is_actionable() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab(
        "tab-1",
        "Repo",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let raw_error = "PTY creation failed: Unable to spawn opencode because: \
No viable candidates found in PATH \
\"/private/var/folders/tmp/node_modules/.bin:/opt/homebrew/bin:/Users/example/.local/bin\"";

    runtime.pending_launch_feedback_contexts.insert(
        "tab-1::agent-1".into(),
        LaunchFeedbackContext {
            client_id: "client-1".to_string(),
            title: "Launch failed".to_string(),
            issue_monitor_issue_number: None,
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: None,
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );
    let events = runtime.apply_owner_terminal_launch_projection(
        &runtime.test_context(),
        "tab-1::agent-1",
        None,
        None,
        WindowProcessStatus::Error,
        Some(raw_error.to_string()),
    );

    let message = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardOpenError { title, message } if title == "Launch failed" => {
                Some(message.as_str())
            }
            _ => None,
        })
        .expect("launch wizard open error");
    assert!(message.contains("OpenCode (`opencode`) was not found"));
    assert!(message.contains("npm i -g opencode-ai"));
    assert!(!message.contains("No viable candidates found in PATH"));
    assert!(!message.contains("/private/var/folders"));
}

#[test]
fn app_runtime_issue_monitor_launch_error_emits_monitor_failure_events() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (root, owner, delivery) = owner_monitor_fixture(temp.path(), 42, true);
    let (detail, evidence, owner_events) = owner_monitor_fail_before_pty(
        &root,
        &owner,
        42,
        "agent",
        delivery,
        None,
        gwt_agent::AgentId::Codex,
    );
    assert!(owner_events.iter().any(|event| matches!(event, gwt::project_runtime::events::OwnerEvent::LaunchFailed { id, incarnation, detail: message } if id == &evidence.id && *incarnation == evidence.incarnation && message == &detail)));
    assert_eq!(
        owner_monitor_read(&root, 42)
            .inbox_item(42)
            .unwrap()
            .error_message
            .as_deref(),
        Some(detail.as_str())
    );
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent",
        root,
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let id = combined_window_id("tab-1", "agent");
    runtime
        .pending_launch_feedback_contexts
        .insert(id.clone(), issue_monitor_feedback(42));
    let events = runtime.apply_owner_terminal_launch_projection(
        &runtime.test_context(),
        &id,
        None,
        None,
        WindowProcessStatus::Error,
        Some(detail.clone()),
    );
    assert!(events.iter().any(|event| matches!(&event.event, BackendEvent::IssueMonitorLaunchFailed { issue_number: 42, message } if message == &detail)));
    assert!(events.iter().any(|event| matches!(&event.event, BackendEvent::IssueMonitorToast { level, message, issue_number: Some(42), .. } if level == "error" && message == &detail)));
    assert!(
        !events.iter().any(|event| matches!(
            event.event,
            BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
        )),
        "GUI failure projection cannot publish a competing Monitor snapshot"
    );
}

#[test]
fn app_runtime_issue_monitor_git_auth_launch_failure_is_actionable() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (root, owner, delivery) = owner_monitor_fixture(temp.path(), 42, true);
    let raw = owner_monitor_git_auth_failure(&root, &owner, 42, delivery);
    assert!(raw.contains("could not read Username"), "{raw}");
    let monitor = owner_monitor_read(&root, 42);
    let durable = monitor
        .inbox_item(42)
        .unwrap()
        .error_message
        .as_deref()
        .unwrap();
    assert!(durable.contains("Git HTTPS credentials are required"));
    assert!(durable.contains("gh auth setup-git"));
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent",
        root,
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let id = combined_window_id("tab-1", "agent");
    runtime
        .pending_launch_feedback_contexts
        .insert(id.clone(), issue_monitor_feedback(42));
    let events = runtime.apply_owner_terminal_launch_projection(
        &runtime.test_context(),
        &id,
        None,
        None,
        WindowProcessStatus::Error,
        Some(raw),
    );
    let message = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorLaunchFailed {
                issue_number: 42,
                message,
            } => Some(message.as_str()),
            _ => None,
        })
        .unwrap();
    assert!(message.contains("Git HTTPS credentials are required"));
    assert!(message.contains("gh auth setup-git"));
    assert!(message.contains("git ls-remote origin HEAD"));
    assert!(message.contains("Original error: "));
    assert!(message.contains("could not read Username"));
}

#[test]
fn app_runtime_issue_monitor_launch_complete_marks_issue_launched_and_keeps_active_capacity() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (root, owner, delivery_id) = owner_monitor_fixture(temp.path(), 42, false);
    let pane = owner_monitor_launch(&root, &owner, 42, "agent", delivery_id);
    let monitor = owner_monitor_read(&root, 42);
    let status = monitor.status_view();
    assert_eq!(
        status.state,
        if status.last_error.is_some() {
            "error"
        } else {
            "active"
        }
    );
    assert_eq!(status.active_count, 1);
    assert_eq!(status.active_issue_number, Some(42));
    let item = monitor.inbox_item(42).unwrap();
    assert_eq!(item.state, gwt::MonitorInboxState::Launched);
    assert_eq!(item.launched_window_id.as_deref(), Some(pane.id.as_str()));
    let prefs =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&root)).unwrap();
    assert_eq!(
        prefs.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: pane.id.clone()
        }]
    );
    assert!(
        pane.pty.try_wait().unwrap().is_none(),
        "capacity belongs to the actual living owner child"
    );
    super::pane_control_tests::control_close(&owner, &pane);
    owner_fixture_block_on(owner.shutdown());
}

/// Issue #4014: closing a window whose PTY child is still alive must finish
/// its close finalizer. The finalizer joins the reader thread, and on Windows
/// the ConPTY output pipe never signals EOF while the pseudoconsole is open,
/// so a finalizer that keeps the master alive while waiting for the reader
/// hangs - inside `env_test_lock`, which then stalls every other test that
/// needs the lock.
#[test]
fn app_runtime_close_finalizer_completes_while_a_live_pty_reader_is_attached() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let _process_home = ScopedEnvVar::set("HOME", temp.path());
    let _process_userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let executor = tokio::runtime::Runtime::new().expect("fixture executor");
    executor.block_on(async {
        use gwt::project_runtime::{events::OwnerCommand, test_support::ProjectOwnerHarness};
        let owner = ProjectOwnerHarness::start(&repo).await.expect("start physical owner");
        let id = owner.pane_id("shell-1");
        let (command, args) = if cfg!(windows) {
            ("cmd".to_string(), vec!["/d".into(), "/s".into(), "/c".into(),
                "ping -n 31 127.0.0.1 >NUL".into()])
        } else {
            ("/bin/sh".to_string(), vec!["-lc".into(), "sleep 30".into()])
        };
        let receipt = owner.command(OwnerCommand::LaunchShell {
            id: id.clone(), command, args, cols: 120, rows: 32,
        }).await.expect("spawn a live pane with reader and status threads");
        let incarnation = receipt["incarnation"].as_u64().expect("exact incarnation");
        let physical = owner.probe(&id, incarnation).await.expect("capture owner PTY");
        let child_pid = physical.pty.process_id().expect("live child PID");
        assert!(gwt::process::is_process_alive(child_pid));
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual close ACK and physical reader drain
        let closed = tokio::time::timeout(Duration::from_secs(30), owner.command(
            OwnerCommand::Close { id, incarnation },
        )).await.expect("close finalizer must complete once the child is reaped; it must not wait on a PTY reader whose EOF depends on the pseudoconsole the finalizer itself keeps open");
        closed.expect("exact owner close acknowledgement");
        assert!(!gwt::process::is_process_alive(child_pid),
            "the close acknowledgement must follow exact child reap");
        owner.shutdown().await;
    });
}

#[test]
fn app_runtime_closing_issue_monitor_window_returns_issue_to_pending() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (root, owner, delivery_id) = owner_monitor_fixture(temp.path(), 42, false);
    let pane = owner_monitor_launch(&root, &owner, 42, "agent", delivery_id);
    assert_eq!(owner_monitor_read(&root, 42).status_view().active_count, 1);
    super::pane_control_tests::control_close(&owner, &pane);
    assert!(
        pane.pty.try_wait().unwrap().is_some(),
        "Monitor requeue must follow the actual child reap"
    );
    let monitor = owner_monitor_read(&root, 42);
    assert_eq!(
        monitor.status_view().active_count,
        0,
        "closing the launched window frees the active slot"
    );
    assert_eq!(
        monitor.inbox_item(42).unwrap().state,
        gwt::MonitorInboxState::Queued,
        "an unmerged close returns the Issue to pending: {:?}",
        monitor.inbox_item(42)
    );
    assert!(
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&root))
            .unwrap()
            .launched_issues
            .is_empty(),
        "closed window is no longer persisted as an active launch"
    );
    owner_fixture_block_on(owner.shutdown());
}

const OWNER_MONITOR_EXIT_DETAIL: &str =
    "Process exited with status 1 — last output: Stop-block hit an error";

fn owner_monitor_runtime_failure(
    temp: &Path,
    from_hook: bool,
) -> (String, Vec<OutboundEvent>, gwt::IssueMonitorPrefs) {
    use gwt::project_runtime::events::OwnerCommand;
    use gwt_core::daemon::{ClientFrame, DaemonFrame};
    let (root, owner, delivery) = owner_monitor_fixture(temp, 42, true);
    let mut physical_output = owner.subscribe();
    let mut config = super::pane_control_tests::control_launch_config(&root, 42);
    let expected_detail = if from_hook {
        "Stop-block hit an error"
    } else {
        OWNER_MONITOR_EXIT_DETAIL
    };
    let expected_last_error = format!("issue #42: {expected_detail}");
    let command = config.args.last_mut().unwrap();
    if !from_hook {
        *command = if cfg!(windows) {
            "$null = [Console]::ReadLine(); [Console]::WriteLine('Stop-block hit an error'); $null = [Console]::ReadLine(); exit 1".into()
        } else {
            "stty -echo; read first; printf 'Stop-block hit an error\n'; read second; exit 1".into()
        };
    }
    *command = if cfg!(windows) {
        format!("[Console]::Write(([char]27).ToString() + '[2J' + ([char]27).ToString() + '[H'); {command}")
    } else {
        format!("printf '\\033[2J\\033[H'; {command}")
    };
    let pane = owner_monitor_launch_with_config(&root, &owner, 42, "agent", delivery, config);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent",
        root.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp, vec![tab], Some("tab-1"));
    runtime.blocking_tasks = BlockingTaskSpawner::queued().0;
    let context = runtime.test_context();
    let identity = owner.client().identity().unwrap().clone();
    let project = runtime.project_state_mut(&context).unwrap();
    project.project_owner_connection_id = 1;
    project.project_owner_identity = Some(identity.clone());
    let executor = TokioRuntime::new().unwrap();
    executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bounds the real provider's startup clear, so libtest gate banners cannot become the error-tail evidence under test.
        tokio::time::timeout(Duration::from_secs(30), async {
            let mut output = Vec::new();
            loop {
                if let gwt::project_runtime::events::OwnerEvent::Output {
                    id,
                    incarnation,
                    data,
                    screen,
                    ..
                } = physical_output.recv().await.unwrap()
                {
                    if id == pane.id && incarnation == pane.incarnation {
                        output.extend(data);
                        if output.windows(4).any(|bytes| bytes == b"\x1b[2J")
                            && screen.trim().is_empty()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .await
        .unwrap();
    });
    let mut subscriber = executor.block_on(async {
        let endpoint = gwt::daemon_publisher::resolve_project_daemon_endpoint(&root)
            .unwrap()
            .unwrap();
        let mut subscriber = gwt::cli::daemon::client::DaemonClient::connect(&endpoint)
            .await
            .unwrap();
        subscriber
            .send_frame(&ClientFrame::SubscribeMaterializer {
                channels: vec![gwt::runtime_daemon_events::ISSUE_MONITOR_CHANNEL.into()],
            })
            .await
            .unwrap();
        loop {
            if subscriber.read_frame::<DaemonFrame>().await.unwrap() == DaemonFrame::Ack {
                break;
            }
        }
        subscriber
    });
    let child_was_live = pane.pty.try_wait().unwrap().is_none();
    if from_hook {
        let mut hook = runtime_hook_state("Error", &pane.session.as_ref().unwrap().id);
        hook.gwt_session_id = Some(pane.session.as_ref().unwrap().id.clone());
        hook.project_root = Some(root.display().to_string());
        hook.message = Some("Stop-block hit an error".into());
        assert!(executor
            .block_on(pane.runtime_hook(&hook))
            .unwrap()
            .is_success());
    } else {
        pane.pane.lock().unwrap().write_input(b"\r").unwrap();
        executor.block_on(async {
            // test-hygiene: allow-wall-clock-deadline Bounds the real child's diagnostic output; exact screen evidence orders its subsequent exit.
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    if let gwt::project_runtime::events::OwnerEvent::Output {
                        id,
                        incarnation,
                        screen,
                        ..
                    } = physical_output.recv().await.unwrap()
                    {
                        if id == pane.id
                            && incarnation == pane.incarnation
                            && screen.trim() == "Stop-block hit an error"
                        {
                            break;
                        }
                    }
                }
            })
            .await
            .expect("real provider diagnostic reached the owner parser");
        });
        pane.pane.lock().unwrap().write_input(b"\r").unwrap();
    }
    let mut events = Vec::new();
    let received = executor.block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bounds actual canonical daemon failure notifications; their exact payloads establish completion.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let frame = tokio::select! {
                    event = physical_output.recv() => {
                        let event = event.expect("actual owner notice stream");
                        if matches!(event, gwt::project_runtime::events::OwnerEvent::Notice { .. }) {
                            events.extend(runtime.handle_project_owner_event(&context, 1, &identity, event));
                        }
                        None
                    }
                    frame = subscriber.read_frame::<DaemonFrame>() => Some(frame.unwrap()),
                };
                if let Some(DaemonFrame::Event { payload, .. }) = frame {
                match crate::issue_monitor_daemon_user_event(payload, &root) {
                    Some(UserEvent::IssueMonitorDaemonStatus { project_root, status })
                        if status.last_error.as_deref() == Some(expected_last_error.as_str()) =>
                        events.extend(runtime.issue_monitor_daemon_status_events(&project_root, status)),
                    Some(UserEvent::IssueMonitorDaemonInbox { project_root, items })
                        if items.iter().any(|item| item.issue.number == 42 && item.state == gwt::MonitorInboxState::AgentFailed) =>
                        events.extend(runtime.issue_monitor_daemon_inbox_events(&project_root, items)),
                    Some(UserEvent::Dispatch(outbound)) => events.extend(outbound),
                    _ => {}
                }
                }
                let status = events.iter().any(|event| matches!(&event.event,
                    BackendEvent::IssueMonitorStatus { status }
                        if status.last_error.as_deref() == Some(expected_last_error.as_str())));
                let inbox = events.iter().any(|event| matches!(&event.event,
                    BackendEvent::IssueMonitorInbox { items }
                        if items.iter().any(|item| item.issue.number == 42 && item.state == gwt::MonitorInboxState::AgentFailed)));
                let toast = events.iter().any(|event| matches!(&event.event,
                    BackendEvent::IssueMonitorToast { level, message, issue_number, .. }
                        if level == "error" && message == expected_detail && *issue_number == Some(42)));
                if status && inbox && (from_hook || toast) { break; }
            }
        }).await
    });
    let prefs =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&root)).unwrap();
    let actual_status = executor.block_on(owner.command(OwnerCommand::Snapshot {
        id: pane.id.clone(),
        incarnation: pane.incarnation,
    }));
    if !from_hook {
        assert!(
            pane.pty.try_wait().unwrap().is_some(),
            "the runtime failure has real exit proof"
        );
    }
    drop(subscriber);
    super::pane_control_tests::control_close(&owner, &pane);
    owner_fixture_block_on(owner.shutdown());
    assert!(
        child_was_live,
        "the observed failure must belong to an actual live physical owner"
    );
    assert!(
        received.is_ok(),
        "canonical failure notifications missing: {events:?}; owner={actual_status:?}"
    );
    (pane.id, events, prefs)
}

#[test]
fn app_runtime_runtime_error_marks_issue_monitor_launched_issue_failed() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (window_id, events, prefs) = owner_monitor_runtime_failure(temp.path(), false);

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(status.state, "idle");
    assert_eq!(status.active_count, 0);
    assert_eq!(
        status.last_error.as_deref(),
        Some("issue #42: Process exited with status 1 — last output: Stop-block hit an error")
    );

    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 42)
        .expect("failed issue row");
    assert_eq!(item.state, gwt::MonitorInboxState::AgentFailed);
    assert_eq!(item.launched_window_id, None);
    assert_eq!(
        item.error_message.as_deref(),
        Some(OWNER_MONITOR_EXIT_DETAIL)
    );
    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast {
                level,
                message,
                issue_number,
                ..
            } if level == "error"
                && message == OWNER_MONITOR_EXIT_DETAIL
                && *issue_number == Some(42)
        )
    }));

    assert!(prefs.launched_issues.is_empty());
    assert_eq!(
        prefs.failed_issues,
        vec![gwt::IssueMonitorFailedIssue {
            issue_number: 42,
            message: OWNER_MONITOR_EXIT_DETAIL.to_string(),
            // #3165 error-window lifecycle: the failed agent window id is
            // retained so an explicit Launch Now can close the stale window.
            window_id: Some(window_id.clone()),
        }]
    );
}

#[test]
fn app_runtime_hook_error_marks_issue_monitor_launched_issue_failed_with_hook_message() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (_, events, _) = owner_monitor_runtime_failure(temp.path(), true);

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(status.state, "idle");
    assert_eq!(
        status.last_error.as_deref(),
        Some("issue #42: Stop-block hit an error")
    );
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 42)
        .expect("failed issue row");
    assert_eq!(item.state, gwt::MonitorInboxState::AgentFailed);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
}

#[test]
fn app_runtime_frontend_ready_replays_launch_error_diagnostic_snapshot_without_runtime() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    init_repo_with_initial_commit(temp.path());
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().to_path_buf(),
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut config =
        gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Custom("fixture".into()))
            .working_dir(temp.path())
            .build();
    config.command = temp
        .path()
        .join("missing-diagnostic-program")
        .display()
        .to_string();
    config.args.clear();
    drive_owner_agent_fixture(&mut runtime, &window_id, config, Default::default());
    assert!(
        !runtime.runtimes.contains_key(&window_id),
        "failed owner has no GUI physical runtime"
    );
    let actual_failure = runtime
        .launch_error_terminal_details
        .get(&window_id)
        .expect("actual owner failure detail")
        .clone();

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

    let snapshot = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalSnapshot { id, data_base64 } if id == &window_id => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data_base64)
                    .expect("decode terminal diagnostic snapshot");
                Some(String::from_utf8_lossy(&decoded).to_string())
            }
            _ => None,
        })
        .expect("launch failure diagnostic terminal snapshot");
    assert!(
        snapshot.contains("Launch failed before PTY started"),
        "snapshot must replay the launch diagnostic after reconnect: {snapshot:?}"
    );
    assert!(
        snapshot.contains(&actual_failure),
        "snapshot must include the launch error detail: {snapshot:?}"
    );
}

#[test]
fn app_runtime_launch_wizard_submit_emits_agent_window_launching_status() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

    let submit_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LaunchWizardAction {
            action: LaunchWizardAction::Submit,
        },
    );
    assert!(submit_events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard)
            } if wizard.launch_materialization_pending
        )
    }));

    let events = dispatch_launch_materialization_request(
        &mut runtime,
        &recorded_events,
        "launch wizard submit materialization",
    );

    let workspace = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::WorkspaceState { workspace } => Some(workspace),
            _ => None,
        })
        .expect("workspace state after wizard submit");
    let agent_window = workspace
        .tabs
        .iter()
        .find(|tab| tab.id == "tab-1")
        .and_then(|tab| {
            tab.workspace
                .windows
                .iter()
                .find(|window| window.preset == WindowPreset::Agent)
        })
        .expect("agent placeholder window");
    assert_eq!(agent_window.title, "Codex");
    assert_eq!(agent_window.agent_id.as_deref(), Some("codex"));

    let launch_status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalStatus { id, detail, .. }
                if detail.as_deref() == Some("Launching...") =>
            {
                Some(id)
            }
            _ => None,
        })
        .expect("launching terminal status");
    assert!(launch_status.ends_with("::agent-1"));
    assert!(events.iter().any(|event| {
        matches!(
            event.event,
            BackendEvent::LaunchWizardState { wizard: None }
        )
    }));
}

#[test]
fn app_runtime_launch_complete_missing_wizard_window_surfaces_open_error() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

    let _submit_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::LaunchWizardAction {
            action: LaunchWizardAction::Submit,
        },
    );
    let (spawner, _launch_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let launch_events = dispatch_launch_materialization_request(
        &mut runtime,
        &recorded_events,
        "launch wizard complete materialization",
    );
    let window_id = launch_events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalStatus { id, detail, .. }
                if detail.as_deref() == Some("Launching...") =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .expect("wizard launch window id");
    let address = runtime
        .window_lookup
        .remove(&window_id)
        .expect("registered agent window");
    let tab = runtime.tab_mut(&address.tab_id).expect("tab");
    assert!(tab.workspace.close_window(&address.raw_id));

    let completion_events = runtime.apply_owner_terminal_launch_projection(
        &runtime.test_context(),
        &window_id,
        None,
        None,
        WindowProcessStatus::Error,
        Some("Window not found".into()),
    );

    assert!(completion_events.iter().any(|event| {
        matches!(
            (&event.target, &event.event),
            (
                DispatchTarget::Client(client_id),
                BackendEvent::LaunchWizardOpenError { title, message }
            ) if client_id == "client-1"
                && title == "Launch Agent"
                && message == "Window not found"
        )
    }));
}

#[test]
fn app_runtime_start_work_launch_completion_registers_unassigned_agent() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260504-1234");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let mut active = sample_active_agent_session("tab-1", &combined_window_id("tab-1", "agent-1"));
    active.session_id = "session-1".into();
    active.branch_name = "work/20260504-1234".into();
    active.worktree_path = worktree.clone();
    active.agent_project_root = repo.display().to_string();
    gwt::project_runtime::workspace::save_start_work_workspace_projection(
        &repo,
        &active,
        "origin/main",
        None,
        None,
        None,
        None,
    )
    .expect("owner publishes unassigned Start Work");

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    assert_eq!(
        projection.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Unknown,
        "Start Work must not make an unassigned Agent an active Workspace"
    );
    assert!(projection.git_details.is_none());
    assert_eq!(projection.agents.len(), 1);
    assert_eq!(projection.agents[0].session_id, "session-1");
    assert_eq!(
        projection.agents[0].affiliation_status,
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned
    );
    assert_eq!(
        projection.agents[0].branch.as_deref(),
        Some("work/20260504-1234")
    );
    assert_eq!(
        projection.agents[0].worktree_path.as_deref(),
        Some(worktree.as_path())
    );
    let work_items =
        gwt_core::workspace_projection::load_workspace_work_items(&repo).expect("load work items");
    assert!(
        work_items.is_none(),
        "Start Work launch must not create Workspace history before explicit assignment"
    );
}

#[test]
fn app_runtime_non_work_launch_registers_unassigned_agent() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut active = sample_active_agent_session("tab-1", &combined_window_id("tab-1", "agent-1"));
    active.session_id = "session-develop".into();
    active.branch_name = "develop".into();
    active.worktree_path = repo.clone();
    active.agent_project_root = repo.display().to_string();
    gwt::project_runtime::workspace::save_start_work_workspace_projection(
        &repo,
        &active,
        "origin/develop",
        None,
        None,
        None,
        None,
    )
    .expect("owner publishes unassigned non-work launch");

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    assert_eq!(projection.agents.len(), 1);
    assert_eq!(projection.agents[0].session_id, "session-develop");
    assert_eq!(
        projection.agents[0].affiliation_status,
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned
    );
    assert_eq!(projection.agents[0].branch.as_deref(), Some("develop"));
}

#[test]
fn app_runtime_linked_launch_projection_failure_is_visible_and_stops_session() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let project_root = temp.path().join("workspace-home");
    let (bare, _develop_worktree) = init_managed_workspace_with_develop_worktree(&project_root);
    let worktree = project_root.join("work").join("issue-3412");
    fs::create_dir_all(worktree.parent().expect("worktree parent"))
        .expect("create worktree parent");
    run_git(
        &bare,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "work/issue-3412",
            worktree.to_str().expect("worktree path"),
            "develop",
        ],
    );
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        project_root.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let project_state_path =
        gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root);
    let project_state_dir = project_state_path.parent().expect("project-state dir");
    fs::create_dir_all(project_state_dir).expect("create project-state directory");
    fs::create_dir(&project_state_path).expect("block only the current projection target");
    let repo_global_work_items_path =
        gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
    let repo_global_close_events_path =
        gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&project_root);
    assert!(
        project_state_dir.is_dir()
            && repo_global_work_items_path.parent() == Some(project_state_dir)
            && repo_global_close_events_path.parent() == Some(project_state_dir),
        "the projection failure fixture must leave sibling Work paths structurally writable"
    );
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let mut config =
        gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Custom("fixture".into()))
            .working_dir(&worktree)
            .branch("work/issue-3412")
            .base_branch("origin/develop")
            .suppress_execution_control()
            .build();
    let (command, args) = long_running_test_command();
    config.command = command;
    config.args = args;
    config.linked_issue_number = Some(3412);
    let events = drive_owner_agent_fixture(
        &mut runtime,
        &window_id,
        config,
        gwt::project_runtime::launch_payload::OwnerLaunchMetadata::default(),
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Error,
            ..
        }
    )));
    assert!(
        events.iter().any(|event| match &event.event {
            BackendEvent::TerminalOutput { data_base64, .. } => {
                base64::engine::general_purpose::STANDARD
                    .decode(data_base64)
                    .ok()
                    .is_some_and(|bytes| String::from_utf8_lossy(&bytes).contains("[gwt]"))
            }
            _ => false,
        }),
        "launch publication failure must be visible in terminal output"
    );
    assert!(
        !runtime.active_agent_sessions.contains_key(&window_id),
        "failed materialization must not leave a ready Session"
    );
    assert!(
        !runtime.runtimes.contains_key(&window_id),
        "failed materialization must stop the spawned PTY"
    );
    assert!(
        !gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root).exists(),
        "visible failure cleanup must not synthesize a repo-global phantom Work"
    );
    assert!(
        !gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&worktree).exists(),
        "visible failure cleanup must not synthesize a worktree WorkItems shadow"
    );
    assert!(
        !gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&project_root)
            .exists(),
        "visible failure cleanup must not append a phantom Pause event"
    );
}

#[test]
fn app_runtime_workspace_resume_launch_completion_carries_context_to_projection() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260507-0001");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    let mut active = sample_active_agent_session("tab-1", &combined_window_id("tab-1", "agent-1"));
    active.session_id = "session-1".into();
    active.branch_name = "work/20260507-0001".into();
    active.worktree_path = worktree.clone();
    active.agent_project_root = repo.display().to_string();
    let context = WorkspaceResumeContext {
        title: Some("Suspended review".into()),
        owner: Some("SPEC-2359".into()),
        summary: Some("Resume the suspended Work card.".into()),
        next_action: Some("Resume the review".into()),
    };
    gwt::project_runtime::workspace::save_resumed_workspace_projection(
        &repo,
        &active,
        None,
        Some(2359),
        None,
        &context,
        None,
    )
    .expect("owner publishes resumed workspace context");

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("projection");
    let details = projection.git_details.expect("git details");
    assert_eq!(projection.title, "Suspended review");
    assert_eq!(projection.owner.as_deref(), Some("SPEC-2359"));
    assert_eq!(
        projection.summary.as_deref(),
        Some("Resume the suspended Work card.")
    );
    assert_eq!(projection.next_action.as_deref(), Some("Resume the review"));
    assert_eq!(details.branch.as_deref(), Some("work/20260507-0001"));
    assert_eq!(details.worktree_path.as_deref(), Some(worktree.as_path()));
    assert!(details.created_by_start_work);
    assert_eq!(projection.agents.len(), 1);
    assert_eq!(projection.agents[0].session_id, "session-1");
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load work items")
        .expect("work items");
    assert_eq!(
        work_items.work_items[0].events[0].kind,
        gwt_core::workspace_projection::WorkEventKind::Resume
    );
}

#[test]
fn app_runtime_unlinked_resume_launch_completion_records_work_projection() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260507-resume");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/issue-2359",
            worktree.to_str().unwrap(),
        ],
    );
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let mut config =
        gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Custom("fixture".into()))
            .working_dir(&worktree)
            .branch("work/issue-2359")
            .session_mode(gwt_agent::SessionMode::Resume)
            .resume_session_id("native-unlinked-resume")
            .build();
    let (command, args) = long_running_test_command();
    config.command = command;
    config.args = args;
    let _events = drive_owner_agent_fixture(
        &mut runtime,
        &window_id,
        config,
        gwt::project_runtime::launch_payload::OwnerLaunchMetadata {
            workspace_resume_context: Some(WorkspaceResumeContext {
                title: Some("Resumed session".into()),
                owner: None,
                summary: Some("Resume the prior conversation.".into()),
                next_action: None,
            }),
            ..Default::default()
        },
    );

    assert!(
        runtime.active_agent_sessions.contains_key(&window_id),
        "the resumed session owns a visible pane"
    );
    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("load projection")
        .expect("an unlinked resume must create or update the Workspace projection");
    assert_eq!(projection.title, "Resumed session");
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load Work projection")
        .expect("an unlinked resume must append its Work events");
    assert_eq!(
        work_items.work_items[0].events[0].kind,
        gwt_core::workspace_projection::WorkEventKind::Resume
    );
    close_test_owner_runtime(&mut runtime, &window_id);
}

#[test]
fn automatic_resume_with_stale_execution_binding_completes_without_genesis_authentication() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-stale-binding");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/stale-binding",
            worktree.to_str().unwrap(),
        ],
    );
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let window_id = combined_window_id("tab-1", "agent-1");
    // A durable Session left behind by an earlier producing execution keeps
    // its (now stale) execution binding. Resuming it without an owner link
    // must degrade to an unbound launch, never hard-fail on genesis
    // authentication.
    let mut session = gwt_agent::Session::new(
        &worktree,
        "work/stale-binding",
        gwt_agent::AgentId::Custom("fixture".into()),
    );
    session.id = "session-stale-binding".to_string();
    session.restore_window_on_startup = true;
    session.linked_issue_number = Some(4242);
    session.project_state_root = Some(repo.clone());
    session.agent_session_id = Some("native-stale-binding".into());
    session.execution_binding = Some(gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: "session-stale-binding".to_string(),
        repo_hash: session
            .repo_hash
            .clone()
            .unwrap_or_else(|| "stale-repo-hash".to_string()),
        owner_kind: "issue".to_string(),
        owner_number: 4242,
        identity: gwt_agent::ExecutionBindingIdentity {
            generation_id: "gen-stale".to_string(),
            binding_id: "binding-stale".to_string(),
            ledger_head_hash: "ledger-stale".to_string(),
        },
        capability_generation: 1,
    });
    session
        .save(&runtime.sessions_dir)
        .expect("persist stale-bound session");
    let (command, args) = long_running_test_command();
    persist_owner_fixture_agent(command, args);
    session.record_hook_event("Stop");
    session.record_completed_stop();
    session.save(&runtime.sessions_dir).unwrap();
    let (executor, owner, events, error, _) = drive_owner_restore_fixture(
        &mut runtime,
        &window_id,
        session,
        gwt_agent::SessionLaunchOrigin::AutomaticRestore,
    );

    assert!(
        runtime.active_agent_sessions.contains_key(&window_id),
        "an automatic resume must never hard-fail on a stale producing binding left by an earlier execution: {events:?}; {error:?}"
    );
    let projection = &runtime.runtimes[&window_id];
    let probe = executor
        .block_on(owner.probe(&projection.owner_pane_id, projection.incarnation))
        .expect("actual restored owner runtime");
    assert!(
        probe
            .session
            .as_ref()
            .expect("actual resumed Session")
            .execution_binding
            .is_none(),
        "a stale producing binding must degrade to the actual unbound launch"
    );
    close_test_owner_runtime(&mut runtime, &window_id);
    executor.block_on(owner.shutdown());
}

#[test]
fn runtime_hook_agent_failure_completion_preserves_replaced_session_and_pane() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().to_path_buf(),
        WindowPreset::Agent,
        WindowProcessStatus::Error,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let identity = runtime
        .runtime_hook_agent_failure_identity(temp.path(), &window_id)
        .expect("failure identity");
    runtime
        .runtime_hook_agent_failures_in_flight
        .insert(window_id.clone(), identity.clone());
    runtime
        .active_agent_sessions
        .get_mut(&window_id)
        .expect("replacement session")
        .session_id = "session-successor".to_string();
    runtime
        .pending_launch_feedback_contexts
        .insert(window_id.clone(), issue_monitor_feedback(43));
    let monitor = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            autonomous_mode: true,
            autonomous_records: vec![issue_monitor_autonomous_record(
                42,
                gwt::AutonomousPhase::Implementing,
                1,
            )],
            ..gwt::IssueMonitorPrefs::default()
        },
    );

    let mut prepared = super::super::PreparedRuntimeHookAgentFailure {
        identity,
        message: "predecessor failed".to_string(),
        issue_number_hint: Some(42),
        result: super::super::PreparedIssueMonitorAgentFailure::Committed {
            monitor: Some(Box::new(monitor)),
            issue_number: Some(42),
            emit_local_snapshot: false,
        },
        status: None,
    };

    let events = runtime.handle_runtime_hook_agent_failure_prepared(prepared.clone());

    assert!(events.is_empty(), "a stale failure must stay silent");
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert_eq!(
        runtime
            .active_agent_sessions
            .get(&window_id)
            .expect("successor session retained")
            .session_id,
        "session-successor"
    );
    assert_eq!(
        runtime.pending_launch_feedback_contexts[&window_id].issue_monitor_issue_number,
        Some(43)
    );
    assert!(runtime.runtime_hook_agent_failures_in_flight.is_empty());

    prepared.identity = runtime
        .runtime_hook_agent_failure_identity(temp.path(), &window_id)
        .expect("same-session predecessor identity");
    runtime
        .runtime_hook_agent_failures_in_flight
        .insert(window_id.clone(), prepared.identity.clone());
    runtime.register_window("tab-1", "agent-1");
    let successor = runtime
        .runtime_hook_agent_failure_identity(temp.path(), &window_id)
        .expect("same-session successor identity");
    assert_eq!(successor.session_id, prepared.identity.session_id);
    assert_ne!(
        successor.lifecycle_generation,
        prepared.identity.lifecycle_generation
    );

    assert!(runtime
        .handle_runtime_hook_agent_failure_prepared(prepared)
        .is_empty());
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert_eq!(
        runtime.pending_launch_feedback_contexts[&window_id].issue_monitor_issue_number,
        Some(43)
    );
    assert!(runtime.runtime_hook_agent_failures_in_flight.is_empty());
}

#[test]
fn runtime_hook_agent_failure_completion_survives_sibling_tab_close() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let sibling_tab = sample_project_tab(
        "tab-a",
        "Sibling",
        temp.path().to_path_buf(),
        ProjectKind::Git,
        &[],
    );
    let owning_tab = sample_project_tab_with_window_at(
        "tab-b",
        "agent-1",
        temp.path().to_path_buf(),
        WindowPreset::Agent,
        WindowProcessStatus::Error,
    );
    let mut runtime = sample_runtime(temp.path(), vec![sibling_tab, owning_tab], Some("tab-b"));
    let (spawner, _finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let window_id = combined_window_id("tab-b", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-b", &window_id),
    );
    let identity = runtime
        .runtime_hook_agent_failure_identity(temp.path(), &window_id)
        .expect("failure identity");
    runtime
        .runtime_hook_agent_failures_in_flight
        .insert(window_id.clone(), identity.clone());

    runtime.close_project_tab_events("tab-a");
    assert!(runtime.window_lookup.contains_key(&window_id));
    let monitor = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            autonomous_mode: true,
            autonomous_records: vec![issue_monitor_autonomous_record(
                42,
                gwt::AutonomousPhase::Implementing,
                1,
            )],
            ..gwt::IssueMonitorPrefs::default()
        },
    );
    let prepared = super::super::PreparedRuntimeHookAgentFailure {
        identity,
        message: "owning pane failed".to_string(),
        issue_number_hint: Some(42),
        result: super::super::PreparedIssueMonitorAgentFailure::Committed {
            monitor: Some(Box::new(monitor)),
            issue_number: Some(42),
            emit_local_snapshot: false,
        },
        status: None,
    };

    let events = runtime.handle_runtime_hook_agent_failure_prepared(prepared);

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { level, message, issue_number, .. }
                if level == "error" && message == "owning pane failed" && *issue_number == Some(42)
        )),
        "the owning pane's current failure must finish after a sibling closes: {events:?}"
    );
    assert!(!runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.runtime_hook_agent_failures_in_flight.is_empty());
}

#[test]
fn runtime_hook_agent_failure_payload_preserves_captured_source_and_typed_failure() {
    let source = gwt::runtime_daemon_events::IssueMonitorAgentFailureSource::Bound {
        issue_number: 42,
        identity: gwt::IssueMonitorLaunchIdentity {
            active: true,
            claim_id: Some("claim-old".to_string()),
            delivery_id: Some("delivery-old".to_string()),
            window_id: Some("tab-1::agent-42".to_string()),
        },
    };
    let failure = gwt::IssueMonitorFailure::ResumeWriterConflict {
        holder_window_id: Some("tab-1::holder".to_string()),
    };
    let payload = AppRuntime::issue_monitor_agent_failed_from_launch_payload(
        "tab-1::agent-42",
        "predecessor failed",
        &source,
        Some(&failure),
    );
    assert!(payload.get("agent_failed").is_none());
    let notification = &payload["agent_failed_from_launch"];
    assert_eq!(notification["window_id"], "tab-1::agent-42");
    assert_eq!(notification["message"], "predecessor failed");
    assert_eq!(
        notification["source"],
        serde_json::to_value(source).unwrap()
    );
    assert_eq!(
        notification["failure"],
        serde_json::to_value(failure).unwrap()
    );
}

#[test]
fn runtime_hook_agent_failure_enqueue_error_does_not_commit_local_fallback() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let traces = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CaptureTracingLayer {
        events: traces.clone(),
    });
    let (warning, before, after) = tracing::subscriber::with_default(subscriber, || {
        let (root, owner, delivery) = owner_monitor_fixture(temp.path(), 42, true);
        let pane = owner_monitor_launch(&root, &owner, 42, "agent", delivery);
        owner_fixture_block_on(owner.wait_monitor_idle()).unwrap();
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&root);
        let before = fs::read(&prefs_path).unwrap();
        let endpoint_path = gwt_core::daemon::RuntimeScope::from_project_root(
            &root,
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .unwrap()
        .endpoint_path(&gwt_core::paths::gwt_home());
        struct RestoreOwnedEndpoint {
            path: PathBuf,
            bytes: Vec<u8>,
        }
        impl Drop for RestoreOwnedEndpoint {
            fn drop(&mut self) {
                fs::write(&self.path, &self.bytes)
                    .expect("restore the exact owned endpoint before owner teardown");
            }
        }
        let endpoint = RestoreOwnedEndpoint {
            bytes: fs::read(&endpoint_path).unwrap(),
            path: endpoint_path,
        };
        fs::remove_file(&endpoint.path).unwrap();
        let mut hook = runtime_hook_state("Error", &pane.session.as_ref().unwrap().id);
        hook.gwt_session_id = Some(pane.session.as_ref().unwrap().id.clone());
        hook.project_root = Some(root.display().to_string());
        hook.message = Some("Stop-block hit an error".into());
        super::super::reset_local_issue_monitor_fallback_commit_count();
        let accepted = owner_fixture_block_on(pane.runtime_hook(&hook));
        let warning = owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline Bounds the actual owner failure-publication warning after its owned endpoint is unavailable; the warning establishes completion.
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    if traces
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|event: &&CapturedTracingEvent| {
                            event.fields.get("message").is_some_and(|message| {
                                message == "owner Monitor notification is unconfirmed"
                            })
                        })
                        .count()
                        >= 2
                    {
                        // This first actual hook emits a heartbeat and its
                        // failure control. Observe both workers before recovery.
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
        });
        // Restore even on a failed observation before any final assertion or
        // physical cleanup can try to publish the captured Monitor source.
        drop(endpoint);
        let after = fs::read(&prefs_path).unwrap();
        super::pane_control_tests::control_close(&owner, &pane);
        owner_fixture_block_on(owner.shutdown());
        assert!(
            accepted.unwrap().is_success(),
            "the actual authenticated hook listener must accept its exact Session"
        );
        (warning, before, after)
    });
    assert!(
        warning.is_ok(),
        "the actual owner must surface unavailable Monitor publication: {:?}",
        traces.lock().unwrap()
    );
    assert_eq!(
        after, before,
        "a transport failure must not commit a local Monitor fallback"
    );
    assert_eq!(super::super::local_issue_monitor_fallback_commit_count(), 0);
}

#[test]
fn runtime_hook_agent_failure_local_fallback_preserves_successor_launch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_without_origin(&repo);
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut prefs = gwt::IssueMonitorPrefs {
        launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: window_id.clone(),
        }],
        launched_claims: std::collections::BTreeMap::from([(42, "claim-old".to_string())]),
        ..gwt::IssueMonitorPrefs::default()
    };
    let source = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs.clone())
        .launch_identity(42);
    // The same Issue and window now belong to a different launch. A GUI-only
    // completion check would be too late to protect this durable state.
    prefs
        .launched_claims
        .insert(42, "claim-successor".to_string());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("save successor");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Error,
    );
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut source = super::super::RuntimeHookAgentFailureSource {
        identity: runtime
            .runtime_hook_agent_failure_identity(&repo, &window_id)
            .expect("failure source identity"),
        generations: runtime.window_lifecycle_generations.clone(),
        launch: Some((42, source)),
    };

    let result = AppRuntime::prepare_issue_monitor_agent_failure(
        &repo,
        Some("tab-1"),
        &window_id,
        "predecessor failed",
        Some(42),
        None,
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
        Err(
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                "daemon unavailable".to_string(),
            ),
        ),
        Some(source.clone()),
    );

    assert!(matches!(
        result,
        super::super::PreparedIssueMonitorAgentFailure::Rejected
    ));
    let actual = gwt::load_issue_monitor_prefs(&prefs_path).expect("load successor");
    assert_eq!(actual.launched_issues, prefs.launched_issues);
    assert_eq!(actual.launched_claims, prefs.launched_claims);
    assert!(actual.failed_issues.is_empty());

    // Retirement during the scan must also be refused, after the initial
    // generation and launch checks succeeded.
    let monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), actual.clone());
    source.launch = Some((42, monitor.launch_identity(42)));
    let generations = source.generations.clone();
    let replacement_window = window_id.clone();
    super::super::RUNTIME_HOOK_AGENT_FAILURE_AFTER_SCAN_TEST_HOOK.with(|slot| {
        assert!(slot
            .replace(Some(Box::new(move || {
                let mut generations = generations.lock().expect("source generations");
                let next = generations.get(&replacement_window).copied().unwrap_or(0) + 1;
                generations.insert(replacement_window, next);
            })))
            .is_none());
    });
    let result = AppRuntime::prepare_issue_monitor_agent_failure(
        &repo,
        Some("tab-1"),
        &window_id,
        "retired during scan",
        Some(42),
        None,
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
        Err(
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                "daemon unavailable".to_string(),
            ),
        ),
        Some(source),
    );
    assert!(matches!(
        result,
        super::super::PreparedIssueMonitorAgentFailure::Rejected
    ));
    let after_scan = gwt::load_issue_monitor_prefs(&prefs_path).expect("load after retired scan");
    assert_eq!(after_scan.launched_issues, prefs.launched_issues);
    assert_eq!(after_scan.launched_claims, prefs.launched_claims);
    assert!(after_scan.failed_issues.is_empty());
}
