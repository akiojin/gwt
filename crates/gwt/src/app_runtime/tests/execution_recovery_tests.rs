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

    let events = fixture
        .runtime
        .fresh_execution_launch_failed_events(&fixture.window_id, "simulated response loss");

    assert!(
        events.is_empty(),
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
    let events = fixture
        .runtime
        .fresh_execution_launch_failed_events(&fixture.window_id, "settle fixture");
    assert!(!events.is_empty());
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

    fixture.runtime.reconcile_durable_fresh_execution_launches();

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

    fixture.runtime.reconcile_durable_fresh_execution_launches();

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

    fixture.runtime.reconcile_durable_fresh_execution_launches();

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
    fixture.runtime.reconcile_durable_fresh_execution_launches();
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

    runtime.reconcile_durable_fresh_execution_launches();

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
    runtime.reconcile_durable_fresh_execution_launches();

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

    restarted.bootstrap();
    // Issue #4378 AC-2: the generation reaper runs on the blocking worker.
    let queued = std::mem::take(
        &mut *tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    for task in queued {
        task();
    }

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

    fixture.runtime.reconcile_durable_fresh_execution_launches();

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
    runtime.reconcile_durable_fresh_execution_launches();

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

    let _events = fixture
        .runtime
        .fresh_execution_launch_failed_events(&fixture.window_id, "simulated launch failure");

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

    restarted.reconcile_durable_fresh_execution_launches();

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

    fixture.runtime.reconcile_durable_fresh_execution_launches();

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
    fs::remove_file(&candidate_path).expect("observe missing startup candidate Session");
    let authority_before = snapshot_optional_files(&exact_continue_authority_artifacts(
        &fixture.repo,
        fixture.owner,
    ));
    let work_before = tracked_workspace_work_store_snapshot(&fixture.repo);
    let sessions_dir = fixture.runtime.sessions_dir.clone();
    let repo = fixture.repo.clone();
    let expected_session_id = fixture.candidate_session_id.clone();
    set_missing_session_cleanup_hook_for_test(Box::new(move |session_id| {
        assert_eq!(session_id, expected_session_id);
        let mut replacement =
            gwt_agent::Session::new(&repo, "work/replacement", gwt_agent::AgentId::Codex);
        replacement.id = session_id.to_string();
        replacement
            .save(&sessions_dir)
            .expect("materialize same-id Session after startup Missing observation");
    }));

    fixture.runtime.reconcile_durable_fresh_execution_launches();

    assert!(
        candidate_path.exists(),
        "replacement Session must be retained"
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

    restarted.reconcile_durable_fresh_execution_launches();

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

    fixture.runtime.reconcile_durable_fresh_execution_launches();

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

    fixture.runtime.reconcile_durable_fresh_execution_launches();

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

    fixture.runtime.reconcile_durable_fresh_execution_launches();

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

    runtime.reconcile_durable_fresh_execution_launches();

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

    runtime.reconcile_durable_fresh_execution_launches();

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

    runtime.reconcile_durable_fresh_execution_launches();

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

    runtime.reconcile_durable_fresh_execution_launches();

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

    restarted.reconcile_durable_fresh_execution_launches();

    assert!(
        candidate_path.exists(),
        "cleanup I/O failure must retain the Session as durable retry evidence",
    );
    assert!(durable_launch_recovery_exists(
        &restarted.sessions_dir,
        &fixture.candidate_session_id,
    ));
    fs::remove_dir(&removal_blocker).expect("clear cleanup I/O blocker");

    restarted.reconcile_durable_fresh_execution_launches();

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
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-launch-completion");
    fixture
        .runtime
        .pending_fresh_execution_launches
        .remove(&fixture.window_id);
    fixture
        .runtime
        .active_agent_sessions
        .remove(&fixture.window_id);
    fixture
        .runtime
        .agent_capability_tokens
        .remove(&fixture.window_id);
    let readiness_nonce = "fresh-launch-completion-readiness";
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "ping -n 31 127.0.0.1 >NUL".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "sleep 30".to_string()],
        )
    };
    let events = fixture.runtime.handle_launch_complete_and_drain(
        fixture.window_id.clone(),
        Ok((
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::from([
                    (
                        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV.to_string(),
                        fixture.token.clone(),
                    ),
                    (
                        gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV.to_string(),
                        readiness_nonce.to_string(),
                    ),
                ]),
                remove_env: Vec::new(),
                cwd: Some(fixture.repo.clone()),
                resource_policy: None,
            },
            fixture.candidate_session_id.clone(),
            "work/issue-2359".to_string(),
            "Codex".to_string(),
            fixture.repo.clone(),
            gwt_agent::AgentId::Codex,
            Some(fixture.owner.number),
            Some("origin/develop".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            gwt_agent::SessionMode::Normal,
            false,
            fixture.repo.display().to_string().into(),
        )),
    );

    let pending = fixture
        .runtime
        .pending_fresh_execution_launches
        .get(&fixture.window_id)
        .expect("launch completion must recover the Prepared receipt");
    assert_eq!(pending.operation_id, fixture.operation_id);
    assert_eq!(pending.readiness_nonce, readiness_nonce);
    assert_eq!(pending.binding, fixture.binding);
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read pre-readiness binding"),
        Some(fixture.predecessor_binding.clone()),
        "PTY spawn must not activate the fresh generation",
    );
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalStatus { detail: Some(detail), .. }
            if detail == "Waiting for authenticated SessionStart..."
    )));

    // Issue #3475: the spawned pane is alive but silent, so the deadline first
    // spends its bounded extension budget.
    assert!(fixture
        .runtime
        .handle_continue_work_ready_timeout(
            &fixture.window_id,
            &ContinueWorkReadinessWatch::new(fixture.operation_id.clone()),
        )
        .is_empty());
    // Issue #3482: the terminal deadline hands the launch off instead of
    // killing a pane whose process is still the exact live launch pane.
    let handoff = fixture.runtime.handle_continue_work_ready_timeout(
        &fixture.window_id,
        &readiness_watch_at_last_extension(&fixture.operation_id, 0),
    );
    assert!(!handoff.is_empty());
    assert!(fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    assert!(fixture.runtime.runtimes.contains_key(&fixture.window_id));

    // Issue #3482 AC-3: once that pane really dies, the supervised deadline
    // still performs the same bounded rollback.
    fixture
        .runtime
        .stop_window_runtime_without_session_projection(&fixture.window_id);
    fixture
        .runtime
        .window_pty_statuses
        .insert(fixture.window_id.clone(), WindowProcessStatus::Stopped);
    let cleanup = fixture.runtime.handle_continue_work_ready_timeout(
        &fixture.window_id,
        &ContinueWorkReadinessWatch {
            handed_off: true,
            ..readiness_watch_at_last_extension(&fixture.operation_id, 0)
        },
    );
    assert!(!cleanup.is_empty());
    assert_pending_fresh_execution_was_rolled_back(&fixture);
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
    let candidate_session_id = "takeover-candidate-session";
    let operation_id = "continue-op-takeover";
    let work_id = "work-active";
    let readiness_nonce = "continue-ready-takeover";
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
    let request = gwt::cli::execution_state::GenerationTakeoverRequest {
        operation_id: operation_id.to_string(),
        principal_id: "gwt-host-continuation".to_string(),
        work_id: Some(work_id.to_string()),
        source: Some("continue-work:resume".to_string()),
        from_session_id: predecessor_session_id.to_string(),
        to_session_id: candidate_session_id.to_string(),
        reason: "continue-work-stale-takeover: all owning Host runtimes are dead".to_string(),
        requested_at: now,
    };
    gwt::cli::execution_state::prepare_generation_takeover(&repo, owner, &request)
        .expect("prepare same-generation takeover");
    let planned_identity =
        gwt::cli::execution_state::prepared_generation_takeover_execution_binding(
            &repo, owner, &request,
        )
        .expect("derive takeover binding");

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
    let mut runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut active = sample_active_agent_session("tab-1", &window_id);
    active.session_id = candidate_session_id.to_string();
    active.branch_name = "work/issue-2359".to_string();
    active.worktree_path = repo.clone();
    active.agent_project_root = project_root.display().to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), active);
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: candidate_session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity: planned_identity.clone(),
        capability_generation: 1,
    };
    let mut candidate =
        gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    candidate.id = candidate_session_id.to_string();
    candidate.project_state_root = Some(project_root.clone());
    candidate.linked_issue_number = Some(owner.number);
    candidate
        .set_execution_binding(Some(binding.clone()))
        .expect("bind candidate");
    candidate
        .save(&runtime.sessions_dir)
        .expect("save candidate");
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let capability = issuer
        .issue_prepared(&repo, candidate_session_id, binding.clone())
        .expect("issue Prepared capability");
    runtime.agent_capability_issuer = Some(issuer.clone());
    runtime
        .agent_capability_tokens
        .insert(window_id.clone(), capability.token.clone());
    runtime.pending_continue_work.insert(
        window_id.clone(),
        PendingContinueWork {
            client_id: "client-1".to_string(),
            operation_id: operation_id.to_string(),
            work_id: work_id.to_string(),
            project_root: project_root.clone(),
            worktree_path: repo.clone(),
            owner,
            work_branch: "work/issue-2359".to_string(),
            work_agent_id: gwt_agent::AgentId::Codex,
            work_agent_session_id: Some(predecessor_session_id.to_string()),
            execution: PendingContinueWorkExecution::Takeover(request),
            binding: binding.clone(),
            readiness_nonce: readiness_nonce.to_string(),
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            resume_context: WorkspaceResumeContext {
                title: Some("Active stale Work".to_string()),
                owner: Some("Issue #2359".to_string()),
                summary: None,
                next_action: None,
            },
            predecessor_session_id: predecessor_session_id.to_string(),
            predecessor_binding: predecessor_binding.clone(),
        },
    );

    let events = runtime.finalize_continue_work_session_start(&window_id, Some(readiness_nonce));

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            operation_id: emitted_operation_id,
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            ..
        } if emitted_operation_id == operation_id
    )));
    assert!(issuer.active_token_is_current(&capability.token, &binding));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&repo, owner)
            .expect("read takeover binding"),
        Some(planned_identity)
    );
    assert!(
        !gwt::cli::execution_state::current_active_execution_binding_matches(
            &repo,
            owner,
            predecessor_session_id,
            &predecessor_binding,
        )
        .expect("verify predecessor fence")
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read ledger")
        .expect("ledger");
    assert_eq!(ledger.current_generation_id, generation_id);
    assert_eq!(ledger.generations.len(), 1);
    assert_eq!(ledger.takeovers.len(), 1);
    assert_eq!(ledger.takeovers[0].to_session_id, candidate_session_id);
    let work = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
        .expect("load Work")
        .expect("Work")
        .work_items
        .into_iter()
        .find(|item| item.id == work_id)
        .expect("active Work");
    assert_eq!(
        work.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Active
    );
    assert!(work
        .agents
        .iter()
        .any(|agent| agent.session_id == candidate_session_id));
    assert_eq!(
        materialized_project_state_sots("works.json"),
        vec![gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root)],
        "Continue Work must not create a worktree-local WorkItems shadow"
    );

    let retry_tab = sample_project_tab_with_window_at(
        "tab-retry",
        "shell-retry",
        project_root.clone(),
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut restarted_runtime = sample_runtime(&runtime_root, vec![retry_tab], Some("tab-retry"));
    let retry_events = restarted_runtime.continue_work_events(
        &restarted_runtime.test_context(),
        "client-retry",
        operation_id.to_string(),
        work_id.to_string(),
        canvas_bounds(),
    );
    assert!(retry_events.iter().all(|event| !matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            ..
        }
    )));
    assert!(
        retry_events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                error_code: Some(code),
                retryable: true,
                ..
            } if code == "continuation_reconciliation_required"
        )),
        "unexpected retry events: {retry_events:#?}"
    );
    let retry_ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read takeover retry ledger")
        .expect("takeover retry ledger");
    assert_eq!(retry_ledger.generations.len(), 1);
    assert_eq!(retry_ledger.takeovers.len(), 1);
}

