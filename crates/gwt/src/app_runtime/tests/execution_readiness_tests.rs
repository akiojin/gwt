use super::*;

#[test]
fn fresh_execution_session_start_queues_io_and_deduplicates_readiness() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-worker-readiness");
    let (spawner, queue) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    let nonce = fixture.runtime.pending_fresh_execution_launches[&fixture.window_id]
        .readiness_nonce
        .clone();

    for _ in 0..2 {
        let events = fixture
            .runtime
            .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&nonce));
        assert!(
            events.is_empty(),
            "readiness dispatch must return before I/O"
        );
    }
    assert!(fixture
        .issuer
        .prepared_token_is_current(&fixture.token, &fixture.binding));
    assert_eq!(
        queue.lock().expect("worker queue").len(),
        1,
        "duplicate authenticated readiness must schedule one transaction"
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner).unwrap(),
        Some(fixture.predecessor_binding.clone())
    );
    let events = commit_pending_fresh_execution(&mut fixture.runtime);
    assert!(!events.is_empty());
    assert!(!fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    let ledger = gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.owner)
        .unwrap()
        .unwrap();
    assert_eq!(ledger.generations.len(), 2);
    let work_events = load_tracked_work_events(&fixture.repo);
    assert_eq!(
        work_events
            .iter()
            .filter(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Start)
            .count(),
        1,
        "duplicate readiness must publish one Work start"
    );
    let queued_after_completion = queue.lock().unwrap().len();
    assert!(fixture
        .runtime
        .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&nonce))
        .is_empty());
    assert_eq!(queue.lock().unwrap().len(), queued_after_completion);
    assert_eq!(load_tracked_work_events(&fixture.repo), work_events);
}

#[test]
fn fresh_execution_finalization_rejects_replaced_window_pending_and_token() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    for replacement in ["window", "pending", "token"] {
        let mut fixture = pending_fresh_execution_fixture(temp.path(), replacement);
        let nonce = fixture.runtime.pending_fresh_execution_launches[&fixture.window_id]
            .readiness_nonce
            .clone();
        fixture
            .runtime
            .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&nonce));
        let completion = take_fresh_execution_finalization(&fixture.runtime);
        let work_events = load_tracked_work_events(&fixture.repo);
        match replacement {
            "window" => {
                fixture
                    .runtime
                    .window_lifecycle_generations
                    .lock()
                    .unwrap()
                    .insert(fixture.window_id.clone(), u64::MAX);
            }
            "pending" => {
                fixture
                    .runtime
                    .pending_fresh_execution_launches
                    .get_mut(&fixture.window_id)
                    .unwrap()
                    .operation_id = "replacement-operation".to_string();
            }
            "token" => {
                fixture
                    .runtime
                    .agent_capability_tokens
                    .insert(fixture.window_id.clone(), "replacement-token".to_string());
            }
            _ => unreachable!(),
        }
        let current_token = fixture.runtime.agent_capability_tokens[&fixture.window_id].clone();
        assert!(
            fixture
                .runtime
                .handle_fresh_execution_finalized(completion)
                .is_empty(),
            "{replacement} replacement must fence GUI completion"
        );
        assert!(fixture
            .runtime
            .pending_fresh_execution_launches
            .contains_key(&fixture.window_id));
        assert_eq!(
            fixture.runtime.active_agent_sessions[&fixture.window_id].session_id,
            fixture.candidate_session_id
        );
        assert_eq!(
            fixture.runtime.agent_capability_tokens[&fixture.window_id],
            current_token
        );
        assert_eq!(load_tracked_work_events(&fixture.repo), work_events);
    }
}

#[test]
fn fresh_execution_queued_readiness_cannot_revive_an_exact_rollback() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-worker-rollback");
    let nonce = fixture.runtime.pending_fresh_execution_launches[&fixture.window_id]
        .readiness_nonce
        .clone();
    fixture
        .runtime
        .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&nonce));
    fixture.runtime.handle_launch_complete_and_drain(
        fixture.window_id.clone(),
        Err("spawn failed before readiness worker".into()),
    );
    assert_pending_fresh_execution_was_rolled_back(&fixture);
    assert!(!fixture
        .runtime
        .pending_fresh_execution_finalizations
        .contains_key(&fixture.window_id));
    assert!(commit_pending_fresh_execution(&mut fixture.runtime).is_empty());
    assert_pending_fresh_execution_was_rolled_back(&fixture);
    assert!(load_tracked_work_events(&fixture.repo).is_empty());
}

#[test]
fn fresh_execution_queued_session_start_rejects_foreign_identity() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-early-readiness");
    let nonce = fixture.runtime.pending_fresh_execution_launches[&fixture.window_id]
        .readiness_nonce
        .clone();
    let mut early = runtime_hook_state_for_event("Working", "SessionStart", "foreign-session");
    early.agent_session_id = Some(fixture.candidate_session_id.clone());
    early.continuation_readiness_nonce = Some(nonce);
    early.project_root = Some(fixture.repo.display().to_string());
    early.branch = Some("work/issue-2359".to_string());
    fixture.runtime.handle_runtime_hook_event(early.clone());
    let BlockingTaskSpawner::Queued(tasks) = &fixture.runtime.blocking_tasks else {
        unreachable!();
    };
    assert!(
        tasks.lock().unwrap().is_empty(),
        "foreign identity cannot authenticate readiness"
    );
    early.gwt_session_id = Some(fixture.candidate_session_id.clone());
    fixture.runtime.handle_runtime_hook_event(early);
    assert!(fixture
        .issuer
        .prepared_token_is_current(&fixture.token, &fixture.binding));
    assert!(!commit_pending_fresh_execution(&mut fixture.runtime).is_empty());
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner).unwrap(),
        Some(fixture.binding.identity)
    );
}

#[test]
fn fresh_execution_queued_readiness_rolls_back_after_pane_or_project_close() {
    for (operation, project_close, close_worker_first) in [
        ("pane-readiness-first", false, false),
        ("pane-close-first", false, true),
        ("project-close-first", true, true),
    ] {
        let temp = tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path());
        let mut fixture = pending_fresh_execution_fixture(temp.path(), operation);
        insert_test_pane_runtime(&mut fixture.runtime, &fixture.window_id);
        let generation = fixture.runtime.runtimes[&fixture.window_id].incarnation;
        fixture
            .runtime
            .window_lifecycle_generations
            .lock()
            .unwrap()
            .insert(fixture.window_id.clone(), generation);
        let nonce = fixture.runtime.pending_fresh_execution_launches[&fixture.window_id]
            .readiness_nonce
            .clone();
        fixture
            .runtime
            .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&nonce));
        let BlockingTaskSpawner::Queued(tasks) = &fixture.runtime.blocking_tasks else {
            unreachable!();
        };
        let tasks = tasks.clone();
        if project_close {
            fixture.runtime.close_project_tab_events("tab-1");
        } else {
            fixture.runtime.close_window_events(&fixture.window_id);
        }
        assert!(
            !fixture
                .runtime
                .pending_fresh_execution_finalizations
                .contains_key(&fixture.window_id),
            "{operation}: close must forget its in-flight entry"
        );
        if !close_worker_first {
            // Run readiness while the closing generation is still present.
            let readiness = tasks.lock().unwrap().remove(0);
            readiness();
        }
        // The queued test spawner drains last-in first-out, so otherwise
        // detached close completes before the earlier readiness transaction.
        assert!(commit_pending_fresh_execution(&mut fixture.runtime).is_empty());
        assert_eq!(
            gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner,)
                .unwrap(),
            Some(fixture.predecessor_binding.clone()),
            "{operation}"
        );
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &fixture.repo,
                fixture.owner,
                &fixture.operation_id,
            )
            .unwrap()
            .unwrap()
            .status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Aborted,
            "{operation}: a closed Prepared candidate must release its owner fence"
        );
        assert!(
            !fixture
                .runtime
                .sessions_dir
                .join(format!("{}.toml", fixture.candidate_session_id))
                .exists(),
            "{operation}"
        );
        assert!(
            !durable_launch_recovery_exists(
                &fixture.runtime.sessions_dir,
                &fixture.candidate_session_id,
            ),
            "{operation}"
        );
        assert!(
            load_tracked_work_events(&fixture.repo).is_empty(),
            "{operation}"
        );
    }
}

#[test]
fn fresh_execution_completed_predecessor_survives_terminal_grace_and_activates_once() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture_with_predecessor_status(
        temp.path(),
        "completed-requeue",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        true,
    );
    let snapshots = fixture.runtime.terminal_window_snapshots();
    assert!(
        snapshots.is_empty(),
        "a Prepared candidate must not be observed as its Completed predecessor: {snapshots:?}"
    );
    let observations = super::super::terminal_convergence::observe_terminal_windows_in_background(
        &fixture.runtime.sessions_dir,
        snapshots,
        Duration::from_secs(60),
    );
    let now = Instant::now();
    fixture.runtime.terminal_convergence_observed_events_at(
        Duration::from_secs(60),
        observations,
        now,
    );
    fixture
        .runtime
        .close_expired_terminal_window_candidates_at(now + Duration::from_secs(61));
    assert!(
        fixture
            .runtime
            .window_lookup
            .contains_key(&fixture.window_id),
        "the Completed predecessor must not close its Prepared successor before Ready"
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner).unwrap(),
        Some(fixture.predecessor_binding.clone()),
    );
    let nonce = format!("readiness-{}", fixture.operation_id);
    fixture
        .runtime
        .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&nonce));
    commit_pending_fresh_execution(&mut fixture.runtime);
    assert!(!fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    assert_eq!(
        fixture.runtime.terminal_window_snapshots().len(),
        1,
        "the activated pane must return to normal terminal observation"
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner).unwrap(),
        Some(fixture.binding.identity.clone()),
        "one authenticated Ready must activate the fresh generation",
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.owner)
        .unwrap()
        .unwrap();
    assert_eq!(ledger.generations.len(), 2);
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Active)
    );
}

#[test]
fn durable_launch_recovery_receipt_is_monotonic_and_replays_base_write_without_downgrade() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "fresh-receipt-monotonic");
    let expected = durable_launch_recovery_session_identity(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    )
    .expect("read bound receipt")
    .expect("bound identity");

    persist_durable_launch_recovery(
        &fixture.runtime.sessions_dir,
        DurableLaunchRecoveryKind::FreshSuccessor {
            operation_id: fixture.operation_id.clone(),
        },
        &fixture.candidate_session_id,
        &fixture.repo,
        &fixture.repo,
        fixture.owner,
        None,
        None,
    )
    .expect("a retry must retain the stronger exact recovery identity");
    assert_eq!(
        durable_launch_recovery_session_identity(
            &fixture.runtime.sessions_dir,
            &fixture.candidate_session_id,
        )
        .expect("read retained bound receipt"),
        Some(expected)
    );
}

