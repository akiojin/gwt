use super::*;

#[test]
fn app_runtime_agent_launch_completion_failure_emits_structured_error_log() {
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

    let events = capture_tracing_events(|| {
        let _ = runtime.handle_launch_complete_and_drain(
            window_id.clone(),
            Err("launch failed before process spawn".into()),
        );
    });

    let event = events
        .iter()
        .find(|event| {
            event.level == Level::ERROR
                && event.target == "gwt::agent_launch"
                && event.fields.get("stage").map(String::as_str) == Some("launch_complete")
        })
        .expect("agent launch completion failure log");
    assert_eq!(
        event.fields.get("window_id").map(String::as_str),
        Some(window_id.as_str())
    );
    assert_eq!(
        event.fields.get("tab_id").map(String::as_str),
        Some("tab-1")
    );
    assert_eq!(
        event.fields.get("error").map(String::as_str),
        Some("launch failed before process spawn")
    );
}

#[test]
fn app_runtime_agent_launch_completion_failure_writes_diagnostic_to_terminal() {
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
    let events = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Err("launch failed before process spawn".into()),
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::TerminalStatus { id, status, detail, .. }
                if id == &window_id
                    && *status == WindowProcessStatus::Error
                    && detail.as_deref() == Some("launch failed before process spawn")
        )
    }));
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
    assert!(
        diagnostic.contains("Launch failed before PTY started"),
        "diagnostic must explain that no PTY output exists yet: {diagnostic:?}"
    );
    assert!(
        diagnostic.contains("launch failed before process spawn"),
        "diagnostic must include the launch error detail: {diagnostic:?}"
    );
}

#[test]
fn stale_pre_pty_launch_failure_preserves_live_agent_and_monitor_delivery() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().to_path_buf(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    let delivery_id = "launch:stale-pre-pty";
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    runtime
        .window_pty_statuses
        .insert(window_id.clone(), WindowProcessStatus::Running);
    runtime.pending_launch_feedback_contexts.insert(
        window_id.clone(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(3851),
            issue_monitor_delivery_id: Some(delivery_id.to_string()),
            issue_monitor_project_root: Some(temp.path().to_path_buf()),
            issue_monitor_session_mode: Some(gwt_agent::SessionMode::Normal),
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );
    runtime.issue_monitor_launch_deliveries.insert(
        delivery_id.to_string(),
        super::super::IssueMonitorLaunchDeliveryState::Materializing {
            window_id: window_id.clone(),
            started_at: Instant::now(),
        },
    );

    let events = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Err("stale preparation failure".into()),
    );

    let feedback_retained = runtime
        .pending_launch_feedback_contexts
        .contains_key(&window_id);
    let delivery_retained = matches!(
        runtime.issue_monitor_launch_deliveries.get(delivery_id),
        Some(super::super::IssueMonitorLaunchDeliveryState::Materializing {
            window_id: delivery_window_id,
            ..
        }) if delivery_window_id == &window_id
    );
    let runtime_retained = runtime.runtimes.contains_key(&window_id);
    let active_session_retained = runtime.active_agent_sessions.contains_key(&window_id);
    let status = runtime.window_status(&window_id);
    let diagnostic_retained = runtime
        .launch_error_terminal_details
        .contains_key(&window_id);
    runtime.active_agent_sessions.remove(&window_id);
    runtime.stop_window_runtime_without_session_projection(&window_id);

    assert!(
        events.is_empty(),
        "a stale pre-PTY result must not emit launch failure events: {events:#?}"
    );
    assert!(
        feedback_retained,
        "the current launch feedback must survive"
    );
    assert!(
        delivery_retained,
        "the current Monitor delivery must survive"
    );
    assert!(runtime_retained, "the live PTY runtime must survive");
    assert!(active_session_retained, "the active Session must survive");
    assert!(
        status.is_some_and(|status| !matches!(
            status,
            WindowProcessStatus::Stopped | WindowProcessStatus::Error
        )),
        "the composed live-agent status must remain non-terminal: {status:?}"
    );
    assert!(
        !diagnostic_retained,
        "a stale result must not append the pre-PTY diagnostic"
    );
}

#[test]
fn review_dispatch_feedback_context_does_not_own_the_issue_launch_binding() {
    // Issue #4041: the independent review window observes the Issue but never
    // materializes its launch.
    let mut context = issue_monitor_feedback(42);
    assert_eq!(
        context.issue_monitor_launch_binding_issue_number(),
        Some(42)
    );
    context.issue_monitor_review_dispatch = true;
    assert_eq!(context.issue_monitor_launch_binding_issue_number(), None);
}

#[test]
fn review_dispatch_launch_completion_keeps_the_implementation_binding() {
    // Issue #4041: on 2026-09-06 the review dispatches for #4037 / #4033 were
    // ACKed as the Issues' launches, so `launched_issues` moved to the review
    // windows, the implementation claims vanished, and the implementation
    // windows lost their Work authority. A review window's launch completion
    // must leave the Issue's binding, claim, and audit untouched; an ordinary
    // launch completion for the same Issue still binds (and is audited as a
    // replacement, AC-3).
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-review-dispatch-binding");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "tab-1::agent-impl".to_string(),
            }],
            launched_claims: std::collections::BTreeMap::from([(42, "claim-impl".to_string())]),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed the running implementation launch");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-review",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let review_window_id = combined_window_id("tab-1", "agent-review");
    runtime
        .window_hook_states
        .insert(review_window_id.clone(), WindowProcessStatus::Running);
    runtime.pending_launch_feedback_contexts.insert(
        review_window_id.clone(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: Some(repo.clone()),
            issue_monitor_session_mode: Some(gwt_agent::SessionMode::Normal),
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: true,
        },
    );

    let events = runtime.handle_launch_complete_and_drain(
        review_window_id.clone(),
        Ok(issue_monitor_review_launch_completion(
            &repo,
            "review-session-42",
        )),
    );
    runtime.stop_window_runtime_without_session_projection(&review_window_id);

    assert!(
        !events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::TerminalStatus { id, status, .. }
                if id == &review_window_id && *status == WindowProcessStatus::Error
        )),
        "the review window must spawn: {events:#?}"
    );
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(
        prefs.launched_issues,
        vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: "tab-1::agent-impl".to_string(),
        }],
        "a review dispatch must not take over the implementation binding"
    );
    assert_eq!(
        prefs.launched_claims.get(&42).map(String::as_str),
        Some("claim-impl"),
        "a review dispatch must not drop the implementation claim"
    );
    assert!(
        prefs.requeue_audit.is_empty(),
        "nothing was replaced, so nothing is audited: {:?}",
        prefs.requeue_audit
    );
}

#[test]
fn genesis_pty_spawn_failure_terminalizes_generation_and_allows_successor_retry() {
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

    let events = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Ok((
            ProcessLaunch {
                initial_prompt_file: None,
                command: "/definitely/missing/gwt-agent".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(repo.clone()),
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
            repo.display().to_string().into(),
        )),
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::TerminalStatus { id, status, .. }
                if id == &window_id && *status == WindowProcessStatus::Error
        )
    }));
    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read failed genesis ledger")
        .expect("failed genesis ledger");
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
        "PTY spawn failure must never leave a ghost Active generation",
    );
    assert!(
        !runtime.active_agent_sessions.contains_key(&window_id),
        "a failed genesis pane must not remain a process-local Active owner",
    );
    assert!(
        !runtime
            .sessions_dir
            .join(format!("{session_id}.toml"))
            .exists(),
        "the exact failed genesis Session must be removed",
    );
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read WorkItems after failed genesis");
    assert!(
        work_items.is_none_or(|items| items.work_items.is_empty()),
        "a pre-readiness genesis failure must not synthesize a Paused ghost Work",
    );
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "genesis-pty-retry".to_string(),
        principal_id: "gwt-host-launch".to_string(),
        work_id: None,
        source: gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
        session_binding_id: "genesis-pty-retry-binding".to_string(),
        initial_session_id: "genesis-pty-retry-session".to_string(),
        entrypoint: "$gwt-execute #2359".to_string(),
        requested_at: Utc::now(),
    };
    gwt::cli::execution_state::prepare_fresh_linked_owner_launch_successor(&repo, owner, &request)
        .expect("terminal genesis must permit a fresh successor retry");
}