#[test]
fn app_runtime_stop_revokes_issue_time_capability_after_project_deletion() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let target = issuer.issue(&repo, "session-1").expect("issue capability");
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.worktree_path = repo.clone();
    session.agent_project_root = repo.display().to_string();
    runtime.agent_capability_issuer = Some(issuer.clone());
    runtime
        .agent_capability_tokens
        .insert(window_id.clone(), target.token.clone());
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);

    fs::remove_dir_all(&repo).expect("delete project after capability issue");
    runtime.mark_agent_session_stopped(&window_id);

    assert!(!issuer.authenticates_token(&target.token));
    assert!(!runtime.agent_capability_tokens.contains_key(&window_id));
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

    let events =
        runtime.handle_launch_complete_and_drain(window_id.clone(), Err(raw_error.to_string()));

    let detail = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalStatus { id, status, detail }
                if id == &window_id && *status == WindowProcessStatus::Error =>
            {
                detail.as_deref()
            }
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
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let raw_error = "PTY creation failed: Unable to spawn agy because: \
No viable candidates found in PATH \
\"/private/var/folders/tmp/node_modules/.bin:/opt/homebrew/bin:/Users/example/.local/bin\"";

    let events = runtime.launch_error_events(
        "tab-1::agent-1".to_string(),
        raw_error.to_string(),
        Some(LaunchFeedbackContext {
            client_id: "client-1".to_string(),
            title: "Launch failed".to_string(),
            issue_monitor_issue_number: None,
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: None,
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        }),
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

    let events =
        runtime.handle_launch_complete_and_drain(window_id.clone(), Err(raw_error.to_string()));

    let detail = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::TerminalStatus { id, status, detail }
                if id == &window_id && *status == WindowProcessStatus::Error =>
            {
                detail.as_deref()
            }
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
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let raw_error = "PTY creation failed: Unable to spawn opencode because: \
No viable candidates found in PATH \
\"/private/var/folders/tmp/node_modules/.bin:/opt/homebrew/bin:/Users/example/.local/bin\"";

    let events = runtime.launch_error_events(
        "tab-1::agent-1".to_string(),
        raw_error.to_string(),
        Some(LaunchFeedbackContext {
            client_id: "client-1".to_string(),
            title: "Launch failed".to_string(),
            issue_monitor_issue_number: None,
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: None,
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        }),
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
    let temp = tempdir().expect("tempdir");
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    init_repo_with_initial_commit(temp.path());
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().to_path_buf(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");

    let events = runtime.launch_error_events(
        window_id,
        "binary missing".to_string(),
        Some(LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: None,
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        }),
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorLaunchFailed {
                issue_number,
                message,
            } if *issue_number == 42 && message == "binary missing"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast {
                level,
                message,
                issue_number,
                ..
            } if level == "error" && message == "binary missing" && *issue_number == Some(42)
        )
    }));
}