#[test]
fn prepared_manual_successor_loser_is_rejected_before_pane_creation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pending_fresh_execution_fixture(temp.path(), "manual-pre-pane-claim");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&fixture.repo)
        .branch("work/issue-2359")
        .linked_issue_number(fixture.owner.number)
        .extra_arg("$gwt-execute #2359")
        .build();
    config.execution_intent =
        gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(fixture.binding.clone());
    let winner = super::super::launch::claim_prepared_manual_successor_launch(
        &fixture.runtime.sessions_dir,
        &fixture.repo,
        &config,
    )
    .expect("claim Prepared candidate")
    .expect("manual successor claim");
    let tab = sample_project_tab("tab-1", "Repo", fixture.repo.clone(), ProjectKind::Git, &[]);
    let runtime_root = fixture
        .runtime
        .sessions_dir
        .parent()
        .expect("runtime root")
        .to_path_buf();
    let mut losing_runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));

    let error = losing_runtime
        .spawn_agent_window("tab-1", config, canvas_bounds(), None)
        .expect_err("the concurrent Prepared materializer must lose before pane creation");

    assert!(error.contains("already being materialized"), "{error}");
    assert!(losing_runtime.window_lookup.is_empty());
    assert!(losing_runtime
        .tab("tab-1")
        .expect("loser tab")
        .workspace
        .persisted()
        .windows
        .is_empty());
    assert!(
        gwt::cli::execution_state::finish_active_session_launch_handshake(
            &fixture.runtime.sessions_dir,
            &winner,
        )
        .expect("release winner claim")
    );
}

#[test]
fn fresh_execution_session_start_routes_monitor_ack_to_feedback_owner_project() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-monitor-owner-routing");
    let monitor_repo = temp.path().join("monitor-owner");
    std::fs::create_dir_all(&monitor_repo).expect("create monitor owner repo");
    init_repo_without_origin(&monitor_repo);

    let active_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.repo);
    gwt::save_issue_monitor_prefs(
        &active_prefs_path,
        &gwt::IssueMonitorPrefs {
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 900,
                window_id: "tab-1::sentinel".to_string(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed active project prefs");
    let owner_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&monitor_repo);
    gwt::save_issue_monitor_prefs(
        &owner_prefs_path,
        &gwt::IssueMonitorPrefs {
            launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
                issue_number: 42,
                claimed_at: None,
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed monitor owner prefs");

    let pending = fixture
        .runtime
        .pending_fresh_execution_launches
        .get_mut(&fixture.window_id)
        .expect("pending fresh execution");
    pending.launch_feedback_context = Some(LaunchFeedbackContext {
        client_id: "__issue_monitor__".to_string(),
        title: "Issue Monitor".to_string(),
        issue_monitor_issue_number: Some(42),
        issue_monitor_delivery_id: None,
        issue_monitor_project_root: Some(monitor_repo.clone()),
        issue_monitor_session_mode: None,
        issue_monitor_autonomous_handoff: None,
        issue_monitor_autonomous_submit_started: false,
        issue_monitor_review_dispatch: false,
    });
    let readiness_nonce = pending.readiness_nonce.clone();

    let mut session_start =
        runtime_hook_state_for_event("Working", "SessionStart", &fixture.candidate_session_id);
    session_start.continuation_readiness_nonce = Some(readiness_nonce);
    session_start.project_root = Some(fixture.repo.display().to_string());
    session_start.branch = Some("work/issue-2359".to_string());
    let (ack_spawner, ack_tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = ack_spawner;
    fixture.runtime.handle_runtime_hook_event(session_start);
    commit_pending_fresh_execution(&mut fixture.runtime);
    fixture.runtime.finish_queued_delivery_acks(&ack_tasks);

    let active_prefs =
        gwt::load_issue_monitor_prefs(&active_prefs_path).expect("reload active project prefs");
    assert_eq!(
        active_prefs.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 900,
            window_id: "tab-1::sentinel".to_string(),
        }],
        "the active execution project must not receive the monitor ACK"
    );
    let owner_prefs =
        gwt::load_issue_monitor_prefs(&owner_prefs_path).expect("reload monitor owner prefs");
    assert_eq!(
        owner_prefs.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: fixture.window_id,
        }],
        "authenticated SessionStart must bind the launch in its monitor owner project"
    );
    assert!(owner_prefs.launching_issues.is_empty());
}

/// SPEC #3200 FR-052: a fresh linked-owner launch is not producing until
/// SessionStart authenticates, so `spawn_agent_window_with_placement` defers
/// the Issue Monitor completion. The SessionStart finalizer therefore owns the
/// durable delivery ACK — if it emitted the plain "launch succeeded" events the
/// delivery tuple would stay pending forever and the daemon would redeliver it.
#[test]
fn fresh_execution_session_start_acks_durable_issue_monitor_launch_delivery() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-monitor-delivery");
    let delivery_id = "launch:effect-fresh-monitor-delivery";
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.repo);

    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        ..gwt::IssueMonitorConfig::default()
    });
    monitor.terminal_queue_push(&[fixture.owner.number], "operator", "2026-07-28T00:00:00Z");
    monitor.record_candidate(gwt::IssueMonitorIssue {
        number: fixture.owner.number,
        title: "fresh linked-owner launch".to_string(),
        labels: vec!["bug".to_string()],
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(
        fixture.owner.number,
        "claim-fresh-monitor-delivery",
        "host/session",
        "effect-fresh-monitor-delivery",
        "2026-07-28T00:00:00Z",
    ));
    assert!(monitor.claim_launch_delivery(
        fixture.owner.number,
        delivery_id,
        &fixture.runtime.issue_monitor_materializer_id,
        std::process::id(),
        &fixture.window_id,
        |_| true,
    ));
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs())
        .expect("seed the durable launch delivery");

    let pending = fixture
        .runtime
        .pending_fresh_execution_launches
        .get_mut(&fixture.window_id)
        .expect("pending fresh launch");
    pending.launch_feedback_context = Some(LaunchFeedbackContext {
        client_id: "__issue_monitor__".to_string(),
        title: "Issue Monitor".to_string(),
        issue_monitor_issue_number: Some(fixture.owner.number),
        issue_monitor_delivery_id: Some(delivery_id.to_string()),
        issue_monitor_project_root: Some(fixture.repo.clone()),
        issue_monitor_session_mode: None,
        issue_monitor_autonomous_handoff: None,
        issue_monitor_autonomous_submit_started: false,
        issue_monitor_review_dispatch: false,
    });
    let readiness_nonce = pending.readiness_nonce.clone();

    let (ack_spawner, ack_tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = ack_spawner;
    fixture
        .runtime
        .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&readiness_nonce));
    let events = commit_pending_fresh_execution(&mut fixture.runtime);

    assert!(
        !events.is_empty(),
        "an authenticated SessionStart must complete the fresh launch"
    );
    fixture.runtime.finish_queued_delivery_acks(&ack_tasks);
    let projection = gwt_core::workspace_projection::load_workspace_projection(&fixture.repo)
        .expect("load fresh current projection")
        .expect("fresh current projection");
    let agent = projection
        .latest_agent_for_session(&fixture.candidate_session_id)
        .expect("fresh launch Agent");
    assert!(
        agent.is_assigned(),
        "fresh linked-owner readiness must publish an assigned Work"
    );
    let work_id = agent.workspace_id.as_deref().expect("fresh Work id");
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&fixture.repo)
        .expect("load fresh WorkItems")
        .expect("fresh WorkItems");
    assert!(work_items.work_items.iter().any(|item| item.id == work_id
        && item
            .agents
            .iter()
            .any(|agent| agent.session_id == fixture.candidate_session_id)));
    assert!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("reload issue monitor prefs")
            .pending_launch_deliveries
            .is_empty(),
        "the fresh-execution finalizer must ACK the durable delivery it deferred"
    );
}

#[test]
fn fresh_execution_session_start_preserves_spec_owner_kind_in_work_projection() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture_with_owner_kind(
        temp.path(),
        "fresh-spec-owner",
        gwt::cli::execution_state::ExecutionOwnerKind::Spec,
    );
    let readiness_nonce = fixture
        .runtime
        .pending_fresh_execution_launches
        .get(&fixture.window_id)
        .expect("pending fresh launch")
        .readiness_nonce
        .clone();

    fixture
        .runtime
        .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&readiness_nonce));
    let events = commit_pending_fresh_execution(&mut fixture.runtime);

    assert!(!events.is_empty(), "SPEC launch must complete");
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&fixture.repo)
        .expect("load SPEC WorkItems")
        .expect("SPEC WorkItems");
    assert!(work_items.work_items.iter().any(|item| {
        item.owner.as_deref() == Some("SPEC-2359")
            && item
                .agents
                .iter()
                .any(|agent| agent.session_id == fixture.candidate_session_id)
    }));
}

// #3426: a stale Issue-kind resume context supplied to a SPEC-owner successor
// must not overwrite the canonical owner in the committed Work projection.
#[test]
fn fresh_execution_session_start_overrides_mis_kinded_resume_context_owner() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture_with_owner_kind(
        temp.path(),
        "fresh-mis-kinded-context",
        gwt::cli::execution_state::ExecutionOwnerKind::Spec,
    );
    let pending = fixture
        .runtime
        .pending_fresh_execution_launches
        .get_mut(&fixture.window_id)
        .expect("pending fresh launch");
    pending.resume_context = Some(WorkspaceResumeContext {
        title: Some("Stale launch title".to_string()),
        owner: Some("Issue #2359".to_string()),
        summary: Some("Stale summary".to_string()),
        next_action: None,
    });
    let readiness_nonce = pending.readiness_nonce.clone();

    fixture
        .runtime
        .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&readiness_nonce));
    let events = commit_pending_fresh_execution(&mut fixture.runtime);

    assert!(!events.is_empty(), "SPEC successor must complete");
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&fixture.repo)
        .expect("load WorkItems")
        .expect("WorkItems");
    let work = work_items
        .work_items
        .iter()
        .find(|item| {
            item.agents
                .iter()
                .any(|agent| agent.session_id == fixture.candidate_session_id)
        })
        .expect("successor Work");
    assert_eq!(
        work.owner.as_deref(),
        Some("SPEC-2359"),
        "canonical execution owner must override the mis-kinded resume context"
    );
    assert_eq!(
        work.title, "Stale launch title",
        "non-owner resume context fields stay presentation-owned"
    );
}

#[test]
fn fresh_execution_ready_timeout_aborts_candidate_and_preserves_blocked_predecessor() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-timeout-operation");

    // The fixture never spawned a PTY, so the base deadline sees no liveness
    // evidence and aborts without spending any extension budget.
    let events = fixture.runtime.handle_continue_work_ready_timeout(
        &fixture.window_id,
        &ContinueWorkReadinessWatch::new(fixture.operation_id.clone()),
    );

    assert!(!events.is_empty());
    assert_pending_fresh_execution_was_rolled_back(&fixture);
}