#[test]
fn genesis_receipt_cleanup_failure_discards_published_work_and_active_owner() {
    assert_genesis_receipt_cleanup_failure(false, false);
}

#[test]
fn genesis_receipt_cleanup_failure_evicts_cached_session_after_window_close() {
    assert_genesis_receipt_cleanup_failure(false, true);
}

#[test]
fn genesis_receipt_cleanup_failure_evicts_cached_session_when_window_already_closed() {
    assert_genesis_receipt_cleanup_failure(true, false);
}

#[test]
fn ordinary_blocked_transition_is_not_genesis_failure_compensation_authority() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-ordinary-blocked-genesis-receipt");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "ordinary-blocked-session";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize generation ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read binding")
        .expect("current binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let context = WorkspaceResumeContext {
        title: Some("Ordinary blocked Work".to_string()),
        owner: Some("Issue #2359".to_string()),
        summary: None,
        next_action: None,
    };
    let mut session = sample_active_agent_session("tab-1", "window-ordinary-blocked");
    session.session_id = session_id.to_string();
    session.branch_name = "work/issue-2359".to_string();
    session.worktree_path = repo.clone();
    session.agent_project_root = repo.display().to_string();
    save_workspace_launch_projection(
        &repo,
        &session,
        Some("develop"),
        Some(owner.number),
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([session.session_id.clone()])),
    )
    .expect("publish Work");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            session_id,
            gwt::cli::execution_state::ExecutionSettlement::Blocked {
                reason: "ordinary agent blocker".to_string(),
                missing_verification: Some("ordinary verification gap".to_string()),
            },
        )
        .expect("settle ordinary blocker"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));

    assert!(
        compensate_terminalized_genesis_workspace_projection(
            &repo,
            &repo,
            owner,
            session_id,
            Some(&binding),
            Some(&session.branch_name),
        )
        .is_err(),
        "ordinary execution.blocked must not authorize failed-genesis Work compensation",
    );
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read WorkItems")
        .expect("WorkItems");
    assert_eq!(work_items.work_items.len(), 1);
    assert!(!work_items.work_items[0].discarded);
}

#[test]
fn terminalized_genesis_compensation_uses_repo_global_work_items() {
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
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3412,
    };
    let session_id = "genesis-split-root-session";
    gwt::cli::execution_state::materialize_at_launch(
        &worktree,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #3412",
        false,
    )
    .expect("materialize execution");
    gwt::cli::execution_state::ensure_generation_ledger(
        &worktree,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("materialize generation ledger");
    let identity = gwt::cli::execution_state::current_execution_binding(&worktree, owner)
        .expect("read binding")
        .expect("current binding");
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&worktree).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let context = WorkspaceResumeContext {
        title: Some("Failed genesis split-root Work".to_string()),
        owner: Some("Issue #3412".to_string()),
        summary: None,
        next_action: None,
    };
    let mut session = sample_active_agent_session("tab-1", "window-genesis-split-root");
    session.session_id = session_id.to_string();
    session.branch_name = "work/issue-3412".to_string();
    session.worktree_path = worktree.clone();
    session.agent_project_root = project_root.display().to_string();
    save_workspace_launch_projection(
        &project_root,
        &session,
        Some("develop"),
        Some(owner.number),
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([session.session_id.clone()])),
    )
    .expect("publish split-root Work");
    gwt::cli::execution_state::block_uncommitted_genesis_launch(
        &worktree,
        owner,
        session_id,
        &binding.identity,
        "test split-root genesis compensation",
    )
    .expect("terminalize failed genesis");

    compensate_terminalized_genesis_workspace_projection(
        &project_root,
        &worktree,
        owner,
        session_id,
        Some(&binding),
        Some(&session.branch_name),
    )
    .expect("compensate split-root Work");

    // Issue #4674: a fresh intake must retain the machine-local close even
    // though shared lifecycle sources deliberately exclude close events.
    let intake = crate::work_events_ingest::ingest_project_work_events_paths(
        &project_root,
        &gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root),
        &gwt_core::paths::gwt_workspace_work_events_intake_state_path_for_repo_path(&project_root),
    );
    assert!(
        intake.projection_rebuilt,
        "fresh intake must rebuild: {intake:?}"
    );

    let current = gwt_core::workspace_projection::load_workspace_projection(&project_root)
        .expect("read current")
        .expect("current");
    assert!(current.latest_agent_for_session(session_id).is_none());
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
        .expect("read repo-global WorkItems")
        .expect("repo-global WorkItems");
    assert_eq!(work_items.work_items.len(), 1);
    assert!(work_items.work_items[0].discarded);
    assert_eq!(
        materialized_project_state_sots("works.json"),
        vec![gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root)],
        "terminalization must leave exactly one repository WorkItems SOT"
    );
    assert!(load_tracked_work_events(&worktree)
        .iter()
        .any(|event| event.agent_session_id.as_deref() == Some(session_id)));
    assert!(!load_tracked_work_events(&worktree)
        .iter()
        .any(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Discard));
    let closes = fs::read_to_string(
        gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&project_root),
    )
    .expect("read machine-local genesis close log");
    assert!(closes.lines().any(|line| {
        let event: gwt_core::workspace_projection::WorkEvent =
            serde_json::from_str(line).expect("decode close event");
        event.kind == gwt_core::workspace_projection::WorkEventKind::Discard
            && event.agent_session_id.as_deref() == Some(session_id)
    }));
}