#[test]
fn app_runtime_issue_monitor_git_auth_launch_failure_is_actionable() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    init_repo_with_initial_commit(temp.path());
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().to_path_buf(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");

    let events = runtime.launch_error_events(
        window_id,
        "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
            .to_string(),
        Some(LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: None,
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        }),
    );

    let message = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorLaunchFailed {
                issue_number,
                message,
            } if *issue_number == 42 => Some(message.as_str()),
            _ => None,
        })
        .expect("issue monitor launch failure message");
    assert!(message.contains("Git HTTPS credentials are required"));
    assert!(message.contains("gh auth setup-git"));
    assert!(message.contains("git ls-remote origin HEAD"));
    assert!(message.contains("Original error: fatal: could not read Username"));
}

#[test]
fn app_runtime_issue_monitor_launch_complete_marks_issue_launched_and_keeps_active_capacity() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "cache_merge_empty");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue Monitor launch success",
            &["bug"],
            "Issue body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            enabled: true,
            max_active_agents: 1,
            ..queued_issue_monitor_prefs(&[42])
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_launch_feedback_contexts.insert(
        window_id.clone(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: None,
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let events = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Ok((
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(repo.clone()),
                resource_policy: None,
            },
            "session-issue-42".to_string(),
            "work/issue-42".to_string(),
            "Codex".to_string(),
            repo.clone(),
            gwt_agent::AgentId::Codex,
            Some(42),
            Some("origin/develop".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            gwt_agent::SessionMode::Normal,
            false,
            repo.display().to_string().into(),
        )),
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    // The bounded cache-only origin probe may time out under a saturated test
    // runner. Scan health then intentionally takes the aggregate status to
    // `error`, but launch capacity must still reflect the committed lifecycle.
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
        .expect("launched issue row");
    assert_eq!(item.state, gwt::MonitorInboxState::Launched);
    assert_eq!(item.launched_window_id.as_deref(), Some(window_id.as_str()));

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    assert_eq!(
        prefs.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id,
        }]
    );
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
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "shell-1",
        WindowPreset::Shell,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "shell-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "ping -n 31 127.0.0.1 >NUL".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "sleep 30".to_string()],
        )
    };
    runtime
        .spawn_process_window_with_console_kind(
            &window_id,
            canvas_bounds(),
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: test_pane_cwd(),
                resource_policy: None,
            },
            None,
        )
        .expect("spawn a live pane with reader and status threads");

    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    assert!(runtime.close_window_outcome(&window_id).closed);
    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued close finalizer");
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        finalizer();
        let _ = done_tx.send(());
    });
    assert!(
        done_rx.recv_timeout(Duration::from_secs(30)).is_ok(),
        "close finalizer must complete once the child is reaped; it must not wait on a PTY reader whose EOF depends on the pseudoconsole the finalizer itself keeps open"
    );
}