#[test]
fn fresh_execution_session_replacement_before_work_commit_preserves_predecessor_authority() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture =
        pending_fresh_execution_fixture(temp.path(), "fresh-session-replacement-before-commit");
    let sessions_dir = fixture.runtime.sessions_dir.clone();
    let session_id = fixture.candidate_session_id.clone();
    set_fresh_execution_pre_work_commit_hook_for_test(Box::new(move || {
        replace_fresh_candidate_session_incarnation(&sessions_dir, &session_id);
    }));

    let readiness_nonce = fixture
        .runtime
        .pending_fresh_execution_launches
        .get(&fixture.window_id)
        .expect("pending fresh launch")
        .readiness_nonce
        .clone();
    fixture
        .runtime
        .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&readiness_nonce));
    let events = commit_pending_fresh_execution(&mut fixture.runtime);

    assert!(
        events.is_empty(),
        "a replaced Session must not complete the fresh launch"
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read authority after Session replacement"),
        Some(fixture.predecessor_binding.clone()),
        "Session replacement must fence generation and Work publication"
    );
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.owner,
            &fixture.operation_id,
        )
        .expect("read continuation attempt")
        .expect("continuation attempt")
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
    );
}

#[test]
fn issue_monitor_scan_reconciles_activated_launch_but_preserves_unready_candidate() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _home_env = gwt_core::test_support::ScopedEnvVar::set("HOME", temp.path());
    let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", temp.path());
    for activated in [false, true] {
        let mut fixture = pending_fresh_execution_fixture(
            temp.path(),
            if activated {
                "monitor-scan-activated"
            } else {
                "monitor-scan-unready"
            },
        );
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.repo);
        gwt::save_issue_monitor_prefs(
            &prefs_path,
            &gwt::IssueMonitorPrefs {
                launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
                    issue_number: fixture.owner.number,
                    claimed_at: None,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        fixture
            .runtime
            .pending_fresh_execution_launches
            .get_mut(&fixture.window_id)
            .unwrap()
            .launch_feedback_context = Some(LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(fixture.owner.number),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: Some(fixture.repo.clone()),
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        });
        if activated {
            leave_fresh_execution_activated_before_projection_commit(&mut fixture);
        }
        let (spawner, queued_tasks) = BlockingTaskSpawner::queued();
        let (proxy, recorded_events) = AppEventProxy::stub();
        fixture.runtime.blocking_tasks = spawner;
        fixture.runtime.proxy = proxy;
        fixture
            .runtime
            .issue_monitor_scheduled_tick_events_at(&Utc::now().to_rfc3339());
        assert!(
            fixture
                .runtime
                .pending_fresh_execution_launches
                .contains_key(&fixture.window_id),
            "scan must defer durable repair and ACK until its blocking task completes"
        );
        assert_eq!(
            gwt::load_issue_monitor_prefs(&prefs_path)
                .unwrap()
                .launching_issues
                .len(),
            1
        );
        drain_queued_blocking_tasks(&queued_tasks);
        let completions = std::mem::take(&mut *recorded_events.lock().unwrap());
        let mut repaired = false;
        for event in completions {
            if let UserEvent::IssueMonitorFreshLaunchRepaired {
                window_id,
                operation_id,
                binding,
            } = event
            {
                repaired = true;
                // A completion for a replaced operation cannot acknowledge this window.
                assert!(fixture
                    .runtime
                    .handle_issue_monitor_fresh_launch_repaired(
                        &window_id,
                        "stale-operation",
                        &binding,
                    )
                    .is_empty());
                assert!(fixture
                    .runtime
                    .pending_fresh_execution_launches
                    .contains_key(&window_id));
                fixture.runtime.handle_issue_monitor_fresh_launch_repaired(
                    &window_id,
                    &operation_id,
                    &binding,
                );
            }
        }
        fixture.runtime.finish_queued_delivery_acks(&queued_tasks);
        assert_eq!(repaired, activated);
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert!(
            fixture
                .runtime
                .active_agent_sessions
                .contains_key(&fixture.window_id),
            "scan must preserve the live candidate"
        );
        assert_eq!(
            fixture
                .runtime
                .pending_fresh_execution_launches
                .contains_key(&fixture.window_id),
            !activated
        );
        if activated {
            assert!(prefs.launching_issues.is_empty());
            assert_eq!(
                prefs.launched_issues,
                vec![gwt::IssueMonitorLaunchedIssue {
                    issue_number: fixture.owner.number,
                    window_id: fixture.window_id
                }]
            );
            assert_eq!(
                gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
                    .unwrap(),
                Some(fixture.binding.identity)
            );
        } else {
            assert_eq!(prefs.launching_issues.len(), 1);
            assert!(prefs.launched_issues.is_empty());
            assert_eq!(
                gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
                    .unwrap(),
                Some(fixture.predecessor_binding)
            );
            assert!(durable_launch_recovery_exists(
                &fixture.runtime.sessions_dir,
                &fixture.candidate_session_id
            ));
        }
    }
}

#[cfg(windows)]
#[test]
fn fresh_monitor_powershell_session_start_activates_prepared_successor_and_acknowledges() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _home_env = gwt_core::test_support::ScopedEnvVar::set("HOME", temp.path());
    let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "monitor-powershell-ready");
    run_git(
        &fixture.repo,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
                issue_number: fixture.owner.number,
                claimed_at: None,
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let pending = fixture
        .runtime
        .pending_fresh_execution_launches
        .get_mut(&fixture.window_id)
        .unwrap();
    pending.launch_feedback_context = Some(LaunchFeedbackContext {
        client_id: "__issue_monitor__".to_string(),
        title: "Issue Monitor".to_string(),
        issue_monitor_issue_number: Some(fixture.owner.number),
        issue_monitor_delivery_id: None,
        issue_monitor_project_root: Some(fixture.repo.clone()),
        issue_monitor_session_mode: None,
        issue_monitor_autonomous_handoff: None,
        issue_monitor_autonomous_submit_started: false,
        issue_monitor_review_dispatch: false,
    });
    let nonce = pending.readiness_nonce.clone();
    let tokio = TokioRuntime::new().unwrap();
    let (proxy, events) = AppEventProxy::stub();
    let mut server = crate::embedded_server::EmbeddedServer::start(
        &tokio,
        proxy,
        crate::embedded_server::ClientHub::default(),
        Arc::clone(&fixture.runtime.pty_writers),
        AttachmentUploadStore::in_system_temp(),
    )
    .unwrap();
    let issuer = server.agent_capability_issuer();
    let target = issuer
        .issue_prepared(
            &fixture.repo,
            &fixture.candidate_session_id,
            fixture.binding.clone(),
        )
        .unwrap();
    fixture.runtime.agent_capability_issuer = Some(issuer.clone());
    fixture
        .runtime
        .agent_capability_tokens
        .insert(fixture.window_id.clone(), target.token.clone());
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("gwtd.exe");
    assert!(
        binary.is_file(),
        "build the checkout gwtd binary before this integration test"
    );
    let spaced_bin = temp.path().join("hook bin");
    fs::create_dir_all(&spaced_bin).unwrap();
    let hook_bin = spaced_bin.join("gwtd.exe");
    fs::copy(binary, &hook_bin).unwrap();
    gwt_skills::generate_codex_hooks_for_mode(
        &fixture.repo,
        gwt_skills::CodexHookDiscoveryMode::WorktreeLocal,
    )
    .unwrap();
    let hooks: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.repo.join(".codex/hooks.json")).unwrap()).unwrap();
    let command = hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let mut child = gwt_core::process::hidden_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .current_dir(&fixture.repo)
        .env(gwt_agent::GWT_BIN_PATH_ENV, &hook_bin)
        .env(gwt_agent::GWT_SESSION_ID_ENV, &fixture.candidate_session_id)
        .env(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            gwt_agent::runtime_state_path(
                &fixture.runtime.sessions_dir,
                &fixture.candidate_session_id,
            ),
        )
        .env(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV, nonce)
        .env(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, &target.url)
        .env(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, &target.token)
        .env_remove("CODEX_THREAD_ID")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::json!({
                "session_id": "native-monitor-powershell", "cwd": fixture.repo,
                "hook_event_name": "SessionStart", "source": "startup"
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let (ack_spawner, ack_tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = ack_spawner;
    let forwarded = std::mem::take(&mut *events.lock().unwrap());
    for event in forwarded {
        if let UserEvent::RuntimeHook(event) = event {
            fixture.runtime.handle_runtime_hook_event(event);
        }
    }
    commit_pending_fresh_execution(&mut fixture.runtime);
    fixture.runtime.finish_queued_delivery_acks(&ack_tasks);
    server.shutdown();
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner).unwrap(),
        Some(fixture.binding.identity.clone()),
        "generated startup hook must activate the prepared successor; shell status={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(issuer.active_token_is_current(&target.token, &fixture.binding));
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    assert!(prefs.launching_issues.is_empty());
    assert_eq!(
        prefs.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: fixture.owner.number,
            window_id: fixture.window_id,
        }]
    );
}