#[test]
fn terminalized_genesis_compensation_pauses_instead_of_discarding_resumed_work() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-resume-compensation");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let failed_session_id = "genesis-failed-resume";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        failed_session_id,
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
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: failed_session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let context = WorkspaceResumeContext {
        title: Some("Existing Work".to_string()),
        owner: Some("Issue #2359".to_string()),
        summary: Some("Keep this Work after a failed resume".to_string()),
        next_action: Some("Continue from the prior conversation".to_string()),
    };
    let mut prior = sample_active_agent_session("tab-1", "window-prior");
    prior.session_id = "prior-session".to_string();
    prior.branch_name = "work/issue-2359".to_string();
    prior.worktree_path = repo.clone();
    prior.agent_project_root = repo.display().to_string();
    save_workspace_launch_projection(
        &repo,
        &prior,
        Some("develop"),
        Some(owner.number),
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([prior.session_id.clone()])),
    )
    .expect("publish existing Work");
    let existing = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read existing WorkItems")
        .expect("existing WorkItems")
        .work_items
        .into_iter()
        .next()
        .expect("existing Work");
    gwt_core::workspace_projection::record_workspace_work_paused_event(
        &repo,
        &existing.id,
        Some(&existing.title),
        existing.summary.as_deref(),
        existing.owner.as_deref(),
        &existing.board_refs,
        existing.execution_containers.first().cloned(),
        Some(&prior.session_id),
        Utc::now(),
    )
    .expect("pause prior Session");
    gwt_core::workspace_projection::mark_workspace_agent_stopped(&repo, &prior.session_id, None)
        .expect("remove prior Session");

    let mut failed = prior.clone();
    failed.window_id = "window-failed".to_string();
    failed.session_id = failed_session_id.to_string();
    save_resumed_workspace_projection(
        &repo,
        &failed,
        None,
        Some(owner.number),
        &context,
        Some(&HashSet::from([failed.session_id.clone()])),
    )
    .expect("publish failed resume attempt");
    gwt::cli::execution_state::block_uncommitted_genesis_launch(
        &repo,
        owner,
        failed_session_id,
        &binding.identity,
        "test failed resume compensation",
    )
    .expect("terminalize failed genesis");
    let successor = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "genesis-late-cleanup-successor".to_string(),
        principal_id: "gwt-host-launch".to_string(),
        work_id: None,
        source: gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
        session_binding_id: "genesis-late-cleanup-binding".to_string(),
        initial_session_id: "genesis-late-cleanup-session".to_string(),
        entrypoint: "$gwt-execute #2359".to_string(),
        requested_at: Utc::now(),
    };
    gwt::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
        &repo, owner, &successor,
    )
    .expect("prepare successor before late genesis cleanup");
    gwt::cli::execution_state::activate_successor(&repo, owner, &successor)
        .expect("activate successor before late genesis cleanup");
    assert_ne!(
        gwt::cli::execution_state::current_execution_binding(&repo, owner)
            .expect("read successor binding"),
        Some(binding.identity.clone()),
        "the genesis generation must be historical before compensation",
    );

    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        failed_session_id,
        Some(&binding),
        Some(&failed.branch_name),
    )
    .expect("compensate failed resume without deleting existing Work");
    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        failed_session_id,
        Some(&binding),
        Some(&failed.branch_name),
    )
    .expect("retry after Pause and exact agent cleanup");

    let intake = crate::work_events_ingest::ingest_project_work_events_paths(
        &repo,
        &gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo),
        &gwt_core::paths::gwt_workspace_work_events_intake_state_path_for_repo_path(&repo),
    );
    assert!(
        intake.projection_rebuilt,
        "fresh intake must rebuild: {intake:?}"
    );

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("read compensated Workspace")
        .expect("compensated Workspace");
    assert!(projection
        .latest_agent_for_session(failed_session_id)
        .is_none());
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read compensated WorkItems")
        .expect("compensated WorkItems");
    assert_eq!(work_items.work_items.len(), 1);
    let retained = &work_items.work_items[0];
    assert_eq!(retained.id, existing.id);
    assert!(
        !retained.discarded,
        "a resumed Work must never be discarded"
    );
    assert_eq!(
        retained.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Idle,
    );
    assert!(retained.events.iter().any(|event| {
        event.kind == gwt_core::workspace_projection::WorkEventKind::Resume
            && event.agent_session_id.as_deref() == Some(failed_session_id)
    }));
    assert!(retained.events.iter().any(|event| {
        event.kind == gwt_core::workspace_projection::WorkEventKind::Pause
            && event.agent_session_id.as_deref() == Some(failed_session_id)
    }));
    assert!(!load_tracked_work_events(&repo)
        .iter()
        .any(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Pause));
    let closes = fs::read_to_string(
        gwt_core::paths::gwt_workspace_work_events_closed_path_for_repo_path(&repo),
    )
    .expect("read machine-local resume close log");
    assert!(closes.lines().any(|line| {
        let event: gwt_core::workspace_projection::WorkEvent =
            serde_json::from_str(line).expect("decode close event");
        event.kind == gwt_core::workspace_projection::WorkEventKind::Pause
            && event.agent_session_id.as_deref() == Some(failed_session_id)
    }));
}

#[test]
fn terminalized_genesis_compensation_retries_after_discard_before_agent_cleanup() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-discard-retry");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-discard-response-loss";
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
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let context = WorkspaceResumeContext {
        title: Some("New failed Work".to_string()),
        owner: Some("Issue #2359".to_string()),
        summary: None,
        next_action: None,
    };
    let mut session = sample_active_agent_session("tab-1", "window-discard-retry");
    session.session_id = session_id.to_string();
    session.branch_name = "work/issue-2359".to_string();
    session.worktree_path = repo.clone();
    session.agent_project_root = repo.display().to_string();
    save_workspace_launch_projection(
        &repo,
        &session,
        Some("develop"),
        Some(owner.number),
        None,
        Some(&context),
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([session.session_id.clone()])),
    )
    .expect("publish failed new Work");
    let published_projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("read published Workspace")
        .expect("published Workspace");
    gwt::cli::execution_state::block_uncommitted_genesis_launch(
        &repo,
        owner,
        session_id,
        &binding.identity,
        "test discard response loss",
    )
    .expect("terminalize failed genesis");
    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        session_id,
        Some(&binding),
        Some(&session.branch_name),
    )
    .expect("discard new Work");
    gwt_core::workspace_projection::save_workspace_projection(&repo, &published_projection)
        .expect("restore pre-agent-cleanup projection after simulated response loss");

    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        session_id,
        Some(&binding),
        Some(&session.branch_name),
    )
    .expect("retry exact agent cleanup after durable Discard");
    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        session_id,
        Some(&binding),
        Some(&session.branch_name),
    )
    .expect("retry after Discard and exact agent cleanup");

    let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
        .expect("read retried Workspace")
        .expect("retried Workspace");
    assert!(projection.latest_agent_for_session(session_id).is_none());
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read retried WorkItems")
        .expect("retried WorkItems");
    assert_eq!(work_items.work_items.len(), 1);
    assert!(work_items.work_items[0].discarded);
    assert_eq!(
        work_items.work_items[0]
            .events
            .iter()
            .filter(|event| {
                event.kind == gwt_core::workspace_projection::WorkEventKind::Discard
            })
            .count(),
        1,
        "retry must not append a duplicate Discard event",
    );
}

#[test]
fn terminalized_genesis_compensation_does_not_synthesize_state_for_unrelated_work() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-no-target");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-without-work-target";
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
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let mut unrelated = gwt_core::workspace_projection::WorkEvent::new(
        gwt_core::workspace_projection::WorkEventKind::Start,
        "unrelated-work",
        Utc::now(),
    );
    unrelated.agent_session_id = Some("unrelated-session".to_string());
    gwt_core::workspace_projection::record_workspace_work_event(&repo, unrelated)
        .expect("persist unrelated Work without current projection");
    assert!(
        gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("read absent current projection")
            .is_none(),
    );
    gwt::cli::execution_state::block_uncommitted_genesis_launch(
        &repo,
        owner,
        session_id,
        &binding.identity,
        "test no-target compensation",
    )
    .expect("terminalize failed genesis");

    compensate_terminalized_genesis_workspace_projection(
        &repo,
        &repo,
        owner,
        session_id,
        Some(&binding),
        None,
    )
    .expect("leave unrelated Work untouched");

    assert!(
        gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("read current projection after no-target compensation")
            .is_none(),
        "no-target compensation must not synthesize a current projection",
    );
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("read unrelated WorkItems")
        .expect("unrelated WorkItems");
    assert_eq!(work_items.work_items.len(), 1);
    assert_eq!(work_items.work_items[0].id, "unrelated-work");
    assert!(!work_items.work_items[0].discarded);
}

#[test]
fn startup_genesis_receipt_imports_flat_authority_and_terminalizes_crash_window() {
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
    let session_id = "genesis-flat-crash-window";
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        session_id,
        "$gwt-execute #2359",
        false,
    )
    .expect("materialize flat genesis authority");
    assert!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read absent genesis ledger")
            .is_none()
    );
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    persist_durable_launch_recovery(
        &runtime.sessions_dir,
        DurableLaunchRecoveryKind::Genesis,
        session_id,
        &repo,
        &repo,
        owner,
        None,
        None,
    )
    .expect("persist genesis crash receipt");

    runtime.reconcile_durable_fresh_execution_launches();

    let ledger = gwt::cli::execution_state::load_generation_ledger(&repo, owner)
        .expect("read recovered genesis ledger")
        .expect("recovered genesis ledger");
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
    );
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
}

