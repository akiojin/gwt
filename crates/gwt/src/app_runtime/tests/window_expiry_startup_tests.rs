use super::*;

/// Issue #3923 AC-2: a hold records what it was formed from — the matched
/// screen text and the poller reading — so a false hold can be diagnosed
/// instead of guessed at.
#[test]
fn a_committed_quota_hold_carries_the_screen_text_and_poller_reading_as_evidence() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (mut runtime, window_id) = quota_live_runtime(temp.path(), "codex");
    runtime.set_provider_usage_accounts(vec![codex_usage_account(100.0, true)]);

    let _ = runtime.observe_provider_quota_notice(
        &window_id,
        Some(CODEX_USAGE_LIMIT_SCREEN),
        instant("2026-09-02T09:00:00Z"),
    );

    let Some(gwt::IssueMonitorFailure::ProviderUsageLimit {
        provider,
        evidence: Some(evidence),
        ..
    }) = runtime.provider_quota_holds.get(&window_id)
    else {
        panic!(
            "the hold must be typed and carry evidence: {:?}",
            runtime.provider_quota_holds.get(&window_id)
        );
    };
    assert_eq!(provider, "codex");
    assert_eq!(evidence.source, "screen_notice");
    assert_eq!(evidence.recorded_at, "2026-09-02T09:00:00.000Z");
    assert_eq!(evidence.window_id.as_deref(), Some(window_id.as_str()));
    let provenance = serde_json::to_value(evidence).expect("evidence JSON");
    assert_eq!(provenance["screen_region"], "provider_response");
    assert_eq!(provenance["matched_pattern"], "codex_usage_limit");
    assert!(
        evidence
            .screen_text
            .as_deref()
            .is_some_and(|text| text.contains("hit your usage limit")),
        "the matched screen text is the evidence: {:?}",
        evidence.screen_text
    );
    assert_eq!(evidence.poller_state.as_deref(), Some("ok"));
    assert_eq!(evidence.poller_limit_reached, Some(true));
    assert_eq!(
        evidence.poller_windows,
        vec![gwt::IssueMonitorProviderQuotaPollerWindow {
            kind: "weekly".to_string(),
            used_percent: 100,
        }]
    );

    // Issue #5037: an exit detail can contain the refusal while the pane still
    // shows startup output. Retain the text that actually authenticated it.
    insert_test_pane_runtime(&mut runtime, &window_id);
    owner_runtime_probe(&runtime, &window_id)
        .pane
        .lock()
        .unwrap()
        .process_bytes(b"Starting provider CLI\r\n");
    runtime.handle_runtime_status_with_exit_confirmation(
        window_id.clone(),
        WindowProcessStatus::Stopped,
        Some(CODEX_USAGE_LIMIT_SCREEN.to_string()),
        true,
    );
    let exit_hold = serde_json::to_value(&runtime.provider_quota_holds[&window_id])
        .expect("exit evidence JSON");
    assert_eq!(exit_hold["evidence"]["screen_region"], "provider_response");
    assert_eq!(
        exit_hold["evidence"]["matched_pattern"],
        "codex_usage_limit"
    );
}

/// Issue #5037: neither full usage nor persistence authenticates a quotation
/// (including the incident's Claude wording printed by a Codex agent).
#[test]
fn quoted_or_other_provider_notices_never_hold_a_codex_pane() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    for screen in [
        r#"{"body":"旧 Claude PM pane: You've hit your weekly limit · resets Oct 8, 6am"}"#,
        "```text\n■ You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage\n  to purchase more credits or try again at Oct 8, 2026 6am.\n```",
        CLAUDE_USAGE_LIMIT_SCREEN,
    ] {
        let (mut runtime, window_id) = quota_live_runtime(temp.path(), "codex");
        runtime.set_provider_usage_accounts(vec![codex_usage_account(100.0, true)]);
        for now in ["2026-10-05T05:31:32Z", "2026-10-05T05:36:32Z"] {
            runtime.observe_provider_quota_notice(&window_id, Some(screen), instant(now));
            assert!(!runtime.provider_quota_holds.contains_key(&window_id), "{screen}");
            assert!(!runtime.provider_quota_candidates.contains_key(&window_id), "{screen}");
        }
        runtime.handle_runtime_status_with_exit_confirmation(
            window_id.clone(),
            WindowProcessStatus::Stopped,
            Some(screen.to_string()),
            true,
        );
        assert!(!runtime.provider_quota_holds.contains_key(&window_id),
            "a terminal status must not authenticate the quotation: {screen}");
    }
}

// Issue #3927 (SPEC #3340 T-624 / AS-40〜42 / FR-045〜046): the Tao thread
// tracks a grace candidate per eligible window, resets it on eligibility loss,
// and closes only the exact window / Session / lifecycle generation it saw.
#[test]
fn terminal_convergence_grace_candidate_resets_and_closes_the_exact_window() {
    use crate::app_runtime::terminal_convergence::{
        TerminalCloseEligibility, TerminalCloseReason, TerminalWindowObservation,
    };
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let tab = sample_project_tab_with_window(
        "tab-1",
        "codex-1",
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "codex-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    runtime
        .window_lifecycle_generations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(window_id.clone(), 7);
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let grace = Duration::from_secs(60);
    let t0 = Instant::now();
    let observation = |eligibility: TerminalCloseEligibility, generation: u64| {
        vec![TerminalWindowObservation {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            lifecycle_generation: Some(generation),
            eligibility,
        }]
    };
    let eligible = TerminalCloseEligibility::Eligible(TerminalCloseReason::SettledExecution);

    // A timer alone cannot reuse old Warning eligibility while a scan is
    // pending. The next observation can discover an action or authority hold.
    runtime.set_window_status("tab-1", "codex-1", WindowProcessStatus::Idle);
    let warning =
        TerminalCloseEligibility::Eligible(TerminalCloseReason::ClosedIssueBookkeepingWarning);
    for cause in ["obligation_open", "execution_blocked_or_corrupt"] {
        runtime.terminal_convergence_observed_events_at(grace, observation(warning, 7), t0);
        runtime.close_expired_terminal_window_candidates_at(t0 + grace);
        assert!(runtime.window_lookup.contains_key(&window_id));
        assert_eq!(runtime.terminal_close_candidates[&window_id].since, t0);
        runtime.terminal_convergence_observed_events_at(
            grace,
            observation(TerminalCloseEligibility::Ineligible(cause), 7),
            t0 + grace,
        );
        assert!(runtime.window_lookup.contains_key(&window_id));
        assert!(runtime.terminal_close_candidates.is_empty());
    }
    runtime.set_window_status("tab-1", "codex-1", WindowProcessStatus::Running);

    runtime.terminal_convergence_observed_events_at(grace, observation(eligible, 7), t0);
    assert_eq!(runtime.terminal_close_grace, grace);
    let since = runtime
        .terminal_close_candidates
        .get(&window_id)
        .expect("eligible observation starts a candidate")
        .since;
    assert_eq!(since, t0);

    // A repeated eligible observation keeps the original start.
    runtime.terminal_convergence_observed_events_at(
        grace,
        observation(eligible, 7),
        t0 + Duration::from_secs(30),
    );
    assert_eq!(runtime.terminal_close_candidates[&window_id].since, t0);
    assert!(
        runtime.window_lookup.contains_key(&window_id),
        "the window stays open inside the grace"
    );

    // Eligibility loss resets the candidate; user activity is not consulted.
    runtime.terminal_convergence_observed_events_at(
        grace,
        observation(TerminalCloseEligibility::Ineligible("obligation_open"), 7),
        t0 + Duration::from_secs(40),
    );
    assert!(runtime.terminal_close_candidates.is_empty());
    runtime.terminal_convergence_observed_events_at(
        grace,
        observation(eligible, 7),
        t0 + Duration::from_secs(50),
    );
    assert_eq!(
        runtime.terminal_close_candidates[&window_id].since,
        t0 + Duration::from_secs(50)
    );
    runtime.close_expired_terminal_window_candidates_at(t0 + Duration::from_secs(100));
    assert!(
        runtime.window_lookup.contains_key(&window_id),
        "fifty seconds since the fresh candidate is inside the grace"
    );

    // A stale lifecycle generation never closes the current window.
    runtime.terminal_convergence_observed_events_at(
        grace,
        observation(eligible, 6),
        t0 + Duration::from_secs(50),
    );
    let events = runtime.close_expired_terminal_window_candidates_at(t0 + Duration::from_secs(200));
    assert!(events.is_empty());
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.terminal_close_candidates.is_empty());

    // The exact generation closes through the shared finalizer once the
    // grace elapsed.
    runtime.terminal_convergence_observed_events_at(
        grace,
        observation(eligible, 7),
        t0 + Duration::from_secs(300),
    );
    let events = runtime.close_expired_terminal_window_candidates_at(t0 + Duration::from_secs(360));
    assert!(events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })));
    assert!(!runtime.window_lookup.contains_key(&window_id));
    assert!(runtime.tabs[0].workspace.window("codex-1").is_none());
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    assert!(runtime.terminal_close_candidates.is_empty());
    assert_eq!(
        finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        1,
        "the terminal close queues exactly one detached finalizer"
    );
}