#[test]
fn fresh_execution_continue_resends_ready_and_commits_work() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    // HTTP workers must resolve the same isolated Session and trusted stores.
    let _home_env = gwt_core::test_support::ScopedEnvVar::set("HOME", temp.path());
    let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-continue-ready");
    run_git(
        &fixture.repo,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );
    let nonce = fixture.runtime.pending_fresh_execution_launches[&fixture.window_id]
        .readiness_nonce
        .clone();
    let tokio = TokioRuntime::new().unwrap();
    let (proxy, recorded_events) = AppEventProxy::stub();
    let mut server = crate::embedded_server::EmbeddedServer::start(
        &tokio,
        proxy,
        crate::embedded_server::ClientHub::default(),
        Arc::clone(&fixture.runtime.pty_writers),
        AttachmentUploadStore::in_system_temp(),
    )
    .unwrap();
    fixture.issuer = server.agent_capability_issuer();
    let target = fixture
        .issuer
        .issue_prepared(
            &fixture.repo,
            &fixture.candidate_session_id,
            fixture.binding.clone(),
        )
        .unwrap();
    fixture.token = target.token.clone();
    fixture.runtime.agent_capability_issuer = Some(fixture.issuer.clone());
    fixture
        .runtime
        .agent_capability_tokens
        .insert(fixture.window_id.clone(), target.token.clone());
    let url = reqwest::Url::parse(&target.url)
        .unwrap()
        .join("/internal/execution-continuation")
        .unwrap();
    let request = gwt::AgentExecutionContinuationRequest {
        schema_version: 1,
        operation_id: "continue-ready-request".to_string(),
        readiness_nonce: Some(nonce),
    };
    assert!(!format!("{request:?}").contains(request.readiness_nonce.as_deref().unwrap()));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let continue_via_host = |fixture: &mut PendingFreshExecutionFixture| {
        let send = client
            .post(url.clone())
            .bearer_auth(&target.token)
            .json(&request)
            .build()
            .unwrap();
        let request_client = client.clone();
        let response = thread::spawn(move || request_client.execute(send).unwrap());
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let event = {
                let mut events = recorded_events.lock().unwrap();
                events
                    .iter()
                    .position(|event| matches!(event, UserEvent::FreshExecutionReadyResend { .. }))
                    .map(|index| events.remove(index))
            };
            if let Some(UserEvent::FreshExecutionReadyResend {
                grant,
                request,
                reply,
            }) = event
            {
                let prepared = grant.principal().prepared_execution_binding().is_some();
                let (ready_reply, ready_result) = std::sync::mpsc::channel();
                fixture
                    .runtime
                    .resend_fresh_execution_ready(&grant, &request, ready_reply);
                if prepared {
                    assert!(
                        matches!(
                            ready_result.try_recv(),
                            Err(std::sync::mpsc::TryRecvError::Empty)
                        ),
                        "the same request must wait for its queued finalization"
                    );
                    let BlockingTaskSpawner::Queued(tasks) = &fixture.runtime.blocking_tasks else {
                        unreachable!();
                    };
                    let worker = tasks.lock().unwrap().remove(0);
                    worker();
                    let (active_reply, active_result) = std::sync::mpsc::channel();
                    fixture.runtime.resend_fresh_execution_ready(
                        &fixture.issuer.grant_for_test(&fixture.token).unwrap(),
                        &gwt::AgentExecutionContinuationRequest {
                            schema_version: 1,
                            operation_id: "active-inflight-retry".to_string(),
                            readiness_nonce: None,
                        },
                        active_reply,
                    );
                    assert!(
                        matches!(
                            active_result.try_recv(),
                            Err(std::sync::mpsc::TryRecvError::Empty)
                        ),
                        "Active resend must join the pending GUI completion"
                    );
                    commit_pending_fresh_execution(&mut fixture.runtime);
                    let active_receipt = active_result.try_recv().unwrap().unwrap().unwrap();
                    assert_eq!(active_receipt.operation_id, "active-inflight-retry");
                    assert_eq!(active_receipt.execution_binding, fixture.binding.identity);
                }
                reply
                    .send(
                        ready_result
                            .try_recv()
                            .expect("readiness completion must reply"),
                    )
                    .unwrap();
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Host did not dispatch readiness resend"
            );
            thread::sleep(Duration::from_millis(100));
        }
        response.join().unwrap()
    };
    let response = continue_via_host(&mut fixture);
    let status = response.status();
    let body = response.text().unwrap();
    assert!(status.is_success(), "{status}: {body}");
    let receipt: gwt::AgentExecutionContinuationReceipt = serde_json::from_str(&body).unwrap();
    assert_eq!(receipt.operation_id, "continue-ready-request");
    assert_eq!(receipt.execution_binding, fixture.binding.identity);
    assert!(receipt.validated);
    let works = gwt_core::workspace_projection::load_workspace_work_items(&fixture.repo)
        .unwrap()
        .unwrap();
    let work_id = gwt_core::workspace_projection::current_work_id(
        &works,
        &fixture.repo,
        Some("work/issue-2359"),
        Some(&fixture.repo),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["work_id"].as_str(),
        Some(work_id.as_str()),
        "successor_created must identify the Work committed by the readiness coordinator"
    );
    assert!(fixture
        .issuer
        .active_token_is_current(&fixture.token, &fixture.binding));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner).unwrap(),
        Some(fixture.binding.identity.clone())
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.owner)
        .unwrap()
        .unwrap();
    assert_eq!(ledger.generations.len(), 2);
    assert_eq!(
        ledger.generations[0].status,
        gwt::cli::execution_state::ExecutionControlStatus::Blocked
    );
    assert!(!fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    let update = client
        .post(url.join("/internal/workspace-update").unwrap())
        .bearer_auth(&target.token)
        .json(&gwt::AgentWorkspaceUpdateRequest {
            schema_version: gwt::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
            claimed_session_id: fixture.candidate_session_id.clone(),
            observation: gwt::AgentRuntimeObservation {
                cwd: fixture.repo.display().to_string(),
                git_toplevel: fixture.repo.display().to_string(),
                repo_hash: fixture.binding.repo_hash.clone(),
                branch: "work/issue-2359".to_string(),
            },
            intent: gwt::AgentWorkspaceUpdateIntent {
                current_focus: Some("verify fresh authority".to_string()),
                ..Default::default()
            },
        })
        .send()
        .unwrap();
    let status = update.status();
    let body = update.text().unwrap();
    assert!(
        status.is_success(),
        "workspace.update with the promoted capability: {status}: {body}"
    );
    let replay = continue_via_host(&mut fixture);
    let status = replay.status();
    let body = replay.text().unwrap();
    assert!(
        status.is_success(),
        "ready response-loss replay: {status}: {body}"
    );
    let replay: gwt::AgentExecutionContinuationReceipt = serde_json::from_str(&body).unwrap();
    assert_eq!(replay.generation_id, receipt.generation_id);
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.owner)
            .unwrap()
            .unwrap()
            .generations
            .len(),
        2
    );
    server.shutdown();
}

#[test]
fn fresh_execution_continue_repairs_activated_response_loss_before_acknowledging() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-continue-response-loss");
    run_git(
        &fixture.repo,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );
    leave_fresh_execution_activated_before_projection_commit(&mut fixture);
    let (reply, response) = std::sync::mpsc::channel();
    fixture.runtime.resend_fresh_execution_ready(
        &fixture.issuer.grant_for_test(&fixture.token).unwrap(),
        &gwt::AgentExecutionContinuationRequest {
            schema_version: 1,
            operation_id: "retry-ready-request".to_string(),
            readiness_nonce: None,
        },
        reply,
    );
    assert!(response
        .try_recv()
        .unwrap()
        .expect("Active capability retry must repair the matching pending fresh coordinator")
        .is_some());
    assert!(!fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner).unwrap(),
        Some(fixture.binding.identity)
    );
    assert!(!durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id
    ));
}

#[test]
fn fresh_execution_continue_refuses_wrong_nonce_without_mutation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-continue-wrong-nonce");
    run_git(
        &fixture.repo,
        &["symbolic-ref", "HEAD", "refs/heads/work/issue-2359"],
    );
    let before =
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.owner).unwrap();
    let diagnosis =
        gwt::cli::execution_state::diagnose(&fixture.repo, Some(&fixture.candidate_session_id));
    assert!(
        !diagnosis
            .available_recoveries
            .iter()
            .any(|operation| operation == "execution.adopt" || operation == "execution.continue"),
        "Prepared recovery needs Host readiness proof: {diagnosis:?}"
    );
    assert_eq!(
        diagnosis.recovery_hint.as_deref(),
        Some("prepared_launch_readiness_required")
    );
    let (reply, response) = std::sync::mpsc::channel();
    let events = fixture.runtime.resend_fresh_execution_ready(
        &fixture.issuer.grant_for_test(&fixture.token).unwrap(),
        &gwt::AgentExecutionContinuationRequest {
            schema_version: 1,
            operation_id: "continue-ready-request".to_string(),
            readiness_nonce: Some("wrong-nonce".to_string()),
        },
        reply,
    );
    assert!(response.try_recv().unwrap().is_err());
    assert!(events.is_empty());
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&fixture.repo, fixture.owner).unwrap(),
        before
    );
    assert!(fixture
        .issuer
        .prepared_token_is_current(&fixture.token, &fixture.binding));
    assert!(fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
}

#[test]
fn fresh_execution_wrong_session_start_nonce_aborts_candidate_and_preserves_blocked_predecessor() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-wrong-nonce");
    insert_test_pane_runtime(&mut fixture.runtime, &fixture.window_id);
    let pane = Arc::clone(&fixture.runtime.runtimes[&fixture.window_id].pane);
    let mut session_start =
        runtime_hook_state_for_event("Working", "SessionStart", &fixture.candidate_session_id);
    session_start.continuation_readiness_nonce = Some("wrong-readiness".to_string());
    session_start.project_root = Some(fixture.repo.display().to_string());
    session_start.branch = Some("work/issue-2359".to_string());

    let mut events = fixture.runtime.handle_runtime_hook_event(session_start);
    events.extend(commit_pending_fresh_execution(&mut fixture.runtime));
    let detached_teardown = Arc::strong_count(&pane) > 1;
    if let BlockingTaskSpawner::Queued(tasks) = &fixture.runtime.blocking_tasks {
        drain_queued_blocking_tasks(tasks);
    }
    assert!(
        detached_teardown,
        "rollback GUI apply must hand PTY ownership to the detached close finalizer"
    );
    assert!(load_tracked_work_events(&fixture.repo).is_empty());

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::TerminalStatus { detail: Some(detail), .. }
                if detail.contains("readiness nonce did not match")
        )),
        "unexpected nonce failure events: {events:#?}"
    );
    assert_pending_fresh_execution_was_rolled_back(&fixture);
}

#[test]
fn fresh_execution_missing_session_start_nonce_aborts_candidate_and_preserves_blocked_predecessor()
{
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-missing-nonce");
    let mut session_start =
        runtime_hook_state_for_event("Working", "SessionStart", &fixture.candidate_session_id);
    session_start.project_root = Some(fixture.repo.display().to_string());
    session_start.branch = Some("work/issue-2359".to_string());

    let mut events = fixture.runtime.handle_runtime_hook_event(session_start);
    events.extend(commit_pending_fresh_execution(&mut fixture.runtime));

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::TerminalStatus { detail: Some(detail), .. }
                if detail.contains("readiness nonce did not match")
        )),
        "missing nonce must produce a fail-closed diagnostic: {events:#?}"
    );
    assert_pending_fresh_execution_was_rolled_back(&fixture);
}

#[test]
fn fresh_execution_session_start_via_daemon_fanout_preserves_readiness_nonce() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-daemon-fanout");
    let readiness_nonce = fixture
        .runtime
        .pending_fresh_execution_launches
        .get(&fixture.window_id)
        .expect("pending fresh launch")
        .readiness_nonce
        .clone();
    let mut session_start =
        runtime_hook_state_for_event("Working", "SessionStart", &fixture.candidate_session_id);
    session_start.continuation_readiness_nonce = Some(readiness_nonce);
    session_start.project_root = Some(fixture.repo.display().to_string());
    session_start.branch = Some("work/issue-2359".to_string());

    let current_pid = std::process::id();
    let source_pid = current_pid.wrapping_add(1);
    let payload = gwt::runtime_daemon_events::runtime_hook_payload(&session_start, source_pid);
    let gwt::runtime_daemon_events::RuntimeDaemonEvent::Hook { event } =
        gwt::runtime_daemon_events::decode_runtime_daemon_event(
            gwt::runtime_daemon_events::RUNTIME_HOOK_CHANNEL,
            payload,
            current_pid,
        )
        .expect("decode foreign daemon fanout")
    else {
        panic!("expected runtime hook fanout");
    };

    let mut events = fixture.runtime.handle_daemon_runtime_hook_event(event);
    events.extend(commit_pending_fresh_execution(&mut fixture.runtime));

    assert!(
        !events.is_empty(),
        "activation must publish committed state"
    );
    assert!(!fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    assert!(fixture
        .runtime
        .active_agent_sessions
        .contains_key(&fixture.window_id));
    assert!(
        fixture
            .issuer
            .active_token_is_current(&fixture.token, &fixture.binding),
        "daemon fanout must activate the exact prepared capability"
    );
}