#[cfg(unix)]
#[test]
fn startup_genesis_recovery_retains_dangling_session_entry_without_mutation() {
    use std::os::unix::fs::symlink;

    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-dangling-session");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-dangling-session";
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
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(repo.clone());
    session.linked_issue_number = Some(owner.number);
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
    .expect("persist genesis receipt");
    let session_path = runtime.sessions_dir.join(format!("{session_id}.toml"));
    fs::remove_file(&session_path).expect("remove Session before dangling entry");
    let dangling_target = runtime.sessions_dir.join("missing-session-target");
    symlink(&dangling_target, &session_path).expect("create dangling Session entry");
    let receipt_path = runtime
        .sessions_dir
        .join("execution-launch-recovery")
        .join(format!("{session_id}.json"));
    let receipt_before = fs::read(&receipt_path).expect("read recovery receipt");
    let authority_before =
        snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));
    let work_before = tracked_workspace_work_store_snapshot(&repo);

    runtime.reconcile_durable_fresh_execution_launches();

    assert!(fs::symlink_metadata(&session_path)
        .expect("dangling Session entry must remain")
        .file_type()
        .is_symlink(),);
    assert_eq!(fs::read_link(&session_path).unwrap(), dangling_target);
    assert_eq!(fs::read(&receipt_path).unwrap(), receipt_before);
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&repo, &work_before);

    fs::remove_file(&session_path).expect("restore genuinely missing genesis Session");
    let sessions_dir = runtime.sessions_dir.clone();
    let replacement_repo = repo.clone();
    set_missing_session_cleanup_hook_for_test(Box::new(move |observed_session_id| {
        assert_eq!(observed_session_id, session_id);
        let mut replacement = gwt_agent::Session::new(
            &replacement_repo,
            "work/replacement",
            gwt_agent::AgentId::Codex,
        );
        replacement.id = observed_session_id.to_string();
        replacement
            .save(&sessions_dir)
            .expect("materialize same-id genesis Session after Missing observation");
    }));

    runtime.reconcile_durable_fresh_execution_launches();

    assert!(
        session_path.exists(),
        "replacement genesis Session must remain"
    );
    assert_eq!(fs::read(&receipt_path).unwrap(), receipt_before);
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&repo, &work_before);

    fs::remove_file(&session_path).expect("restore true Missing genesis state");
    runtime.reconcile_durable_fresh_execution_launches();
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read terminalized genesis ledger")
            .expect("terminalized genesis ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
    );
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
}

#[test]
fn startup_repairs_ledger_first_genesis_terminalization_before_cleanup() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo-genesis-ledger-first-restart");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let session_id = "genesis-ledger-first-restart";
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
    let binding = gwt_agent::SessionExecutionBinding {
        schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        identity,
        capability_generation: 1,
    };
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let mut session = gwt_agent::Session::new(&repo, "work/issue-2359", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(repo.clone());
    session.linked_issue_number = Some(owner.number);
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
    .expect("persist genesis receipt");
    let session_path = runtime.sessions_dir.join(format!("{session_id}.toml"));
    let mut replacement = session.clone();
    replacement.agent_id = gwt_agent::AgentId::Custom("review-bot".to_string());
    replacement
        .save(&runtime.sessions_dir)
        .expect("replace genesis Session Agent");
    let replacement_before = fs::read(&session_path).expect("read replacement Session");
    let authority_before =
        snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));
    let work_before = tracked_workspace_work_store_snapshot(&repo);

    runtime.reconcile_durable_fresh_execution_launches();

    assert_eq!(
        fs::read(&session_path).expect("read retained replacement Session"),
        replacement_before,
    );
    assert!(durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
    assert_optional_files_unchanged(&authority_before);
    assert_tracked_workspace_work_store_unchanged(&repo, &work_before);
    session
        .save(&runtime.sessions_dir)
        .expect("restore exact genesis Session");
    gwt::cli::execution_state::block_uncommitted_genesis_launch(
        &repo,
        owner,
        session_id,
        &binding.identity,
        "simulated launch failure",
    )
    .expect("commit terminal ledger event");
    let trusted_dir = gwt::cli::trusted_store::trusted_dir_for_worktree(&repo)
        .expect("trusted worktree directory");
    fs::remove_file(trusted_dir.join("execution-control.json"))
        .expect("remove committed projection");
    fs::remove_file(trusted_dir.join("execution-generation-pointer.json"))
        .expect("remove committed pointer");
    assert!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner).is_err(),
        "fixture must reproduce the ledger-first unreadable strict view",
    );

    runtime.reconcile_durable_fresh_execution_launches();

    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("read repaired authority")
            .expect("repaired ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
    );
    assert!(!runtime
        .sessions_dir
        .join(format!("{session_id}.toml"))
        .exists());
    assert!(!durable_launch_recovery_exists(
        &runtime.sessions_dir,
        session_id,
    ));
}

#[test]
fn continue_work_launch_failure_aborts_without_pausing_candidate_work() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let ContinueWorkLaunchFailureFixture {
        mut runtime,
        repo,
        owner,
        window_id,
        selected_work_id,
        candidate_session_id,
        candidate_session,
        candidate_runtime_path,
    } = continue_work_launch_failure_fixture(temp.path());

    let candidate_path = runtime
        .sessions_dir
        .join(format!("{candidate_session_id}.toml"));
    let mut replacement = candidate_session.clone();
    replacement.branch = "work/replacement-race".to_string();
    replacement
        .save(&runtime.sessions_dir)
        .expect("replace candidate after launch validation");
    let replacement_before = fs::read(&candidate_path).expect("read replacement before callback");
    let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
    let work_items_before = fs::read(&work_items_path).expect("read Work projection before race");
    let work_events_before = tracked_work_event_store_snapshot(&repo);
    let authority_before =
        snapshot_optional_files(&exact_continue_authority_artifacts(&repo, owner));

    let rejected = runtime.handle_launch_complete_and_drain(
        window_id.clone(),
        Err("candidate replacement race".into()),
    );

    assert!(rejected.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            outcome: gwt::ContinueWorkOutcomeKind::ConflictUnknown,
            retryable: true,
            ..
        }
    )));
    assert!(runtime.pending_continue_work.contains_key(&window_id));
    assert_eq!(
        fs::read(&candidate_path).expect("read retained replacement"),
        replacement_before,
    );
    assert_eq!(
        fs::read(&work_items_path).expect("read Work projection after race"),
        work_items_before,
    );
    assert_eq!(tracked_work_event_store_snapshot(&repo), work_events_before,);
    assert_optional_files_unchanged(&authority_before);

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        fs::remove_file(&candidate_path).expect("remove branch-replacement candidate");
        let missing_target = runtime.sessions_dir.join("missing-live-continue-candidate");
        symlink(&missing_target, &candidate_path).expect("create dangling live Continue candidate");

        let dangling = runtime.handle_launch_complete_and_drain(
            window_id.clone(),
            Err("dangling candidate replacement race".into()),
        );

        assert!(
            dangling.iter().any(|event| matches!(
                &event.event,
                BackendEvent::ContinueWorkOutcome {
                    outcome: gwt::ContinueWorkOutcomeKind::Failed,
                    error_code: Some(code),
                    retryable: true,
                    ..
                } if code == "continuation_reconciliation_required"
            )),
            "dangling candidate outcome: {dangling:?}"
        );
        assert!(runtime.pending_continue_work.contains_key(&window_id));
        assert!(fs::symlink_metadata(&candidate_path)
            .expect("dangling Continue Session must remain")
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_link(&candidate_path).unwrap(), missing_target);
        assert_eq!(
            fs::read(&work_items_path).expect("read Work projection after dangling race"),
            work_items_before,
        );
        assert_eq!(tracked_work_event_store_snapshot(&repo), work_events_before,);
        assert_optional_files_unchanged(&authority_before);
        fs::remove_file(&candidate_path).expect("remove dangling Continue candidate");
    }
    candidate_session
        .save(&runtime.sessions_dir)
        .expect("restore exact candidate after race");

    let events = runtime
        .handle_launch_complete_and_drain(window_id.clone(), Err("candidate spawn failed".into()));

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                operation_id,
                work_id,
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                error_code: Some(error_code),
                ..
            } if operation_id == "continue-op-1"
                && work_id == selected_work_id
                && error_code == "launch_failed"
        )),
        "pre-activation launch failure must settle the correlated operation",
    );
    assert!(!runtime.pending_continue_work.contains_key(&window_id));
    assert!(!runtime.active_agent_sessions.contains_key(&window_id));
    assert!(
        !runtime
            .sessions_dir
            .join(format!("{candidate_session_id}.toml"))
            .exists(),
        "the exact aborted candidate Session must be discarded",
    );
    assert!(
        !candidate_runtime_path.exists(),
        "the aborted candidate runtime sidecar must be discarded",
    );

    let projection = gwt_core::workspace_projection::load_workspace_work_items(&repo)
        .expect("load Work projection")
        .expect("Work projection");
    assert!(
        projection
            .work_items
            .iter()
            .all(|item| item.id != format!("work-session-{candidate_session_id}")),
        "candidate failure must not materialize a ghost Paused Work",
    );
    let selected = projection
        .work_items
        .iter()
        .find(|item| item.id == selected_work_id)
        .expect("selected Work");
    assert_eq!(
        selected.status_category,
        gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
        "the settled predecessor Work must remain unchanged",
    );
}