/// Issue #5078 AC-4: a restored Completed holder with no Workspace identity
/// can reach real terminal delivery settlement and the exact window close.
#[test]
fn completed_restore_reaches_monitor_settlement_and_exact_window_close() {
    completed_restore_reaches_exact_close(false);
}

#[test]
fn closed_completed_idle_pane_releases_capacity_with_bookkeeping_warning() {
    completed_restore_reaches_exact_close(true);
}

fn completed_restore_reaches_exact_close(bookkeeping_warning: bool) {
    use crate::app_runtime::terminal_convergence::{
        observe_terminal_windows_in_background, RestoreAdmission, TerminalCloseEligibility,
        TerminalCloseReason, TerminalWindowSnapshot,
    };
    use gwt::cli::{
        execution_state as execution,
        hook::{workflow_policy, HookEvent, HookOutput},
    };

    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _review_dispatch = ScopedEnvVar::unset("GWT_REVIEW_DISPATCH");
    let repo = temp.path().join("repo");
    init_git_clone_with_default_branch(&repo, "feature/demo");
    seed_owner_fixture_roster(&repo, 8);
    configure_owner_fixture_tools(&repo, &["pane-contract-fixture"]);
    let owner = Arc::new(
        owner_fixture_block_on(
            gwt::project_runtime::test_support::ProjectOwnerHarness::start(&repo),
        )
        .unwrap(),
    );
    let mut config = pane_control_tests::control_launch_config(&repo, 42);
    config.working_dir = Some(repo.clone());
    config.branch = Some("feature/demo".into());
    config.base_branch = Some("feature/demo".into());
    if bookkeeping_warning {
        config.launch_route = gwt_agent::LaunchRoute::Autonomous;
    }
    configure_owner_fixture_start_gate(&owner);
    let owner_id = owner.pane_id("agent-42");
    let launched = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::LaunchAgent {
            id: owner_id.clone(),
            operation_id: format!("completed-restore-{}", uuid::Uuid::new_v4()),
            tab_id: owner_id.split_once("::").unwrap().0.into(),
            config: Box::new(config),
            metadata: Box::default(),
            cols: 80,
            rows: 24,
        },
    ))
    .expect("actual producing fixture launch");
    assert!(
        launched.get("launch_failure").is_none()
            && launched.get("compensated_launch_failure").is_none()
            && launched.get("monitor_failure_closed").is_none(),
        "the Completed fixture requires a live producing runtime: {launched}"
    );
    let incarnation = launched["incarnation"]
        .as_u64()
        .expect("actual launch incarnation");
    let physical =
        owner_fixture_block_on(owner.probe(&owner_id, incarnation)).unwrap_or_else(|error| {
            panic!("actual launch identity is unavailable: {error}; receipt={launched}")
        });
    assert_eq!(
        physical.session.as_ref().unwrap().id,
        launched["session_id"]
    );
    assert!(physical
        .session
        .as_ref()
        .unwrap()
        .execution_binding
        .is_some());
    assert!(physical.grant.is_some());
    let session_id = physical.session.as_ref().unwrap().id.clone();
    let window_id = combined_window_id("tab-1", "agent-42");
    let mut runtime = super::managed_launch_tests::managed_project_owner_projection(
        temp.path(),
        &repo,
        &owner,
        &[&physical],
    );
    runtime
        .test_project_owners
        .insert(repo.clone(), owner.clone());
    runtime.register_window("tab-1", "agent-42");
    let context = runtime.test_context();
    let identity = owner.client().identity().unwrap().clone();
    let state = runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = Some(identity.clone());
    let idle = owner_runtime_status_event(&owner, &physical, "Idle", "Stop");
    runtime.handle_project_owner_event(&context, 1, &identity, idle);
    if bookkeeping_warning {
        // The authenticated owner hook above updates its live projection;
        // the provider's CLI hook also persists that Stop in Session metadata.
        physical.with_agent_environment(|| {
            gwt::daemon_runtime::handle_runtime_state("Stop", "{}")
                .expect("actual provider Stop metadata");
        });
    }
    let session_path = runtime.sessions_dir.join(format!("{session_id}.toml"));
    let session = gwt_agent::Session::load(&session_path).expect("load actual bound holder");
    assert!(session.restore_window_on_startup);
    if bookkeeping_warning {
        assert_eq!(session.launch_route, gwt_agent::LaunchRoute::Autonomous);
        assert_eq!(session.status, gwt_agent::AgentStatus::Idle);
        assert_eq!(
            owner_fixture_block_on(owner.probe(&owner_id, incarnation))
                .unwrap()
                .status,
            WindowProcessStatus::Idle
        );
    }
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load(&runtime.sessions_dir);
    save_workspace_launch_projection(
        &repo,
        &runtime.active_agent_sessions[&window_id],
        None,
        Some(42),
        Some(execution::ExecutionOwnerKey {
            kind: execution::ExecutionOwnerKind::Issue,
            number: 42,
        }),
        None,
        WorkspaceLaunchProjectionKind::StartWork,
        None,
    )
    .expect("materialize assigned Work with missing purpose and focus");
    let _session_env = ScopedEnvVar::set(gwt_agent::session::GWT_SESSION_ID_ENV, &session_id);
    let event = HookEvent {
        tool_name: Some("Write".to_string()),
        tool_input: Some(serde_json::json!({ "file_path": repo.join("fixture.rs") })),
        transcript_path: None,
        active_skill: None,
        active_plugin: None,
        cwd: Some(repo.display().to_string()),
    };
    assert!(matches!(
        workflow_policy::evaluate(&event, &repo).expect("evaluate Active identity gate"),
        HookOutput::PreToolUsePermission { .. }
    ));
    run_git(&repo, &["add", ".gwt/work"]);
    run_git(
        &repo,
        &["commit", "-qm", "chore(work): deliver restore fixture"],
    );
    run_git(&repo, &["push", "-qu", "origin", "feature/demo"]);
    let receipt =
        gwt::cli::verification_record::save_work_event_settlement_record(&repo, &session_id, false)
            .expect("record actual delivery of the Work event");
    assert!(
        receipt.status.is_settled(),
        "the exact Work event must be remotely delivered before completion: {receipt:?}"
    );
    assert!(matches!(
        execution::settle(
            &repo,
            &session_id,
            execution::ExecutionSettlement::Completed
        )
        .expect("settle actual Completed record"),
        execution::SettleResult::Settled(_)
    ));
    if bookkeeping_warning {
        // Preserve the original shard bytes while reproducing untracked
        // bookkeeping and unavailable upstream readback after completion.
        run_git(&repo, &["rm", "-r", "--cached", ".gwt/work/events"]);
        run_git(
            &repo,
            &[
                "config",
                "branch.feature/demo.merge",
                "refs/heads/missing-upstream",
            ],
        );
        gwt::cli::verification_record::save_work_event_settlement_record(&repo, &session_id, true)
            .expect("record the current generation's bookkeeping warning");
    }
    let diagnosis = execution::diagnose_for_projection(&repo, Some(&session_id));
    assert_eq!(
        diagnosis.ecr_status,
        execution::ExecutionDiagnosisState::Completed
    );
    assert_eq!(
        diagnosis.binding_state,
        execution::ExecutionBindingState::Terminal
    );
    assert_eq!(
        diagnosis.settlement_severity,
        if bookkeeping_warning {
            "warning"
        } else {
            "clear"
        }
    );
    assert_eq!(diagnosis.settlement_obligation_open, bookkeeping_warning);
    assert!(diagnosis.open_obligations.is_empty());
    assert_eq!(
        diagnosis.work_event_receipt_matches_current_generation,
        Some(true)
    );
    let execution_bytes = fs::read(execution::state_path(&repo)).unwrap();
    let receipt_bytes = gwt::cli::trusted_store::read(&repo, "work-event-settlement.json").unwrap();
    let work_bytes: Vec<_> = fs::read_dir(repo.join(".gwt/work/events"))
        .unwrap()
        .flat_map(|prefix| fs::read_dir(prefix.unwrap().path()).unwrap())
        .map(|entry| entry.unwrap().path())
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    assert!(!work_bytes.is_empty());
    assert_eq!(
        workflow_policy::evaluate(&event, &repo)
            .expect("evaluate restored Completed identity gate"),
        HookOutput::Silent
    );

    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    prefs.enabled = true;
    prefs.launched_issues = if bookkeeping_warning {
        vec![]
    } else {
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: window_id.clone(),
        }]
    };
    prefs.closure_records = if bookkeeping_warning {
        vec![gwt::issue_monitor::IssueClosureRecord {
            issue_number: 42,
            generation: 1,
            state: gwt::issue_monitor::IssueClosureState::Closed,
            evidence: gwt::issue_monitor::IssueClosureEvidence::DirectRelease,
            issue_updated_at: None,
            reopened_after_close: false,
        }]
    } else {
        vec![]
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed exact Monitor delivery");
    let accounting = gwt::IssueMonitorState::with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
    );
    if bookkeeping_warning {
        let snapshot = runtime
            .issue_monitor_window_snapshot_for_tab("tab-1", &chrono::Utc::now().to_rfc3339())
            .unwrap();
        assert_eq!(snapshot.windows.len(), 1);
        assert_eq!(
            snapshot.windows[0].session_id.as_deref(),
            Some(session_id.as_str())
        );
        assert!(snapshot.windows[0].monitor_owned);
        // The real owner already published this physical pane's host-qualified
        // census. The GUI's tab-1 alias observes that Session; adding it again
        // without its owner origin would describe a second physical pane.
        assert_eq!(
            accounting.agent_status().occupied_slot_count,
            Some(1),
            "the actual owner census accounts for its one physical pane"
        );
    }
    let reason = if bookkeeping_warning {
        TerminalCloseReason::ClosedIssueBookkeepingWarning
    } else {
        TerminalCloseReason::SettledExecution
    };
    assert_eq!(
        runtime.restore_work_terminality(&session, &repo, Some(&window_id)),
        RestoreAdmission::RefuseTerminal(reason)
    );
    let generation = runtime
        .window_lifecycle_generations
        .lock()
        .unwrap()
        .get(&window_id)
        .copied()
        .expect("actual GUI window generation");
    let observations = observe_terminal_windows_in_background(
        &runtime.sessions_dir,
        vec![TerminalWindowSnapshot {
            window_id: window_id.clone(),
            session_id: session_id.to_string(),
            project_root: repo.clone(),
            worktree_path: repo.clone(),
            window_status: WindowProcessStatus::Idle,
            lifecycle_generation: Some(generation),
        }],
        Duration::from_secs(5),
    );
    assert_eq!(
        observations[0].eligibility,
        TerminalCloseEligibility::Eligible(reason)
    );
    assert_eq!(
        runtime.restore_work_terminality(&session, &repo, Some(&window_id)),
        RestoreAdmission::RefuseTerminal(reason)
    );
    assert!(gwt::load_issue_monitor_prefs(&prefs_path)
        .expect("read settlement")
        .launched_issues
        .is_empty());
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let grace = Duration::from_secs(60);
    let t0 = Instant::now();
    if bookkeeping_warning {
        let mut previously_clear = observations.clone();
        previously_clear[0].eligibility =
            TerminalCloseEligibility::Eligible(TerminalCloseReason::SettledExecution);
        runtime.terminal_convergence_observed_events_at(
            grace,
            previously_clear,
            t0 - Duration::from_secs(30),
        );
    }
    runtime.terminal_convergence_observed_events_at(grace, observations.clone(), t0);
    assert!(runtime.window_lookup.contains_key(&window_id));
    assert_eq!(runtime.terminal_close_candidates[&window_id].reason, reason);
    assert_eq!(runtime.terminal_close_candidates[&window_id].since, t0);
    let expires = if bookkeeping_warning {
        let running = owner_runtime_status_event(&owner, &physical, "Running", "UserPromptSubmit");
        runtime.handle_project_owner_event(&context, 1, &identity, running);
        assert_eq!(
            runtime.window_status(&window_id),
            Some(WindowProcessStatus::Running)
        );
        runtime.close_expired_terminal_window_candidates_at(t0 + grace);
        assert!(
            runtime.window_lookup.contains_key(&window_id),
            "renewed work stays open"
        );
        assert!(!runtime.terminal_close_candidates.contains_key(&window_id));
        let idle = owner_runtime_status_event(&owner, &physical, "Idle", "Stop");
        runtime.handle_project_owner_event(&context, 1, &identity, idle);
        assert_eq!(
            runtime.window_status(&window_id),
            Some(WindowProcessStatus::Idle)
        );
        runtime.terminal_convergence_observed_events_at(grace, observations.clone(), t0 + grace);
        t0 + grace + grace
    } else {
        t0 + grace
    };
    runtime.close_expired_terminal_window_candidates_at(expires);
    if bookkeeping_warning {
        assert!(
            runtime.window_lookup.contains_key(&window_id),
            "the bare timer retains a Warning until a fresh background observation"
        );
        let latest = observe_terminal_windows_in_background(
            &runtime.sessions_dir,
            runtime.terminal_window_snapshots(),
            Duration::from_secs(5),
        );
        assert_eq!(
            latest[0].eligibility,
            TerminalCloseEligibility::Eligible(reason)
        );
        runtime.terminal_convergence_observed_events_at(grace, latest, expires);
    }
    assert!(!runtime.window_lookup.contains_key(&window_id));
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    assert!(runtime.tabs[0].workspace.window("agent-42").is_none());
    let finalizer = finalizers
        .lock()
        .unwrap()
        .pop()
        .expect("exact close finalizer");
    finalizer();
    let closed = gwt_agent::Session::load(&session_path).expect("read closed holder");
    assert!(physical.pty.tree_exit_confirmed());
    assert!(!physical
        .writer
        .as_ref()
        .unwrap()
        .current
        .load(std::sync::atomic::Ordering::Acquire));
    assert!(owner_fixture_block_on(owner.probe(&physical.id, physical.incarnation)).is_err());
    assert_eq!(closed.status, gwt_agent::AgentStatus::Stopped);
    assert!(!closed.restore_window_on_startup);
    if bookkeeping_warning {
        // The physical finalizer precedes capacity release. Draining the real
        // owner joins its final runtime census publication before readback.
        runtime.test_project_owners.remove(&repo);
        owner_fixture_block_on(
            Arc::try_unwrap(owner)
                .ok()
                .expect("fixture's sole owner")
                .shutdown(),
        );
        let closed_prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        let accounting =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), closed_prefs);
        let snapshot = runtime
            .issue_monitor_window_snapshot_for_tab("tab-1", &chrono::Utc::now().to_rfc3339())
            .unwrap();
        assert!(snapshot.windows.is_empty());
        assert_eq!(accounting.agent_status().occupied_slot_count, Some(0));
    }
    assert_eq!(
        fs::read(execution::state_path(&repo)).unwrap(),
        execution_bytes
    );
    assert_eq!(
        gwt::cli::trusted_store::read(&repo, "work-event-settlement.json").unwrap(),
        receipt_bytes
    );
    for (path, bytes) in work_bytes {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

// Issue #3927 (SPEC #3340 T-627 / PM ruling): the settlement bridge releases
// the Monitor slot through the exact terminal-delivery transition (daemon
// control, local exact-CAS fallback here) and refuses a stale identity; the
// observer then classifies a durably closed Issue as eligible while a window
// whose Session has no Issue link is never eligible.
#[test]
fn terminal_convergence_observer_settles_monitor_owned_delivery_before_eligibility() {
    use crate::app_runtime::terminal_convergence::{
        observe_terminal_windows_in_background,
        settle_issue_monitor_terminal_delivery_in_background, TerminalCloseEligibility,
        TerminalCloseReason, TerminalWindowSnapshot,
    };
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let sessions_dir = temp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
    let window_id = "tab-1::agent-42".to_string();

    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            enabled: true,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: window_id.clone(),
            }],
            launched_claims: std::collections::BTreeMap::from([(42, "claim-live".to_string())]),
            ..gwt::IssueMonitorPrefs::default()
        },
    );
    assert_eq!(seeded.record_attempt(42), 1, "one attempt already spent");
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded.prefs()).expect("seed prefs");

    // Settlement bridge: exact identity commits, stale identity is refused.
    assert!(
        settle_issue_monitor_terminal_delivery_in_background(
            &repo,
            "tab-1::agent-99",
            42,
            Duration::from_secs(5)
        )
        .is_err(),
        "a window the Monitor does not bind cannot settle the launch"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(
        persisted.launched_issues.len(),
        1,
        "a refused settlement mutates nothing"
    );
    settle_issue_monitor_terminal_delivery_in_background(
        &repo,
        &window_id,
        42,
        Duration::from_secs(5),
    )
    .expect("exact settlement commits through the local fallback");
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert!(
        persisted.launched_issues.is_empty(),
        "the settlement released the Monitor slot"
    );
    assert_eq!(
        persisted
            .autonomous_records
            .iter()
            .find(|record| record.issue_number == 42)
            .map(|record| record.attempts),
        Some(1),
        "settlement spends no attempt"
    );

    // Observer classification: a durable closed-Issue record is eligible; a
    // Session without an Issue link is not.
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            closure_records: vec![gwt::issue_monitor::IssueClosureRecord {
                issue_number: 42,
                generation: 1,
                state: gwt::issue_monitor::IssueClosureState::Closed,
                evidence: gwt::issue_monitor::IssueClosureEvidence::DirectRelease,
                issue_updated_at: None,
                reopened_after_close: false,
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed closed prefs");
    let mut linked = gwt_agent::Session::new(&repo, "work/issue-42", gwt_agent::AgentId::Codex);
    linked.id = "session-linked".to_string();
    linked.linked_issue_number = Some(42);
    linked.update_status(gwt_agent::AgentStatus::Idle);
    linked.save(&sessions_dir).expect("save linked session");
    let mut manual = gwt_agent::Session::new(&repo, "feature/manual", gwt_agent::AgentId::Codex);
    manual.id = "session-manual".to_string();
    manual.update_status(gwt_agent::AgentStatus::Idle);
    manual.save(&sessions_dir).expect("save manual session");

    let snapshot = |window_id: &str, session_id: &str| TerminalWindowSnapshot {
        window_id: window_id.to_string(),
        session_id: session_id.to_string(),
        project_root: repo.clone(),
        worktree_path: repo.clone(),
        window_status: WindowProcessStatus::Running,
        lifecycle_generation: Some(1),
    };
    let observations = observe_terminal_windows_in_background(
        &sessions_dir,
        vec![
            snapshot(&window_id, "session-linked"),
            snapshot("tab-1::agent-7", "session-manual"),
        ],
        Duration::from_secs(5),
    );
    assert_eq!(
        observations[0].eligibility,
        TerminalCloseEligibility::Eligible(TerminalCloseReason::ClosedIssue)
    );
    assert_eq!(
        observations[1].eligibility,
        TerminalCloseEligibility::Ineligible("no_linked_issue")
    );
}

// Issue #3927 (SPEC #3340 T-625 / AS-43 / FR-047): startup restore refuses a
// persisted window whose linked Issue is durably closed, marks the Session
// restore-disabled, and removes its placeholder, while an unlinked
// placeholder restores exactly as before.
#[test]
fn app_runtime_startup_auto_resume_refuses_closed_issue_window_and_disables_restore() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let codex_home = temp.path().join(".codex");
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", &codex_home);
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    // Restore remains eligible only while this checkout has unlanded work.
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let worktree = temp.path().join("worktrees").join("terminal-restore");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "work/terminal-restore",
            worktree.to_str().expect("worktree path"),
        ],
    );
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&worktree),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            merged_issues: vec![42],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");

    let mut persisted = empty_workspace_state();
    let mut closed_window = sample_window(
        "agent-closed",
        WindowPreset::Codex,
        WindowProcessStatus::Stopped,
    );
    closed_window.agent_id = Some("codex".to_string());
    closed_window.session_id = Some("session-closed".to_string());
    let mut open_window = sample_window(
        "agent-open",
        WindowPreset::Codex,
        WindowProcessStatus::Stopped,
    );
    open_window.agent_id = Some("codex".to_string());
    open_window.session_id = Some("session-open".to_string());
    persisted.windows.push(closed_window);
    persisted.windows.push(open_window);
    persisted.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-terminal".to_string(),
        title: "Terminal Restore".to_string(),
        project_root: worktree.clone(),
        kind: ProjectKind::Git,
        workspace: WindowRegistry::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let (mut runtime, recorded) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-terminal"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    pin_live_pm_fixture_agents(&runtime, temp.path());
    seed_owner_fixture_roster(&worktree, 8);
    configure_owner_fixture_tools(&worktree, &["codex"]);
    gwt::persistence::save_workspace_state_durable(
        &gwt::workspace_state_path(&worktree),
        runtime.tabs[0].workspace.persisted(),
    )
    .unwrap();
    fs::create_dir_all(codex_home.join("sessions")).unwrap();
    let open_native_id = uuid::Uuid::new_v4().to_string();
    for (session_id, native_id, linked_issue) in [
        ("session-closed", uuid::Uuid::new_v4().to_string(), Some(42)),
        ("session-open", open_native_id.clone(), None),
    ] {
        let mut session = gwt_agent::Session::new(
            &worktree,
            "work/terminal-restore",
            gwt_agent::AgentId::Codex,
        );
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_id.clone());
        session.project_state_root = Some(worktree.clone());
        session.linked_issue_number = linked_issue;
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
        fs::write(
            codex_home
                .join("sessions")
                .join(format!("rollout-{native_id}.jsonl")),
            serde_json::json!({"type":"session_meta","payload":{"id":native_id,"cwd":worktree}})
                .to_string(),
        )
        .unwrap();
    }
    runtime.launch_wizard_cache = LaunchWizardMemoryCache::load(&runtime.sessions_dir);
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.bootstrap();
    drain_queued_blocking_tasks(&tasks);
    assert!(
        recorded.lock().unwrap().iter().any(|event| matches!(
            recorded_project_payload(event),
            UserEvent::StartupGenerationReaperCompleted
        )),
        "the real startup worker releases its gate"
    );
    runtime.handle_startup_generation_reaper_completed();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {},
    );
    let pending_ids = runtime
        .pending_launch_completions
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for id in &pending_ids {
        project_owner_fixture(&mut runtime, id);
    }
    let restore_events = drain_owner_launch_fixtures(&mut runtime, &tasks);

    let windows = runtime.tabs[0].workspace.persisted().windows.clone();
    assert!(
        !windows
            .iter()
            .any(|window| window.session_id.as_deref() == Some("session-closed")),
        "the closed Issue's placeholder is removed before any spawn: {windows:?}; events={restore_events:?}"
    );
    assert_eq!(
        windows
            .iter()
            .filter(|window| crate::runtime_support::window_is_agent_pane(window))
            .count(),
        1,
        "only the unlinked placeholder is replaced by a resumed window"
    );
    assert!(
        runtime
            .pending_auto_resume_sources
            .values()
            .all(|source| source == "session-open"),
        "no resume was queued for the closed Issue"
    );
    let refused =
        gwt_agent::Session::load_and_migrate(&runtime.sessions_dir.join("session-closed.toml"))
            .expect("reload refused session");
    assert!(
        !refused.restore_window_on_startup,
        "the refused Session is durably restore-disabled"
    );
    assert_eq!(refused.status, gwt_agent::AgentStatus::Stopped);
    assert_eq!(
        runtime.runtimes.len(),
        1,
        "only the eligible source owns a real PTY"
    );
    let id = runtime.runtimes.keys().next().unwrap().clone();
    let live = owner_runtime_probe(&runtime, &id);
    assert_eq!(
        live.session.as_ref().unwrap().agent_session_id.as_deref(),
        Some(open_native_id.as_str())
    );
    assert!(live.pty.try_wait().unwrap().is_none());
    close_test_owner_runtime(&mut runtime, &id);
}