#[test]
fn fresh_execution_session_start_requires_gwt_session_identity_before_nonce_validation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-foreign-gwt-session");
    let readiness_nonce = fixture
        .runtime
        .pending_fresh_execution_launches
        .get(&fixture.window_id)
        .expect("pending fresh launch")
        .readiness_nonce
        .clone();
    let mut session_start =
        runtime_hook_state_for_event("Working", "SessionStart", "foreign-gwt-session");
    session_start.agent_session_id = Some(fixture.candidate_session_id.clone());
    session_start.continuation_readiness_nonce = Some(readiness_nonce);
    session_start.project_root = Some(fixture.repo.display().to_string());
    session_start.branch = Some("work/issue-2359".to_string());

    let events = fixture.runtime.handle_runtime_hook_event(session_start);

    assert!(fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    assert!(fixture
        .runtime
        .active_agent_sessions
        .contains_key(&fixture.window_id));
    assert!(
        fixture
            .issuer
            .prepared_token_is_current(&fixture.token, &fixture.binding),
        "foreign gwt identity must neither activate nor abort the candidate"
    );
    assert!(
        events.iter().all(|outbound| !matches!(
            &outbound.event,
            BackendEvent::TerminalStatus { id, .. } if id == &fixture.window_id
        )),
        "foreign gwt identity must not emit candidate status events: {events:#?}"
    );
}

#[test]
fn fresh_execution_worker_spawn_failure_aborts_its_reconstructed_candidate() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (mut fixture, recorded, tasks, mut completion, _) =
        queued_fresh_execution_completion_fixture(temp.path(), "fresh-worker-spawn-failure");
    completion.0.command = temp
        .path()
        .join("missing-agent-program")
        .display()
        .to_string();
    completion.0.args.clear();
    fixture
        .runtime
        .handle_launch_complete(fixture.window_id.clone(), Ok(completion));
    drain_queued_blocking_tasks(&tasks);
    let events = fixture
        .runtime
        .handle_agent_launch_prepared(take_prepared_agent_launch(&recorded));
    drain_queued_blocking_tasks(&tasks);
    assert!(
        !events.is_empty(),
        "the physical spawn failure remains visible"
    );
    assert_pending_fresh_execution_was_rolled_back(&fixture);
}

#[test]
fn fresh_execution_worker_dropped_completion_aborts_exact_prepared_candidate() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (mut fixture, recorded, tasks, completion, _) =
        queued_fresh_execution_completion_fixture(temp.path(), "fresh-worker-dropped");
    fixture
        .runtime
        .handle_launch_complete(fixture.window_id.clone(), Ok(completion));
    drain_queued_blocking_tasks(&tasks);
    drop(take_prepared_agent_launch(&recorded));
    drain_queued_blocking_tasks(&tasks);
    assert_pending_fresh_execution_was_rolled_back(&fixture);
}

#[test]
fn fresh_execution_worker_stale_completion_preserves_activated_generation() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let (mut fixture, recorded, tasks, completion, pending) =
        queued_fresh_execution_completion_fixture(temp.path(), "fresh-worker-activated");
    fixture
        .runtime
        .handle_launch_complete(fixture.window_id.clone(), Ok(completion));
    drain_queued_blocking_tasks(&tasks);
    let prepared = take_prepared_agent_launch(&recorded);
    let reader = super::super::launch::drain_prepared_fresh_launch_output_for_test(&prepared);
    gwt::cli::execution_state::activate_successor(&fixture.repo, fixture.owner, &pending.request)
        .expect("activate exact fresh candidate before stale completion");
    fixture.runtime.close_window_events(&fixture.window_id);
    fixture.runtime.handle_agent_launch_prepared(prepared);
    drain_queued_blocking_tasks(&tasks);
    reader.join().expect("fresh launch output reader");
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read activated generation"),
        Some(fixture.binding.identity.clone()),
    );
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.owner,
            &fixture.operation_id,
        )
        .expect("read activated attempt")
        .expect("activated attempt")
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Activated,
    );
    let session = gwt_agent::Session::load(
        &fixture
            .runtime
            .sessions_dir
            .join(format!("{}.toml", fixture.candidate_session_id)),
    )
    .expect("activated candidate remains discoverable");
    assert_eq!(session.execution_binding.as_ref(), Some(&fixture.binding));
    assert_eq!(session.status, gwt_agent::AgentStatus::Stopped);
    assert!(!session.restore_window_on_startup);
}

#[test]
fn fresh_execution_spawn_failure_aborts_candidate_and_preserves_blocked_predecessor() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-spawn-operation");

    let events = fixture.runtime.handle_launch_complete_and_drain(
        fixture.window_id.clone(),
        Err("candidate spawn failed".into()),
    );

    assert!(!events.is_empty());
    assert_pending_fresh_execution_was_rolled_back(&fixture);
}

#[test]
fn fresh_execution_generic_close_aborts_candidate_in_background() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture_with_predecessor_status(
        temp.path(),
        "fresh-generic-close",
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        true,
    );
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    let mut pending = fixture.runtime.pending_fresh_execution_launches[&fixture.window_id].clone();
    let mut candidate = gwt_agent::Session::load(
        &fixture
            .runtime
            .sessions_dir
            .join(format!("{}.toml", fixture.candidate_session_id)),
    )
    .expect("candidate seed");
    let mut active = fixture.runtime.active_agent_sessions[&fixture.window_id].clone();
    let baseline = fixture.runtime.window_scoped_state_entry_count()
        - fixture
            .runtime
            .window_scoped_state_residue(&fixture.window_id)
            .len();
    let root_pid = sysinfo::Pid::from_u32(std::process::id());
    let mut system = sysinfo::System::new();
    let mut rss_bytes = Vec::new();

    // AC-2/AC-3: the same host runtime must accept the next attempt without
    // retaining the failed candidate, PTY, writer, or window buffers.
    for round in 0..5 {
        if round > 0 {
            let runtime = &mut fixture.runtime;
            pending.operation_id = format!("fresh-generic-close-{round}");
            candidate
                .set_execution_binding(None)
                .expect("clear previous launch seed binding");
            candidate.id = format!("candidate-{}", pending.operation_id);
            pending.request.operation_id = pending.operation_id.clone();
            pending.request.initial_session_id = candidate.id.clone();
            pending.request.session_binding_id = format!("binding-{}", pending.operation_id);
            pending.request.requested_at = Utc::now();
            gwt::cli::execution_state::prepare_successor(
                &fixture.repo,
                fixture.owner,
                &pending.request,
            )
            .expect("prepare next launch after failed close");
            pending.binding.identity =
                gwt::cli::execution_state::prepared_successor_execution_binding(
                    &fixture.repo,
                    fixture.owner,
                    &pending.request,
                )
                .expect("next candidate binding");
            pending.binding.session_id = candidate.id.clone();
            candidate
                .set_execution_binding(Some(pending.binding.clone()))
                .expect("bind next candidate");
            candidate
                .save(&runtime.sessions_dir)
                .expect("save next candidate");
            pending.session_identity =
                gwt_agent::SessionExecutionIdentity::for_binding(&candidate, &pending.binding)
                    .expect("next candidate identity");
            persist_durable_launch_recovery(
                &runtime.sessions_dir,
                DurableLaunchRecoveryKind::FreshSuccessor {
                    operation_id: pending.operation_id.clone(),
                },
                &candidate.id,
                &fixture.repo,
                &fixture.repo,
                fixture.owner,
                Some(&pending.binding),
                Some(&gwt_agent::AgentId::Codex),
            )
            .expect("persist next recovery receipt");
            let capability = fixture
                .issuer
                .issue_prepared(&fixture.repo, &candidate.id, pending.binding.clone())
                .expect("issue next Prepared capability");
            let raw_id = runtime
                .tab_mut("tab-1")
                .unwrap()
                .workspace
                .add_window(WindowPreset::Agent, canvas_bounds())
                .id;
            runtime.register_window("tab-1", &raw_id);
            fixture.window_id = combined_window_id("tab-1", &raw_id);
            fixture.operation_id = pending.operation_id.clone();
            fixture.candidate_session_id = candidate.id.clone();
            fixture.binding = pending.binding.clone();
            fixture.token = capability.token.clone();
            active.window_id = fixture.window_id.clone();
            active.session_id = candidate.id.clone();
            runtime
                .active_agent_sessions
                .insert(fixture.window_id.clone(), active.clone());
            runtime
                .agent_capability_tokens
                .insert(fixture.window_id.clone(), capability.token);
            runtime
                .pending_fresh_execution_launches
                .insert(fixture.window_id.clone(), pending.clone());
            runtime.launch_wizard_cache = LaunchWizardMemoryCache::load(&runtime.sessions_dir);
        }
        insert_test_pane_runtime(&mut fixture.runtime, &fixture.window_id);
        let pty = fixture.runtime.runtimes[&fixture.window_id].pty.clone();
        let closed = fixture.runtime.close_window_outcome(&fixture.window_id);
        assert!(closed.closed);
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &fixture.repo,
                fixture.owner,
                &fixture.operation_id,
            )
            .unwrap()
            .unwrap()
            .status,
            gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
            "close acceptance must not perform durable rollback",
        );
        assert!(pty.try_wait().expect("probe live child").is_none());
        for task in std::mem::take(&mut *tasks.lock().expect("queued finalizers")) {
            task();
        }
        assert!(pty.try_wait().expect("probe reaped child").is_some());
        drop(pty);
        assert_pending_fresh_execution_was_rolled_back(&fixture);
        assert_eq!(fixture.runtime.window_scoped_state_entry_count(), baseline);
        assert!(fixture.runtime.pty_writers.read().unwrap().is_empty());
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[root_pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing().with_memory(),
        );
        rss_bytes.push(
            system
                .process(root_pid)
                .expect("measured host process")
                .memory(),
        );
    }
    eprintln!(
        "Issue #4959 AC-3: {}",
        serde_json::json!({
            "failed_launches": 5,
            "host_rss_bytes_after_cleanup": rss_bytes,
            "retained_window_entries": fixture.runtime.window_scoped_state_entry_count(),
            "retained_pty_writers": fixture.runtime.pty_writers.read().unwrap().len(),
        })
    );
}

#[test]
fn fresh_execution_generic_close_preserves_activated_or_replaced_candidate() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    for activated in [false, true] {
        let operation = if activated {
            "close-activated"
        } else {
            "close-replaced"
        };
        let mut fixture = pending_fresh_execution_fixture(temp.path(), operation);
        let request = fixture.runtime.pending_fresh_execution_launches[&fixture.window_id]
            .request
            .clone();
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        fixture.runtime.blocking_tasks = spawner;
        let closed = fixture.runtime.close_window_outcome(&fixture.window_id);
        assert!(closed.closed);
        if activated {
            gwt::cli::execution_state::activate_successor(&fixture.repo, fixture.owner, &request)
                .expect("activation wins before finalizer");
        } else {
            replace_fresh_candidate_session_incarnation(
                &fixture.runtime.sessions_dir,
                &fixture.candidate_session_id,
            );
        }
        let candidate_path = fixture
            .runtime
            .sessions_dir
            .join(format!("{}.toml", fixture.candidate_session_id));
        let before = fs::read(&candidate_path).expect("candidate before finalizer");
        for task in std::mem::take(&mut *tasks.lock().expect("queued finalizers")) {
            task();
        }
        assert_eq!(
            fs::read(&candidate_path).expect("retained candidate"),
            before
        );
        assert!(durable_launch_recovery_exists(
            &fixture.runtime.sessions_dir,
            &fixture.candidate_session_id
        ));
        assert_eq!(
            gwt::cli::execution_state::continuation_attempt_for_operation(
                &fixture.repo,
                fixture.owner,
                &fixture.operation_id,
            )
            .unwrap()
            .unwrap()
            .status,
            if activated {
                gwt::cli::execution_state::ContinuationAttemptStatus::Activated
            } else {
                gwt::cli::execution_state::ContinuationAttemptStatus::Prepared
            },
        );
    }
}