#[test]
fn continue_work_ready_timeout_is_correlated_and_aborts_only_current_pending_attempt() {
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
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        owner.kind,
        owner.number,
        "predecessor-session",
        "gwt-execute",
        false,
    )
    .expect("materialize predecessor");
    assert!(matches!(
        gwt::cli::execution_state::settle(
            &repo,
            "predecessor-session",
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        )
        .expect("settle predecessor"),
        gwt::cli::execution_state::SettleResult::Settled(_)
    ));
    gwt::cli::execution_state::ensure_generation_ledger(
        &repo,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
    )
    .expect("import predecessor");
    let predecessor_binding = gwt::cli::execution_state::current_execution_binding(&repo, owner)
        .expect("read predecessor binding")
        .expect("predecessor binding");
    let request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "current-operation".to_string(),
        principal_id: "gwt-host-continuation".to_string(),
        work_id: Some("work-current".to_string()),
        source: "continue-work:resume".to_string(),
        session_binding_id: "candidate-binding".to_string(),
        initial_session_id: "candidate-session".to_string(),
        entrypoint: "gwt-execute".to_string(),
        requested_at: chrono::Utc::now(),
    };
    gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
        .expect("prepare successor");
    let identity =
        gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
            .expect("derive successor binding");
    let mut runtime = sample_runtime(
        &temp.path().join(".gwt"),
        vec![sample_project_tab_with_window_at(
            "tab-1",
            "candidate",
            repo.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        )],
        Some("tab-1"),
    );
    let window_id = "tab-1::candidate".to_string();
    runtime.pending_continue_work.insert(
        window_id.clone(),
        PendingContinueWork {
            client_id: "client-1".to_string(),
            operation_id: "current-operation".to_string(),
            work_id: "work-current".to_string(),
            project_root: repo.clone(),
            worktree_path: repo.clone(),
            owner,
            work_branch: "work/issue-2359".to_string(),
            work_agent_id: gwt_agent::AgentId::Codex,
            work_agent_session_id: None,
            execution: PendingContinueWorkExecution::Successor(request),
            binding: gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: "candidate-session".to_string(),
                repo_hash: detect_repo_hash(&repo).expect("repo hash").to_string(),
                owner_kind: "issue".to_string(),
                owner_number: 2359,
                identity,
                capability_generation: 1,
            },
            readiness_nonce: "private-ready".to_string(),
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            resume_context: WorkspaceResumeContext {
                title: None,
                owner: Some("Issue #2359".to_string()),
                summary: None,
                next_action: None,
            },
            predecessor_session_id: "predecessor-session".to_string(),
            predecessor_binding,
        },
    );

    assert!(runtime
        .handle_continue_work_ready_timeout(
            &window_id,
            &ContinueWorkReadinessWatch::new("stale-operation".to_string()),
        )
        .is_empty());
    assert!(runtime.pending_continue_work.contains_key(&window_id));

    // No PTY runtime is registered for this window, so the very first deadline
    // has no liveness evidence to extend on and aborts exactly as before.
    let events = runtime.handle_continue_work_ready_timeout(
        &window_id,
        &ContinueWorkReadinessWatch::new("current-operation".to_string()),
    );
    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::ContinueWorkOutcome {
                operation_id,
                outcome: gwt::ContinueWorkOutcomeKind::Failed,
                error_code: Some(code),
                ..
            } if operation_id == "current-operation" && code == "launch_failed"
        )),
        "unexpected timeout events: {events:#?}"
    );
    assert!(!runtime.pending_continue_work.contains_key(&window_id));
}

/// Issue #3475: the readiness deadline is progress-aware. A pane that is alive
/// and still emitting PTY output buys another extension instead of aborting a
/// prepared successor mid-bootstrap, and the silent streak resets.
#[test]
fn continue_work_readiness_deadline_extends_while_the_pane_is_alive_and_producing_output() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 0,
        silent_extensions: 1,
        observed_output_bytes: 128,
        handed_off: false,
    };

    assert_eq!(
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, 4096),
        ReadinessDeadlineDecision::Extend(ContinueWorkReadinessWatch {
            operation_id: "op".to_string(),
            extensions: 1,
            silent_extensions: 0,
            observed_output_bytes: 4096,
            handed_off: false,
        }),
    );
}

/// Issue #3482 AC-4 (slow progress): a pane that is nearly out of extension
/// budget but still emitting output keeps extending. Slowness alone is never
/// evidence that the launch is dead.
#[test]
fn continue_work_readiness_deadline_extends_a_slow_but_progressing_pane() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 3,
        silent_extensions: 2,
        observed_output_bytes: 12_288,
        handed_off: false,
    };

    assert_eq!(
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, 12_289),
        ReadinessDeadlineDecision::Extend(ContinueWorkReadinessWatch {
            operation_id: "op".to_string(),
            extensions: 4,
            silent_extensions: 0,
            observed_output_bytes: 12_289,
            handed_off: false,
        }),
    );
}

/// Issue #3475: no liveness evidence means the deadline still aborts on the
/// spot — the extension budget must never keep a dead launch pending.
/// Issue #3482 AC-3: a dead pane is exactly the case that may still be torn
/// down, because there is no live process left to destroy.
#[test]
fn continue_work_readiness_deadline_aborts_at_once_when_the_pane_process_is_gone() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 1,
        silent_extensions: 0,
        observed_output_bytes: 10,
        handed_off: false,
    };

    let ReadinessDeadlineDecision::Abort { detail, pane } =
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Dead, 4096)
    else {
        panic!("a dead pane must abort the readiness deadline");
    };
    assert_eq!(pane, LaunchPaneDisposition::Teardown);
    assert!(detail.contains("after 150s"), "{detail}");
    assert!(detail.contains("no longer running"), "{detail}");
}

/// Issue #3482 AC-3 (identity mismatch): when another runtime owns the launch
/// window, the durable candidate is still rolled back — but the pane that is
/// there now belongs to somebody else and must survive untouched.
#[test]
fn continue_work_readiness_deadline_retains_a_foreign_pane_while_rolling_the_candidate_back() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 1,
        silent_extensions: 0,
        observed_output_bytes: 10,
        handed_off: false,
    };

    let ReadinessDeadlineDecision::Abort { detail, pane } =
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Foreign, 4096)
    else {
        panic!("a foreign pane must still roll the candidate back");
    };
    assert_eq!(
        pane,
        LaunchPaneDisposition::Retain,
        "cleanup must never terminate a pane this launch does not own",
    );
    assert!(detail.contains("after 150s"), "{detail}");
    assert!(detail.contains("no longer owns"), "{detail}");
}

/// Issue #3482 AC-2 (silent live): the silent streak still bounds *waiting*,
/// but reaching the bound hands the launch to the user instead of killing a
/// process that is demonstrably alive.
#[test]
fn continue_work_readiness_deadline_hands_off_a_silent_live_pane_without_killing_it() {
    let mut watch = ContinueWorkReadinessWatch::new("op".to_string());
    let mut granted = 0_u32;
    let (handoff, detail) = loop {
        match continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, 0) {
            ReadinessDeadlineDecision::Extend(next) => {
                granted += 1;
                assert!(granted <= 8, "a silent pane must not extend forever");
                watch = next;
            }
            ReadinessDeadlineDecision::HandOff {
                watch: next,
                detail,
            } => {
                break (
                    next,
                    detail.expect("the handoff transition must be diagnosable"),
                )
            }
            ReadinessDeadlineDecision::Abort { .. } => {
                panic!("a live pane must never be aborted by the readiness deadline")
            }
        }
    };

    assert_eq!(granted, 2);
    assert!(handoff.handed_off);
    assert!(detail.contains("after 210s"), "{detail}");
    assert!(detail.contains("never produced any output"), "{detail}");
}