// Issue #3927 (SPEC #3340 T-626 / AS-44〜45 / FR-048): a Monitor-owned
// restored window that fails before PTY start is closed only after the
// concrete failure is durably committed as the Issue's `error_message`; a
// manual window that fails the same way keeps its diagnostic pane.
#[test]
fn monitor_owned_pre_pty_launch_failure_is_committed_then_closed_while_manual_is_retained() {
    use gwt::project_runtime::{
        events::{OwnerCommand, OwnerEvent},
        test_support::ProjectOwnerHarness,
    };
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    seed_owner_fixture_roster(&repo, 8);
    configure_owner_fixture_tools(&repo, &["fixture"]);
    persist_owner_fixture_agent(
        if cfg!(windows) {
            "powershell".into()
        } else {
            "/bin/sh".into()
        },
        if cfg!(windows) {
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                "while ($null -ne [Console]::ReadLine()) {}".into(),
            ]
        } else {
            vec!["-c".into(), "while IFS= read -r value; do :; done".into()]
        },
    );
    let config_path = gwt_core::paths::gwt_config_path();
    let mut agents = gwt_agent::load_stored_custom_agents_from_path(&config_path).unwrap();
    agents[0].agent.skip_permissions_args = vec!["fixture-skip-permissions".into()];
    gwt_agent::save_stored_custom_agents_to_path(&config_path, &agents).unwrap();
    let mut registry = empty_workspace_state();
    let mut sources = Vec::new();
    for (raw, issue, name) in [("agent-42", Some(42), "Alpha"), ("agent-7", None, "Bravo")] {
        let branch = format!("work/legacy-{raw}");
        let worktree = temp.path().join(raw);
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                &branch,
                worktree.to_str().unwrap(),
                "HEAD",
            ],
        );
        let mut source = gwt_agent::Session::new(
            &worktree,
            &branch,
            gwt_agent::AgentId::Custom("fixture".into()),
        );
        source.id = format!("session-{raw}");
        source.project_state_root = Some(repo.clone());
        source.linked_issue_number = issue;
        source.agent_name = Some(name.into());
        source.agent_member_id =
            gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
                .unwrap()
                .agent_roster
                .unwrap()
                .members
                .iter()
                .find(|member| member.name == name)
                .map(|member| member.agent_member_id.clone());
        assert!(source.agent_member_id.is_some());
        source.record_terminal_stop();
        source.restore_window_on_startup = true;
        source.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let mut row = sample_window(raw, WindowPreset::Agent, WindowProcessStatus::Stopped);
        row.session_id = Some(source.id.clone());
        row.agent_id = Some("fixture".into());
        row.agent_name = Some(name.into());
        row.linked_issue_number = issue;
        registry.windows.push(row);
        sources.push(source);
    }
    registry.next_z_index = 3;
    gwt::persistence::save_workspace_state_durable(
        &gwt::persistence::workspace_state_path(&repo),
        &registry,
    )
    .unwrap();
    let project_key = gwt_core::paths::project_scope_hash(&repo);
    let owned_id = format!("{project_key}::agent-42");
    let manual_id = format!("{project_key}::agent-7");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    prefs.enabled = true;
    prefs.launched_issues = vec![gwt::IssueMonitorLaunchedIssue {
        issue_number: 42,
        window_id: owned_id.clone(),
    }];
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "legacy Monitor restore",
            &["bug"],
            "legacy owner",
            &Utc::now().to_rfc3339(),
        ))
        .unwrap();
    let owner = owner_fixture_block_on(ProjectOwnerHarness::start_with_monitor(&repo)).unwrap();
    owner_fixture_block_on(owner.wait_monitor_idle()).unwrap();
    owner_fixture_block_on(
        owner.set_pty_start_gate(temp.path().join("missing-legacy-restore-gate"), Vec::new()),
    )
    .unwrap();
    let mut observed = owner.subscribe();
    let restore = |source: &gwt_agent::Session, id: &str, operation: &str| {
        OwnerCommand::RestoreSavedSession {
            id: id.into(),
            operation_id: operation.into(),
            source_session_id: source.id.clone(),
            expected_source: Box::new(source.clone()),
            origin: gwt_agent::SessionLaunchOrigin::UserRestart,
            cols: 80,
            rows: 24,
        }
    };
    let owned = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", sources[0].id)),
    )
    .unwrap();
    let receipt =
        owner_fixture_block_on(owner.command(restore(&owned, &owned_id, "legacy-owned-failure")))
            .unwrap();
    let failure = receipt["monitor_failure_closed"]["detail"].as_str()
        .expect("the actual missing PTY gate fails before provider spawn with completed Monitor cleanup proof");
    let events = std::iter::from_fn(|| observed.try_recv().ok()).collect::<Vec<_>>();
    let incarnation = events
        .iter()
        .find_map(|event| match event {
            OwnerEvent::WindowRemoved {
                id, incarnation, ..
            } if id == &owned_id => Some(*incarnation),
            _ => None,
        })
        .expect("actual exact Monitor window cleanup receipt");
    assert!(events.iter().any(|event| matches!(event, OwnerEvent::LaunchFailed { id, incarnation: generation, detail } if id == &owned_id && *generation == incarnation && detail.contains(failure))));
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    assert!(
        persisted
            .failed_issues
            .iter()
            .any(|failed| failed.issue_number == 42 && failed.message.contains(failure)),
        "the actual concrete failure must be durable before the owner removes its old Monitor view"
    );
    assert!(
        persisted.launched_issues.is_empty(),
        "the exact Monitor slot is released"
    );
    assert_eq!(
        events
            .iter()
            .filter(
                |event| matches!(event, OwnerEvent::WindowRemoved { id, .. } if id == &owned_id)
            )
            .count(),
        1,
        "the actual no-child finalizer completes once"
    );
    let rows =
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(&repo))
            .unwrap();
    assert!(rows.windows.iter().all(|row| row.id != "agent-42"));
    assert!(owner_fixture_block_on(owner.probe(&owned_id, incarnation)).is_err());
    let before_replay = fs::read(&prefs_path).unwrap();
    assert_eq!(
        owner_fixture_block_on(owner.command(restore(&owned, &owned_id, "legacy-owned-failure")))
            .unwrap(),
        receipt
    );
    assert_eq!(fs::read(&prefs_path).unwrap(), before_replay);
    assert!(
        !std::iter::from_fn(|| observed.try_recv().ok())
            .any(|event| matches!(event, OwnerEvent::WindowRemoved { id, .. } if id == owned_id)),
        "replay cannot finalize the same owned window twice"
    );
    let manual = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", sources[1].id)),
    )
    .unwrap();
    let receipt = owner_fixture_block_on(owner.command(restore(
        &manual,
        &manual_id,
        "legacy-manual-failure",
    )))
    .unwrap();
    assert!(
        receipt.get("launch_failure").is_some(),
        "an actual unowned failed restore retains its diagnostic Session: {receipt}"
    );
    let after_manual = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    assert_eq!(
        after_manual.failed_issues, persisted.failed_issues,
        "a manual failure cannot report a Monitor failure"
    );
    assert_eq!(after_manual.launched_issues, persisted.launched_issues);
    assert_eq!(
        after_manual.pending_launch_deliveries,
        persisted.pending_launch_deliveries
    );
    let manual_session = receipt["session_id"].as_str().unwrap();
    let retained = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{manual_session}.toml")),
    )
    .unwrap();
    assert_eq!(retained.status, gwt_agent::AgentStatus::Stopped);
    assert!(retained.restore_window_on_startup);
    let rows =
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(&repo))
            .unwrap();
    assert!(rows.windows.iter().any(|row| row.id == "agent-7"
        && row.session_id.as_deref() == Some(manual_session)
        && row.status == WindowProcessStatus::Error));
    assert!(
        !std::iter::from_fn(|| observed.try_recv().ok())
            .any(|event| matches!(event, OwnerEvent::WindowRemoved { id, .. } if id == manual_id)),
        "the manual diagnostic must remain visible"
    );
    owner_fixture_block_on(owner.shutdown());
}