#[test]
fn automatic_resume_successor_created_installs_active_authority_before_pty_spawn() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_codex = write_fake_codex(temp.path());
    let _path = prepend_tool_parent_to_path(&fake_codex);
    let repo = temp.path().join("repo-auto-resume-successor");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    run_git(&repo, &["branch", "-M", "feature/demo"]);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-successor",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let (mut runtime, _runtime_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    fs::create_dir_all(&runtime.sessions_dir).expect("create canonical sessions dir");
    let predecessor_session_id = "auto-resume-successor-predecessor";
    let predecessor_identity = install_manual_launch_holder(
        &mut runtime,
        &repo,
        predecessor_session_id,
        gwt_agent::AgentStatus::Interrupted,
        None,
    );
    let predecessor_path = runtime
        .sessions_dir
        .join(format!("{predecessor_session_id}.toml"));
    let mut predecessor =
        gwt_agent::Session::load(&predecessor_path).expect("load predecessor Session");
    predecessor.agent_session_id = Some("native-auto-resume-successor".to_string());
    predecessor.restore_window_on_startup = true;
    predecessor
        .save(&runtime.sessions_dir)
        .expect("persist resumable predecessor Session");

    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            predecessor_session_id,
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle predecessor generation"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    // Issue #3625 explicitly excludes stale sidecar fencing, which has its
    // own startup-recovery owner. Keep this fixture focused on the activated
    // SuccessorCreated binding while retaining the durable predecessor Session.
    fs::remove_file(gwt_agent::runtime_state_path(
        &runtime.sessions_dir,
        predecessor_session_id,
    ))
    .expect("remove out-of-scope stale predecessor runtime proof");
    let continuation_diagnosis =
        gwt::cli::execution_state::diagnose(&repo, Some(predecessor_session_id));
    assert!(
        continuation_diagnosis
            .available_recoveries
            .iter()
            .any(|operation| operation == "execution.continue"),
        "terminal predecessor must be eligible for successor continuation: {continuation_diagnosis:#?}"
    );

    let mut config = super::super::launch_config_from_persisted_session(&predecessor);
    assert_eq!(
        config.linked_issue_number,
        Some(42),
        "the Issue #3625 regression fixture must reach Prepared validation before failing"
    );
    config.command = fake_codex.display().to_string();
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45155/internal/hook-live",
        "ws://127.0.0.1:46255/ws",
        "ws://127.0.0.1:45155/internal/pane-ws",
    );
    runtime.agent_capability_issuer = Some(issuer.clone());
    let window_id = combined_window_id("tab-1", "agent-successor");
    let (proxy, launch_events) = AppEventProxy::stub();

    AppRuntime::spawn_agent_window_async(
        proxy,
        runtime.sessions_dir.clone(),
        repo.display().to_string(),
        window_id.clone(),
        config,
        temp.path().join("missing-profile-config.toml"),
        Some(issuer.clone()),
    );

    wait_for_recorded_event(
        "SuccessorCreated launch completion",
        &launch_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchComplete {
                        window_id: event_window_id,
                        ..
                    } if event_window_id == &window_id
                )
            })
        },
    );
    let recorded = launch_events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let completion = recorded
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchComplete {
                window_id: event_window_id,
                result,
            } if event_window_id == &window_id => Some(result.as_ref().clone()),
            _ => None,
        })
        .expect("SuccessorCreated LaunchComplete event")
        .unwrap_or_else(|error| {
            panic!("SuccessorCreated auto-resume must reach PTY handoff: {error}")
        });

    assert!(
        !completion.10,
        "an already-Activated successor must not be classified as Prepared"
    );
    assert!(
        runtime.runtimes.is_empty(),
        "the launch worker must install authority before PTY spawn"
    );
    let successor =
        gwt_agent::Session::load(&runtime.sessions_dir.join(format!("{}.toml", completion.1)))
            .expect("load activated successor Session");
    let successor_identity = gwt_agent::SessionExecutionIdentity::from_session(&successor)
        .expect("validate successor Session identity")
        .expect("activated successor identity");
    assert_eq!(
        successor.linked_issue_number,
        Some(42),
        "the authenticated continuation owner must be installed before the binding"
    );
    assert_ne!(
        successor_identity.execution_binding.identity.generation_id,
        predecessor_identity
            .execution_binding
            .identity
            .generation_id,
        "SuccessorCreated must advance the execution generation"
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(
            &repo,
            gwt::cli::execution_state::ExecutionOwnerKey {
                kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 42,
            },
        )
        .expect("read current successor binding"),
        Some(successor_identity.execution_binding.identity.clone()),
    );
    assert_eq!(
        completion.11.expected_execution_identity.as_ref(),
        Some(&successor_identity),
        "the PTY handoff must carry the exact activated successor identity"
    );
    assert!(
        completion.11.active_launch_handshake.is_some(),
        "an in-place Active relaunch must be fenced before capability issuance"
    );
    let token = completion
        .0
        .env
        .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
        .expect("activated successor capability token");
    assert!(issuer.active_token_is_current(token, &successor_identity.execution_binding,));
    assert!(!issuer.prepared_token_is_current(token, &successor_identity.execution_binding,));
    let grant = issuer
        .grant_for_test(token)
        .expect("authenticate activated successor capability");
    assert!(grant.principal().authorizes_producing_mutation());

    let events = runtime.handle_launch_complete_and_drain(window_id.clone(), Ok(completion));
    assert!(events.iter().all(|event| !matches!(
        &event.event,
        BackendEvent::TerminalStatus {
            status: WindowProcessStatus::Error,
            ..
        }
    )));
    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(runtime.active_agent_sessions.contains_key(&window_id));
    let running = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(
        &runtime.sessions_dir,
        predecessor_session_id,
    ))
    .expect("load running successor runtime proof");
    assert_eq!(running.status, gwt_agent::AgentStatus::Running);
    assert_eq!(
        running.execution_identity.as_ref(),
        Some(&successor_identity)
    );
    assert!(running.child_pid.is_some_and(|pid| pid > 0));
    assert!(running
        .child_started_at
        .is_some_and(|started_at| started_at > 0));
}

#[cfg(unix)]
#[test]
fn manual_terminal_launch_persists_recovery_before_prepared_readiness() {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-manual-terminal");
    let bin = temp.path().join("bin");
    let runtime_root = temp.path().join(".gwt");
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&bin).expect("create bin");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    init_repo(&repo);
    let direct = bin.join("codex");
    fs::write(&direct, "#!/bin/sh\nprintf 'codex-cli 1.0.0\\n'\n").expect("write healthy runner");
    fs::set_permissions(&direct, fs::Permissions::from_mode(0o755)).expect("chmod healthy runner");
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3547,
    };
    let predecessor_session_id = "manual-terminal-predecessor";
    let mut predecessor =
        gwt_agent::Session::new(&repo, "work/issue-3547", gwt_agent::AgentId::Codex);
    predecessor.id = predecessor_session_id.to_string();
    predecessor.project_state_root = Some(repo.clone());
    predecessor.linked_issue_number = Some(owner.number);
    predecessor.status = gwt_agent::AgentStatus::Stopped;
    predecessor
        .save(&sessions_dir)
        .expect("save terminal predecessor");
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        predecessor_session_id,
        "$gwt-execute #3547",
        false,
    )
    .expect("materialize predecessor authority");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize predecessor generation");
    let predecessor_binding = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read predecessor binding")
        .expect("predecessor binding");
    predecessor
        .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: predecessor_session_id.to_string(),
            repo_hash: predecessor
                .repo_hash
                .clone()
                .expect("predecessor repository hash"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: predecessor_binding.clone(),
            capability_generation: 1,
        }))
        .expect("bind predecessor Session");
    predecessor
        .save(&sessions_dir)
        .expect("persist bound predecessor");
    let predecessor_identity = gwt_agent::SessionExecutionIdentity::from_session(&predecessor)
        .expect("validate predecessor identity")
        .expect("predecessor identity");
    gwt_agent::SessionRuntimeState::for_execution_process(
        gwt_agent::AgentStatus::Stopped,
        &predecessor_identity,
        1,
        gwt::process::host_process_start_time(std::process::id())
            .expect("test Host process start time"),
        i32::MAX as u32,
        1,
    )
    .save(&gwt_agent::runtime_state_path(
        &sessions_dir,
        predecessor_session_id,
    ))
    .expect("persist exact terminal runtime sidecar");
    let operation_id = "manual-terminal-launch-operation";
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&repo)
        .branch("work/issue-3547")
        .linked_issue_number(owner.number)
        .extra_arg("$gwt-execute #3547")
        .build();
    config.command = direct.display().to_string();
    config
        .env_vars
        .insert("PATH".to_string(), bin.display().to_string());
    let candidate_session_id = "manual-terminal-successor";
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: operation_id.to_string(),
        principal_id: "gwt-host-manual-launch".to_string(),
        work_id: None,
        source: gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
        session_binding_id: "manual-terminal-successor-binding".to_string(),
        initial_session_id: candidate_session_id.to_string(),
        entrypoint: "$gwt-execute".to_string(),
        requested_at: chrono::Utc::now(),
    };
    persist_durable_launch_recovery(
        &sessions_dir,
        DurableLaunchRecoveryKind::FreshSuccessor {
            operation_id: operation_id.to_string(),
        },
        candidate_session_id,
        &repo,
        &repo,
        owner,
        None,
        None,
    )
    .expect("persist pre-prepare recovery receipt");
    gwt::cli::execution_state::prepare_exact_manual_launch_successor(
        &repo,
        owner,
        &request,
        gwt::cli::execution_state::ExactManualLaunchPredecessor {
            sessions_dir: &sessions_dir,
            session: Some(&predecessor_identity),
            runtime: Some(gwt_agent::ManualLaunchRuntimeEvidence::Proof(
                gwt_agent::ManualLaunchRuntimeProof {
                    host_pid: std::process::id(),
                    runtime_incarnation: 1,
                },
            )),
            binding: &predecessor_identity.execution_binding.identity,
            status: gwt::cli::execution_state::SuccessorPredecessorStatus::Active,
            terminal_reason: "exact producing runtime terminated before manual Launch Agent",
        },
    )
    .expect("prepare exact terminal successor");
    let successor_identity =
        gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
            .expect("derive Prepared successor binding");
    config.execution_intent = gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(
        gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: candidate_session_id.to_string(),
            repo_hash: predecessor
                .repo_hash
                .clone()
                .expect("predecessor repository hash"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: successor_identity,
            capability_generation: 1,
        },
    );
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45155/internal/hook-live",
        "ws://127.0.0.1:46255/ws",
        "ws://127.0.0.1:45155/internal/pane-ws",
    );
    let (proxy, events) = AppEventProxy::stub();

    AppRuntime::spawn_agent_window_async(
        proxy,
        sessions_dir.clone(),
        repo.display().to_string(),
        "tab-1::agent-manual-terminal".to_string(),
        config,
        temp.path().join("missing-profile-config.toml"),
        Some(issuer.clone()),
    );

    let recorded = events.lock().expect("event log").clone();
    let completion = recorded
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchComplete { result, .. } => result.as_ref().as_ref().ok().cloned(),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!("successful manual terminal LaunchComplete event: {recorded:#?}")
        });
    let candidate_session_id = completion.1.clone();
    let readiness_nonce = completion
        .0
        .env
        .get(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV)
        .cloned()
        .expect("manual successor readiness nonce");
    assert!(!readiness_nonce.is_empty());
    assert!(durable_launch_recovery_exists(
        &sessions_dir,
        &candidate_session_id,
    ));
    let candidate =
        gwt_agent::Session::load(&sessions_dir.join(format!("{candidate_session_id}.toml")))
            .expect("persisted manual successor Session");
    let binding = candidate
        .execution_binding
        .as_ref()
        .expect("Prepared manual successor binding");
    let candidate_execution_identity =
        gwt_agent::SessionExecutionIdentity::from_session(&candidate)
            .expect("validate Prepared manual successor identity")
            .expect("Prepared manual successor identity");
    assert_eq!(
        completion.11.expected_execution_identity.as_ref(),
        Some(&candidate_execution_identity),
        "the launch worker must carry the exact persisted identity to the PTY handoff",
    );
    assert!(issuer.prepared_token_is_current(
        completion
            .0
            .env
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .expect("Prepared capability token"),
        binding,
    ));
    let current_after_prepare = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read current predecessor after Prepared launch")
        .expect("terminal predecessor remains current");
    assert_eq!(
        current_after_prepare.generation_id, predecessor_binding.generation_id,
        "readiness preparation must not activate the successor generation",
    );
    assert_eq!(
        current_after_prepare.binding_id, predecessor_binding.binding_id,
        "readiness preparation must preserve the predecessor binding",
    );
    let attempt =
        gwt::cli::execution_state::continuation_attempt_for_operation(&repo, owner, operation_id)
            .expect("read manual successor attempt")
            .expect("manual successor attempt");
    assert_eq!(
        attempt.status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
    );

    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-manual-terminal",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Starting,
    );
    let mut runtime = sample_runtime(&runtime_root, vec![tab], Some("tab-1"));
    runtime.agent_capability_issuer = Some(issuer);
    let window_id = combined_window_id("tab-1", "agent-manual-terminal");
    let launch_events = runtime.handle_launch_complete_and_drain(window_id.clone(), Ok(completion));
    let runtime_incarnation = runtime
        .runtimes
        .get(&window_id)
        .expect("manual successor PTY runtime")
        .incarnation;
    let running_runtime = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(
        &sessions_dir,
        &candidate_session_id,
    ))
    .expect("load exact Prepared manual successor runtime state");
    assert_eq!(
        running_runtime.execution_identity.as_ref(),
        Some(&candidate_execution_identity),
    );
    assert_eq!(
        running_runtime.runtime_incarnation,
        Some(runtime_incarnation),
    );
    let pending = runtime
        .pending_fresh_execution_launches
        .get(&window_id)
        .expect("manual successor must reuse fresh readiness finalization");
    assert_eq!(pending.operation_id, operation_id);
    assert_eq!(pending.readiness_nonce, readiness_nonce);
    assert!(launch_events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::TerminalStatus { detail: Some(detail), .. }
            if detail == "Waiting for authenticated SessionStart..."
    )));
    let candidate_binding = pending.binding.identity.clone();
    let (spawner, _) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let queued_events =
        runtime.finalize_fresh_execution_launch_session_start(&window_id, Some(&readiness_nonce));
    assert!(queued_events.is_empty());
    let ready_events = commit_pending_fresh_execution(&mut runtime);
    assert!(
        !ready_events.is_empty(),
        "authenticated readiness must finish the manual successor"
    );
    let activated = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read activated manual successor");
    assert_eq!(
        activated,
        Some(candidate_binding),
        "manual readiness did not activate: events={ready_events:#?}; attempt={:?}",
        gwt::cli::execution_state::continuation_attempt_for_operation(&repo, owner, operation_id,)
            .expect("read post-ready attempt"),
    );
}