/// Issue #3482 AC-2: progress buys time but not an unbounded wait. Reaching the
/// absolute cap stops the waiting, not the agent — the launch is handed off
/// with the pane and its in-flight resume intact.
#[test]
fn continue_work_readiness_deadline_hands_off_a_progressing_pane_at_the_absolute_cap() {
    let mut watch = ContinueWorkReadinessWatch::new("op".to_string());
    let mut output_bytes = 0_u64;
    let mut granted = 0_u32;
    let (handoff, detail) = loop {
        output_bytes += 4096;
        match continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, output_bytes) {
            ReadinessDeadlineDecision::Extend(next) => {
                granted += 1;
                assert!(granted <= 8, "a progressing pane must not extend forever");
                watch = next;
            }
            ReadinessDeadlineDecision::HandOff {
                watch: next,
                detail,
            } => {
                break (
                    next,
                    detail.expect("the handoff transition must be diagnosable"),
                )
            }
            ReadinessDeadlineDecision::Abort { .. } => {
                panic!("a live pane must never be aborted by the readiness deadline")
            }
        }
    };

    assert_eq!(granted, 4);
    assert!(handoff.handed_off);
    assert!(detail.contains("after 330s"), "{detail}");
    assert!(detail.contains("never reported"), "{detail}");
}

/// Issue #3482: after the handoff the deadline degrades into a supervision
/// loop. It keeps re-arming so a later death is still reaped, but it must not
/// repeat the diagnostic every minute for a pane the user already owns.
#[test]
fn continue_work_readiness_supervision_stays_quiet_while_the_handed_off_pane_lives() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 4,
        silent_extensions: 0,
        observed_output_bytes: 4096,
        handed_off: true,
    };

    assert_eq!(
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Live, 8192),
        ReadinessDeadlineDecision::HandOff {
            watch: watch.clone(),
            detail: None,
        },
    );
}

/// Issue #3482 AC-3: handing off does not abandon the durable candidate. Once
/// the supervised pane really dies, the same bounded cleanup runs.
#[test]
fn continue_work_readiness_supervision_cleans_up_once_the_handed_off_pane_dies() {
    let watch = ContinueWorkReadinessWatch {
        operation_id: "op".to_string(),
        extensions: 4,
        silent_extensions: 0,
        observed_output_bytes: 4096,
        handed_off: true,
    };

    let ReadinessDeadlineDecision::Abort { detail, pane } =
        continue_work_readiness_decision(&watch, ReadinessPaneEvidence::Dead, 8192)
    else {
        panic!("a supervised pane that died must roll the candidate back");
    };
    assert_eq!(pane, LaunchPaneDisposition::Teardown);
    assert!(detail.contains("no longer running"), "{detail}");
}

/// Issue #3475: end-to-end on the runtime — a Continue work candidate whose
/// agent pane is still alive survives the base deadline instead of losing its
/// prepared successor, which is the exact regression a 12.5 MB Codex resume hit.
#[test]
fn continue_work_ready_timeout_extends_while_the_agent_pty_is_still_live() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "readiness-live-pane");
    insert_test_pane_runtime(&mut fixture.runtime, &fixture.window_id);
    fixture
        .runtime
        .window_pty_statuses
        .insert(fixture.window_id.clone(), WindowProcessStatus::Running);

    let events = fixture.runtime.handle_continue_work_ready_timeout(
        &fixture.window_id,
        &ContinueWorkReadinessWatch::new(fixture.operation_id.clone()),
    );

    assert!(
        events.is_empty(),
        "a live agent pane must extend the readiness deadline: {events:#?}"
    );
    assert!(
        fixture
            .runtime
            .pending_fresh_execution_launches
            .contains_key(&fixture.window_id),
        "the prepared successor must survive an extended deadline"
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read binding after extension"),
        Some(fixture.predecessor_binding.clone()),
        "extending must not touch the predecessor generation",
    );
}

/// Issue #3482 AC-1: liveness is not identity. The deadline classifies the pane
/// it is about to act on by window presence, the Session bound to that window,
/// and the PTY process — in that order.
#[test]
fn readiness_pane_evidence_separates_live_dead_and_foreign_panes() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "readiness-evidence");
    let window_id = fixture.window_id.clone();
    let session_id = fixture.candidate_session_id.clone();

    assert_eq!(
        fixture
            .runtime
            .readiness_pane_evidence(&window_id, &session_id),
        ReadinessPaneEvidence::Dead,
        "the exact Session is bound but no PTY runtime was ever installed",
    );

    insert_test_pane_runtime(&mut fixture.runtime, &window_id);
    fixture
        .runtime
        .window_pty_statuses
        .insert(window_id.clone(), WindowProcessStatus::Running);
    assert_eq!(
        fixture
            .runtime
            .readiness_pane_evidence(&window_id, &session_id),
        ReadinessPaneEvidence::Live,
    );

    assert_eq!(
        fixture
            .runtime
            .readiness_pane_evidence(&window_id, "another-session"),
        ReadinessPaneEvidence::Foreign,
        "a live pane bound to a different Session is not this launch's pane",
    );
    assert_eq!(
        fixture
            .runtime
            .readiness_pane_evidence("tab-1::never-registered", &session_id),
        ReadinessPaneEvidence::Foreign,
        "a window gwt does not track cannot be torn down by this launch",
    );

    fixture
        .runtime
        .stop_window_runtime_without_session_projection(&window_id);
    assert_eq!(
        fixture
            .runtime
            .readiness_pane_evidence(&window_id, &session_id),
        ReadinessPaneEvidence::Dead,
    );
}

/// Issue #5194 AC-2: an unready handoff is not silent. It lands in the error
/// ledger with the hook configuration the agent should have discovered, so a
/// missing `.codex/hooks.json` is visible from `errors.list`.
#[test]
fn continue_work_ready_timeout_handoff_records_the_missing_hook_config() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "readiness-handoff-ledger");
    let project_root = temp.path().join("separate-project-state-root");
    fs::create_dir_all(&project_root).unwrap();
    fixture
        .runtime
        .pending_fresh_execution_launches
        .get_mut(&fixture.window_id)
        .unwrap()
        .project_root = project_root.clone();
    let works_lock = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root)
        .with_extension("lock");
    fs::create_dir_all(works_lock.parent().unwrap()).unwrap();
    let _holder =
        gwt_core::operation_deadline::NamedFileLock::acquire_quiet(&works_lock, "startup intake")
            .unwrap();
    insert_test_pane_runtime(&mut fixture.runtime, &fixture.window_id);
    fixture
        .runtime
        .window_pty_statuses
        .insert(fixture.window_id.clone(), WindowProcessStatus::Running);

    fixture.runtime.handle_continue_work_ready_timeout(
        &fixture.window_id,
        &readiness_watch_at_last_extension(&fixture.operation_id, 0),
    );

    let rows = gwt_core::error_ledger::list_since(None).expect("read error ledger");
    let row = rows
        .iter()
        .find(|row| row.target.window_id.as_deref() == Some(fixture.window_id.as_str()))
        .unwrap_or_else(|| panic!("the handoff must be recorded: {rows:#?}"));
    assert_eq!(row.kind, gwt_core::error_ledger::ErrorKind::LaunchFailure);
    assert_eq!(row.target.issue, Some(fixture.owner.number));
    assert_eq!(
        fixture.runtime.window_details.get(&fixture.window_id),
        Some(&row.message),
        "the pane and errors.list must share the same readiness diagnosis",
    );
    assert_eq!(
        fixture.runtime.pane_hold_reason(&fixture.window_id),
        Some(row.message.clone()),
        "the Monitor canvas must carry the pending readiness diagnosis",
    );
    assert!(row.message.contains("SessionStart"), "{}", row.message);
    assert!(
        row.message.contains("works.lock contention observed: yes;"),
        "{}",
        row.message
    );
    assert!(
        row.message.contains(".codex") && row.message.contains("hooks.json missing"),
        "the ledger row must name the undiscovered hook config: {}",
        row.message
    );
}