#[test]
fn app_runtime_closing_issue_monitor_window_returns_issue_to_pending() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue Monitor close returns to pending",
            &["bug"],
            "Issue body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            enabled: true,
            max_active_agents: 1,
            ..queued_issue_monitor_prefs(&[42])
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_launch_feedback_contexts.insert(
        window_id.clone(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: None,
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };
    let _ = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Ok((
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(repo.clone()),
                resource_policy: None,
            },
            "session-issue-42".to_string(),
            "work/issue-42".to_string(),
            "Codex".to_string(),
            repo.clone(),
            gwt_agent::AgentId::Codex,
            Some(42),
            Some("origin/develop".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            gwt_agent::SessionMode::Normal,
            false,
            repo.display().to_string().into(),
        )),
    );

    // Closing the launched window must free the active slot and return the
    // (unmerged) Issue to pending — never a fabricated completion state.
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let immediate = runtime.close_window_events(&window_id);
    assert!(immediate.iter().all(|event| !matches!(
        event.event,
        BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
    )));
    let finalizer = finalizers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .expect("queued Issue Monitor close finalizer");
    finalizer();
    let events = apply_recorded_window_close_finalized(&mut runtime, &recorded_events);
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status after close");
    assert_eq!(
        status.active_count, 0,
        "closing the launched window frees the active slot"
    );
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox after close");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 42)
        .expect("issue row after close");
    assert_eq!(
        item.state,
        gwt::MonitorInboxState::Queued,
        "an unmerged close returns the Issue to pending"
    );

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    assert!(
        prefs.launched_issues.is_empty(),
        "closed window is no longer persisted as an active launch"
    );
}