#[cfg(unix)]
#[test]
fn production_host_launch_all_runner_failure_leaves_no_session_or_success_dispatch() {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-runner-failure");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&bin).expect("create bin");
    init_repo(&repo);
    let direct = bin.join("openclaw");
    fs::write(&direct, "#!/bin/sh\necho 'runner broken' >&2\nexit 1\n")
        .expect("write broken direct runner");
    fs::set_permissions(&direct, fs::Permissions::from_mode(0o755))
        .expect("chmod broken direct runner");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::OpenClaw)
        .working_dir(&repo)
        .branch("work/issue-2359")
        .session_mode(gwt_agent::SessionMode::Continue)
        .build();
    config.command = direct.display().to_string();
    config
        .env_vars
        .insert("PATH".to_string(), bin.display().to_string());
    config
        .env_vars
        .insert("HOME".to_string(), temp.path().display().to_string());
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45155/internal/hook-live",
        "ws://127.0.0.1:46255/ws",
        "ws://127.0.0.1:45155/internal/pane-ws",
    );
    let (proxy, events) = AppEventProxy::stub();

    AppRuntime::spawn_agent_window_async(
        proxy,
        sessions_dir.clone(),
        repo.display().to_string(),
        "tab-1::agent-runner-failure".to_string(),
        config,
        temp.path().join("missing-profile-config.toml"),
        Some(issuer),
    );

    let recorded = events.lock().expect("event log");
    let error = recorded
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchComplete { result, .. } => result.as_ref().as_ref().err(),
            _ => None,
        })
        .expect("failed LaunchComplete event");
    assert!(error.detail.contains("OpenClaw"), "{error}");
    assert!(error.detail.contains("exit status 1"), "{error}");
    assert!(recorded
        .iter()
        .all(|event| !matches!(recorded_project_payload(event), UserEvent::LaunchComplete { result, .. } if result.is_ok())));
    let persisted_sessions = fs::read_dir(&sessions_dir)
        .expect("read sessions dir")
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect::<Vec<_>>();
    assert!(
        persisted_sessions.is_empty(),
        "runner-health failure must precede Session persistence: {persisted_sessions:?}"
    );
}

#[cfg(unix)]
#[test]
fn production_codex_health_failure_runs_once_before_managed_asset_or_session_mutation() {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-codex-health-order");
    let bin = temp.path().join("bin");
    let invocation_counter = temp.path().join("codex-invocations.txt");
    fs::create_dir_all(&repo).expect("create repo");
    fs::create_dir_all(&bin).expect("create bin");
    init_repo(&repo);
    let direct = bin.join("codex");
    fs::write(
        &direct,
        "#!/bin/sh\nprintf 'probe\\n' >> \"${0%/*}/../codex-invocations.txt\"\n/bin/sleep 8\necho 'runner broken' >&2\nexit 1\n",
    )
    .expect("write broken Codex runner");
    fs::set_permissions(&direct, fs::Permissions::from_mode(0o755))
        .expect("chmod broken Codex runner");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&repo)
        .branch("work/issue-2359")
        .build();
    config.command = direct.display().to_string();
    config.env_vars.extend([
        ("PATH".to_string(), bin.display().to_string()),
        ("HOME".to_string(), temp.path().display().to_string()),
    ]);
    let (proxy, events) = AppEventProxy::stub();

    let started = std::time::Instant::now();
    AppRuntime::spawn_agent_window_async(
        proxy,
        sessions_dir.clone(),
        repo.display().to_string(),
        "tab-1::agent-codex-health-order".to_string(),
        config,
        temp.path().join("missing-profile-config.toml"),
        None,
    );
    let elapsed = started.elapsed();

    let recorded = events.lock().expect("event log");
    assert!(recorded
        .iter()
        .any(|event| matches!(recorded_project_payload(event), UserEvent::LaunchComplete { result, .. } if result.is_err())));
    assert_eq!(
        fs::read_to_string(&invocation_counter)
            .expect("Codex invocation counter")
            .lines()
            .count(),
        1,
        "canonical health must be the only Codex version invocation",
    );
    assert!(
        elapsed >= std::time::Duration::from_secs(4),
        "production launch must not return before the canonical five-second deadline: {elapsed:?}",
    );
    assert!(
        !repo.join(".codex").exists(),
        "runner health must fail before managed Codex asset mutation",
    );
    assert!(
        fs::read_dir(&sessions_dir)
            .expect("read sessions dir")
            .flatten()
            .all(|entry| entry.path().extension().and_then(|ext| ext.to_str()) != Some("toml")),
        "runner health must fail before Session persistence",
    );
}

#[test]
fn failed_precommit_fresh_launch_does_not_orphan_its_persisted_session() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-precommit-failure");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        "blocked-precommit-session",
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize predecessor");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            "blocked-precommit-session",
            gwt::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "precommit regression fixture".to_string(),
                missing_verification: Some("fresh launch pending".to_string()),
            },
        )
        .expect("settle Blocked predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import Blocked predecessor");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&repo)
        .branch("work/issue-2359")
        .linked_issue_number(owner.number)
        .extra_arg("$gwt-execute #2359")
        .build();
    pin_config_fixture_runners(&mut config, temp.path(), &["codex", "npx", "bunx"]);
    let (proxy, events) = AppEventProxy::stub();

    AppRuntime::spawn_agent_window_async(
        proxy,
        sessions_dir.clone(),
        repo.display().to_string(),
        "tab-1::agent-precommit".to_string(),
        config,
        temp.path().join("missing-profile-config.toml"),
        None,
    );

    let launch_error = events
        .lock()
        .expect("event log")
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchComplete { result, .. } => result.as_ref().as_ref().err().cloned(),
            _ => None,
        })
        .expect("failed LaunchComplete event");
    assert!(launch_error.detail.contains("Host capability issuer"));
    let persisted_candidates = fs::read_dir(&sessions_dir)
        .expect("read sessions dir")
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect::<Vec<_>>();
    assert!(
        persisted_candidates.is_empty(),
        "a launch that never returned its Session id must not leave an undiscoverable candidate: {persisted_candidates:?}"
    );
    let recovery_receipts = fs::read_dir(sessions_dir.join("execution-launch-recovery"))
        .map(|entries| entries.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    assert!(
        recovery_receipts.is_empty(),
        "preflight failure must not publish a pending launch receipt: {recovery_receipts:?}",
    );
}