/// Issue #3489 AC-1: a durable Session that carries no owner linkage must not
/// seed the continuation with `None`. Continue work mints the binding from the
/// Work owner, so an inherited `None` fails the binding install before the PTY
/// starts and leaves the pane with a bare invariant string.
#[test]
fn continue_work_durable_seed_without_owner_linkage_resolves_to_work_owner() {
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3489,
    };
    let (config, _outcome) = continuation_launch_config(
        &issue_3489_durable_seed(None),
        Path::new("/tmp/gwt-issue-3489/work"),
        owner,
        None,
    );

    assert_eq!(config.linked_issue_number, Some(owner.number));
    assert!(
        issue_3489_binding_installs(&config, owner.number),
        "an unbound durable seed must still install the owner binding before the PTY starts"
    );
}

/// Issue #3489 AC-2: a durable Session linked to another owner must be
/// re-resolved to the Work owner the continuation binding is minted for.
#[test]
fn continue_work_durable_seed_with_foreign_owner_resolves_to_work_owner() {
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3489,
    };
    let (config, _outcome) = continuation_launch_config(
        &issue_3489_durable_seed(Some(3464)),
        Path::new("/tmp/gwt-issue-3489/work"),
        owner,
        None,
    );

    assert_eq!(config.linked_issue_number, Some(owner.number));
    assert!(
        issue_3489_binding_installs(&config, owner.number),
        "a foreign-owner durable seed must be re-resolved before the PTY starts"
    );
}