#[test]
fn app_runtime_runtime_error_marks_issue_monitor_launched_issue_failed() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _projection_timeout = ScopedEnvVar::set(
        "GWT_TEST_ISSUE_MONITOR_FALLBACK_PROJECTION_TIMEOUT_MS",
        "10000",
    );
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let fake_bin = temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("create fake tool directory");
    let fake_gh = write_fake_gh_issue_list(&fake_bin);
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue Monitor runtime failure",
            &["bug"],
            "Issue body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    let window_id = combined_window_id("tab-1", "agent-1");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            enabled: true,
            max_active_agents: 5,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: window_id.clone(),
            }],
            ..queued_issue_monitor_prefs(&[42])
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("Stop-block hit an error".to_string()),
        true,
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(status.state, "error");
    assert_eq!(status.active_count, 0);
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
    assert_eq!(item.launched_window_id, None);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
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
                && message == "Stop-block hit an error"
                && *issue_number == Some(42)
        )
    }));

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    assert!(prefs.launched_issues.is_empty());
    assert_eq!(
        prefs.failed_issues,
        vec![gwt::IssueMonitorFailedIssue {
            issue_number: 42,
            message: "Stop-block hit an error".to_string(),
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
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue Monitor hook failure",
            &["bug"],
            "Issue body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    let window_id = combined_window_id("tab-1", "agent-1");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            enabled: true,
            max_active_agents: 5,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: window_id.clone(),
            }],
            ..queued_issue_monitor_prefs(&[42])
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    let mut hook = runtime_hook_state("Error", "session-1");
    hook.project_root = Some(repo.display().to_string());
    hook.message = Some("Stop-block hit an error".to_string());

    let events = runtime.handle_runtime_hook_event(hook);

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(status.state, "error");
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
    let _ = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Err("launch failed before process spawn".to_string()),
    );

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
        snapshot.contains("launch failed before process spawn"),
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
            bounds: Some(canvas_bounds()),
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
            BackendEvent::WindowCanvasState { workspace } => Some(workspace),
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
            bounds: Some(canvas_bounds()),
        },
    );
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

    let completion_events =
        runtime.handle_launch_complete_and_drain(window_id, Err("Window not found".to_string()));

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
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let _events = runtime.handle_launch_complete_and_drain(
        window_id,
        Ok((
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(worktree.clone()),
                resource_policy: None,
            },
            "session-1".to_string(),
            "work/20260504-1234".to_string(),
            "Codex".to_string(),
            worktree.clone(),
            gwt_agent::AgentId::Codex,
            None,
            Some("origin/main".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            gwt_agent::SessionMode::Normal,
            false,
            worktree.display().to_string().into(),
        )),
    );

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
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let _events = runtime.handle_launch_complete_and_drain(
        window_id,
        Ok((
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(repo.clone()),
                resource_policy: None,
            },
            "session-develop".to_string(),
            "develop".to_string(),
            "Codex".to_string(),
            repo.clone(),
            gwt_agent::AgentId::Codex,
            None,
            Some("origin/develop".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            gwt_agent::SessionMode::Normal,
            false,
            repo.display().to_string().into(),
        )),
    );

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
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "ping -n 31 127.0.0.1 >NUL".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "sleep 30".to_string()],
        )
    };

    let events = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Ok((
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(worktree.clone()),
                resource_policy: None,
            },
            "session-projection-failure".to_string(),
            "work/issue-3412".to_string(),
            "Codex".to_string(),
            worktree.clone(),
            gwt_agent::AgentId::Codex,
            Some(3412),
            Some("origin/develop".to_string()),
            gwt_agent::LaunchRuntimeTarget::Host,
            gwt_agent::SessionMode::Normal,
            false,
            project_root.display().to_string().into(),
        )),
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
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_workspace_resume_contexts.insert(
        window_id.clone(),
        WorkspaceResumeContext {
            title: Some("Suspended review".to_string()),
            owner: Some("SPEC-2359".to_string()),
            summary: Some("Resume the suspended Work card.".to_string()),
            next_action: Some("Resume the review".to_string()),
        },
    );
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let _events = runtime.handle_launch_complete_and_drain(
        window_id,
        Ok((
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(worktree.clone()),
                resource_policy: None,
            },
            "session-1".to_string(),
            "work/20260507-0001".to_string(),
            "Codex".to_string(),
            worktree.clone(),
            gwt_agent::AgentId::Codex,
            Some(2359),
            None,
            gwt_agent::LaunchRuntimeTarget::Host,
            gwt_agent::SessionMode::Resume,
            true,
            worktree.display().to_string().into(),
        )),
    );

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
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-20260507-resume");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_workspace_resume_contexts.insert(
        window_id.clone(),
        WorkspaceResumeContext {
            title: Some("Resumed session".to_string()),
            owner: None,
            summary: Some("Resume the prior conversation.".to_string()),
            next_action: None,
        },
    );
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let _events = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Ok((
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(worktree.clone()),
                resource_policy: None,
            },
            "session-unlinked-resume".to_string(),
            "work/issue-2359".to_string(),
            "Codex".to_string(),
            worktree,
            gwt_agent::AgentId::Codex,
            None,
            None,
            gwt_agent::LaunchRuntimeTarget::Host,
            gwt_agent::SessionMode::Resume,
            false,
            repo.display().to_string().into(),
        )),
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
}