/// Issue #3482 AC-2: the readiness deadline bounds *waiting*, not the life of
/// the agent. When the budget runs out on a pane that is still the exact live
/// launch pane, the launch is handed to the user with its process, its window,
/// and its prepared candidate intact.
#[test]
fn continue_work_ready_timeout_hands_off_a_live_pane_instead_of_killing_it() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "readiness-handoff-live-pane");
    insert_test_pane_runtime(&mut fixture.runtime, &fixture.window_id);
    fixture
        .runtime
        .window_pty_statuses
        .insert(fixture.window_id.clone(), WindowProcessStatus::Running);

    let events = fixture.runtime.handle_continue_work_ready_timeout(
        &fixture.window_id,
        &readiness_watch_at_last_extension(&fixture.operation_id, 0),
    );

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::TerminalStatus { id, detail: Some(detail), .. }
                if id == &fixture.window_id && detail.contains("210s")
        )),
        "the handoff must be diagnosable on the pane: {events:#?}"
    );
    assert!(
        fixture.runtime.runtimes.contains_key(&fixture.window_id),
        "a live agent pane must survive the readiness deadline",
    );
    assert!(
        fixture
            .runtime
            .window_lookup
            .contains_key(&fixture.window_id),
        "the launch window must stay open for the handoff",
    );
    assert!(
        fixture
            .runtime
            .pending_fresh_execution_launches
            .contains_key(&fixture.window_id),
        "a handed-off launch stays pending so a late SessionStart can still activate it",
    );
    assert!(
        durable_launch_recovery_exists(
            &fixture.runtime.sessions_dir,
            &fixture.candidate_session_id,
        ),
        "the handoff must not discard the candidate's durable recovery receipt",
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
        "the handoff must leave the prepared candidate alone",
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read binding after handoff"),
        Some(fixture.predecessor_binding.clone()),
        "handing off must not touch the predecessor generation",
    );

    fixture
        .runtime
        .stop_window_runtime_without_session_projection(&fixture.window_id);
}

/// Issue #3482 AC-4 (late ready): the whole point of keeping the pane is that a
/// slow resume can still finish. A SessionStart that lands after the handoff
/// activates the candidate exactly as an on-time one would.
#[test]
fn continue_work_ready_timeout_late_session_start_still_activates_after_a_handoff() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "readiness-late-session-start");
    insert_test_pane_runtime(&mut fixture.runtime, &fixture.window_id);
    fixture
        .runtime
        .window_pty_statuses
        .insert(fixture.window_id.clone(), WindowProcessStatus::Running);
    let readiness_nonce = fixture
        .runtime
        .pending_fresh_execution_launches
        .get(&fixture.window_id)
        .expect("pending fresh launch")
        .readiness_nonce
        .clone();

    let _handoff = fixture.runtime.handle_continue_work_ready_timeout(
        &fixture.window_id,
        &readiness_watch_at_last_extension(&fixture.operation_id, 0),
    );
    assert!(
        fixture
            .runtime
            .window_details
            .contains_key(&fixture.window_id),
        "the handoff must leave a diagnostic a reconnecting client can replay",
    );
    fixture
        .runtime
        .finalize_fresh_execution_launch_session_start(&fixture.window_id, Some(&readiness_nonce));
    let events = commit_pending_fresh_execution(&mut fixture.runtime);

    assert!(
        !events.is_empty(),
        "a late authenticated SessionStart must still complete the launch"
    );
    assert!(
        !fixture
            .runtime
            .window_details
            .contains_key(&fixture.window_id),
        "activating must retire the readiness handoff diagnostic",
    );
    assert_eq!(fixture.runtime.pane_hold_reason(&fixture.window_id), None);
    assert!(!fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&fixture.window_id));
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read binding after late SessionStart"),
        Some(fixture.binding.identity.clone()),
        "the handed-off candidate must become the current generation",
    );

    fixture
        .runtime
        .stop_window_runtime_without_session_projection(&fixture.window_id);
}

/// Issue #3482 AC-3 (identity mismatch): a readiness deadline that fires for a
/// window another Session now owns still rolls its own candidate back, but it
/// must not stop or close the pane that is there now.
#[test]
fn continue_work_ready_timeout_does_not_terminate_a_window_owned_by_another_session() {
    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let mut fixture = pending_fresh_execution_fixture(temp.path(), "readiness-foreign-window");
    insert_test_pane_runtime(&mut fixture.runtime, &fixture.window_id);
    fixture
        .runtime
        .window_pty_statuses
        .insert(fixture.window_id.clone(), WindowProcessStatus::Running);
    fixture
        .runtime
        .active_agent_sessions
        .get_mut(&fixture.window_id)
        .expect("active candidate Session")
        .session_id = "some-other-session".to_string();

    let events = fixture.runtime.handle_continue_work_ready_timeout(
        &fixture.window_id,
        &ContinueWorkReadinessWatch::new(fixture.operation_id.clone()),
    );

    assert!(
        !events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::TerminalStatus { id, status: WindowProcessStatus::Error, .. }
                if id == &fixture.window_id
        )),
        "cleanup must not paint a window this launch no longer owns as failed: {events:#?}"
    );
    assert!(
        fixture.runtime.runtimes.contains_key(&fixture.window_id),
        "cleanup must not terminate a pane this launch no longer owns",
    );
    assert!(
        fixture
            .runtime
            .window_lookup
            .contains_key(&fixture.window_id),
        "cleanup must not remove a window this launch no longer owns",
    );
    assert!(
        !fixture
            .runtime
            .pending_fresh_execution_launches
            .contains_key(&fixture.window_id),
        "the mismatched candidate must still be rolled back",
    );
    assert!(
        !durable_launch_recovery_exists(
            &fixture.runtime.sessions_dir,
            &fixture.candidate_session_id,
        ),
        "the mismatched candidate must release its durable recovery receipt",
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(&fixture.repo, fixture.owner)
            .expect("read binding after mismatch cleanup"),
        Some(fixture.predecessor_binding.clone()),
    );

    fixture
        .runtime
        .stop_window_runtime_without_session_projection(&fixture.window_id);
}

#[test]
fn continue_work_ready_timeout_starts_only_after_pty_handoff() {
    let source = include_str!("../launch.rs");
    let launch_dispatch_start = source
        .find("fn spawn_agent_window_with_placement(")
        .expect("launch dispatch function");
    let async_preparation_start = source[launch_dispatch_start..]
        .find("pub(crate) fn spawn_agent_window_async(")
        .map(|offset| launch_dispatch_start + offset)
        .expect("async launch preparation function");
    let launch_dispatch = &source[launch_dispatch_start..async_preparation_start];
    assert!(
        !launch_dispatch.contains("arm_continue_work_readiness_deadline"),
        "Continue work readiness timeout must not start before async launch preparation"
    );

    let apply_start = source
        .find("pub(crate) fn handle_agent_launch_prepared(")
        .expect("prepared launch apply");
    let apply_end = source[apply_start..]
        .find("pub(super) fn launch_error_events_with_continue_work(")
        .map(|offset| apply_start + offset)
        .expect("launch failure boundary");
    let apply = &source[apply_start..apply_end];
    let pty_install = apply
        .find("self.install_process_window(")
        .expect("prepared PTY install");
    let last_ready_timeout = apply
        .rfind("arm_continue_work_readiness_deadline")
        .expect("readiness timeout scheduling");
    assert!(
        last_ready_timeout > pty_install,
        "Continue work readiness timeout must be armed after successful PTY installation"
    );
    let enqueue_start = source
        .find("pub(crate) fn handle_launch_complete(")
        .expect("enqueue handler");
    assert!(
        !source[enqueue_start..apply_start].contains("arm_continue_work_readiness_deadline"),
        "worker preparation cannot start the readiness deadline"
    );
}