#[test]
fn continue_work_from_autonomous_session_uses_manual_launch_route() {
    let mut seed = issue_3489_durable_seed(Some(4217));
    let ContinueWorkLaunchSeed::DurableSession(session) = &mut seed else {
        unreachable!("durable fixture");
    };
    session.launch_route = gwt_agent::LaunchRoute::Autonomous;
    // Restoring the same launch must retain its route; an explicit Continue
    // work action starts a new manual launch from that conversation.
    assert_eq!(
        super::super::launch_config_from_persisted_session(session).launch_route,
        gwt_agent::LaunchRoute::Autonomous
    );
    let (config, _) = continuation_launch_config(
        &seed,
        Path::new("/tmp/gwt-issue-3489/work"),
        gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 4217,
        },
        None,
    );

    assert_eq!(config.launch_route, gwt_agent::LaunchRoute::Manual);
}

/// Issue #3489 AC-2: the Work projection seed keeps the same single source of
/// owner truth, so both seeds stay interchangeable for the binding install.
#[test]
fn continue_work_projection_seed_resolves_to_work_owner() {
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3489,
    };
    let (config, _outcome) = continuation_launch_config(
        &ContinueWorkLaunchSeed::WorkProjection {
            agent_id: gwt_agent::AgentId::Codex,
            display_name: Some("Projection seed".to_string()),
            branch: "work/issue-3489".to_string(),
        },
        Path::new("/tmp/gwt-issue-3489/work"),
        owner,
        None,
    );

    assert_eq!(config.linked_issue_number, Some(owner.number));
    assert_eq!(config.display_name, "Projection seed");
}