#[test]
fn automatic_resume_with_stale_execution_binding_completes_without_genesis_authentication() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("repo-work-stale-binding");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&worktree).expect("create worktree");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    // A durable Session left behind by an earlier producing execution keeps
    // its (now stale) execution binding. Resuming it without an owner link
    // must degrade to an unbound launch, never hard-fail on genesis
    // authentication.
    let mut session =
        gwt_agent::Session::new(&worktree, "work/stale-binding", gwt_agent::AgentId::Codex);
    session.id = "session-stale-binding".to_string();
    session.linked_issue_number = Some(4242);
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
        .save(&temp.path().join("sessions"))
        .expect("persist stale-bound session");
    let (command, args) = if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/d".to_string(),
                "/s".to_string(),
                "/c".to_string(),
                "exit /b 0".to_string(),
            ],
        )
    } else {
        (
            "/bin/sh".to_string(),
            vec!["-lc".to_string(), "exit 0".to_string()],
        )
    };

    let events = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Ok((
            ProcessLaunch {
                initial_prompt_file: None,
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(worktree.clone()),
                resource_policy: None,
            },
            "session-stale-binding".to_string(),
            "work/stale-binding".to_string(),
            "Codex".to_string(),
            worktree,
            gwt_agent::AgentId::Codex,
            None,
            None,
            gwt_agent::LaunchRuntimeTarget::Host,
            gwt_agent::SessionMode::Resume,
            false,
            repo.display().to_string().into(),
        )),
    );

    assert!(
        runtime.active_agent_sessions.contains_key(&window_id),
        "an automatic resume must never hard-fail on a stale producing binding left by an earlier execution: {events:?}"
    );
}