#[test]
fn genesis_final_runtime_persistence_failure_terminalizes_generation() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-final-persist");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    fs::write(sessions_dir.join("runtime"), "not-a-directory")
        .expect("block runtime sidecar directory creation");
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&repo)
        .branch("work/issue-2359")
        .linked_issue_number(owner.number)
        .extra_arg("$gwt-execute #2359")
        .build();
    pin_config_fixture_runners(&mut config, temp.path(), &["codex", "npx", "bunx"]);
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:45155/internal/hook-live",
        "ws://127.0.0.1:46255/ws",
        "ws://127.0.0.1:45155/internal/pane-ws",
    );
    let (proxy, events) = AppEventProxy::stub();

    AppRuntime::spawn_agent_window_async(
        proxy,
        sessions_dir.clone(),
        repo.display().to_string(),
        "tab-1::agent-genesis-final-persist".to_string(),
        config,
        temp.path().join("missing-profile-config.toml"),
        Some(issuer),
    );

    let error = events
        .lock()
        .expect("event log")
        .iter()
        .find_map(|event| match event {
            UserEvent::LaunchComplete { result, .. } => result.as_ref().as_ref().err().cloned(),
            _ => None,
        })
        .expect("failed LaunchComplete event");
    assert!(
        error.detail.contains("runtime") || error.detail.contains("directory"),
        "{error}"
    );
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read failed genesis ledger")
        .expect("failed genesis ledger");
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
    );
    let persisted_sessions = fs::read_dir(&sessions_dir)
        .expect("read sessions dir")
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("toml"))
        .collect::<Vec<_>>();
    assert_eq!(
        persisted_sessions.len(),
        1,
        "cleanup I/O failure must retain exact Session evidence: {persisted_sessions:?}",
    );
    let retained_session_id = persisted_sessions[0]
        .path()
        .file_stem()
        .and_then(|value| value.to_str())
        .expect("retained Session id")
        .to_string();
    let recovery_dir = sessions_dir.join("execution-launch-recovery");
    let recovery_receipts = fs::read_dir(&recovery_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry.path().extension().and_then(|ext| ext.to_str()) == Some("json")
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert_eq!(recovery_receipts.len(), 1, "{recovery_receipts:?}");
    assert!(
        recovery_dir
            .join(format!("{retained_session_id}.lock"))
            .exists(),
        "the durable receipt keeps its exact cross-process lock file"
    );

    fs::remove_file(sessions_dir.join("runtime")).expect("remove runtime path blocker");
    let mut restarted = sample_runtime(temp.path(), Vec::new(), None);
    restarted.reconcile_durable_fresh_execution_launches();

    assert!(!sessions_dir
        .join(format!("{retained_session_id}.toml"))
        .exists());
    assert!(!durable_launch_recovery_exists(
        &sessions_dir,
        &retained_session_id,
    ));
}

#[test]
fn fresh_execution_prevalidation_conflict_leaves_work_writable_and_retains_candidate() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-callback-failure");
    fixture
        .runtime
        .pending_fresh_execution_launches
        .get_mut(&fixture.window_id)
        .expect("pending fresh launch")
        .request
        .principal_id = "mismatched-process-local-principal".to_string();
    let readiness_nonce = fixture
        .runtime
        .pending_fresh_execution_launches
        .get(&fixture.window_id)
        .expect("pending fresh launch")
        .readiness_nonce
        .clone();

    fixture
        .runtime
        .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&readiness_nonce));
    let _events = commit_pending_fresh_execution(&mut fixture.runtime);

    let attempt = gwt::cli::execution_state::continuation_attempt_for_operation(
        &fixture.repo,
        fixture.owner,
        &fixture.operation_id,
    )
    .expect("read durable attempt")
    .expect("durable attempt");
    assert_eq!(
        attempt.status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Prepared,
        "the callback refusal must not be misclassified as a durable abort"
    );
    assert!(
        fixture
            .runtime
            .sessions_dir
            .join(format!("{}.toml", fixture.candidate_session_id))
            .exists(),
        "cleanup must retain the candidate Session until Aborted is durably proven"
    );
    assert!(durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));

    let mut unrelated = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "work-unrelated-after-fresh-callback-failure",
        Utc::now(),
    );
    unrelated.title = Some("Unrelated writer after rejected callback".to_string());
    gwt_core::workspace_projection::record_workspace_work_event(&fixture.repo, unrelated)
        .expect("prevalidation refusal must not create a partial Work transaction");
}

#[cfg(unix)]
#[test]
fn fresh_execution_launch_failure_retains_dangling_session_without_mutation() {
    use std::os::unix::fs::symlink;

    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-dangling-live");
    let candidate_path = fixture
        .runtime
        .sessions_dir
        .join(format!("{}.toml", fixture.candidate_session_id));
    fs::remove_file(&candidate_path).expect("remove materialized candidate Session");
    let missing_target = fixture.runtime.sessions_dir.join("missing-live-candidate");
    symlink(&missing_target, &candidate_path).expect("create dangling candidate Session");
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

    let _events = fixture
        .runtime
        .fresh_execution_launch_failed_events(&fixture.window_id, "simulated launch failure");

    assert!(fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    assert!(fs::symlink_metadata(&candidate_path)
        .expect("dangling candidate entry must remain")
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

    fs::remove_file(&candidate_path).expect("restore true Missing fresh startup state");
    fixture.runtime.reconcile_durable_fresh_execution_launches();
    assert!(!durable_launch_recovery_exists(
        &fixture.runtime.sessions_dir,
        &fixture.candidate_session_id,
    ));
    assert_eq!(
        gwt::cli::execution_state::continuation_attempt_for_operation(
            &fixture.repo,
            fixture.owner,
            &fixture.operation_id,
        )
        .expect("read aborted attempt")
        .expect("aborted attempt")
        .status,
        gwt::cli::execution_state::ContinuationAttemptStatus::Aborted,
    );
}

#[test]
fn fresh_execution_activated_response_loss_repairs_projection_pointer_and_workspace_commit() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "fresh-activated-repair");
    leave_fresh_execution_activated_before_projection_commit(&mut fixture);

    let events = fixture
        .runtime
        .fresh_execution_launch_failed_events(&fixture.window_id, "simulated response loss");

    assert!(
        !events.is_empty(),
        "Activated reconciliation must converge to the normal completion events"
    );
    assert!(!fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read repaired current binding"),
        Some(fixture.binding.identity.clone()),
    );
    assert_eq!(
        gwt_core::workspace_projection::resolve_workspace_state_external_commit(
            &fixture.repo,
            &fixture.operation_id,
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
        )
        .expect("read committed Workspace transaction"),
        gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
    );
}

#[test]
fn continuation_resolves_legacy_mixed_root_external_commit_after_upgrade() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());

    for (label, decision, expected) in [
        (
            "reject",
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Reject,
            gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Rejected,
        ),
        (
            "commit",
            gwt_core::workspace_projection::ExternalWorkspaceCommitDecision::Commit,
            gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
        ),
    ] {
        let project_root = temp.path().join(format!("legacy-mixed-{label}"));
        let work_event_root = project_root.join("work").join("issue-3412");
        fs::create_dir_all(&work_event_root).expect("legacy worktree root");
        let current = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root);
        let canonical_works =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
        let legacy_works =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&work_event_root);
        let events = gwt_core::paths::gwt_repo_local_work_events_path(&work_event_root);
        let operation_id = format!("legacy-mixed-root-{label}");
        let now = Utc::now();
        let session_id = format!("legacy-mixed-session-{label}");
        let work_id = format!("legacy-mixed-work-{label}");
        let resume_event_id = format!("legacy-mixed-resume-{label}");
        let branch = "work/issue-3412";
        let container = gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: Some(branch.to_string()),
            worktree_path: Some(work_event_root.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        };
        let mut current_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&project_root);
        current_projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: session_id.clone(),
                window_id: None,
                agent_id: "codex".to_string(),
                display_name: "Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Idle,
                current_focus: None,
                title_summary: None,
                worktree_path: Some(work_event_root.clone()),
                branch: Some(branch.to_string()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned,
                workspace_id: None,
                updated_at: now - chrono::Duration::seconds(2),
            });
        gwt_core::workspace_projection::save_workspace_projection_to_path(
            &current,
            &current_projection,
        )
        .expect("seed legacy current");

        let mut start = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            &work_id,
            now - chrono::Duration::seconds(2),
        );
        start.title = Some("Legacy mixed-root target".to_string());
        start.owner = Some("Issue #3412".to_string());
        start.status_category = Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Idle);
        start.agent_session_id = Some("legacy-predecessor".to_string());
        start.agent_id = Some("codex".to_string());
        start.execution_container = Some(container.clone());
        let mut legacy_projection =
            gwt_core::workspace_projection::WorkItemsProjection::empty(start.updated_at);
        legacy_projection.apply_event(start);
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &legacy_works,
            &legacy_projection,
        )
        .expect("seed legacy worktree WorkItems");

        let mut canonical_projection = legacy_projection;
        let unrelated_work_id = format!("canonical-unrelated-{label}");
        let mut unrelated = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            &unrelated_work_id,
            now - chrono::Duration::seconds(1),
        );
        unrelated.title = Some("Existing canonical Work".to_string());
        canonical_projection.apply_event(unrelated);
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &canonical_works,
            &canonical_projection,
        )
        .expect("seed pre-existing canonical WorkItems SOT");

        let prepared = gwt_core::workspace_projection::transact_workspace_state_at_with_commit(
            &current,
            &legacy_works,
            &events,
            &project_root,
            &operation_id,
            |projection, _, _| {
                assert!(projection.assign_agent(&session_id, &work_id, None, None, now));
                let mut resume = gwt_core::workspace_projection::WorkEvent::new(
                    gwt_core::workspace_projection::WorkEventKind::Resume,
                    &work_id,
                    now,
                );
                resume.id = resume_event_id.clone();
                resume.status_category =
                    Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
                resume.agent_session_id = Some(session_id.clone());
                resume.agent_id = Some("codex".to_string());
                resume.execution_container = Some(container.clone());
                Ok(((), vec![resume]))
            },
            || {
                Err(gwt_core::error::GwtError::Other(
                    "simulated legacy activation response loss".to_string(),
                ))
            },
        );
        assert!(prepared.is_err(), "legacy transaction must remain Prepared");

        assert_eq!(
            resolve_split_workspace_state_external_commit(
                &project_root,
                &work_event_root,
                &operation_id,
                decision,
            )
            .expect("new Host must resolve the legacy mixed-root transaction"),
            expected,
        );
        gwt_core::workspace_projection::transact_workspace_state_for_work_event_root(
            &project_root,
            &work_event_root,
            |_, _, _| Ok(((), Vec::new())),
        )
        .expect("resolved legacy transaction must not block the split-root writer");
        let canonical = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
            .expect("load canonical WorkItems")
            .expect("canonical WorkItems");
        assert!(
            canonical
                .work_items
                .iter()
                .any(|item| item.id == unrelated_work_id),
            "legacy recovery must preserve pre-existing canonical Works"
        );
        let target = canonical
            .work_items
            .iter()
            .find(|item| item.id == work_id)
            .expect("canonical target Work");
        let has_resumed_session = target
            .agents
            .iter()
            .any(|agent| agent.session_id == session_id)
            && target
                .events
                .iter()
                .any(|event| event.id == resume_event_id);
        assert_eq!(
            has_resumed_session,
            expected
                == gwt_core::workspace_projection::ExternalWorkspaceCommitResolution::Committed,
            "only a committed legacy event may be folded into canonical WorkItems"
        );
    }
}