/// Issue #3489 AC-4: the owner-mismatch refusal fails before the PTY starts, so
/// the pane only ever shows this one line. It must name both sides of the
/// mismatch and the recovery route instead of the bare invariant string.
#[test]
fn owner_mismatch_launch_failure_names_both_owners_and_the_recovery_route() {
    let mut session = gwt_agent::Session::new(
        Path::new("/tmp/gwt-issue-3489"),
        "work/issue-3489",
        gwt_agent::AgentId::Codex,
    );
    session.id = "continuation-3489".to_string();
    session.repo_hash = Some("repo-3489".to_string());
    session.linked_issue_number = Some(3464);
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session.id.clone(),
        repo_hash: "repo-3489".to_string(),
        owner_kind: "issue".to_string(),
        owner_number: 3489,
        identity: gwt_agent::ExecutionBindingIdentity {
            generation_id: "generation-3489".to_string(),
            binding_id: "binding-3489".to_string(),
            ledger_head_hash: "head-3489".to_string(),
        },
        capability_generation: 1,
    };
    let detail = session
        .set_execution_binding(Some(binding))
        .expect_err("a foreign linked owner must be refused");

    assert!(
        detail.contains("continuation-3489") && detail.contains("3464") && detail.contains("3489"),
        "the refusal must name the Session and both owners: {detail}"
    );

    let diagnostic = String::from_utf8(AppRuntime::launch_error_terminal_bytes(
        &AppRuntime::user_facing_launch_error_detail(&detail),
    ))
    .expect("diagnostic bytes are UTF-8");
    assert!(
        diagnostic.contains("execution.status"),
        "the pane diagnostic must name the recovery probe: {diagnostic}"
    );
    assert!(
        diagnostic.contains("Continue work"),
        "the pane diagnostic must name the continuation route: {diagnostic}"
    );
}

/// Issue #3489 AC-4: unrelated launch failures keep their exact detail, so the
/// recovery hint never widens beyond the owner-mismatch refusal.
#[test]
fn unrelated_launch_failure_detail_is_not_rewritten_with_the_owner_hint() {
    let detail = "execution binding session does not match the durable Session";

    assert_eq!(
        AppRuntime::user_facing_launch_error_detail(detail),
        detail,
        "only the owner mismatch gains the recovery route"
    );
}

/// Issue #3489 AC-1: the owner-linkage regression above proves the binding
/// install accepts the resolved config. That only matters because the install
/// runs on the pre-PTY path — pin that ordering so a later refactor cannot move
/// the refusal behind a started pane, where it would stop being a launch
/// failure at all.
#[test]
fn launch_worker_installs_the_execution_binding_before_the_process_launch() {
    let source = include_str!("../../project_runtime/launch.rs");
    let worker = source
        .split("pub fn prepare_agent_launch")
        .nth(1)
        .and_then(|tail| tail.split("pub fn resolve_agent_resource_launch").next())
        .expect("project owner launch preparation body");
    let install = worker
        .find("set_execution_binding(Some(binding.clone()))?")
        .expect("prepared continuation binding install");
    let process_launch = worker
        .find("let process_launch = ProcessLaunch {")
        .expect("process launch construction");
    assert!(
        install < process_launch,
        "the execution binding must be installed before the PTY process launch is built"
    );
}

#[test]
fn bootstrap_settles_update_resume_marker_and_bypasses_auto_resume_freshness() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (worktree, mut runtime) = update_resume_fixture(temp.path(), "work/update-resume");
    run_git(
        &worktree,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let current_version = env!("CARGO_PKG_VERSION");
    let marker = update_resume_marker_for(&worktree, current_version);
    assert_eq!(
        runtime.update_resume_projects(),
        marker.projects,
        "the marker written at apply time records the raised drain per project"
    );
    gwt_core::update::persist_update_resume_marker(&marker).expect("persist marker");
    assert!(update_drain_is_raised(&worktree));

    runtime.bootstrap();

    assert!(
        !update_drain_is_raised(&worktree),
        "the settling bootstrap releases the update_drain hold on disk"
    );
    let restored =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&worktree))
            .expect("prefs after the settling bootstrap");
    assert!(
        restored.enabled,
        "the monitor stays enabled across the apply"
    );
    assert!(restored.autonomous_mode);
    assert_eq!(restored.max_active_agents, 3, "parallelism is restored");
    assert_eq!(
        restored
            .launched_issues
            .iter()
            .map(|launch| launch.issue_number)
            .collect::<Vec<_>>(),
        vec![4076],
        "the launches being drained stay attributable after the restart"
    );

    assert_eq!(
        runtime.pending_startup_auto_resume_sessions.len(),
        1,
        "sessions of a marker project bypass the 24h freshness gate"
    );
    assert!(
        gwt_core::update::load_update_resume_marker().is_none(),
        "a successful settle removes the marker"
    );
    let result = gwt_core::update::load_update_apply_result().expect("apply result recorded");
    assert_eq!(
        result.outcome,
        gwt_core::update::UpdateApplyOutcome::Success
    );
    assert_eq!(result.to_version, current_version);
    assert_eq!(
        runtime.update_drain_released_projects,
        vec![marker.projects[0].hash.clone()],
        "the drained project is released for the Issue Monitor (#4037 seam)"
    );

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {},
    );
    let toasts = update_resume_toasts(&events);
    assert_eq!(toasts.len(), 1, "exactly one resume notice: {toasts:?}");
    assert_eq!(toasts[0].0, "info");
    assert!(
        toasts[0]
            .1
            .contains(&format!("Updated to v{current_version}"))
            && toasts[0].1.contains("execution resumed"),
        "notice names the version and the resumption: {}",
        toasts[0].1
    );
    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(
        agent_windows, 1,
        "the stale session restarted after the update"
    );

    // Second bootstrap: no marker, no notice, no drain release (idempotent).
    let tab = sample_project_tab(
        "tab-update",
        "Update Resume",
        worktree.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut restarted = sample_runtime(temp.path(), vec![tab], Some("tab-update"));
    restarted.bootstrap();
    assert!(restarted.update_drain_released_projects.is_empty());
    assert!(!update_drain_is_raised(&worktree));
    let events = restarted.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {},
    );
    assert!(
        update_resume_toasts(&events).is_empty(),
        "no marker means no update notice"
    );
    assert_eq!(
        gwt_core::update::load_update_apply_result().expect("apply result kept"),
        result,
        "the recorded result must not be rewritten by a plain launch"
    );
}

// Issue #4038 (AC-5 / AC-7): when the restarted binary is not `to_version`
// (#3807 no-op apply), bootstrap still releases the drain, records a failure,
// notifies, bumps `attempt`, and keeps the marker. Sessions keep the normal
// freshness gate because nothing was actually applied.
#[test]
fn bootstrap_records_failed_update_resume_when_version_mismatches() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (worktree, mut runtime) = update_resume_fixture(temp.path(), "work/update-mismatch");
    let marker = update_resume_marker_for(&worktree, "0.0.1-never-installed");
    gwt_core::update::persist_update_resume_marker(&marker).expect("persist marker");
    assert!(update_drain_is_raised(&worktree));

    runtime.bootstrap();

    assert!(
        runtime.pending_startup_auto_resume_sessions.is_empty(),
        "a failed apply does not bypass the freshness gate"
    );
    assert!(
        !update_drain_is_raised(&worktree),
        "a failed apply still releases the update_drain hold (AC-5)"
    );
    let remaining = gwt_core::update::load_update_resume_marker().expect("marker kept");
    assert_eq!(remaining.attempt, 2);
    let result = gwt_core::update::load_update_apply_result().expect("apply result recorded");
    assert_eq!(
        result.outcome,
        gwt_core::update::UpdateApplyOutcome::Failure
    );
    assert_eq!(result.observed_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        runtime.update_drain_released_projects,
        vec![marker.projects[0].hash.clone()],
        "the Issue Monitor must not stay drained after a failed apply"
    );

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {},
    );
    let toasts = update_resume_toasts(&events);
    assert_eq!(toasts.len(), 1, "exactly one failure notice: {toasts:?}");
    assert_eq!(toasts[0].0, "error");
    assert!(
        toasts[0].1.contains("0.0.1-never-installed") && toasts[0].1.contains("failed"),
        "notice names the expected version and the failure: {}",
        toasts[0].1
    );
}