#[test]
fn continue_work_rejects_parallel_operation_for_same_work_before_preparing_authority() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab_with_window_at(
            "tab-1",
            "candidate",
            repo.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        )],
        Some("tab-1"),
    );
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 2359,
    };
    let identity = gwt_agent::ExecutionBindingIdentity {
        generation_id: "candidate-generation".to_string(),
        binding_id: "candidate-binding".to_string(),
        ledger_head_hash: "candidate-head".to_string(),
    };
    runtime.pending_continue_work.insert(
        "tab-1::candidate".to_string(),
        PendingContinueWork {
            client_id: "client-1".to_string(),
            operation_id: "first-operation".to_string(),
            work_id: "work-shared".to_string(),
            project_root: repo.clone(),
            worktree_path: repo,
            owner,
            work_branch: "work/issue-2359".to_string(),
            work_agent_id: gwt_agent::AgentId::Codex,
            work_agent_session_id: None,
            execution: PendingContinueWorkExecution::Successor(
                gwt::cli::execution_state::SuccessorRequest {
                    operation_id: "first-operation".to_string(),
                    principal_id: "gwt-host-continuation".to_string(),
                    work_id: Some("work-shared".to_string()),
                    source: "continue-work:resume".to_string(),
                    session_binding_id: "candidate-binding".to_string(),
                    initial_session_id: "candidate-session".to_string(),
                    entrypoint: "gwt-execute".to_string(),
                    requested_at: chrono::Utc::now(),
                },
            ),
            binding: gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: "candidate-session".to_string(),
                repo_hash: "repo-hash".to_string(),
                owner_kind: "issue".to_string(),
                owner_number: 2359,
                identity: identity.clone(),
                capability_generation: 1,
            },
            readiness_nonce: "private-ready".to_string(),
            outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
            resume_context: WorkspaceResumeContext {
                title: None,
                owner: Some("Issue #2359".to_string()),
                summary: None,
                next_action: None,
            },
            predecessor_session_id: "predecessor-session".to_string(),
            predecessor_binding: identity,
        },
    );

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-2",
        "second-operation".to_string(),
        "work-shared".to_string(),
        canvas_bounds(),
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            operation_id,
            work_id,
            outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code),
            retryable,
            ..
        } if operation_id == "second-operation"
            && work_id == "work-shared"
            && code == "continue_work_in_progress"
            && *retryable
    )));
    assert_eq!(
        runtime.pending_continue_work.len(),
        1,
        "the in-flight operation must remain the sole Prepared owner"
    );
}

#[test]
fn continue_work_empty_operation_identity_failure_is_not_cached() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab-1",
            "Repo",
            temp.path().join("repo"),
            ProjectKind::NonRepo,
            &[],
        )],
        Some("tab-1"),
    );

    let events = runtime.continue_work_events(
        &runtime.test_context(),
        "client-invalid",
        String::new(),
        "work-a".to_string(),
        canvas_bounds(),
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            operation_id,
            work_id,
            outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code),
            retryable,
            ..
        } if operation_id.is_empty()
            && work_id == "work-a"
            && code == "invalid_request"
            && !retryable
    )));
    assert!(
        !runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .continue_work_outcomes
            .contains_key(""),
        "an invalid operation identity must never become a replay-cache key"
    );
}

#[test]
fn continue_work_invalid_identity_cannot_overwrite_cached_outcome_or_drain_waiters() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path().join(".gwt"));
    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab-1",
            "Repo",
            temp.path().join("repo"),
            ProjectKind::NonRepo,
            &[],
        )],
        Some("tab-1"),
    );
    let operation_id = "immutable-operation";
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .continue_work_outcomes
        .insert(
            operation_id.to_string(),
            CachedContinueWorkOutcome {
                work_id: "work-a".to_string(),
                outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                message: None,
                error_code: None,
                retryable: false,
            },
        );
    runtime
        .project_state_mut(&runtime.test_context())
        .expect("test project state")
        .continue_work_waiters
        .insert(
            operation_id.to_string(),
            HashSet::from(["client-waiter".to_string()]),
        );

    let invalid = runtime.continue_work_events(
        &runtime.test_context(),
        "client-invalid",
        operation_id.to_string(),
        String::new(),
        canvas_bounds(),
    );

    assert!(invalid.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            operation_id: emitted_operation_id,
            work_id,
            outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code),
            retryable,
            ..
        } if emitted_operation_id == operation_id
            && work_id.is_empty()
            && code == "invalid_request"
            && !retryable
    )));
    let cached_preserved = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .continue_work_outcomes
        .get(operation_id)
        .is_some_and(|cached| {
            cached.work_id == "work-a"
                && cached.outcome == gwt::ContinueWorkOutcomeKind::ContinuedConversation
                && cached.error_code.is_none()
        });
    let waiter_preserved = runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .continue_work_waiters
        .get(operation_id)
        .is_some_and(|waiters| waiters == &HashSet::from(["client-waiter".to_string()]));
    let invalid_replied_only_to_caller = invalid.len() == 1
        && matches!(
            &invalid[0].target,
            DispatchTarget::Client(client_id) if client_id == "client-invalid"
        );
    assert!(
        cached_preserved && waiter_preserved && invalid_replied_only_to_caller,
        "invalid identity mutated replay state: cached={:?}, waiters={:?}, events={invalid:?}",
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .continue_work_outcomes
            .get(operation_id),
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .continue_work_waiters
            .get(operation_id),
    );

    let replay = runtime.continue_work_events(
        &runtime.test_context(),
        "client-replay",
        operation_id.to_string(),
        "work-a".to_string(),
        canvas_bounds(),
    );
    let replay_clients = replay
        .iter()
        .filter_map(|event| match (&event.target, &event.event) {
            (
                DispatchTarget::Client(client_id),
                BackendEvent::ContinueWorkOutcome {
                    operation_id: emitted_operation_id,
                    work_id,
                    outcome: gwt::ContinueWorkOutcomeKind::ContinuedConversation,
                    error_code: None,
                    retryable: false,
                    ..
                },
            ) if emitted_operation_id == operation_id && work_id == "work-a" => {
                Some(client_id.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        replay_clients.len(),
        2,
        "a valid replay must emit exactly one cached outcome per recipient: {replay:?}"
    );
    assert_eq!(
        replay_clients
            .iter()
            .filter(|client_id| **client_id == "client-replay")
            .count(),
        1,
        "the replay caller must receive the cached outcome exactly once: {replay:?}"
    );
    assert_eq!(
        replay_clients
            .iter()
            .filter(|client_id| **client_id == "client-waiter")
            .count(),
        1,
        "the retained waiter must receive the cached outcome exactly once: {replay:?}"
    );
    assert!(!runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .continue_work_waiters
        .contains_key(operation_id));

    let conflict = runtime.continue_work_events(
        &runtime.test_context(),
        "client-conflict",
        operation_id.to_string(),
        "work-b".to_string(),
        canvas_bounds(),
    );
    assert!(conflict.iter().any(|event| matches!(
        &event.event,
        BackendEvent::ContinueWorkOutcome {
            operation_id: emitted_operation_id,
            work_id,
            outcome: gwt::ContinueWorkOutcomeKind::Failed,
            error_code: Some(code),
            retryable,
            ..
        } if emitted_operation_id == operation_id
            && work_id == "work-b"
            && code == "operation_conflict"
            && !retryable
    )));
    assert_eq!(
        conflict
            .iter()
            .filter(|event| matches!(
                &event.event,
                BackendEvent::ContinueWorkOutcome {
                    operation_id: emitted_operation_id,
                    work_id,
                    error_code: Some(code),
                    ..
                } if emitted_operation_id == operation_id
                    && work_id == "work-b"
                    && code == "operation_conflict"
            ))
            .count(),
        1,
        "a conflicting replay must emit exactly one conflict outcome: {conflict:?}"
    );
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .continue_work_outcomes
        .get(operation_id)
        .is_some_and(|cached| {
            cached.work_id == "work-a"
                && cached.outcome == gwt::ContinueWorkOutcomeKind::ContinuedConversation
                && cached.error_code.is_none()
        }));
}