// Issue #4075 AC-1: startup writes the managed key into the profile-derived
// `~/.codex/config.toml` when it is absent.
#[test]
fn codex_managed_config_startup_writes_experimental_mode_into_home_codex_config() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let config_path = super::super::startup::codex_home_for_startup(None).join("config.toml");
    assert_eq!(config_path, home.path().join(".codex/config.toml"));

    super::super::startup::ensure_codex_recommended_config_at_path(
        &config_path,
        gwt_skills::CodexFeaturesSchema::AcceptsTables,
    );

    let config: toml::Value = toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(
        config
            .get("features")
            .and_then(|f| f.get("context_management"))
            .and_then(|c| c.get("experimental_mode")),
        Some(&toml::Value::Boolean(true))
    );
    assert!(
        gwt_core::error_ledger::list_since(None).unwrap().is_empty(),
        "a successful managed config write must not touch the error ledger"
    );
}

// Issue #4075: `CODEX_HOME` wins over the profile-derived home.
#[test]
fn codex_managed_config_startup_prefers_codex_home_env() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let codex_home = tempdir().expect("codex home");

    let resolved = super::super::startup::codex_home_for_startup(Some(codex_home.path().into()));

    assert_eq!(resolved, codex_home.path());
    assert_eq!(
        super::super::startup::codex_home_for_startup(Some(std::ffi::OsString::new())),
        home.path().join(".codex"),
        "an empty CODEX_HOME must fall back to the profile home"
    );
}

// Issue #4075 AC-5: an unparseable config never blocks startup and lands in
// `errors.list` with the path and cause.
#[test]
fn codex_managed_config_startup_records_operation_refusal_on_unparseable_config() {
    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let config_path = home.path().join(".codex/config.toml");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let broken = "[features
not toml";
    fs::write(&config_path, broken).unwrap();

    super::super::startup::ensure_codex_recommended_config_at_path(
        &config_path,
        gwt_skills::CodexFeaturesSchema::AcceptsTables,
    );

    assert_eq!(fs::read_to_string(&config_path).unwrap(), broken);
    let rows = gwt_core::error_ledger::list_since(None).unwrap();
    assert_eq!(
        rows.len(),
        1,
        "expected exactly one ledger row, got {rows:?}"
    );
    assert_eq!(
        rows[0].kind,
        gwt_core::error_ledger::ErrorKind::OperationRefusal
    );
    assert_eq!(rows[0].scope, gwt_core::error_ledger::ErrorScope::Host);
    assert!(
        rows[0]
            .message
            .contains("features.context_management.experimental_mode")
            && rows[0].message.contains("parse failed"),
        "ledger row must carry the key and the cause, got: {}",
        rows[0].message
    );
}

// Issue #4229 AC-5: the codex gwt launches (`bunx @openai/codex@latest`, which
// loads the table) and the `codex` on PATH are different binaries. The managed
// key follows the PATH codex, so an old one there keeps the table out.
#[test]
fn codex_managed_config_follows_path_codex_not_launch_target() {
    use gwt_skills::CodexFeaturesSchema::{AcceptsTables, BooleansOnly};

    let home = tempdir().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let config_path = home.path().join(".codex/config.toml");
    let schema_for = |version: Option<&str>| {
        super::super::startup::codex_features_schema_for_path_codex(Some(
            &gwt_agent::DetectedAgent {
                agent_id: gwt_agent::AgentId::Codex,
                version: version.map(str::to_string),
                path: std::path::PathBuf::from("codex"),
            },
        ))
    };

    assert_eq!(schema_for(Some("codex-cli 0.148.0")), BooleansOnly);
    assert_eq!(schema_for(Some("codex-cli 0.152.0")), BooleansOnly);
    assert_eq!(schema_for(Some("codex-cli 0.153.0")), AcceptsTables);
    assert_eq!(schema_for(Some("codex-cli 0.154.0")), AcceptsTables);
    assert_eq!(
        schema_for(None),
        BooleansOnly,
        "a PATH codex whose version gwt cannot read must not risk the table"
    );
    assert_eq!(
        super::super::startup::codex_features_schema_for_path_codex(None),
        AcceptsTables,
        "with no PATH codex only the gwt launch target reads the config"
    );

    super::super::startup::ensure_codex_recommended_config_at_path(
        &config_path,
        schema_for(Some("codex-cli 0.148.0")),
    );

    assert!(
        fs::read_to_string(&config_path)
            .map(|content| !content.contains("context_management"))
            .unwrap_or(true),
        "PATH codex 0.148.0 must never receive the table"
    );
    assert!(gwt_core::error_ledger::list_since(None).unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Issue #4143: session restore admission, and pre-PTY restore failures that
// used to persist an empty `Launch failed before PTY started.` pane into the
// next generation.
// ---------------------------------------------------------------------------

/// Reopened #4143 AC-5/7/8: a landed branch must not spend a PTY even
/// when diagnostic retention would keep its stopped placeholder.
#[test]
fn startup_restore_refuses_landed_worktree_before_launch() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    run_git(
        &repo,
        &["update-ref", "refs/remotes/origin/develop", "HEAD"],
    );
    let tab = restore_fixture_tab(
        "tab-landed",
        &repo,
        &[("agent-landed".into(), "session-landed".into())],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-landed"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    save_restore_fixture_session(
        &runtime.sessions_dir,
        "session-landed",
        &repo,
        Some("native-landed"),
        Some(4143),
    );
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        4143,
        "session-landed",
        "$gwt-execute #4143",
        false,
    )
    .expect("materialize fixture Work");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            "session-landed",
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle fixture Work"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    save_workspace_state(
        &workspace_state_path(&repo),
        runtime.tabs[0].workspace.persisted(),
    )
    .expect("persist the actual landed placeholder");
    let owner = owner_fixture_block_on(production_owner_fixture(&repo));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let logs = capture_tracing_events(|| {
        runtime.queue_startup_auto_resume_sessions(&HashSet::new());
        runtime.startup_auto_resume_ready_events();
        drain_queued_blocking_tasks(&tasks);
        let completion = take_remote_owner_launch_completion(match &runtime.proxy {
            AppEventProxy::Stub(events) => events,
            _ => panic!("recorded restore completion"),
        });
        assert_eq!(
            completion
                .restore_refusal
                .as_ref()
                .map(|refusal| refusal.code.as_str()),
            Some("landed_worktree"),
            "actual owner refusal: {:?}",
            completion.result.as_ref().err()
        );
        runtime.handle_remote_owner_launch_complete(completion);
    });
    assert!(runtime.pending_startup_auto_resume_sessions.is_empty());
    assert_eq!(
        restore_admission_refusals(&logs)
            .get("session-landed")
            .map(String::as_str),
        Some("landed_worktree")
    );
    let summary = restore_admission_summary(&logs);
    assert_eq!(
        summary.fields.get("suppressed").map(String::as_str),
        Some("1")
    );
    assert!(summary
        .fields
        .get("reasons")
        .unwrap()
        .contains("landed_worktree=1"));
    // An unlinked conversation has no terminal Work fact, but its landed
    // checkout still must not restart automatically.
    let mut unlinked = gwt_agent::Session::new(&repo, "work/landed", gwt_agent::AgentId::Codex);
    unlinked.agent_session_id = Some("native-unlinked-landed".into());
    assert_eq!(
        runtime.restore_admission(&unlinked, &repo, None),
        Err(super::super::startup::RestoreRefusal::LandedWorktree)
    );
    let actual = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    assert!(
        actual["panes"].as_array().unwrap().is_empty(),
        "landed restore spends no PTY: {actual}"
    );
    assert!(actual["pending"].as_array().unwrap().is_empty());
    assert!(runtime.runtimes.is_empty());
    assert!(runtime.pending_auto_resume_sources.is_empty());
    owner_fixture_block_on(owner.shutdown());
}

#[test]
fn startup_restore_removes_empty_unlinked_landed_windows_but_keeps_diagnostics() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let mut tab = restore_fixture_tab(
        "tab-landed",
        &repo,
        &[
            ("empty".into(), "session-empty".into()),
            ("diagnostic".into(), "session-diagnostic".into()),
            ("error".into(), "session-error".into()),
        ],
    );
    tab.workspace
        .set_status("error", WindowProcessStatus::Error);
    tab.workspace.set_dynamic_title_with_detail(
        "diagnostic",
        None,
        Some("The previous launch failed; inspect its diagnostic.".into()),
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-landed"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    for id in ["session-empty", "session-diagnostic", "session-error"] {
        save_restore_fixture_session(&runtime.sessions_dir, id, &repo, Some(id), None);
    }
    runtime.window_details.insert(
        combined_window_id("tab-landed", "diagnostic"),
        "The previous launch failed; inspect its diagnostic.".into(),
    );
    save_workspace_state(
        &workspace_state_path(&repo),
        runtime.tabs[0].workspace.persisted(),
    )
    .expect("persist empty and diagnostic rows for the actual owner");
    let owner = owner_fixture_block_on(production_owner_fixture(&repo));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.queue_startup_auto_resume_sessions(&HashSet::new());
    runtime.startup_auto_resume_ready_events();
    let events = drain_owner_launch_fixtures(&mut runtime, &tasks);
    let actual = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    assert!(
        actual["panes"].as_array().unwrap().is_empty(),
        "landed sources spend no PTY: {actual}"
    );
    let durable_rows = gwt::load_workspace_state(&workspace_state_path(&repo)).unwrap();
    let durable_empty =
        gwt_agent::Session::load(&runtime.sessions_dir.join("session-empty.toml")).unwrap();

    assert!(runtime.pending_startup_auto_resume_sessions.is_empty());
    let windows = &runtime
        .tab("tab-landed")
        .unwrap()
        .workspace
        .persisted()
        .windows;
    assert_eq!(
        windows.len(),
        2,
        "only the empty landed placeholder disappears; actual inventory={actual}; durable rows={durable_rows:?}; empty source restore={}; events={events:?}",
        durable_empty.restore_window_on_startup,
    );
    assert!(windows.iter().all(|window| window.id != "empty"));
    assert_eq!(
        windows
            .iter()
            .find(|window| window.id == "error")
            .unwrap()
            .status,
        WindowProcessStatus::Error
    );
    let empty = gwt_agent::Session::load(&runtime.sessions_dir.join("session-empty.toml"))
        .expect("load removed session");
    assert!(!empty.restore_window_on_startup);
    runtime.restore_open_project_windows("tab-landed");
    drain_owner_launch_fixtures(&mut runtime, &tasks);
    assert!(runtime.pending_auto_resume_sources.is_empty());
    assert!(runtime
        .window_details
        .contains_key(&combined_window_id("tab-landed", "diagnostic")));
    let canonical = gwt::load_workspace_state(&workspace_state_path(&repo)).unwrap();
    assert_eq!(canonical.windows.len(), 2);
    assert_eq!(
        canonical
            .windows
            .iter()
            .find(|row| row.id == "diagnostic")
            .unwrap()
            .dynamic_title_detail
            .as_deref(),
        Some("The previous launch failed; inspect its diagnostic.")
    );
    owner_fixture_block_on(owner.shutdown());
}

#[test]
fn open_project_empty_landed_cleanup_preserves_same_session_diagnostic_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let mut tab = restore_fixture_tab(
        "tab-landed",
        &repo,
        &[
            ("diagnostic".into(), "session-shared".into()),
            ("empty".into(), "session-shared".into()),
        ],
    );
    tab.workspace.set_dynamic_title_with_detail(
        "diagnostic",
        None,
        Some("Retained failure details".into()),
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-landed"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    save_restore_fixture_session(
        &runtime.sessions_dir,
        "session-shared",
        &repo,
        Some("native-shared"),
        None,
    );
    runtime.window_details.insert(
        combined_window_id("tab-landed", "diagnostic"),
        "Retained failure details".into(),
    );
    save_workspace_state(
        &workspace_state_path(&repo),
        runtime.tabs[0].workspace.persisted(),
    )
    .expect("persist both exact aliases of the same source Session");
    let owner = owner_fixture_block_on(production_owner_fixture(&repo));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.restore_open_project_windows("tab-landed");
    drain_queued_blocking_tasks(&tasks);
    let completion = take_remote_owner_launch_completion(match &runtime.proxy {
        AppEventProxy::Stub(events) => events,
        _ => panic!("recorded restore completion"),
    });
    let refusal = completion.restore_refusal.clone();
    let requested_alias = completion.window_id.clone();
    let events = runtime.handle_remote_owner_launch_complete(completion);
    let actual = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    assert!(
        actual["panes"].as_array().unwrap().is_empty(),
        "landed restore spends no PTY: {actual}"
    );
    let durable_rows = gwt::load_workspace_state(&workspace_state_path(&repo)).unwrap();
    let durable_source =
        gwt_agent::Session::load(&runtime.sessions_dir.join("session-shared.toml")).unwrap();
    let windows = &runtime
        .tab("tab-landed")
        .unwrap()
        .workspace
        .persisted()
        .windows;
    assert_eq!(windows.len(), 1);
    assert_eq!(
        windows[0].id, "diagnostic",
        "remove the exact empty window, not another window sharing its Session; requested alias={requested_alias}; refusal={refusal:?}; actual inventory={actual}; durable rows={durable_rows:?}; source restore={}; events={events:?}",
        durable_source.restore_window_on_startup,
    );
    let canonical = gwt::load_workspace_state(&workspace_state_path(&repo)).unwrap();
    assert_eq!(
        canonical.windows.len(),
        1,
        "the owner retains the same diagnostic alias"
    );
    assert_eq!(canonical.windows[0].id, "diagnostic");
    assert_eq!(
        canonical.windows[0].dynamic_title_detail.as_deref(),
        Some("Retained failure details")
    );
    assert!(runtime.pending_auto_resume_sources.is_empty());
    owner_fixture_block_on(owner.shutdown());
}

#[test]
fn startup_restore_queues_only_one_session_per_worktree() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    run_git(&repo, &["checkout", "-b", "work/restore-fixture"]);
    run_git(
        &repo,
        &["commit", "--allow-empty", "-m", "unlanded restore fixture"],
    );
    let tab = restore_fixture_tab(
        "tab-duplicate",
        &repo,
        &[
            ("agent-a".into(), "session-a".into()),
            ("agent-b".into(), "session-b".into()),
        ],
    );
    let (mut runtime, recorded) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-duplicate"));
    runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
    pin_live_pm_fixture_agents(&runtime, temp.path());
    for id in ["session-a", "session-b"] {
        save_restore_fixture_session(&runtime.sessions_dir, id, &repo, Some(id), None);
    }
    save_workspace_state(
        &workspace_state_path(&repo),
        runtime.tabs[0].workspace.persisted(),
    )
    .expect("persist both saved-source placeholders");
    let owner = owner_fixture_block_on(production_owner_fixture(&repo));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.queue_startup_auto_resume_sessions(&HashSet::new());
    assert_eq!(
        runtime.pending_startup_auto_resume_sessions.len(),
        2,
        "source routing drafts do not grant a second physical owner"
    );
    // Opening the same project while startup restore is queued must not
    // start a second process for that worktree.
    runtime.restore_open_project_windows("tab-duplicate");
    assert!(runtime.pending_auto_resume_sources.is_empty());
    runtime.startup_auto_resume_ready_events();
    drain_queued_blocking_tasks(&tasks);
    let mut installed = 0;
    let mut refused = 0;
    for _ in 0..2 {
        let completion = take_remote_owner_launch_completion(&recorded);
        match &completion.result {
            Ok(super::super::RemoteOwnerLaunchOutcome::Runtime(..)) => installed += 1,
            Err(_)
                if completion
                    .restore_refusal
                    .as_ref()
                    .is_some_and(|refusal| refusal.code == "worktree_already_restoring") =>
            {
                refused += 1
            }
            _ => panic!(
                "actual same-worktree restore outcome: error={:?}; refusal={:?}",
                completion.result.as_ref().err(),
                completion.restore_refusal
            ),
        }
        runtime.handle_remote_owner_launch_complete(completion);
    }
    assert_eq!((installed, refused), (1, 1));
    let actual = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    assert_eq!(
        actual["panes"].as_array().unwrap().len(),
        1,
        "the physical owner admits exactly one saved conversation: {actual}"
    );
    assert!(actual["pending"].as_array().unwrap().is_empty());
    assert_eq!(runtime.runtimes.len(), 1);
    let id = runtime.runtimes.keys().next().unwrap().clone();
    let projection = &runtime.runtimes[&id];
    let pane =
        owner_fixture_block_on(owner.probe(&projection.owner_pane_id, projection.incarnation))
            .unwrap();
    assert!(pane.pty.try_wait().unwrap().is_none());
    close_test_owner_runtime(&mut runtime, &id);
    assert!(pane.pty.tree_exit_confirmed());
    owner_fixture_block_on(owner.shutdown());
}
