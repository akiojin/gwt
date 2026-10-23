use super::*;

/// Issue #3852 AC-4 / AC-6: the operator's recovery commands share the same
/// durable owner with the real Host materialization, not separate state mocks.
#[test]
#[cfg(windows)]
fn issue_3852_stop_close_requeue_launch_now_materializes_an_active_successor() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Assert delivery admission and replay outcomes, rather than native Git latency.
    let _projection_timeout = ScopedEnvVar::set(
        "GWT_TEST_ISSUE_MONITOR_FALLBACK_PROJECTION_TIMEOUT_MS",
        "10000",
    );
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "operator-stop-close-requeue",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
        LinkedIssueKind::Issue,
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "failed predecessor".into(),
            missing_verification: None,
        },
    );
    let raw_id = fixture.runtime.tabs[0]
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds())
        .id;
    let window_id = combined_window_id("tab-1", &raw_id);
    fixture.runtime.register_window("tab-1", &raw_id);
    assert!(fixture.runtime.tabs[0]
        .workspace
        .set_session_id(&raw_id, Some(fixture.source_session_id.clone()),));
    let old_session = ActiveAgentSession {
        window_id: window_id.clone(),
        session_id: fixture.source_session_id.clone(),
        branch_name: "work/issue-3165".into(),
        worktree_path: fixture.worktree.clone(),
        agent_project_root: fixture.worktree.display().to_string(),
        ..sample_active_agent_session("tab-1", &raw_id)
    };
    fixture
        .runtime
        .active_agent_sessions
        .insert(window_id.clone(), old_session);
    insert_test_pane_runtime(&mut fixture.runtime, &window_id);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    prefs.enabled = true;
    prefs.launched_issues = vec![gwt::IssueMonitorLaunchedIssue {
        issue_number: 3165,
        window_id: window_id.clone(),
    }];
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
    let mut cli = gwt::cli::TestEnv::new(fixture.project_root.clone());
    assert_eq!(
        gwt::cli::run(
            &mut cli,
            gwt::cli::CliCommand::Issue(gwt::cli::IssueCommand::MonitorStop {
                project_root: Some(fixture.project_root.clone()),
                number: 3165,
                reason: "recover failed agent".into(),
                remove_from_queue: true,
                claim_id: None,
                delivery_id: None,
                window_id: Some(window_id.clone()),
            },)
        )
        .unwrap(),
        0,
        "{}",
        String::from_utf8_lossy(&cli.stdout)
    );
    let (spawner, close_tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    let closed = fixture.runtime.handle_agent_frontend_event(
        "operator".into(),
        AgentSessionPrincipal::for_test(&fixture.project_root, "operator-session").unwrap(),
        AgentFrontendRequest::CloseWindow {
            id: window_id.clone(),
            request_id: None,
            responder: None,
        },
    );
    assert!(!closed
        .iter()
        .any(|event| matches!(event.event, BackendEvent::PaneCloseResult { .. })));
    drain_queued_blocking_tasks(&close_tasks);
    assert!(fixture.recorded_events.lock().unwrap().iter().any(|event| {
        match recorded_project_payload(event) {
            UserEvent::Dispatch(events) => events.iter().any(|event| {
                event.target == DispatchTarget::Client("operator".into())
                    && matches!(&event.event,
                        BackendEvent::PaneCloseResult { window_id: closed, ok: true, reason: None, .. }
                            if closed == &window_id)
            }),
            _ => false,
        }
    }));
    assert!(!fixture.runtime.runtimes.contains_key(&window_id));

    // Reuse the production authenticated bridge. Its scan ACK and the Host
    // materializer are driven separately while sharing the real persisted prefs.
    let network = tokio::runtime::Runtime::new().unwrap();
    let (proxy, requests) = AppEventProxy::stub();
    let (sent, notifications) = mpsc::channel();
    let clients = crate::embedded_server::ClientHub::default();
    let mut server = crate::embedded_server::EmbeddedServer::start(
        &network,
        AppEventProxy::Notifying {
            inner: Box::new(proxy),
            sent,
        },
        clients.clone(),
        Arc::new(RwLock::new(HashMap::new())),
        AttachmentUploadStore::in_system_temp(),
    )
    .unwrap();
    let issuer = server.agent_capability_issuer();
    let capability = issuer
        .issue(&fixture.project_root, "operator-session")
        .unwrap();
    let _pane_url = ScopedEnvVar::set(
        gwt_agent::GWT_PANE_WS_URL_ENV,
        issuer.agent_pane_websocket_url(),
    );
    let _token = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, &capability.token);
    let runtime_path = temp.path().join(".gwt/sessions/runtime/123/session.json");
    fs::create_dir_all(runtime_path.parent().unwrap()).unwrap();
    fs::write(&runtime_path, "{}").unwrap();
    let _runtime = ScopedEnvVar::set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
    let project = fixture.project_root.clone();
    let recovery = thread::spawn(move || {
        let mut env = gwt::cli::TestEnv::new(project.clone());
        let requeued = gwt::cli::run(
            &mut env,
            gwt::cli::CliCommand::Issue(gwt::cli::IssueCommand::MonitorRequeue {
                project_root: Some(project.clone()),
                number: 3165,
                reason: "restart stopped owner".into(),
                force: false,
            }),
        )
        .unwrap();
        let requeue: serde_json::Value = serde_json::from_slice(&env.stdout).unwrap();
        env.stdout.clear();
        let launched = gwt::cli::run(
            &mut env,
            gwt::cli::CliCommand::Issue(gwt::cli::IssueCommand::MonitorLaunchNow {
                project_root: Some(project),
                expected_priority_order: None,
                number: 3165,
            }),
        )
        .unwrap();
        (
            requeued,
            requeue,
            launched,
            serde_json::from_slice::<serde_json::Value>(&env.stdout).unwrap(),
        )
    });
    for _ in 0..2 {
        gwt_core::test_support::recv_event(&notifications, "operator immediate scan request");
        let request = requests.lock().unwrap().remove(0);
        let UserEvent::AgentFrontend {
            client_id,
            grant,
            request:
                AgentFrontendRequest::IssueMonitorScanNow {
                    expected_project_scope,
                },
        } = request
        else {
            panic!("expected authenticated immediate scan");
        };
        assert!(grant
            .principal()
            .authorizes_project_root(&fixture.project_root));
        assert_eq!(
            expected_project_scope,
            gwt_core::paths::project_scope_hash(&fixture.project_root).as_str()
        );
        clients.dispatch(vec![OutboundEvent::reply(
            client_id,
            BackendEvent::IssueMonitorScanRequestResult {
                accepted: true,
                reason: None,
            },
        )]);
    }
    let (requeued, requeue, launched, launch) = recovery.join().unwrap();
    server.shutdown();
    assert_eq!(requeued, 0, "{requeue}");
    assert_eq!(requeue["status"], "requeued");
    assert_eq!(launched, 0, "{launch}");
    assert_eq!(launch["scan_delivery"], "immediate");
    assert_eq!(launch["admission_confirmed"], false);
    assert!(
        gwt::cli::execution_state::diagnose_owner(&fixture.worktree, fixture.execution_owner)
            .blocking_prepared_transactions
            .is_empty()
    );

    // The fixture provider answers version probes normally and stays alive
    // for the real PTY launch until the test explicitly closes it.
    fs::write(temp.path().join("operator-stop-close-requeue/fixture-runner-bin/codex.cmd"),
        "@echo off\r\nif \"%~1\"==\"--version\" (echo 1.2.3 & exit /b 0)\r\nset /p GWT_TEST_OPERATOR_INPUT=\r\n").unwrap();
    let mut monitor = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
    );
    monitor.set_gui_connected(true);
    let issue = gwt::IssueMonitorIssue {
        author_login: None,
        number: 3165,
        title: "fix: operator recovery".into(),
        labels: vec!["bug".into()],
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: None,
    };
    let now = Utc::now().to_rfc3339();
    gwt::scan_issue_monitor_candidates_with_provenance(
        &mut monitor,
        &[issue],
        gwt::IssueMonitorCandidateSource::Live,
        &fixture.project_root,
        &now,
    );
    let client = FakeIssueClient::new();
    client.seed(sample_issue_snapshot(
        3165,
        "fix: operator recovery",
        &["bug"],
        "recovered owner",
        &now,
    ));
    let launch_requests = monitor.claim_next_launch_requests(&client, "operator-recovery", &now);
    assert_eq!(
        launch_requests
            .iter()
            .map(|request| request.issue_number)
            .collect::<Vec<_>>(),
        vec![3165]
    );
    let delivery_id = monitor
        .pending_launch_delivery_id(3165)
        .expect("claimed target delivery");
    let strategy = monitor
        .prefs()
        .pending_launch_deliveries
        .into_iter()
        .find(|delivery| delivery.delivery_id == delivery_id)
        .unwrap()
        .launch_session_strategy;
    assert_eq!(
        strategy,
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired
    );
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).unwrap();
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Issue,
            Some(delivery_id),
            strategy,
        );
    loop {
        drain_queued_blocking_tasks(&tasks);
        if !fixture
            .recorded_events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, UserEvent::IssueMonitorLaunchPrepared(_)))
        {
            break;
        }
        fixture
            .runtime
            .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
                &fixture.recorded_events,
            ));
    }
    let (fresh_window, result) =
        take_monitor_launch_complete_event("operator recovery launch", &fixture.recorded_events);
    let completion = result.expect("requeued owner launches without a Prepared fence");
    let session_id = completion.1.clone();
    let readiness_nonce = completion.0.env[gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV].clone();
    fixture
        .runtime
        .handle_launch_complete_and_drain(fresh_window.clone(), Ok(completion));
    let (spawner, _) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    let mut ready = runtime_hook_state_for_event("Working", "SessionStart", &session_id);
    ready.continuation_readiness_nonce = Some(readiness_nonce);
    ready.project_root = Some(fixture.project_root.display().to_string());
    ready.branch = Some("work/issue-3165".into());
    fixture.runtime.handle_runtime_hook_event(ready);
    commit_pending_fresh_execution(&mut fixture.runtime);
    let candidate =
        gwt_agent::Session::load(&fixture.sessions_dir.join(format!("{session_id}.toml"))).unwrap();
    assert_ne!(session_id, fixture.source_session_id);
    assert_eq!(
        fixture.runtime.active_agent_sessions[&fresh_window].session_id,
        session_id
    );
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(
            &fixture.worktree,
            fixture.execution_owner
        )
        .unwrap(),
        Some(candidate.execution_binding.unwrap().identity)
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(
            &fixture.worktree,
            fixture.execution_owner
        )
        .unwrap()
        .unwrap()
        .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Active)
    );
    assert!(matches!(
        fixture.runtime.runtimes[&fresh_window]
            .pane
            .lock()
            .unwrap()
            .check_status()
            .unwrap(),
        gwt_terminal::PaneStatus::Running
    ));
    settle_test_pane_child(&fixture.runtime, &fresh_window);
}

#[test]
fn app_runtime_monitor_fresh_completed_slice_handoff_stays_with_its_owner() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    for (case, work_owner) in [
        ("completed-slice-handoff", Some("SPEC-3165")),
        ("foreign-slice-handoff", Some("SPEC-9999")),
        ("missing-slice-handoff", None),
    ] {
        let mut fixture = monitor_relaunch_fixture_with_settlement(
            temp.path(),
            case,
            MonitorProviderConversationFixture::Present,
            MonitorNativeHolderFixture::None,
            false,
            LinkedIssueKind::Spec,
            gwt::cli::execution_state::ExecutionSettlement::Completed,
        );
        if let Some(work_owner) = work_owner {
            let predecessor = ActiveAgentSession {
                session_id: fixture.source_session_id.clone(),
                branch_name: "work/issue-3165".into(),
                worktree_path: fixture.worktree.clone(),
                agent_project_root: fixture.worktree.display().to_string(),
                ..sample_active_agent_session("tab-1", "predecessor-window")
            };
            save_workspace_launch_projection(
                &fixture.project_root,
                &predecessor,
                Some("origin/develop"),
                Some(fixture.execution_owner.number),
                Some(fixture.execution_owner),
                None,
                WorkspaceLaunchProjectionKind::StartWork,
                Some(&HashSet::from([fixture.source_session_id.clone()])),
            )
            .unwrap();
            gwt_core::workspace_projection::transact_workspace_close_state_for_work_event_root(
                &fixture.project_root,
                &fixture.worktree,
                |projection, _, _| {
                    let mut done = gwt_core::workspace_projection::WorkEvent::new(
                        gwt_core::workspace_projection::WorkEventKind::Done,
                        &projection.id,
                        Utc::now(),
                    );
                    done.summary = Some("Delivered queue repair token=private-value".into());
                    done.next_action =
                        Some("AC-12: minimal Plan is to verify successor input".into());
                    Ok(((), vec![done]))
                },
            )
            .unwrap();
            if work_owner != "SPEC-3165" {
                let mut works = gwt_core::workspace_projection::load_workspace_work_items(
                    &fixture.project_root,
                )
                .unwrap()
                .unwrap();
                for item in &mut works.work_items {
                    item.owner = Some(work_owner.into());
                }
                let project_key =
                    gwt_core::paths::resolve_project_scope(&fixture.project_root).hash;
                gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
                    &gwt_core::paths::gwt_workspace_work_items_path(&project_key),
                    &works,
                )
                .unwrap();
            }
        }
        prepare_monitor_relaunch(
            &mut fixture,
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
        let result = take_monitor_launch_complete(case, &fixture.recorded_events);
        let prompt = launch_payload_fragments(&result.as_ref().unwrap().0).join("\n");
        let expected = work_owner == Some("SPEC-3165");
        assert_eq!(
            prompt.contains("Delivered queue repair"),
            expected,
            "{case}: {prompt}"
        );
        assert_eq!(
            prompt.contains("AC-12: minimal Plan"),
            expected,
            "{case}: {prompt}"
        );
        assert!(!prompt.contains("private-value"), "{case}: {prompt}");
        if expected {
            assert!(prompt.contains("Re-read the current SPEC"), "{prompt}");
            assert!(prompt.contains(&fixture.predecessor_execution_binding.generation_id));
        }
        assert_monitor_fresh_successor(result, &fixture);
    }
}

#[test]
fn app_runtime_monitor_resume_from_terminal_owner_retains_execution_authority() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "resume-terminal-authority",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
        LinkedIssueKind::Spec,
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "monitor predecessor failed".to_string(),
            missing_verification: Some("successor verification pending".to_string()),
        },
    );
    prepare_monitor_relaunch(
        &mut fixture,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let result =
        take_monitor_launch_complete("resume-terminal-authority", &fixture.recorded_events);
    let (.., session_mode, _, _) = result.as_ref().expect("monitor launch succeeds");
    if *session_mode == gwt_agent::SessionMode::Resume {
        let (_, session_id, ..) = result.as_ref().unwrap();
        let resumed =
            gwt_agent::Session::load(&fixture.sessions_dir.join(format!("{session_id}.toml")))
                .unwrap();
        let binding = resumed
            .execution_binding
            .as_ref()
            .expect("Resume must retain execution authority (#4788)");
        assert_eq!(binding.session_id, *session_id);
        assert_eq!(binding.owner_number, fixture.execution_owner.number);
        assert_eq!(
            gwt::cli::execution_state::current_execution_binding(
                &fixture.worktree,
                fixture.execution_owner,
            )
            .unwrap(),
            Some(binding.identity.clone())
        );
        assert_eq!(
            gwt::cli::execution_state::load_generation_ledger(
                &fixture.worktree,
                fixture.execution_owner,
            )
            .unwrap()
            .unwrap()
            .current_effective_status(),
            Some(gwt::cli::execution_state::ExecutionControlStatus::Active)
        );
    } else {
        assert_monitor_fresh_successor(result, &fixture);
    }
}

// The binary harness links a non-test gwt library. Its caller-facing
// verify.run starts this executable as a watchdog, so provide the same entry
// point as gwtd without requiring an unbuilt sibling binary in CI.
#[ctor::ctor(unsafe)]
fn run_monitor_fixture_verification_watchdog() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str)
        == Some(gwt::cli::verification_record::interruption::WATCHDOG_ARG)
    {
        let success = match argv.as_slice() {
            [_, _, worktree, record_id] => {
                gwt::cli::verification_record::interruption::run_watchdog(
                    Path::new(worktree),
                    record_id,
                )
                .is_ok()
            }
            _ => false,
        };
        std::process::exit(if success { 0 } else { 1 });
    }
}

/// Issue #5254 / SPEC #4074 AC-1, AC-6: Monitor retries must publish the
/// successor Work at authenticated readiness before the new agent can mutate it.
fn assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
    case: &str,
    settlement: gwt::cli::execution_state::ExecutionSettlement,
    terminal_kind: gwt_core::workspace_projection::WorkEventKind,
) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        case,
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        !matches!(case, "completed-done" | "blocked-discarded"),
        LinkedIssueKind::Spec,
        settlement,
    );
    let mut launch_strategy = if case == "completed-done" {
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe
    } else {
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired
    };
    if matches!(case, "completed-done" | "blocked-discarded") {
        // Issue #4783 AC-R4: derive this delivery from a reopened owner,
        // rather than only seeding a fresh delivery over terminal history.
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
        let mut monitor = test_monitor_with_prefs(
            gwt::IssueMonitorConfig::default(),
            gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
        );
        monitor.set_enabled(true);
        monitor.set_gui_connected(true);
        let number = fixture.execution_owner.number;
        let mut issue = gwt::IssueMonitorIssue {
            author_login: None,
            number,
            title: "SPEC: Monitor owner with remaining work".into(),
            labels: vec!["gwt-spec".into()],
            state: gwt::IssueMonitorIssueState::Open,
            body: Some("## Acceptance Criteria\n- [ ] AC-1: remaining work\n".into()),
            url: None,
            readiness: gwt::IssueMonitorReadiness::ReadyWithOpenTasks,
            updated_at: Some("2026-08-13T00:01:00Z".into()),
        };
        if case == "completed-done" {
            monitor.record_closed(number, Some("2026-08-13T00:00:00Z".into()));
            assert!(monitor.record_reopened(number, Some("2026-08-13T00:01:00Z".into())));
        }
        monitor.terminal_queue_push(&[number], "operator", "2026-08-13T00:01:00Z");
        if case == "blocked-discarded" {
            // SPEC #4074 AC-6: the PM holds a dependency-blocked owner until
            // the prerequisite merges. Admission must not prepare its successor.
            let predecessor = gwt::cli::execution_state::load_generation_ledger(
                &fixture.worktree,
                fixture.execution_owner,
            )
            .unwrap();
            issue.labels.push("hold".into());
            gwt::scan_issue_monitor_candidates_with_provenance(
                &mut monitor,
                std::slice::from_ref(&issue),
                gwt::IssueMonitorCandidateSource::Live,
                &fixture.project_root,
                "2026-08-13T00:01:00Z",
            );
            assert_eq!(
                monitor.inbox_item(number).unwrap().state,
                gwt::MonitorInboxState::HoldExcluded
            );
            assert!(monitor.queued_issue_numbers().is_empty());
            assert!(monitor.claim_probe_plan(1).1.is_empty());
            assert!(monitor
                .next_launch_request("2026-08-13T00:01:00Z")
                .is_none());
            assert!(monitor.pending_launch_delivery_id(number).is_none());
            assert_eq!(
                gwt::cli::execution_state::load_generation_ledger(
                    &fixture.worktree,
                    fixture.execution_owner,
                )
                .unwrap(),
                predecessor,
                "hold must preserve the Blocked generation without a Prepared successor"
            );
            // The PM confirms the dependency merged, removes hold and
            // requeues the same owner; dependency interpretation stays in PM.
            issue.labels.retain(|label| label != "hold");
            issue.updated_at = Some("2026-08-13T00:01:01Z".into());
            monitor.record_launch_failed(number, "dependency-blocked predecessor stopped");
            gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).unwrap();
            let mut env = gwt::cli::TestEnv::new(fixture.project_root.clone());
            let code = gwt::cli::run(
                &mut env,
                gwt::cli::CliCommand::Issue(gwt::cli::IssueCommand::MonitorRequeue {
                    project_root: Some(fixture.project_root.clone()),
                    number,
                    reason: "dependency merged".into(),
                    force: false,
                }),
            )
            .expect("PM requeues the dependency-blocked owner");
            assert_eq!(code, 0, "{}", String::from_utf8_lossy(&env.stdout));
            monitor = test_monitor_with_prefs(
                gwt::IssueMonitorConfig::default(),
                gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
            );
            monitor.set_gui_connected(true);
        }
        gwt::scan_issue_monitor_candidates_with_provenance(
            &mut monitor,
            &[issue],
            gwt::IssueMonitorCandidateSource::Live,
            &fixture.project_root,
            "2026-08-13T00:01:01Z",
        );
        assert_eq!(monitor.queued_issue_numbers(), vec![number]);
        assert_eq!(monitor.claim_probe_plan(1), (1, vec![number]));
        assert!(monitor.apply_confirmed_claim(
            number,
            "claim-reopened",
            "host/session",
            "effect-reopened",
            &Utc::now().to_rfc3339(),
        ));
        fixture.delivery_id = monitor.pending_launch_delivery_id(number);
        if case == "blocked-discarded" {
            launch_strategy = monitor
                .prefs()
                .pending_launch_deliveries
                .iter()
                .find(|delivery| Some(&delivery.delivery_id) == fixture.delivery_id.as_ref())
                .expect("requeue produces a durable launch delivery")
                .launch_session_strategy;
            assert_eq!(
                launch_strategy,
                gwt::IssueMonitorLaunchSessionStrategy::FreshRequired
            );
        }
        gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).unwrap();
    }
    let unique_head = if case.contains("ownerless") {
        fs::write(fixture.worktree.join("retained-source.txt"), case).unwrap();
        run_git(&fixture.worktree, &["add", "retained-source.txt"]);
        run_git(&fixture.worktree, &["commit", "-m", "retain unique source"]);
        Some(git_stdout(&fixture.worktree, &["rev-parse", "HEAD"]))
    } else {
        None
    };
    let staged_diff = (case == "3824-active-parentless-successor").then(|| {
        fs::write(fixture.worktree.join("retained-staged-source.txt"), case).unwrap();
        run_git(&fixture.worktree, &["add", "retained-staged-source.txt"]);
        git_stdout(&fixture.worktree, &["diff", "--cached", "--binary"])
    });
    let source_path = fixture
        .sessions_dir
        .join(format!("{}.toml", fixture.source_session_id));
    let source_bytes = fs::read(&source_path).expect("predecessor Session bytes");
    let predecessor_active = ActiveAgentSession {
        session_id: fixture.source_session_id.clone(),
        branch_name: "work/issue-3165".to_string(),
        worktree_path: fixture.worktree.clone(),
        agent_project_root: fixture.worktree.display().to_string(),
        ..sample_active_agent_session("tab-1", "predecessor-window")
    };
    save_workspace_launch_projection(
        &fixture.project_root,
        &predecessor_active,
        Some("origin/develop"),
        Some(fixture.execution_owner.number),
        Some(fixture.execution_owner),
        None,
        WorkspaceLaunchProjectionKind::StartWork,
        Some(&HashSet::from([fixture.source_session_id.clone()])),
    )
    .expect("publish predecessor Work");
    let mut predecessor_id =
        gwt_core::workspace_projection::transact_workspace_close_state_for_work_event_root(
            &fixture.project_root,
            &fixture.worktree,
            |projection, _, _| {
                let event = gwt_core::workspace_projection::WorkEvent::new(
                    terminal_kind,
                    projection.id.clone(),
                    Utc::now(),
                );
                Ok((projection.id.clone(), vec![event]))
            },
        )
        .expect("terminalize predecessor Work");
    // Launch starts shared-event intake. Exercise that rebuild before the
    // readiness handshake so the predecessor's local close must survive it.
    let project_key = gwt_core::paths::resolve_project_scope(&fixture.project_root).hash;
    let intake = crate::work_events_ingest::ingest_project_work_events_paths(
        &fixture.project_root,
        &gwt_core::paths::gwt_workspace_work_items_path(&project_key),
        &gwt_core::paths::gwt_workspace_work_events_intake_state_path(&project_key),
    );
    assert!(intake.load_error.is_none(), "{case}: {intake:?}");
    // Issue #5288: these persisted shapes survived real launches on
    // different hosts. Seed the machine-local projection after intake,
    // rather than pretending every old Work still has this host's path.
    let works_path = gwt_core::paths::gwt_workspace_work_items_path(&project_key);
    let mut history =
        gwt_core::workspace_projection::load_workspace_work_items(&fixture.project_root)
            .unwrap()
            .unwrap();
    if case == "5696-foreign-terminal-owner" {
        // Issue #5696 AC-2: a terminal row for another owner occupied the
        // branch-derived canonical ID (work/issue-2359, owner 3350).
        let old = history
            .work_items
            .iter_mut()
            .find(|work| work.id == predecessor_id)
            .unwrap();
        old.owner = Some("3350".to_string());
        for event in &mut old.events {
            event.owner = Some("3350".to_string());
        }
    }
    if case == "3287-done-resume-chain" {
        use gwt_core::workspace_projection::{successor_work_id, WorkEventKind};
        let mut start = history
            .work_items
            .iter()
            .find(|work| work.id == predecessor_id)
            .unwrap()
            .events
            .iter()
            .find(|event| event.kind == WorkEventKind::Start)
            .unwrap()
            .clone();
        start.execution_container.as_mut().unwrap().worktree_path =
            Some(dunce::canonicalize(&fixture.worktree).unwrap());
        start.owner = Some("SPEC-3165".to_string());
        for index in 0..3 {
            start.id = format!("3287-successor-{index}");
            start.work_item_id = successor_work_id(&predecessor_id);
            start.related_work_item_id = Some(predecessor_id.clone());
            start.agent_session_id = None;
            start.updated_at = Utc::now();
            history.apply_event(start.clone());
            let mut resume = start.clone();
            resume.id = format!("3287-resume-{index}");
            resume.kind = WorkEventKind::Resume;
            resume.title = Some("Notification delivery".to_string());
            resume.next_action = Some("Check Board for latest updates".to_string());
            history.apply_event(resume);
            predecessor_id = start.work_item_id.clone();
            if index < 2 {
                history.apply_event(gwt_core::workspace_projection::WorkEvent::new(
                    WorkEventKind::Done,
                    &predecessor_id,
                    Utc::now(),
                ));
            }
        }
        // Earlier repair erased the active tail's owner. Fresh authority
        // may restore it, but the next ordinary intake must preserve it.
        history
            .work_items
            .iter_mut()
            .find(|work| work.id == predecessor_id)
            .unwrap()
            .owner = None;
        use sha2::Digest;
        let canonical = format!(
            "{{\"session_id\":{},\"result\":{{\"outcome\":\"deferred\",\"reason\":\"fixture\"}},\"recorded_at\":{},\"content_hash\":\"\"}}",
            serde_json::to_string(&fixture.source_session_id).unwrap(),
            serde_json::to_string(&Utc::now()).unwrap(),
        );
        let mut revival: serde_json::Value = serde_json::from_str(&canonical).unwrap();
        revival["content_hash"] =
            format!("{:x}", sha2::Sha256::digest(canonical.as_bytes())).into();
        gwt::cli::trusted_store::write(
            &fixture.worktree,
            "action-obligation-revival.json",
            &serde_json::to_vec(&revival).unwrap(),
        )
        .unwrap();
    }
    if case.contains("foreign-container") {
        let predecessor = history
            .work_items
            .iter_mut()
            .find(|work| work.id == predecessor_id)
            .unwrap();
        predecessor.execution_containers[0].worktree_path =
            Some(PathBuf::from(r"E:\gwt\work\issue-3165"));
    }
    if case == "4777-ownerless-root" {
        history
            .work_items
            .iter_mut()
            .find(|work| work.id == predecessor_id)
            .unwrap()
            .owner = None;
    }
    if case.starts_with("4249") || case.starts_with("4777") || case.starts_with("5082") {
        let mut start = history
            .work_items
            .iter()
            .find(|work| work.id == predecessor_id)
            .unwrap()
            .events
            .iter()
            .find(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Start)
            .unwrap()
            .clone();
        start.id = format!("{case}-successor-start");
        start.work_item_id = gwt_core::workspace_projection::successor_work_id(&predecessor_id);
        start.related_work_item_id = Some(predecessor_id.clone());
        start.agent_session_id = None;
        start.updated_at = Utc::now();
        start.execution_container.as_mut().unwrap().worktree_path =
            Some(dunce::canonicalize(&fixture.worktree).unwrap());
        history.apply_event(start.clone());
        if case.starts_with("4249") {
            history
                .work_items
                .iter_mut()
                .find(|work| work.id == predecessor_id)
                .unwrap()
                .execution_containers
                .clear();
        } else {
            predecessor_id = start.work_item_id.clone();
            history.apply_event(gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Discard,
                &predecessor_id,
                Utc::now(),
            ));
            if case == "4777-missing-historical-owner" || case.contains("ownerless") {
                // Real historical chains can lose owners at the root,
                // terminal middle generations, or active latest Work.
                history
                    .work_items
                    .iter_mut()
                    .find(|work| work.id == predecessor_id)
                    .unwrap()
                    .owner = None;
                if case == "5082-ownerless-current" {
                    start.id = format!("{case}-middle-start");
                    start.work_item_id =
                        gwt_core::workspace_projection::successor_work_id(&predecessor_id);
                    start.related_work_item_id = Some(predecessor_id.clone());
                    start.owner = None;
                    history.apply_event(start.clone());
                    predecessor_id = start.work_item_id.clone();
                    history.apply_event(gwt_core::workspace_projection::WorkEvent::new(
                        gwt_core::workspace_projection::WorkEventKind::Discard,
                        &predecessor_id,
                        Utc::now(),
                    ));
                }
                start.id = format!("{case}-latest-start");
                start.work_item_id =
                    gwt_core::workspace_projection::successor_work_id(&predecessor_id);
                start.related_work_item_id = Some(predecessor_id.clone());
                history.apply_event(start);
            }
        }
    }
    if case.contains("placeholder") {
        let mut placeholder = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Update,
            gwt_core::workspace_projection::successor_work_id(&predecessor_id),
            Utc::now(),
        );
        placeholder.owner =
            (case != "4777-discarded-successor-placeholder").then(|| "SPEC-3165".to_string());
        placeholder.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some("work/issue-3165".to_string()),
                worktree_path: Some(dunce::canonicalize(&fixture.worktree).unwrap()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        let placeholder_id = placeholder.work_item_id.clone();
        // Reproduce the compacted 49 Update + Pause tail seen in the
        // real rows; lost inline history must not prevent exact recovery.
        for index in 0..60 {
            let mut update = placeholder.clone();
            update.id = format!("{case}-update-{index}");
            history.apply_event(update);
        }
        let mut paused = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Pause,
            placeholder_id,
            Utc::now(),
        );
        paused.agent_session_id = Some(format!("stranded-{case}"));
        history.apply_event(paused);
    }
    if case.starts_with("5257") {
        let work = history
            .work_items
            .iter_mut()
            .find(|work| work.id == predecessor_id)
            .unwrap();
        work.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Idle;
        work.execution_containers[0].worktree_path =
            Some(dunce::canonicalize(&fixture.worktree).unwrap());
        work.agents.clear();
        work.events.clear();
        let mut backfill = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Backfill,
            &predecessor_id,
            Utc::now(),
        );
        backfill.owner = work.owner.clone();
        backfill.execution_container = work.execution_containers.first().cloned();
        history.apply_event(backfill);
    }
    let superseded_occupancy = if case.contains("-successor") && !case.contains("placeholder") {
        let mut invalid = history
            .work_items
            .iter()
            .find(|work| work.id == predecessor_id)
            .unwrap()
            .events
            .iter()
            .find(|event| event.kind == gwt_core::workspace_projection::WorkEventKind::Start)
            .unwrap()
            .clone();
        invalid.id = format!("invalid-{case}");
        invalid.work_item_id = gwt_core::workspace_projection::successor_work_id(&predecessor_id);
        invalid.agent_session_id = Some(format!("stranded-{case}"));
        invalid.related_work_item_id = case.starts_with("4783").then(|| predecessor_id.clone());
        invalid.updated_at = Utc::now();
        let invalid_id = invalid.work_item_id.clone();
        history.apply_event(invalid);
        if case.starts_with("4783") {
            history
                .work_items
                .iter_mut()
                .find(|work| work.id == invalid_id)
                .unwrap()
                .execution_containers
                .clear();
        }
        if case.starts_with("5288") {
            history.apply_event(gwt_core::workspace_projection::WorkEvent::new(
                gwt_core::workspace_projection::WorkEventKind::Done,
                &invalid_id,
                Utc::now(),
            ));
        }
        Some(invalid_id)
    } else if case.contains("placeholder") {
        Some(gwt_core::workspace_projection::successor_work_id(
            &predecessor_id,
        ))
    } else {
        None
    };
    let terminal_history: Vec<_> = if case == "4777-missing-historical-owner"
        || case.contains("ownerless")
        || case == "3287-done-resume-chain"
    {
        history
            .work_items
            .iter()
            .filter(|work| work.is_terminal())
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
        &works_path,
        &history,
    )
    .expect("seed observed historical Work shape");
    let persisted_successor_detachment = matches!(
        case,
        "4783-active-empty-container-successor" | "4777-discarded-successor-placeholder"
    );
    if persisted_successor_detachment {
        let invalid_id = superseded_occupancy.as_ref().unwrap();
        let mut container = history
            .work_items
            .iter()
            .find(|work| &work.id == invalid_id)
            .unwrap()
            .events
            .iter()
            .find_map(|event| event.execution_container.clone())
            .unwrap();
        container.worktree_path = Some(dunce::canonicalize(&fixture.worktree).unwrap());
        // Issues #4783/#4777: the durable repair receipt also removes the
        // container reintroduced by the reserved supersession audit.
        let detachments = std::collections::BTreeMap::from([(invalid_id.clone(), vec![container])]);
        fs::write(
            works_path.with_extension("detachments.json"),
            serde_json::to_vec(&detachments).unwrap(),
        )
        .expect("seed persistent successor container detachment");
    }
    let predecessor =
        gwt_core::workspace_projection::load_workspace_work_items(&fixture.project_root)
            .unwrap()
            .unwrap()
            .work_items
            .into_iter()
            .find(|work| work.id == predecessor_id)
            .expect("terminal predecessor Work");
    let predecessor_terminal = predecessor.is_terminal();

    let before =
        gwt::cli::execution_state::diagnose_owner(&fixture.worktree, fixture.execution_owner);
    assert_eq!(
        before.recommended_recovery, "gwt-execute",
        "{case}: {before:?}"
    );

    prepare_monitor_relaunch(&mut fixture, launch_strategy);
    let (window_id, result) = take_monitor_launch_complete_event(case, &fixture.recorded_events);
    let completion = result.expect("Monitor fresh launch preparation");
    let session_id = completion.1.clone();
    assert_ne!(session_id, fixture.source_session_id, "{case}");
    let readiness_nonce = completion
        .0
        .env
        .get(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV)
        .cloned()
        .expect("Monitor successor readiness nonce");
    fixture
        .runtime
        .handle_launch_complete_and_drain(window_id.clone(), Ok(completion));
    assert!(fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&window_id));
    if case == "completed-done" {
        let observations =
            super::super::terminal_convergence::observe_terminal_windows_in_background(
                &fixture.sessions_dir,
                fixture.runtime.terminal_window_snapshots(),
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
            fixture.runtime.window_lookup.contains_key(&window_id),
            "the reopened owner's launch survives terminal grace before Ready"
        );
    }
    let (spawner, _) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    let mut ready = runtime_hook_state_for_event("Working", "SessionStart", &session_id);
    ready.continuation_readiness_nonce = Some(readiness_nonce);
    ready.project_root = Some(fixture.project_root.display().to_string());
    ready.branch = Some("work/issue-3165".to_string());
    fixture.runtime.handle_runtime_hook_event(ready);
    let events = commit_pending_fresh_execution(&mut fixture.runtime);
    assert!(
        !events.iter().any(|event| matches!(
            event.event,
            BackendEvent::TerminalStatus {
                status: WindowProcessStatus::Error,
                ..
            }
        )),
        "{case}: fresh readiness must commit, not roll back: {events:?}",
    );
    assert!(!fixture
        .runtime
        .pending_fresh_execution_launches
        .contains_key(&window_id));
    let candidate =
        gwt_agent::Session::load(&fixture.sessions_dir.join(format!("{session_id}.toml")))
            .expect("committed successor Session");
    assert_eq!(
        gwt::cli::execution_state::current_execution_binding(
            &fixture.worktree,
            fixture.execution_owner,
        )
        .expect("committed successor execution"),
        Some(
            candidate
                .execution_binding
                .expect("committed successor Session binding")
                .identity,
        ),
        "{case}: readiness must activate the candidate before Work readback",
    );

    let works = gwt_core::workspace_projection::load_workspace_work_items(&fixture.project_root)
        .unwrap()
        .unwrap();
    let successor_id = gwt_core::workspace_projection::current_work_id(
        &works,
        &fixture.project_root,
        Some("work/issue-3165"),
        Some(&fixture.worktree),
    )
    .expect("current canonical Work");
    let successor = works
        .work_items
        .iter()
        .find(|work| work.id == successor_id)
        .expect("successor Work");
    if predecessor_terminal {
        assert_ne!(successor_id, predecessor_id, "{case}");
    }
    assert!(!successor.is_terminal(), "{case}");
    if predecessor_terminal {
        assert!(successor
            .related_work_item_ids
            .contains(superseded_occupancy.as_ref().unwrap_or(&predecessor_id)));
    }
    if let Some(invalid_id) = &superseded_occupancy {
        let invalid = works
            .work_items
            .iter()
            .find(|work| &work.id == invalid_id)
            .unwrap();
        assert!(
            invalid.is_terminal(),
            "{case}: invalid occupancy is audit-closed"
        );
        assert!(
            invalid
                .events
                .iter()
                .any(|event| event.is_successor_supersession()),
            "{case}"
        );
        if persisted_successor_detachment {
            assert!(
                invalid.execution_containers.is_empty(),
                "{case}: the durable detachment still applies after the supersession audit"
            );
            if case == "4777-discarded-successor-placeholder" {
                assert_eq!(invalid.owner, None, "{case}: preserve ownerless history");
            }
        }
    }
    if staged_diff.is_some() {
        let start = successor
            .events
            .iter()
            .find(|event| {
                event.kind == gwt_core::workspace_projection::WorkEventKind::Start
                    && event.agent_session_id.as_deref() == Some(session_id.as_str())
            })
            .expect("committed successor Start");
        assert_eq!(
            start.related_work_item_id.as_deref(),
            Some(
                superseded_occupancy
                    .as_ref()
                    .unwrap_or(&predecessor_id)
                    .as_str()
            ),
            "{case}"
        );
        assert!(
            gwt_core::workspace_projection::workspace_work_event_shard_matches(
                &fixture.worktree,
                start,
            )
            .unwrap(),
            "{case}: stored shard retains the returned Start lineage"
        );
        // A cold intake must derive the same linked lifetime from the
        // immutable sources, including the original malformed Start.
        let cold_intake_path = temp.path().join(format!("{case}-cold-intake.json"));
        let intake = crate::work_events_ingest::ingest_project_work_events_paths(
            &fixture.project_root,
            &works_path,
            &cold_intake_path,
        );
        assert!(intake.load_error.is_none(), "{case}: {intake:?}");
        assert!(intake.projection_rebuilt, "{case}: {intake:?}");
        let reloaded =
            gwt_core::workspace_projection::load_workspace_work_items(&fixture.project_root)
                .unwrap()
                .unwrap();
        assert_eq!(
            gwt_core::workspace_projection::current_work_id(
                &reloaded,
                &fixture.project_root,
                Some("work/issue-3165"),
                Some(&fixture.worktree),
            )
            .as_deref(),
            Some(successor_id.as_str()),
            "{case}: cold intake keeps successor authority"
        );
    }
    let projection =
        gwt_core::workspace_projection::load_workspace_projection(&fixture.project_root)
            .unwrap()
            .unwrap();
    let assigned = projection
        .latest_agent_for_session(&session_id)
        .expect("successor Session assignment");
    assert!(assigned.is_assigned(), "{case}");
    assert_eq!(
        assigned.workspace_id.as_deref(),
        Some(successor_id.as_str())
    );

    let _current_session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &session_id);
    if case == "3287-done-resume-chain" {
        gwt_core::workspace_projection::repair_resume_owner_bleed_for_repo(
            &fixture.project_root,
            Utc::now(),
        )
        .expect("ordinary intake owner repair");
        let diagnosis =
            gwt::cli::execution_state::diagnose_owner(&fixture.worktree, fixture.execution_owner);
        let revival = serde_json::to_value(diagnosis.obligation_revival.unwrap()).unwrap();
        assert_eq!(revival["outcome"], "status_unreadable");
        assert_eq!(revival["validation"]["check"], "identity");
        assert_eq!(
            revival["validation"]["expected_content_hash"],
            revival["validation"]["actual_content_hash"]
        );
    }
    let mut env = gwt::cli::TestEnv::new(fixture.worktree.clone());
    gwt::cli::run(
        &mut env,
        gwt::cli::CliCommand::Workspace(gwt::cli::WorkspaceCommand::Ensure {
            agent_session: session_id.clone(),
            title_summary: "Monitor successor Work".to_string(),
            current_focus: None,
            spec: Some(fixture.execution_owner.number),
            issue: None,
            topic: None,
            boundary: None,
        }),
    )
    .expect("Monitor successor workspace.ensure must be accepted");
    assert!(String::from_utf8(env.stdout)
        .unwrap()
        .contains(&successor_id));
    let session =
        gwt_agent::Session::load(&fixture.sessions_dir.join(format!("{session_id}.toml"))).unwrap();
    let update = gwt::apply_bound_authenticated_workspace_update(
        &fixture.project_root,
        &session_id,
        session.execution_binding.as_ref().unwrap(),
        gwt::AgentWorkspaceUpdateRequest {
            schema_version: gwt::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
            claimed_session_id: session_id.clone(),
            observation: gwt::observe_agent_runtime(&fixture.worktree).unwrap(),
            intent: gwt::AgentWorkspaceUpdateIntent {
                current_focus: Some("Monitor successor accepted".to_string()),
                ..Default::default()
            },
        },
    )
    .expect("Monitor successor workspace.update must be accepted");
    assert_eq!(update.work_id, successor_id, "{case}");
    if case == "5696-foreign-terminal-owner" {
        let receipt = gwt::probe_bound_authenticated_work_materialization(
            &fixture.project_root,
            &session_id,
            session.execution_binding.as_ref().unwrap(),
            gwt::AgentWorkMaterializationProbeRequest {
                schema_version: gwt::AGENT_WORK_MATERIALIZATION_PROBE_SCHEMA_VERSION,
                claimed_session_id: session_id.clone(),
                owner_number: fixture.execution_owner.number,
                observation: gwt::observe_agent_runtime(&fixture.worktree).unwrap(),
            },
        )
        .expect("build.start canonical materialization preflight must accept the successor");
        assert_eq!(receipt.work_id, successor_id);
    }
    let after =
        gwt::cli::execution_state::diagnose_owner(&fixture.worktree, fixture.execution_owner);
    assert_ne!(after.generation_id, before.generation_id, "{case}");
    assert_ne!(
        after.recommended_recovery, before.recommended_recovery,
        "{case}: {after:?}"
    );
    assert_eq!(fs::read(&source_path).unwrap(), source_bytes, "{case}");
    if let Some(unique_head) = unique_head {
        assert_eq!(
            git_stdout(&fixture.worktree, &["rev-parse", "HEAD"]),
            unique_head,
            "{case}: retain unique commit"
        );
    }
    if let Some(staged_diff) = staged_diff {
        assert_eq!(
            git_stdout(&fixture.worktree, &["diff", "--cached", "--binary"]),
            staged_diff,
            "{case}: retain staged source bytes"
        );
    }
    let updated = gwt_core::workspace_projection::load_workspace_work_items(&fixture.project_root)
        .unwrap()
        .unwrap();
    if predecessor_terminal {
        assert_eq!(
            updated
                .work_items
                .iter()
                .find(|work| work.id == predecessor_id),
            Some(&predecessor),
            "{case}: preserve terminal Work and Session membership",
        );
    }
    for historical in &terminal_history {
        assert_eq!(
            updated
                .work_items
                .iter()
                .find(|work| work.id == historical.id),
            Some(historical),
            "{case}: preserve every historical terminal generation",
        );
    }
    let mut verify_env = gwt::cli::TestEnv::new(fixture.worktree.clone());
    let code = gwt::cli::run(
        &mut verify_env,
        gwt::cli::CliCommand::Verify(gwt::cli::verification_record::VerifyCommand::Plan {
            // This throwaway Git project has no Rust manifest. Exercise
            // canonical verify.run authority with a real portable command.
            commands: vec!["git diff --check".to_string()],
            derive: false,
        }),
    )
    .expect("successor verify.plan");
    assert_eq!(
        code,
        0,
        "{case}: {}",
        String::from_utf8_lossy(&verify_env.stdout)
    );
    let plan = gwt::cli::verification_record::load_plan(&fixture.worktree)
        .unwrap()
        .unwrap();
    let mut verify_env = gwt::cli::TestEnv::new(fixture.worktree.clone());
    let code = gwt::cli::run(
        &mut verify_env,
        gwt::cli::CliCommand::Verify(gwt::cli::verification_record::VerifyCommand::Run {
            commands: plan.commands.clone(),
            max_wait_secs: None,
            user_verification_result: None,
            headed_e2e_commands: Vec::new(),
        }),
    )
    .expect("successor verify.run");
    assert_eq!(
        code,
        0,
        "{case}: {}",
        String::from_utf8_lossy(&verify_env.stdout)
    );
    fixture
        .runtime
        .stop_window_runtime_without_session_projection(&window_id);
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_3287_done_resume_chain() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "3287-done-resume-chain",
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "previous holder child exited".to_string(),
            missing_verification: None,
        },
        gwt_core::workspace_projection::WorkEventKind::Done,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_blocked_discarded() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "blocked-discarded",
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "waiting for dependency Issue #3906 to merge".to_string(),
            missing_verification: None,
        },
        gwt_core::workspace_projection::WorkEventKind::Discard,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_completed_done() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "completed-done",
        gwt::cli::execution_state::ExecutionSettlement::Completed,
        gwt_core::workspace_projection::WorkEventKind::Done,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_4117_done_foreign_container() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "4117-done-foreign-container",
        gwt::cli::execution_state::ExecutionSettlement::Completed,
        gwt_core::workspace_projection::WorkEventKind::Done,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_5096_active_foreign_container() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "5096-active-foreign-container",
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "previous holder exited".to_string(),
            missing_verification: None,
        },
        gwt_core::workspace_projection::WorkEventKind::Update,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_4249_empty_predecessor_container() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "4249-empty-predecessor-container",
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "previous holder exited".to_string(),
            missing_verification: None,
        },
        gwt_core::workspace_projection::WorkEventKind::Done,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_4777_discarded_successor_placeholder() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "4777-discarded-successor-placeholder",
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "previous holder exited".to_string(),
            missing_verification: None,
        },
        gwt_core::workspace_projection::WorkEventKind::Discard,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_4777_missing_historical_owner() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "4777-missing-historical-owner",
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "previous holder exited".to_string(),
            missing_verification: None,
        },
        gwt_core::workspace_projection::WorkEventKind::Discard,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_4777_ownerless_root() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "4777-ownerless-root",
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "previous holder exited".to_string(),
            missing_verification: None,
        },
        gwt_core::workspace_projection::WorkEventKind::Discard,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_5082_ownerless_current() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "5082-ownerless-current",
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "previous holder exited".to_string(),
            missing_verification: None,
        },
        gwt_core::workspace_projection::WorkEventKind::Discard,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_5257_idle_work_after_refused_adoption() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "5257-idle-work-after-refused-adoption",
        gwt::cli::execution_state::ExecutionSettlement::Blocked {
            reason: "adoption did not complete".to_string(),
            missing_verification: None,
        },
        gwt_core::workspace_projection::WorkEventKind::Update,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_5254_done_successor_placeholder() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "5254-done-successor-placeholder",
        gwt::cli::execution_state::ExecutionSettlement::Completed,
        gwt_core::workspace_projection::WorkEventKind::Done,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_4783_active_empty_container_successor() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "4783-active-empty-container-successor",
        gwt::cli::execution_state::ExecutionSettlement::Completed,
        gwt_core::workspace_projection::WorkEventKind::Done,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_3824_active_parentless_successor() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "3824-active-parentless-successor",
        gwt::cli::execution_state::ExecutionSettlement::Completed,
        gwt_core::workspace_projection::WorkEventKind::Done,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_5288_terminal_invalid_successor() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "5288-terminal-invalid-successor",
        gwt::cli::execution_state::ExecutionSettlement::Completed,
        gwt_core::workspace_projection::WorkEventKind::Done,
    );
}

#[test]
fn app_runtime_monitor_fresh_terminal_predecessor_5696_foreign_terminal_owner() {
    assert_monitor_fresh_terminal_predecessor_materializes_usable_successor_work(
        "5696-foreign-terminal-owner",
        gwt::cli::execution_state::ExecutionSettlement::Completed,
        gwt_core::workspace_projection::WorkEventKind::Done,
    );
}

#[test]
fn invalid_successor_tombstones_have_content_bound_identity_across_hosts() {
    use gwt_core::workspace_projection as work;
    let temp = tempdir().unwrap();
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-21T01:54:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let parent = "work-parent-5867";
    let occupied_id = work::successor_work_id(parent);
    let mut works = work::WorkItemsProjection::empty(now);
    works.apply_event(work::WorkEvent::new(
        work::WorkEventKind::Start,
        parent,
        now,
    ));
    works.apply_event(work::WorkEvent::new(work::WorkEventKind::Done, parent, now));
    works.apply_event(work::WorkEvent::new(
        work::WorkEventKind::Start,
        &occupied_id,
        now,
    ));
    let make = |path: &str, time| {
        super::super::workspace::invalid_successor_supersession_event(
            temp.path(),
            works
                .work_items
                .iter()
                .find(|item| item.id == parent)
                .unwrap(),
            works
                .work_items
                .iter()
                .find(|item| item.id == occupied_id)
                .unwrap(),
            work::WorkspaceExecutionContainerRef {
                branch: Some("work/issue-5867".to_string()),
                worktree_path: Some(PathBuf::from(path)),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
            super::super::workspace::SuccessorSupersessionAuthority::RegisteredPm,
            &HashSet::new(),
            time,
        )
        .unwrap()
    };
    let windows = make(r"E:\gwt\work\issue-5867", now);
    let mac = make(
        "/Users/example/gwt/work/issue-5867",
        now + chrono::Duration::minutes(31),
    );
    assert_ne!(
        windows.id, mac.id,
        "different host payloads must never target one immutable shard"
    );
    assert_eq!(
        windows,
        make(r"E:\gwt\work\issue-5867", now),
        "same payload retries stay idempotent"
    );
    assert!(windows.is_successor_supersession());
    assert!(mac.is_successor_supersession());
    let mut changed = windows.clone();
    changed.updated_at = mac.updated_at;
    assert!(
        !changed.is_successor_supersession(),
        "a content-bound identity must authenticate its payload"
    );
    let mut legacy = windows;
    legacy.id = format!("superseded-{}", legacy.work_item_id);
    assert!(
        legacy.is_successor_supersession(),
        "historical tombstones remain readable"
    );
}

#[test]
fn invalid_successor_repair_rechecks_exact_runtime_outside_the_local_registry() {
    use gwt_core::workspace_projection as work;
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let fixture = monitor_relaunch_fixture(
        temp.path(),
        "hidden-writer",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    let source = gwt_agent::Session::load(
        &fixture
            .sessions_dir
            .join(format!("{}.toml", fixture.source_session_id)),
    )
    .unwrap();
    let identity = gwt_agent::SessionExecutionIdentity::from_session(&source)
        .unwrap()
        .unwrap();
    let now = Utc::now();
    let predecessor_id =
        work::canonical_work_id(&fixture.project_root, Some("work/issue-3165"), None).unwrap();
    let container = work::WorkspaceExecutionContainerRef {
        branch: Some("work/issue-3165".to_string()),
        worktree_path: Some(fixture.worktree.clone()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    };
    let mut works = work::WorkItemsProjection::empty(now);
    let mut start = work::WorkEvent::new(work::WorkEventKind::Start, &predecessor_id, now);
    start.owner = Some("SPEC-3165".to_string());
    start.execution_container = Some(container.clone());
    works.apply_event(start.clone());
    works.apply_event(work::WorkEvent::new(
        work::WorkEventKind::Done,
        &predecessor_id,
        now,
    ));
    start.id = "invalid-hidden-writer".to_string();
    start.work_item_id = work::successor_work_id(&predecessor_id);
    start.agent_session_id = Some(source.id.clone());
    let occupied_id = start.work_item_id.clone();
    works.apply_event(start);
    let runtime_path = gwt_agent::runtime_state_path(&fixture.sessions_dir, &source.id);
    let process_start = gwt::process::host_process_start_time(std::process::id()).unwrap();
    gwt_agent::SessionRuntimeState::for_execution_process(
        gwt_agent::AgentStatus::Running,
        &identity,
        41,
        process_start,
        std::process::id(),
        process_start,
    )
    .save(&runtime_path)
    .unwrap();
    let repair = || {
        super::super::workspace::invalid_successor_supersession_event(
            &fixture.project_root,
            works
                .work_items
                .iter()
                .find(|work| work.id == predecessor_id)
                .unwrap(),
            works
                .work_items
                .iter()
                .find(|work| work.id == occupied_id)
                .unwrap(),
            container.clone(),
            super::super::workspace::SuccessorSupersessionAuthority::ExecutionOwner("SPEC-3165"),
            &HashSet::new(),
            now,
        )
    };
    assert!(
        repair().is_err(),
        "a live exact writer is protected even before local registry publication"
    );
    fs::write(&runtime_path, b"{malformed runtime").unwrap();
    assert!(
        repair().is_err(),
        "ambiguous runtime evidence remains protected"
    );
    fs::remove_file(runtime_path).unwrap();
    assert!(
        repair().is_ok(),
        "runtime absence permits the same bounded repair"
    );

    // Issue #5696 AC-1: the real Stopped member had no binding, while a
    // legacy Running sidecar remained after both Host and child disappeared.
    let mut legacy = source.clone();
    legacy.execution_binding = None;
    legacy.status = gwt_agent::AgentStatus::Stopped;
    legacy.save(&fixture.sessions_dir).unwrap();
    let legacy_path =
        gwt_agent::runtime_state_path_for_pid(&fixture.sessions_dir, u32::MAX, &legacy.id);
    let mut stale = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running);
    stale.child_pid = Some(u32::MAX);
    stale.save(&legacy_path).unwrap();
    assert!(
        repair().is_ok(),
        "a stopped legacy member with no remaining runtime permits recovery"
    );
    stale.child_pid = Some(std::process::id());
    stale.child_started_at = Some(process_start);
    stale.save(&legacy_path).unwrap();
    assert!(
        repair().is_err(),
        "Stopped must not authorize retiring a live legacy child"
    );
    fs::write(&legacy_path, b"{malformed runtime").unwrap();
    assert!(
        repair().is_err(),
        "unreadable legacy evidence remains protected"
    );
}

#[test]
fn fresh_launch_supersedes_dead_invalid_successor_occupancy() {
    use gwt_core::workspace_projection::{
        canonical_work_id, current_work_id, successor_work_id, WorkEvent, WorkEventKind,
        WorkItemsProjection, WorkspaceExecutionContainerRef, WorkspaceProjection,
    };
    let project = Path::new("/fixture-project");
    let session = ActiveAgentSession {
        session_id: "fresh-session".to_string(),
        branch_name: "work/issue-42".to_string(),
        worktree_path: project.join("checkout"),
        ..sample_active_agent_session("tab", "window")
    };
    let now = Utc::now();
    let predecessor_id = canonical_work_id(project, Some(&session.branch_name), None).unwrap();
    let occupied_id = successor_work_id(&predecessor_id);
    let expected_id = successor_work_id(&occupied_id);
    let live = HashSet::new();
    for case in [
        "missing-container",
        "missing-parent",
        "terminal-missing-parent",
    ] {
        let mut works = WorkItemsProjection::empty(now);
        let mut start = WorkEvent::new(WorkEventKind::Start, &predecessor_id, now);
        start.owner = Some("Issue #42".to_string());
        start.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some(session.branch_name.clone()),
            worktree_path: Some(session.worktree_path.clone()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        works.apply_event(start.clone());
        works.apply_event(WorkEvent::new(WorkEventKind::Done, &predecessor_id, now));
        let predecessor = works.work_items[0].clone();
        start.id = format!("occupied-{case}");
        start.work_item_id = occupied_id.clone();
        if case == "missing-container" {
            start.related_work_item_id = Some(predecessor_id.clone());
        }
        works.apply_event(start.clone());
        if case == "missing-container" {
            works
                .work_items
                .iter_mut()
                .find(|work| work.id == occupied_id)
                .unwrap()
                .execution_containers
                .clear();
        }
        if case == "terminal-missing-parent" {
            works.apply_event(WorkEvent::new(WorkEventKind::Done, &occupied_id, now));
            works
                .work_items
                .iter_mut()
                .find(|work| work.id == occupied_id)
                .unwrap()
                .owner = None;
        }
        let mut projection = WorkspaceProjection::default_for_project(project);
        let events = super::super::workspace::apply_workspace_launch_for_current_work(
            project,
            &mut projection,
            &works,
            &session,
            super::super::workspace::WorkspaceLaunchTransition {
                work_id: None,
                base_branch: None,
                linked_issue_number: Some(42),
                canonical_owner: Some(gwt::cli::execution_state::ExecutionOwnerKey {
                    kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                    number: 42,
                }),
                resume_context: None,
                kind: WorkspaceLaunchProjectionKind::StartWork,
                live_session_ids: &live,
                now,
            },
            true,
        )
        .unwrap_or_else(|error| panic!("{case}: {error}"));
        if case == "terminal-missing-parent" {
            let audit = events
                .iter()
                .find(|event| event.is_successor_supersession())
                .unwrap();
            assert!(
                audit.updated_at > now,
                "audit follows the exact terminal boundary"
            );
            assert!(events.last().unwrap().updated_at >= audit.updated_at);
        }
        for event in events {
            works.apply_event(event);
        }
        assert_eq!(projection.id, expected_id, "{case}: create a new lifetime");
        let occupied = works
            .work_items
            .iter()
            .find(|work| work.id == occupied_id)
            .unwrap();
        assert!(
            occupied.is_terminal(),
            "{case}: audit-close invalid occupancy"
        );
        assert!(
            occupied.events.contains(&start),
            "{case}: preserve original event"
        );
        assert_eq!(
            works
                .work_items
                .iter()
                .find(|work| work.id == predecessor_id)
                .unwrap(),
            &predecessor,
            "{case}: preserve authoritative predecessor"
        );
        assert_eq!(
            current_work_id(&works, project, Some(&session.branch_name), None),
            Some(expected_id.clone()),
            "{case}: readers follow the audited chain"
        );
    }
}

#[test]
fn successor_placeholder_cannot_take_over_related_foreign_or_live_work() {
    use gwt_core::workspace_projection::{
        canonical_work_id, successor_work_id, WorkEvent, WorkEventKind, WorkItemsProjection,
        WorkspaceExecutionContainerRef, WorkspaceProjection,
    };
    let project = Path::new("/fixture-project");
    let session = ActiveAgentSession {
        session_id: "fresh-session".to_string(),
        branch_name: "work/issue-42".to_string(),
        worktree_path: project.join("checkout"),
        ..sample_active_agent_session("tab", "window")
    };
    let now = Utc::now();
    let predecessor_id = canonical_work_id(project, Some(&session.branch_name), None).unwrap();
    let successor_id = successor_work_id(&predecessor_id);
    let mut start = WorkEvent::new(WorkEventKind::Start, &predecessor_id, now);
    start.owner = Some("Issue #42".to_string());
    start.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some(session.branch_name.clone()),
        worktree_path: Some(session.worktree_path.clone()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    for invalid in [
        "owner",
        "related",
        "live",
        "foreign-history",
        "active",
        "terminal",
    ] {
        let mut works = WorkItemsProjection::empty(now);
        works.apply_event(start.clone());
        works.apply_event(WorkEvent::new(WorkEventKind::Done, &predecessor_id, now));
        let mut placeholder = start.clone();
        placeholder.id = format!("placeholder-{invalid}");
        placeholder.kind = WorkEventKind::Update;
        placeholder.work_item_id = successor_id.clone();
        if invalid == "owner" {
            placeholder.owner = Some("Issue #43".to_string());
        }
        if invalid == "related" {
            placeholder.related_work_item_id = Some("another-work".to_string());
        }
        works.apply_event(placeholder);
        let mut pause = WorkEvent::new(WorkEventKind::Pause, &successor_id, now);
        pause.agent_session_id = Some("stranded-session".to_string());
        works.apply_event(pause);
        if invalid == "foreign-history" {
            works
                .work_items
                .iter_mut()
                .find(|work| work.id == successor_id)
                .unwrap()
                .events[0]
                .execution_container
                .as_mut()
                .unwrap()
                .branch = Some("work/issue-43".to_string());
        }
        if matches!(invalid, "active" | "live" | "terminal") {
            works
                .work_items
                .iter_mut()
                .find(|work| work.id == successor_id)
                .unwrap()
                .status_category = if invalid != "terminal" {
                gwt_core::workspace_projection::WorkspaceStatusCategory::Active
            } else {
                gwt_core::workspace_projection::WorkspaceStatusCategory::Done
            };
        }
        let live = if invalid == "live" {
            HashSet::from(["stranded-session".to_string()])
        } else {
            HashSet::new()
        };
        let mut projection = WorkspaceProjection::default_for_project(project);
        let before = projection.clone();
        let authenticated = !matches!(invalid, "active" | "terminal");
        let result = super::super::workspace::apply_workspace_launch_for_current_work(
            project,
            &mut projection,
            &works,
            &session,
            super::super::workspace::WorkspaceLaunchTransition {
                work_id: None,
                base_branch: None,
                linked_issue_number: Some(42),
                canonical_owner: authenticated.then_some(
                    gwt::cli::execution_state::ExecutionOwnerKey {
                        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                        number: 42,
                    },
                ),
                resume_context: None,
                kind: WorkspaceLaunchProjectionKind::StartWork,
                live_session_ids: &live,
                now,
            },
            authenticated,
        );
        assert!(result.is_err(), "{invalid}");
        assert_eq!(projection, before, "{invalid}: refuse before mutation");
    }
}

/// SPEC #3165 T-226 / FR-102: provider ownership is a fail-closed preflight;
/// only a present rollout owned by the selected worktree may exact Resume.
fn assert_monitor_resume_if_safe_provider_conversation(
    case_name: &str,
    availability: MonitorProviderConversationFixture,
    exact_resume_expected: bool,
) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        case_name,
        availability,
        MonitorNativeHolderFixture::None,
        false,
    );
    let previous_spawner = fixture.runtime.blocking_tasks.clone();
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    let project_root = fixture.runtime.test_context().project_root;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &project_root,
            3165,
            LinkedIssueKind::Spec,
            None,
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
    drain_queued_blocking_tasks(&queued);
    fixture.runtime.blocking_tasks = previous_spawner;
    let prepared = take_issue4803_monitor_preparation(&fixture.recorded_events);
    let events = fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared);
    // Provider ownership is determined by the completed launch result, not
    // by how quickly a loaded runner returns from the preparation handler.
    let result = take_monitor_launch_complete(case_name, &fixture.recorded_events);
    if exact_resume_expected {
        assert_monitor_exact_resume(result, &fixture);
    } else {
        assert_monitor_fresh_successor(result, &fixture);
    }
    let fresh_reason = events.iter().find_map(|event| match &event.event {
        BackendEvent::IssueMonitorToast { message, .. }
            if message.starts_with("Issue Monitor started a fresh session because ") =>
        {
            Some(message.as_str())
        }
        _ => None,
    });
    let expected = match availability {
        MonitorProviderConversationFixture::Present => None,
        MonitorProviderConversationFixture::Missing => Some("native conversation is missing"),
        MonitorProviderConversationFixture::Foreign
        | MonitorProviderConversationFixture::Corrupt => {
            Some("native conversation ownership does not match this worktree")
        }
        MonitorProviderConversationFixture::Unknown => {
            Some("native conversation could not be verified")
        }
    };
    if let Some(expected) = expected {
        let reason = fresh_reason.expect("fresh fallback must report its resume failure reason");
        assert!(reason.contains(expected), "{case_name}: {reason}");
        assert!(!reason.contains("held by window"), "{case_name}: {reason}");
    } else {
        assert!(fresh_reason.is_none());
    }
}

#[test]
fn app_runtime_monitor_resume_if_safe_resumes_only_present_provider_conversation_present() {
    assert_monitor_resume_if_safe_provider_conversation(
        "present",
        MonitorProviderConversationFixture::Present,
        true,
    );
}

#[test]
fn app_runtime_monitor_resume_if_safe_resumes_only_present_provider_conversation_missing() {
    assert_monitor_resume_if_safe_provider_conversation(
        "missing",
        MonitorProviderConversationFixture::Missing,
        false,
    );
}

#[test]
fn app_runtime_monitor_resume_if_safe_resumes_only_present_provider_conversation_foreign() {
    assert_monitor_resume_if_safe_provider_conversation(
        "foreign",
        MonitorProviderConversationFixture::Foreign,
        false,
    );
}

#[test]
fn app_runtime_monitor_resume_if_safe_resumes_only_present_provider_conversation_unknown() {
    assert_monitor_resume_if_safe_provider_conversation(
        "unknown",
        MonitorProviderConversationFixture::Unknown,
        false,
    );
}

#[test]
fn app_runtime_monitor_resume_if_safe_resumes_only_present_provider_conversation_corrupt() {
    assert_monitor_resume_if_safe_provider_conversation(
        "corrupt",
        MonitorProviderConversationFixture::Corrupt,
        false,
    );
}

/// SPEC #3165 T-226 / FR-102: writer exclusion keys on the exact native
/// conversation, not branch equality, and preserves a known holder window id.
///
/// Issue #4802 AC-2 narrows the live-holder rows: a holder that is a live
/// agent pane in the Issue's worktree is no longer answered with a second
/// pane (fresh successor or exact resume). The launch is refused and
/// acknowledged onto that pane, still naming the holder window id. Only the
/// materializing holder, which has no live pane yet, still gets a successor.
#[test]
fn app_runtime_monitor_resume_if_safe_uses_exact_native_writer_identity() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    for (case_name, holder, live_pane_refuses) in [
        (
            "active-same-native",
            MonitorNativeHolderFixture::ActiveSameConversation,
            true,
        ),
        (
            "materializing-same-native",
            MonitorNativeHolderFixture::MaterializingSameConversation,
            false,
        ),
        (
            "active-other-native",
            MonitorNativeHolderFixture::ActiveOtherConversation,
            true,
        ),
    ] {
        let mut fixture = monitor_relaunch_fixture(
            temp.path(),
            case_name,
            MonitorProviderConversationFixture::Present,
            holder,
            false,
        );
        let events = fixture.runtime.auto_launch_issue_monitor_delivery_events(
            &fixture.runtime.test_context(),
            3165,
            LinkedIssueKind::Spec,
            None,
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        let holder_window_id = fixture
            .holder_window_id
            .as_deref()
            .expect("same-native holder window id");
        match holder {
            MonitorNativeHolderFixture::ActiveSameConversation => assert_eq!(
                fixture.runtime.window_status(holder_window_id),
                Some(WindowProcessStatus::Running),
                "the active holder case must represent a live Running window",
            ),
            // No hook state: the other-conversation holder sits at its
            // prompt, which is still a live pane for Issue #4802 AC-2.
            MonitorNativeHolderFixture::ActiveOtherConversation => assert_eq!(
                fixture.runtime.window_status(holder_window_id),
                Some(WindowProcessStatus::Idle),
                "the other-conversation holder must represent a live Idle window",
            ),
            MonitorNativeHolderFixture::MaterializingSameConversation => {
                assert!(fixture.runtime.window_lookup.contains_key(holder_window_id));
                assert!(fixture
                    .runtime
                    .inflight_launches
                    .values()
                    .any(|(window_id, _)| window_id == holder_window_id));
            }
            _ => unreachable!("same-native live holder matrix"),
        }
        assert!(
            events.iter().any(|event| matches!(
                &event.event,
                BackendEvent::IssueMonitorToast { message, .. }
                    if message.contains(holder_window_id)
            )),
            "known holder diagnostic must include {holder_window_id}",
        );
        if live_pane_refuses {
            // Issue #4802 AC-2: the live pane in the Issue worktree is the
            // launch; no successor window is opened and nothing is spawned.
            assert!(
                events.iter().any(|event| matches!(
                    &event.event,
                    BackendEvent::IssueMonitorToast { message, .. }
                        if message.contains("did not open a second pane")
                )),
                "{case_name}: the refusal is reported",
            );
            assert_eq!(
                fixture.runtime.tabs[0]
                    .workspace
                    .persisted()
                    .windows
                    .iter()
                    .filter(|window| window.preset == WindowPreset::Agent)
                    .count(),
                1,
                "{case_name}: the live holder stays the only agent pane",
            );
            assert!(
                !fixture
                    .recorded_events
                    .lock()
                    .expect("event log")
                    .iter()
                    .any(|event| matches!(
                        recorded_project_payload(event),
                        UserEvent::LaunchComplete { .. }
                    )),
                "{case_name}: a refused launch completes nothing",
            );
            continue;
        }
        let result = take_monitor_launch_complete(case_name, &fixture.recorded_events);
        assert_monitor_fresh_successor(result, &fixture);
    }
}

/// Issue #3716 AC-2: when the exact native conversation is still held by its
/// live pane, a canonical Monitor answer continues that holder instead of
/// spawning a fresh successor that cannot receive the parked decision.
#[test]
fn app_runtime_answered_handoff_continues_the_exact_live_holder() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);
    let grok_home = temp.path().join(".grok");
    let _grok_home = ScopedEnvVar::set("GROK_HOME", temp.path().join("wrong-grok-home"));
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "answered-live-holder",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::ActiveSameConversation,
        true,
    );
    convert_monitor_relaunch_fixture_to_grok(&mut fixture, &grok_home);
    pin_monitor_fixture_agents(
        &fixture,
        temp.path(),
        &[("GROK_HOME", grok_home.to_str().expect("UTF-8 Grok home"))],
    );
    seed_resumed_autonomous_handoff(&fixture, "Approved by PM");
    // Own static claim inputs so this fixture reaches the real PTY submission.
    let _projection =
        super::super::ScopedLocalIssueMonitorFallbackProjection::set(&fixture.project_root);
    let holder_window_id = fixture
        .holder_window_id
        .clone()
        .expect("live holder window id");
    insert_test_pane_runtime(&mut fixture.runtime, &holder_window_id);
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = blocking_tasks;
    let delivery_id = fixture.delivery_id.clone().expect("durable delivery id");

    let events = fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some(delivery_id.clone()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("load delivered handoff");
    assert!(
        prefs.autonomous_handoffs[0].delivered_at.is_none(),
        "scheduling a live delivery must not consume the answer before physical submit",
    );
    assert!(
        events.is_empty(),
        "completion is emitted by the worker callback: {events:#?}"
    );
    run_queued_monitor_gui_diagnostics(&queued_tasks, &[1]);
    assert_eq!(queued_tasks.lock().expect("queued tasks").len(), 1);
    assert!(matches!(
        fixture.runtime.issue_monitor_launch_deliveries.get(&delivery_id),
        Some(super::super::IssueMonitorLaunchDeliveryState::Materializing { window_id, .. })
            if window_id == &holder_window_id
    ));
    let claimed = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("load rebound live delivery");
    assert!(matches!(
        claimed.autonomous_handoffs[0].delivery,
        gwt::autonomous_handoff::AutonomousHandoffDeliveryState::Attempting { attempt: 1, .. }
    ));
    let delivery = claimed
        .pending_launch_deliveries
        .iter()
        .find(|delivery| delivery.delivery_id == delivery_id)
        .expect("pending live delivery");
    assert_eq!(
        delivery.materializer_window_id.as_deref(),
        Some(holder_window_id.as_str()),
        "the durable claim must be rebound from the proposed window to the exact holder",
    );
    assert_eq!(delivery.materialized_window_id, None);
    assert_eq!(delivery.workspace_durable_window_id, None);

    let task = queued_tasks
        .lock()
        .expect("queued tasks")
        .pop()
        .expect("answer delivery worker");
    task();
    let completion = {
        let mut recorded = fixture.recorded_events.lock().expect("recorded events");
        let index = recorded
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::IssueMonitorAnswerDeliveryComplete(_)
                )
            })
            .expect("physical answer delivery completion");
        recorded.remove(index)
    };
    let UserEvent::IssueMonitorAnswerDeliveryComplete(delivery) = completion else {
        unreachable!("matched answer delivery completion")
    };
    assert!(
        delivery.result.is_ok(),
        "physical submit must succeed: {:?}",
        delivery.result
    );
    let awaiting_callback = gwt::load_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root),
    )
    .expect("load delivery awaiting physical callback");
    assert!(awaiting_callback
        .pending_launch_deliveries
        .iter()
        .any(|delivery| delivery.delivery_id == delivery_id));
    let handoff = &awaiting_callback.autonomous_handoffs[0];
    let target = match &handoff.delivery {
        gwt::autonomous_handoff::AutonomousHandoffDeliveryState::Attempting {
            target: Some(target),
            ..
        } => target,
        other => panic!("expected bound answer target, got {other:?}"),
    };
    let receipt_identity = gwt::autonomous_handoff::AutonomousHandoffReceiptIdentity {
        gwt_session_id: target.gwt_session_id.clone(),
        native_session_id: target.native_session_id.clone(),
        provider: target.provider.clone(),
        issue_number: target.issue_number,
        repo_hash: target.repo_hash.clone(),
        project_state_root: target.project_state_root.clone(),
    };
    let body = gwt::issue_monitor::autonomous_handoff_answer_prompt(handoff);
    let receipt_prompt = gwt::autonomous_handoff::protected_autonomous_handoff_answer_prompt(
        &body,
        &handoff.handoff_id,
        &handoff.session_id,
        1,
    )
    .expect("protected receipt prompt");
    assert!(
        gwt::acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
            &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root),
            &handoff.session_id,
            &receipt_identity,
            &receipt_prompt,
            "2026-08-20T00:03:00Z",
        )
        .expect("acknowledge provider receipt before physical callback")
    );
    let receipted = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("load provider receipt");
    assert!(receipted.autonomous_handoffs[0].delivered_at.is_some());
    assert!(receipted
        .pending_launch_deliveries
        .iter()
        .any(|delivery| delivery.delivery_id == delivery_id));
    let restored = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), receipted.clone());
    assert_eq!(
        restored.launched_window_issue(&holder_window_id),
        None,
        "the semantic receipt must not publish an undurable window before the physical callback"
    );

    let mut completion_events = fixture
        .runtime
        .handle_issue_monitor_answer_delivery_complete(delivery);
    completion_events.extend(fixture.runtime.finish_queued_delivery_acks(&queued_tasks));
    let settled = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("load settled live delivery");
    assert!(
        settled.autonomous_handoffs[0].delivered_at.is_some(),
        "the later physical callback must not rewind the provider receipt",
    );
    assert!(settled
        .pending_launch_deliveries
        .iter()
        .all(|delivery| delivery.delivery_id != delivery_id));
    let monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), settled.clone());
    assert_eq!(monitor.launched_window_issue(&holder_window_id), Some(3165));
    assert!(!fixture
        .runtime
        .issue_monitor_launch_deliveries
        .contains_key(&delivery_id));
    assert!(completion_events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { message, .. }
            if message == "Issue Monitor submitted the human answer; waiting for the provider receipt"
    )));
    assert!(fixture
        .recorded_events
        .lock()
        .expect("recorded events")
        .iter()
        .all(|event| !matches!(
            recorded_project_payload(event),
            UserEvent::LaunchComplete { .. }
        )));
}

/// Issue #3716 AC-2: once a worker may have written the prompt body, an error
/// is outcome-ambiguous and must park instead of automatically replaying it.
#[test]
fn app_runtime_failed_live_handoff_delivery_keeps_the_answer_pending() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let grok_home = temp.path().join(".grok");
    let _grok_home = ScopedEnvVar::set("GROK_HOME", &grok_home);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "failed-live-answer-delivery",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::ActiveSameConversation,
        true,
    );
    convert_monitor_relaunch_fixture_to_grok(&mut fixture, &grok_home);
    seed_resumed_autonomous_handoff(&fixture, "Retry this answer");
    // Own static claim inputs so this fixture reaches the ambiguous PTY write.
    let _projection =
        super::super::ScopedLocalIssueMonitorFallbackProjection::set(&fixture.project_root);
    let holder_window_id = fixture
        .holder_window_id
        .clone()
        .expect("live holder window id");
    insert_test_pane_runtime(&mut fixture.runtime, &holder_window_id);
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = blocking_tasks;
    let delivery_id = fixture.delivery_id.clone().expect("durable delivery id");

    assert!(fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events(
            &fixture.runtime.test_context(),
            3165,
            LinkedIssueKind::Spec,
            Some(delivery_id.clone()),
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        )
        .is_empty());
    fixture
        .runtime
        .runtimes
        .get(&holder_window_id)
        .expect("holder runtime")
        .pane
        .lock()
        .expect("holder pane")
        .shared_pty()
        .invalidate_input_generation();
    run_queued_monitor_gui_diagnostics(&queued_tasks, &[1]);
    queued_tasks
        .lock()
        .expect("queued tasks")
        .pop()
        .expect("answer delivery worker")();
    let completion = {
        let mut recorded = fixture.recorded_events.lock().expect("recorded events");
        let index = recorded
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::IssueMonitorAnswerDeliveryComplete(_)
                )
            })
            .expect("failed answer delivery completion");
        recorded.remove(index)
    };
    let UserEvent::IssueMonitorAnswerDeliveryComplete(delivery) = completion else {
        unreachable!("matched answer delivery completion")
    };
    assert!(
        delivery.result.is_err(),
        "invalidated PTY must reject the delivery"
    );
    let events = fixture
        .runtime
        .handle_issue_monitor_answer_delivery_complete(delivery);

    let parked = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("load ambiguous delivery");
    assert!(parked.autonomous_handoffs[0].delivered_at.is_none());
    assert_eq!(
        parked.autonomous_handoffs[0].state,
        gwt::autonomous_handoff::AutonomousHandoffState::AwaitingHuman,
    );
    assert!(matches!(
        parked.autonomous_handoffs[0].delivery,
        gwt::autonomous_handoff::AutonomousHandoffDeliveryState::Ambiguous { .. }
    ));
    assert!(parked
        .pending_launch_deliveries
        .iter()
        .all(|delivery| delivery.delivery_id != delivery_id));
    assert!(!fixture
        .runtime
        .issue_monitor_launch_deliveries
        .contains_key(&delivery_id));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, .. } if level == "error"
    )));

    let (retry_spawner, retry_tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = retry_spawner;
    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some(delivery_id.clone()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    run_queued_monitor_gui_diagnostics(&retry_tasks, &[0]);
    assert!(
        retry_tasks.lock().expect("retry tasks").is_empty(),
        "an ambiguous submit must never enqueue a second physical write",
    );
}

/// Issue #3716 AC-2: a materializing writer reservation is not yet a live
/// conversation holder and must never receive the human answer.
#[test]
fn app_runtime_answered_handoff_waits_for_a_materializing_holder() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "answered-materializing-holder",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::MaterializingSameConversation,
        false,
    );
    seed_resumed_autonomous_handoff(&fixture, "Wait for the live holder");
    let holder_window_id = fixture
        .holder_window_id
        .clone()
        .expect("materializing holder id");
    insert_test_pane_runtime(&mut fixture.runtime, &holder_window_id);
    let (blocking_tasks, queued_tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = blocking_tasks;

    assert!(fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events(
            &fixture.runtime.test_context(),
            3165,
            LinkedIssueKind::Spec,
            None,
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        )
        .is_empty());
    run_queued_monitor_gui_diagnostics(&queued_tasks, &[0]);
    assert!(queued_tasks.lock().expect("queued tasks").is_empty());
    let pending = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("load materializing handoff");
    assert!(pending.autonomous_handoffs[0].delivered_at.is_none());
    assert!(fixture
        .recorded_events
        .lock()
        .expect("recorded events")
        .iter()
        .all(|event| !matches!(
            recorded_project_payload(event),
            UserEvent::LaunchComplete { .. }
        )));
}

/// Issue #3716 AC-2: a queued answer belongs to the gwt Session recorded by
/// the handoff, even when a newer resumable Session exists on the same branch.
#[test]
fn app_runtime_answered_handoff_ignores_a_newer_branch_session() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "answered-exact-asking-session",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    seed_resumed_autonomous_handoff(&fixture, "Approved for the asking session");
    let source = gwt_agent::Session::load(
        &fixture
            .sessions_dir
            .join(format!("{}.toml", fixture.source_session_id)),
    )
    .expect("load asking Session");
    let mut newer = source.clone();
    newer.id = "session-newer-same-branch".to_string();
    newer.agent_session_id = Some("native-newer-same-branch".to_string());
    newer.updated_at += chrono::Duration::hours(1);
    newer.last_activity_at += chrono::Duration::hours(1);
    newer
        .save(&fixture.sessions_dir)
        .expect("save newer Session");
    let codex_home = PathBuf::from(std::env::var_os("CODEX_HOME").expect("isolated CODEX_HOME"));
    let rollout_dir = codex_home.join("sessions/2026/08/20");
    fs::create_dir_all(&rollout_dir).expect("create newer rollout directory");
    fs::write(
        rollout_dir.join("rollout-native-newer-same-branch.jsonl"),
        format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"native-newer-same-branch\",\"cwd\":{}}}}}\n",
            serde_json::to_string(&fixture.worktree.display().to_string()).expect("serialize cwd"),
        ),
    )
    .expect("write newer rollout");
    fixture
        .runtime
        .apply_refreshed_launch_wizard_sessions(vec![source, newer]);

    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        None,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let result =
        take_monitor_launch_complete("answered exact asking session", &fixture.recorded_events);
    assert_monitor_exact_resume(result, &fixture);
}

/// Issue #3478 AC-1/AC-5 and Issue #3716 AC-2: a stopped holder still takes
/// the exact native Resume path, gets the answer once, and retains autonomous
/// markers so a later question is intercepted again.
#[test]
fn app_runtime_answered_handoff_exact_resume_retains_autonomous_context() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);
    let grok_home = temp.path().join(".grok");
    let _grok_home = ScopedEnvVar::set("GROK_HOME", &grok_home);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "answered-exact-resume",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    convert_monitor_relaunch_fixture_to_grok(&mut fixture, &grok_home);
    seed_resumed_autonomous_handoff(&fixture, "Approved by PM");

    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        None,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let (window_id, mut result) =
        take_monitor_launch_complete_event("answered-exact-resume", &fixture.recorded_events);

    let prepared = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("load prepared exact Resume handoff");
    assert!(
        prepared.autonomous_handoffs[0].delivered_at.is_none(),
        "preparing exact Resume must not consume the answer before launch completion",
    );
    assert_eq!(
        result.as_ref().expect("exact Resume result").5,
        gwt_agent::AgentId::GrokBuild,
    );
    assert_monitor_exact_resume(result.clone(), &fixture);
    let process = &mut result.as_mut().expect("exact Resume result").0;
    let provider_args = std::mem::take(&mut process.args);
    assert!(provider_args
        .iter()
        .any(|argument| argument.contains("[gwt-autonomous-answer:v1:")));
    if cfg!(windows) {
        process.command = "cmd".to_string();
        process.args = vec![
            "/d".to_string(),
            "/s".to_string(),
            "/c".to_string(),
            "ping -n 30 127.0.0.1 > nul".to_string(),
        ];
        process.args.extend(provider_args);
    } else {
        process.command = "/bin/sh".to_string();
        process.args = vec![
            "-lc".to_string(),
            "sleep 30".to_string(),
            "grok-test".to_string(),
        ];
        process.args.extend(provider_args);
    }
    fixture
        .runtime
        .handle_launch_complete_and_drain(window_id, result);
    let awaiting_receipt = gwt::load_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root),
    )
    .expect("load completed exact Resume handoff");
    assert!(
        awaiting_receipt.autonomous_handoffs[0]
            .delivered_at
            .is_none(),
        "child spawn success cannot stand in for the provider's UserPromptSubmit receipt",
    );
    assert!(matches!(
        awaiting_receipt.autonomous_handoffs[0].delivery,
        gwt::autonomous_handoff::AutonomousHandoffDeliveryState::Attempting { .. }
    ));
}

/// SPEC-1921 L1a: a saved version cannot select a package runner on exact Resume.
/// An installed fixture CLI succeeds without a package-runner sandbox pin.
#[test]
fn monitor_legacy_version_resumes_installed_without_pinned_package_runners() {
    // Never published, so no host package cache can answer for it.
    const EXACT_PACKAGE_VERSION: &str = "0.0.0";
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _sandbox = ScopedEnvVar::unset(gwt_core::process_console::RUNNER_PROBE_SANDBOX_MARKER);
    let _live = ScopedEnvVar::unset(gwt_core::process_console::ALLOW_REAL_RUNNER_PROBE_MARKER);
    let installed_bin = write_fixture_runners(temp.path(), &["codex"]);
    let installed_cli = installed_bin.join(if cfg!(windows) { "codex.cmd" } else { "codex" });
    let _path = prepend_tool_parent_to_path(&installed_cli);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "unpinned-package-runner",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    write_profile_config(
        fixture
            .runtime
            .profile_config_path
            .as_deref()
            .expect("fixture profile config path"),
        &Settings::default(),
    );
    let mut source = gwt_agent::Session::load(
        &fixture
            .sessions_dir
            .join(format!("{}.toml", fixture.source_session_id)),
    )
    .expect("load monitored Session");
    source.tool_version = Some(EXACT_PACKAGE_VERSION.to_string());
    source
        .save(&fixture.sessions_dir)
        .expect("save exact-version Session");
    // The resume candidate is read from the launch wizard cache, not from disk.
    fixture.runtime.launch_wizard_cache.record_session(source);

    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        None,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let (_window_id, result) =
        take_monitor_launch_complete_event("unpinned-package-runner", &fixture.recorded_events);

    let prepared = result.expect("the saved version must not select a package runner");
    let installed_cli = installed_cli.to_string_lossy();
    assert!(
        prepared.0.command == installed_cli
            || prepared
                .0
                .args
                .iter()
                .any(|arg| arg.contains(installed_cli.as_ref())),
        "exact Resume must use the installed fixture CLI"
    );
    assert!(
        !prepared
            .0
            .args
            .iter()
            .any(|arg| arg.contains("@openai/codex@")),
        "the ignored saved version must not reach the package runner"
    );
    assert_monitor_exact_resume(Ok(prepared), &fixture);
}

/// Issue #3716 AC-2: an exact Resume preparation failure occurs before the
/// provider spawn boundary, so it keeps the bounded same-session retry ladder.
#[test]
fn app_runtime_pre_spawn_exact_handoff_failure_uses_bounded_retry() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let grok_home = temp.path().join(".grok");
    let _grok_home = ScopedEnvVar::set("GROK_HOME", &grok_home);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "failed-exact-answer-resume",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    convert_monitor_relaunch_fixture_to_grok(&mut fixture, &grok_home);
    seed_resumed_autonomous_handoff(&fixture, "Retry exact Resume");
    let delivery_id = fixture.delivery_id.clone().expect("delivery id");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let mut stale = gwt::load_issue_monitor_prefs(&prefs_path).expect("load stale strategy");
    stale.pending_launch_deliveries[0].launch_session_strategy =
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired;
    gwt::save_issue_monitor_prefs(&prefs_path, &stale).expect("save stale strategy");

    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some(delivery_id.clone()),
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
    );
    let (window_id, mut result) =
        take_monitor_launch_complete_event("failed exact answer Resume", &fixture.recorded_events);
    let process = &mut result.as_mut().expect("prepared exact Resume").0;
    process.command = "/definitely/missing/gwt-answered-resume".to_string();
    process.cwd = Some(fixture.worktree.clone());
    fixture
        .runtime
        .handle_launch_complete_and_drain(window_id.clone(), result);

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("load failed exact Resume state");
    assert!(prefs.autonomous_handoffs[0].delivered_at.is_none());
    assert_eq!(
        prefs.autonomous_handoffs[0].state,
        gwt::autonomous_handoff::AutonomousHandoffState::Resumed,
    );
    assert!(
        matches!(
            prefs.autonomous_handoffs[0].delivery,
            gwt::autonomous_handoff::AutonomousHandoffDeliveryState::RetryBackoff {
                attempt: 1,
                ..
            }
        ),
        "unexpected delivery state: {:?}",
        prefs.autonomous_handoffs[0].delivery
    );
    assert!(prefs
        .pending_launch_deliveries
        .iter()
        .all(|delivery| delivery.delivery_id != delivery_id));
    assert_eq!(
        prefs.queued_launch_session_strategies.get(&3165),
        Some(&gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe),
    );
    assert!(
        !fixture.runtime.tracked_window_exists(&window_id),
        "a definitely pre-submit Monitor failure must not leave an Error pane"
    );
    assert!(fixture.runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .is_empty());
}

/// Issue #3716 AC-2: an incomplete provider receipt is a retryable handoff
/// preflight failure, not authority to launch a fresh successor.
#[test]
fn app_runtime_incomplete_grok_handoff_store_stays_retryable() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let grok_home = temp.path().join(".grok");
    let _grok_home = ScopedEnvVar::set("GROK_HOME", &grok_home);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "incomplete-grok-answer-store",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    convert_monitor_relaunch_fixture_to_grok(&mut fixture, &grok_home);
    seed_resumed_autonomous_handoff(&fixture, "Wait for provider receipt");
    let updates = grok_home
        .join("sessions/%2Ffixture%2Fworktree")
        .join(&fixture.native_conversation_id)
        .join("updates.jsonl");
    fs::remove_file(updates).expect("remove authoritative updates log");
    let delivery_id = fixture.delivery_id.clone().expect("delivery id");

    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some(delivery_id.clone()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("load incomplete store state");
    assert!(prefs.autonomous_handoffs[0].delivered_at.is_none());
    assert!(matches!(
        prefs.autonomous_handoffs[0].delivery,
        gwt::autonomous_handoff::AutonomousHandoffDeliveryState::RetryBackoff { .. }
    ));
    assert!(prefs
        .pending_launch_deliveries
        .iter()
        .all(|delivery| delivery.delivery_id != delivery_id));
    assert_eq!(
        prefs.queued_launch_session_strategies.get(&3165),
        Some(&gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe),
    );
    assert!(prefs
        .failed_issues
        .iter()
        .all(|failed| failed.issue_number != 3165));
    assert!(fixture
        .recorded_events
        .lock()
        .expect("recorded events")
        .iter()
        .all(|event| !matches!(
            recorded_project_payload(event),
            UserEvent::LaunchComplete { .. }
        )));
}

/// SPEC #3165 T-228 / FR-104: if another live gwt window wins the native
/// conversation after preflight, the typed late-race failure retains that
/// known holder instead of degrading every production payload to `None`.
#[test]
fn app_runtime_late_writer_conflict_preserves_known_holder_window_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "late-known-holder",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::ActiveSameConversation,
        false,
    );
    let source_raw_id = fixture.runtime.tabs[0]
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds())
        .id;
    assert!(fixture.runtime.tabs[0]
        .workspace
        .set_session_id(&source_raw_id, Some(fixture.source_session_id.clone())));
    fixture.runtime.register_window("tab-1", &source_raw_id);
    let source_window_id = combined_window_id("tab-1", &source_raw_id);
    fixture.runtime.active_agent_sessions.insert(
        source_window_id.clone(),
        ActiveAgentSession {
            window_id: source_window_id.clone(),
            session_id: fixture.source_session_id.clone(),
            agent_id: "codex".to_string(),
            branch_name: "work/issue-3165".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: fixture.worktree.clone(),
            agent_project_root: fixture.worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        },
    );
    let detail = "Error: Failed to resume session from ~/.codex/sessions/rollout.jsonl: \
        thread/resume failed during TUI bootstrap: thread 019 already has an active writer \
        (code -32600)";

    let failure = fixture.runtime.issue_monitor_failure_for_window(
        &source_window_id,
        detail,
        gwt_agent::SessionMode::Resume,
    );
    assert_eq!(
        failure,
        Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
            holder_window_id: fixture.holder_window_id.clone(),
        })
    );
    let payload = AppRuntime::issue_monitor_agent_failed_payload_with_failure(
        &source_window_id,
        detail,
        Some(3165),
        failure.as_ref(),
    );
    assert_eq!(
        payload
            .pointer("/agent_failed/failure/holder_window_id")
            .and_then(serde_json::Value::as_str),
        fixture.holder_window_id.as_deref(),
        "the production AgentFailed envelope must retain the resolved holder"
    );
    assert_eq!(
        payload
            .pointer("/agent_failed/failure/kind")
            .and_then(serde_json::Value::as_str),
        Some("resume_writer_conflict")
    );

    fixture
        .runtime
        .active_agent_sessions
        .remove(&source_window_id);
    assert_eq!(
        fixture.runtime.issue_monitor_failure_for_window(
            &source_window_id,
            detail,
            gwt_agent::SessionMode::Resume,
        ),
        Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
            holder_window_id: fixture.holder_window_id.clone(),
        }),
        "PTY teardown removes the active source before failure publication, so the persisted window Session must retain holder resolution"
    );

    let holder_window_id = fixture
        .holder_window_id
        .as_deref()
        .expect("known holder window");
    fixture
        .runtime
        .active_agent_sessions
        .remove(holder_window_id);
    assert_eq!(
        fixture.runtime.issue_monitor_failure_for_window(
            &source_window_id,
            detail,
            gwt_agent::SessionMode::Resume,
        ),
        Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
            holder_window_id: None,
        }),
        "the failing source window must never identify itself as the holder"
    );
}

#[test]
fn app_runtime_monitor_holder_resolution_prefers_durable_session_over_stale_cache() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);

    let fixture = monitor_relaunch_fixture(
        temp.path(),
        "durable-holder-refresh",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::ActiveOtherConversation,
        false,
    );
    let holder_window_id = fixture.holder_window_id.as_deref().expect("holder window");
    let holder_session_id = fixture
        .runtime
        .active_agent_sessions
        .get(holder_window_id)
        .expect("active holder")
        .session_id
        .clone();
    let holder_path = fixture
        .sessions_dir
        .join(format!("{holder_session_id}.toml"));
    let mut durable_holder = gwt_agent::Session::load(&holder_path).expect("durable holder");
    assert_ne!(
        durable_holder.exact_resume_session_id(),
        Some(fixture.native_conversation_id.as_str()),
        "fixture cache and disk initially describe another conversation"
    );
    durable_holder.agent_session_id = Some(fixture.native_conversation_id.clone());
    durable_holder
        .save(&fixture.sessions_dir)
        .expect("refresh durable holder");
    assert_ne!(
        fixture
            .runtime
            .launch_wizard_cache
            .session_by_id(&holder_session_id)
            .and_then(gwt_agent::Session::exact_resume_session_id),
        Some(fixture.native_conversation_id.as_str()),
        "the in-memory launch cache intentionally remains stale"
    );
    let candidate = gwt_agent::Session::load(
        &fixture
            .sessions_dir
            .join(format!("{}.toml", fixture.source_session_id)),
    )
    .expect("resume candidate");

    assert_eq!(
        fixture
            .runtime
            .issue_monitor_native_conversation_holder_excluding(&candidate, None),
        Some(holder_window_id.to_string()),
        "holder safety must use the latest durable Session written by hooks or another process"
    );
}

/// Exact native identity alone is not a writer conflict: stale runtime maps
/// must not force a fresh successor after their window has stopped or vanished.
#[test]
fn app_runtime_monitor_resume_if_safe_ignores_non_live_active_holder_entries() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    for (case_name, holder, expected_status) in [
        (
            "active-same-native-missing-window",
            MonitorNativeHolderFixture::ActiveSameConversationMissingWindow,
            None,
        ),
        (
            "active-same-native-stopped",
            MonitorNativeHolderFixture::ActiveSameConversationStopped,
            Some(WindowProcessStatus::Stopped),
        ),
        (
            "active-same-native-error",
            MonitorNativeHolderFixture::ActiveSameConversationError,
            Some(WindowProcessStatus::Error),
        ),
    ] {
        let mut fixture = monitor_relaunch_fixture(
            temp.path(),
            case_name,
            MonitorProviderConversationFixture::Present,
            holder,
            false,
        );
        let holder_window_id = fixture
            .holder_window_id
            .as_deref()
            .expect("stale active holder id");
        assert_eq!(
            fixture.runtime.window_status(holder_window_id),
            expected_status,
            "fixture must expose the intended non-live window status",
        );
        fixture.runtime.auto_launch_issue_monitor_delivery_events(
            &fixture.runtime.test_context(),
            3165,
            LinkedIssueKind::Spec,
            None,
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        let result = take_monitor_launch_complete(case_name, &fixture.recorded_events);
        assert_monitor_exact_resume(result, &fixture);
    }
}

/// A pending resume source blocks exact Resume only while its target window is
/// actually materializing. A stale source map without lookup/inflight evidence
/// is not a native-conversation writer.
#[test]
fn app_runtime_monitor_resume_if_safe_ignores_stale_pending_auto_resume_source() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let case_name = "pending-same-native-missing-window-and-inflight";
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        case_name,
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::StaleMaterializingSameConversation,
        false,
    );
    let holder_window_id = fixture
        .holder_window_id
        .as_deref()
        .expect("stale pending holder id");
    assert!(!fixture.runtime.window_lookup.contains_key(holder_window_id));
    assert!(!fixture
        .runtime
        .inflight_launches
        .values()
        .any(|(window_id, _)| window_id == holder_window_id));

    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        None,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let result = take_monitor_launch_complete(case_name, &fixture.recorded_events);
    assert_monitor_exact_resume(result, &fixture);
}

/// SPEC #3165 T-226 / FR-103: failover/retry delivery policy bypasses the
/// resumable-session search and never forwards the old native conversation id.
#[test]
fn app_runtime_monitor_fresh_required_delivery_skips_resumable_session() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "fresh-required",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        fixture.delivery_id.clone(),
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
    );
    let delivery_id = fixture.delivery_id.as_deref().expect("durable delivery id");
    let materializer_id = fixture.runtime.issue_monitor_materializer_id.clone();
    let persisted = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("reload FreshRequired delivery after materializer claim");
    assert_eq!(persisted.pending_launch_deliveries.len(), 1);
    let delivery = &persisted.pending_launch_deliveries[0];
    assert_eq!(delivery.delivery_id, delivery_id);
    assert_eq!(delivery.claim_id, "claim-fresh-required");
    assert_eq!(delivery.claim_owner, "host/session");
    assert_eq!(
        delivery.launch_session_strategy,
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired
    );
    assert_eq!(
        delivery.materializer_id.as_deref(),
        Some(materializer_id.as_str())
    );
    assert_eq!(delivery.materializer_pid, Some(std::process::id()));
    let bound_window_id = delivery
        .materializer_window_id
        .as_deref()
        .expect("FreshRequired delivery materializer window binding");
    assert!(matches!(
        fixture.runtime.issue_monitor_launch_deliveries.get(delivery_id),
        Some(super::super::IssueMonitorLaunchDeliveryState::Materializing { window_id, .. })
            if window_id == bound_window_id
    ));
    let result = take_monitor_launch_complete("FreshRequired delivery", &fixture.recorded_events);
    assert_monitor_fresh_successor(result, &fixture);
}

/// SPEC #3165 T-226 / FR-103: FreshRequired selects the current Monitor
/// provider profile as a whole; the resumable source provider is not sticky.
#[test]
fn app_runtime_monitor_fresh_required_switches_to_current_provider_profile() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "fresh-required-provider-switch",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load Codex Monitor profile");
    prefs.launch_profile = Some(claude_issue_monitor_launch_profile());
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs)
        .expect("save current Claude Monitor profile");

    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        None,
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
    );
    let result = take_monitor_launch_complete(
        "FreshRequired provider profile switch",
        &fixture.recorded_events,
    );
    let Ok((process, session_id, _, _, _, agent_id, _, _, _, session_mode, _, _)) = result else {
        panic!("Issue Monitor provider-switch launch failed: {result:?}");
    };
    assert_eq!(agent_id, gwt_agent::AgentId::ClaudeCode);
    assert_eq!(session_mode, gwt_agent::SessionMode::Normal);
    assert!(
        process
            .args
            .iter()
            .all(|argument| !argument.contains(&fixture.native_conversation_id)),
        "provider switch must not pass the old Codex conversation id: {:?}",
        process.args,
    );
    let successor =
        gwt_agent::Session::load(&fixture.sessions_dir.join(format!("{session_id}.toml")))
            .expect("load Claude fresh successor Session");
    assert_eq!(successor.agent_id, gwt_agent::AgentId::ClaudeCode);
    assert_eq!(successor.session_mode, gwt_agent::SessionMode::Normal);
    assert!(successor.agent_session_id.is_none());
    assert_eq!(successor.model.as_deref(), Some("sonnet"));
    assert_eq!(successor.reasoning_level.as_deref(), Some("low"));
    assert_eq!(successor.tool_version.as_deref(), Some("1.2.3"));
    assert!(successor.tool_version_selector.is_none());
    assert!(successor.skip_permissions);
    assert!(!successor.fast_mode);
    assert!(!successor.codex_fast_mode);
}

/// Issue #3676 AC-1: `ResumeIfSafe` must not re-bind the launch to a stored
/// session whose provider differs from the Monitor's current launch profile.
/// A provider-mismatched resumable session is skipped and the launch falls
/// through to a fresh session on the profile provider.
#[test]
fn app_runtime_monitor_resume_if_safe_skips_provider_mismatched_session() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "resume-provider-mismatch",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load Codex Monitor profile");
    prefs.launch_profile = Some(claude_issue_monitor_launch_profile());
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs)
        .expect("save current Claude Monitor profile");

    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        None,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let result =
        take_monitor_launch_complete("ResumeIfSafe provider mismatch", &fixture.recorded_events);
    let Ok((process, session_id, _, _, _, agent_id, _, _, _, session_mode, _, _)) = result else {
        panic!("Issue Monitor provider-mismatch launch failed: {result:?}");
    };
    assert_eq!(
        agent_id,
        gwt_agent::AgentId::ClaudeCode,
        "resume must not adopt the stored Codex session when the profile says claude",
    );
    assert_eq!(session_mode, gwt_agent::SessionMode::Normal);
    assert!(
        process
            .args
            .iter()
            .all(|argument| !argument.contains(&fixture.native_conversation_id)),
        "provider mismatch must not pass the old Codex conversation id: {:?}",
        process.args,
    );
    let successor =
        gwt_agent::Session::load(&fixture.sessions_dir.join(format!("{session_id}.toml")))
            .expect("load Claude fresh successor Session");
    assert_eq!(successor.agent_id, gwt_agent::AgentId::ClaudeCode);
    assert_eq!(successor.session_mode, gwt_agent::SessionMode::Normal);
    assert!(successor.agent_session_id.is_none());
    assert_eq!(successor.model.as_deref(), Some("sonnet"));
}

/// Issue #3676 AC-2: a Monitor launch whose profile provider is definitively
/// unauthenticated must fail identifiably before a terminal is spawned, so
/// the slot is released through the normal launch-failed path instead of
/// burning on a login screen.
#[test]
fn app_runtime_monitor_launch_preflight_refuses_unauthenticated_provider() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _openai_key = ScopedEnvVar::unset("OPENAI_API_KEY");
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "unauthenticated-provider",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    // The fixture writes rollouts but no auth.json: this CODEX_HOME is a
    // definitively unauthenticated Codex CLI for the real probe.
    let codex_home = PathBuf::from(std::env::var_os("CODEX_HOME").expect("CODEX_HOME is isolated"));
    assert!(!codex_home.join("auth.json").exists());
    fixture.runtime.issue_monitor_provider_auth_probe =
        gwt::issue_monitor::provider_auth_state_from_env;

    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Spec,
            fixture.delivery_id.clone(),
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
    // This fixture has no daemon: claim/failure publication exercises the
    // existing offline fallback, including its Git-backed cache validation.
    // Keep this an authorization regression; request latency is tested separately.
    drain_queued_blocking_tasks(&queued);
    let mut events =
        fixture
            .runtime
            .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
                &fixture.recorded_events,
            ));
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let claimed = gwt::load_issue_monitor_prefs(&prefs_path).expect("read claimed rejection");
    assert!(
        claimed.pending_launch_deliveries.iter().any(|delivery| {
            Some(&delivery.delivery_id) == fixture.delivery_id.as_ref()
                && delivery.materializer_id.as_deref()
                    == Some(fixture.runtime.issue_monitor_materializer_id.as_str())
                && delivery.materializer_window_id.is_some()
        }),
        "the rejected launch must own its exact delivery before failure settlement runs"
    );
    // Rejection settlement now uses the same worker boundary as launch preparation.
    drain_queued_blocking_tasks(&queued);
    events.extend(fixture.runtime.handle_issue_monitor_launch_prepared(
        take_issue4803_monitor_preparation(&fixture.recorded_events),
    ));
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("read rejected delivery");
    assert!(
        prefs
            .failed_issues
            .iter()
            .any(|failed| failed.issue_number == 3165),
        "prepared rejection must claim the exact delivery before recording its failure"
    );
    let message = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorLaunchFailed {
                issue_number,
                message,
            } if *issue_number == 3165 => Some(message.clone()),
            _ => None,
        })
        .expect("unauthenticated provider launch must fail identifiably before spawning");
    assert!(
        message.contains("provider_unauthenticated"),
        "failure must carry the machine marker: {message}",
    );
    assert!(
        message.contains("codex"),
        "failure must name the provider: {message}"
    );
    // Run all queued work before proving that refusal dispatched no terminal.
    drain_queued_blocking_tasks(&queued);
    let recorded = fixture
        .recorded_events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        recorded.iter().all(|event| !matches!(
            recorded_project_payload(event),
            UserEvent::LaunchComplete { .. }
        )),
        "refused launch must not spawn a PTY",
    );
}

#[test]
fn app_runtime_issue_monitor_status_reports_last_settings_source() {
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

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut previous = gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex);
    previous.model = Some("gpt-5.5".to_string());
    previous.reasoning_level = Some("high".to_string());
    previous.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
    previous.save(&sessions_dir).expect("save previous session");

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.completed_issue_monitor_list_for_test(
        "client-1",
        gwt::daemon_publisher::read_issue_monitor_status,
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(
        status.launch_profile_source,
        gwt::IssueMonitorLaunchProfileSource::LastSettings
    );
    assert_eq!(
        status.launch_profile_summary,
        "codex / gpt-5.5 / high / host / fast:off"
    );
}

#[test]
fn app_runtime_issue_monitor_configure_recovers_malformed_prefs_without_launching() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    gwt::machine_agent_settings::ensure_migrated().expect("initialize machine settings");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    fs::create_dir_all(prefs_path.parent().expect("prefs parent")).expect("create prefs directory");
    fs::write(&prefs_path, b"{").expect("seed malformed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorConfigureIssue {
            issue_number: 3165,
            linked_issue_kind: Some(LinkedIssueKind::Spec),
        },
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor settings opened" && *issue_number == Some(3165)
        )
    }));
    let view = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("launch wizard view");
    assert_eq!(view.primary_action_label, "Continue");
    assert_eq!(view.linked_issue_number, Some(3165));
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .as_ref()
        .expect("launch wizard")
        .issue_monitor_profile_save
        .is_some());
    assert_eq!(
        runtime
            .project_state(&runtime.test_context()).expect("test project state").launch_wizard
            .as_ref()
            .expect("launch wizard")
            .wizard
            .initial_prompt,
        "$gwt-execute #3165\n\nThis prompt was generated by Issue Monitor. It is not a statement, approval, or visual confirmation by a human user."
    );

    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SetModel {
            model: "gpt-5.5".to_string(),
        },
        None,
    );
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SetReasoning {
            reasoning: "high".to_string(),
        },
        None,
    );
    runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SetSkipPermissions { enabled: true },
        None,
    );
    runtime.handle_launch_wizard_action(&runtime.test_context(), LaunchWizardAction::Submit, None);
    wait_for_recorded_event(
        "issue monitor settings runtime resolution",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
        },
    );
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    let confirm_events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        None,
    );
    let confirm_view = confirm_events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("confirm wizard view");
    assert_eq!(confirm_view.primary_action_label, "Save settings");

    // Recovery content must not depend on completing fsync inside the GUI budget.
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
    let saved_events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        None,
    );

    assert!(
        saved_events.iter().any(|event| {
            matches!(
                &event.event,
                BackendEvent::IssueMonitorToast { message, issue_number, .. }
                    if message == "Issue Monitor settings saved" && *issue_number == Some(3165)
            )
        }),
        "profile recovery save must commit successfully: {saved_events:?}"
    );
    assert!(runtime
        .project_state(&runtime.test_context())
        .expect("test project state")
        .launch_wizard
        .is_none());
    assert!(
        runtime.window_details.is_empty(),
        "saving Issue Monitor settings must not spawn an agent window"
    );

    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load issue monitor prefs");
    assert_eq!(
        prefs.legacy_git_launch_failure_migration_version, 0,
        "saving settings after recovery cannot mark the live-scan migration complete"
    );
    let profile = prefs.launch_profile.expect("saved launch profile");
    assert_eq!(profile.agent_id, "codex");
    assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(profile.reasoning.as_deref(), Some("high"));
    assert!(profile.skip_permissions);
}

fn queue_issue_monitor_profile_worker(runtime: &mut AppRuntime) -> BlockingTestTaskQueue {
    let (spawner, queued) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    queued
}

fn finish_queued_issue_monitor_profile_save(runtime: &mut AppRuntime) -> Vec<OutboundEvent> {
    let BlockingTaskSpawner::Queued(tasks) = &runtime.blocking_tasks else {
        panic!("profile save test needs a queued worker");
    };
    let tasks = tasks.clone();
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
    drain_queued_blocking_tasks(&tasks);
    let AppEventProxy::Stub(recorded) = &runtime.proxy else {
        panic!("profile save test needs a stub proxy");
    };
    let completion = {
        let mut recorded = recorded.lock().expect("profile completions");
        let index = recorded
            .iter()
            .position(|event| matches!(event, UserEvent::IssueMonitorProfilesSaved(_)))
            .expect("profile completion must use the base proxy");
        recorded.remove(index)
    };
    let UserEvent::IssueMonitorProfilesSaved(completion) = completion else {
        unreachable!("matched profile completion");
    };
    runtime.handle_issue_monitor_profiles_saved(*completion)
}

fn replay_next_issue_monitor_profile_frontend(
    runtime: &mut AppRuntime,
) -> Option<Vec<OutboundEvent>> {
    let AppEventProxy::Stub(recorded) = &runtime.proxy else {
        panic!("profile replay test needs a stub proxy");
    };
    let event = {
        let mut recorded = recorded.lock().expect("deferred frontend events");
        let index = recorded
            .iter()
            .position(|event| matches!(recorded_project_payload(event), UserEvent::Frontend { .. }))
            .expect("deferred frontend event");
        recorded.remove(index)
    };
    let UserEvent::Frontend {
        client_id,
        client_scope: Some(scope),
        event,
        ..
    } = runtime.accept_project_completion(event)?
    else {
        panic!("deferred request must retain its authenticated project scope");
    };
    Some(runtime.handle_frontend_event_in_scope(client_id, event, &scope))
}

fn issue_monitor_profile_reasoning_edit(request_id: u64, reasoning: &str) -> FrontendEvent {
    serde_json::from_value(serde_json::json!({
        "kind": "issue_monitor_profiles_set",
        "profiles": [{"agent_id": "codex", "reasoning": reasoning}],
        "request_id": request_id
    }))
    .expect("candidate edit")
}

#[test]
fn app_runtime_issue_monitor_profiles_set_preserves_sparse_candidate_settings() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    gwt::machine_agent_settings::set(
        Some(7),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-22T00:00:00Z",
    )
    .expect("seed machine Manual capacity");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut claude = pool_profile("claude");
    claude.model = Some("saved-claude-model".into());
    claude.reasoning = Some("high".into());
    claude.skip_permissions = true;
    claude.prefer_for = vec!["type:bug".into()];
    let mut codex = pool_profile("codex");
    codex.model = Some("saved-codex-model".into());
    codex.fast_mode = true;
    let mut seeded = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        max_active_agents: 7,
        launch_usage_threshold_percent: 83,
        ..Default::default()
    };
    seeded.set_launch_profile_pool(vec![claude.clone(), codex.clone()]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed pool");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    queue_issue_monitor_profile_worker(&mut runtime);
    let event = serde_json::from_value(serde_json::json!({
        "kind": "issue_monitor_profiles_set",
        "profiles": [{"agent_id": "codex", "prefer_for": ["kind:feature"]},
                     {"agent_id": "claude"}, {"agent_id": "grok"}]
    }))
    .expect("candidate editing event");
    let mut events = runtime.handle_frontend_event("client-1".into(), event);
    events.extend(finish_queued_issue_monitor_profile_save(&mut runtime));
    let errors: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.event {
            BackendEvent::IssueMonitorToast { level, message, .. } if level == "error" => {
                Some(message)
            }
            _ => None,
        })
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let saved = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload pool");
    codex.prefer_for = vec!["kind:feature".into()];
    assert_eq!(saved.launch_profiles[0], codex);
    assert_eq!(saved.launch_profiles[1], claude);
    assert_eq!(saved.launch_profiles[2].agent_id, "grok");
    assert!(
        saved.launch_profiles[2].skip_permissions,
        "new candidate inherits shared settings"
    );
    assert_eq!(saved.launch_profile.as_ref(), Some(&codex));
    assert_eq!(saved.launch_usage_threshold_percent, 80);
    assert_eq!(saved.max_active_agents, 7);
    assert_eq!(
        saved.effect_authority_epoch,
        seeded.effect_authority_epoch + 1
    );

    // Removing candidates and supplying a legacy threshold retain the remaining
    // candidate's provider-specific configuration and its routing tags.
    let event = serde_json::from_value(serde_json::json!({
        "kind": "issue_monitor_profiles_set",
        "profiles": [{"agent_id": "codex"}], "usage_threshold_percent": 0
    }))
    .expect("candidate removal event");
    runtime.handle_frontend_event("client-1".into(), event);
    finish_queued_issue_monitor_profile_save(&mut runtime);
    let saved = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload reduced pool");
    assert_eq!(saved.launch_profiles, vec![codex]);
    assert_eq!(saved.launch_usage_threshold_percent, 80);
    let wire = serde_json::to_value(&saved).expect("saved prefs JSON");
    assert!(wire.get("launch_usage_threshold_percent").is_none());
    assert_eq!(saved.max_active_agents, 7);
}

#[test]
fn app_runtime_issue_monitor_profiles_set_waits_for_durable_worker() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut profile = pool_profile("codex");
    profile.model = Some("saved-model-outside-catalog".to_string());
    let mut seeded = gwt::IssueMonitorPrefs::default();
    seeded.set_launch_profile_pool(vec![profile.clone()]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed pool");
    let before = fs::read(&prefs_path).expect("original prefs bytes");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let queued = queue_issue_monitor_profile_worker(&mut runtime);
    let event = issue_monitor_profile_reasoning_edit(57875, "xhigh");
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());

    let events = runtime.handle_frontend_event("client-1".into(), event);

    assert!(
        events.iter().all(|event| !matches!(
            event.event,
            BackendEvent::IssueMonitorProfiles {
                request_id: Some(57875),
                ..
            }
        )),
        "profile save must wait for its durable worker before acknowledging success"
    );
    assert_eq!(
        fs::read(&prefs_path).expect("prefs before worker"),
        before,
        "dispatch must leave persisted profiles and unknown saved values untouched"
    );
    assert_eq!(queued.lock().expect("queued tasks").len(), 1);
    assert!(
        runtime
            .handle_frontend_event("client-2".into(), FrontendEvent::GetIssueMonitorProfiles)
            .is_empty(),
        "a reconnected client must wait for the pending durable profile save"
    );
    assert!(runtime
        .handle_frontend_event(
            "client-1".into(),
            FrontendEvent::SetIssueMonitorEnabled { enabled: true },
        )
        .is_empty());
    assert!(!gwt::load_issue_monitor_prefs(&prefs_path).unwrap().enabled);
    assert_eq!(queued.lock().expect("queued tasks").len(), 1);

    let events = finish_queued_issue_monitor_profile_save(&mut runtime);
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorProfiles { profiles, request_id: Some(57875), .. }
            if profiles[0].model == profile.model
    )));
    profile.reasoning = Some("xhigh".to_string());
    let saved = gwt::load_issue_monitor_prefs(&prefs_path).expect("canonical profiles");
    assert_eq!(saved.launch_profile_pool(), vec![profile.clone()]);
    assert!(!saved.enabled, "Enable must not overtake the profile ACK");
    assert!(serde_json::to_value(&saved)
        .unwrap()
        .get("choices")
        .is_none());

    let readback =
        replay_next_issue_monitor_profile_frontend(&mut runtime).expect("reconnected profile read");
    assert!(readback.iter().any(|event| {
        matches!(&event.target, DispatchTarget::Client(client_id) if client_id == "client-2")
            && matches!(
                &event.event,
                BackendEvent::IssueMonitorProfiles { profiles, request_id: None, .. }
                    if profiles == &vec![profile.clone()]
            )
    }));
    replay_next_issue_monitor_profile_frontend(&mut runtime).expect("current Enable request");
    let enabled = gwt::load_issue_monitor_prefs(&prefs_path).expect("enabled after save");
    assert!(enabled.enabled);
    assert_eq!(enabled.launch_profile_pool(), vec![profile]);
    assert!(runtime.issue_monitor_profile_saves.is_empty());
}

#[test]
fn app_runtime_issue_monitor_profiles_set_orders_reopened_project_after_old_worker() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs::default();
    seeded.set_launch_profile_pool(vec![pool_profile("codex")]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed pool");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let old_context = runtime.test_context();
    let queued = queue_issue_monitor_profile_worker(&mut runtime);
    runtime.handle_frontend_event(
        "client-1".into(),
        issue_monitor_profile_reasoning_edit(57876, "high"),
    );
    runtime.handle_frontend_event(
        "client-1".into(),
        issue_monitor_profile_reasoning_edit(57877, "xhigh"),
    );

    // Closing and reopening replaces the project state while the runtime lane survives.
    runtime
        .project_tab_incarnations
        .get_mut("tab-1")
        .unwrap()
        .generation += 1;
    runtime.refresh_project_state("tab-1");
    assert!(!runtime.project_context_is_current(&old_context));
    assert!(runtime
        .issue_monitor_profile_saves
        .contains_key(&old_context.project_key));
    runtime.handle_frontend_event(
        "client-2".into(),
        issue_monitor_profile_reasoning_edit(57878, "low"),
    );
    assert_eq!(queued.lock().expect("old worker").len(), 1);

    assert!(
        finish_queued_issue_monitor_profile_save(&mut runtime).is_empty(),
        "the old generation releases its lane without publishing a stale ACK"
    );
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .launch_profile_pool()[0]
            .reasoning
            .as_deref(),
        Some("high")
    );
    assert!(replay_next_issue_monitor_profile_frontend(&mut runtime).is_none());
    assert!(queued.lock().expect("stale request discarded").is_empty());
    assert!(replay_next_issue_monitor_profile_frontend(&mut runtime)
        .expect("new generation request")
        .is_empty());
    assert_eq!(queued.lock().expect("new worker").len(), 1);
    let events = finish_queued_issue_monitor_profile_save(&mut runtime);
    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::IssueMonitorProfiles {
            request_id: Some(57878),
            ..
        }
    )));
    let saved = gwt::load_issue_monitor_prefs(&prefs_path).expect("new generation save");
    assert_eq!(
        saved.launch_profile_pool()[0].reasoning.as_deref(),
        Some("low")
    );
    assert_eq!(
        saved.effect_authority_epoch,
        seeded.effect_authority_epoch + 2
    );
    assert!(runtime.issue_monitor_profile_saves.is_empty());
}

#[test]
fn app_runtime_issue_monitor_profiles_set_keeps_revocation_off_immediate() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs {
        enabled: true,
        autonomous_mode: true,
        ..Default::default()
    };
    seeded.set_launch_profile_pool(vec![pool_profile("codex")]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed enabled pool");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let queued = queue_issue_monitor_profile_worker(&mut runtime);
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
    runtime.handle_frontend_event(
        "client-1".into(),
        issue_monitor_profile_reasoning_edit(57879, "xhigh"),
    );
    runtime.handle_frontend_event(
        "client-1".into(),
        FrontendEvent::SetIssueMonitorEnabled { enabled: false },
    );
    runtime.handle_frontend_event(
        "client-1".into(),
        FrontendEvent::SetIssueMonitorAutonomousMode { enabled: false },
    );
    let revoked = gwt::load_issue_monitor_prefs(&prefs_path).expect("immediate revocation");
    assert!(!revoked.enabled && !revoked.autonomous_mode);
    assert_eq!(queued.lock().expect("profile still waiting").len(), 1);
    assert_eq!(runtime.issue_monitor_profile_saves.len(), 1);

    let events = finish_queued_issue_monitor_profile_save(&mut runtime);
    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::IssueMonitorProfiles {
            request_id: Some(57879),
            ..
        }
    )));
    let saved = gwt::load_issue_monitor_prefs(&prefs_path).expect("save after revocation");
    assert!(!saved.enabled && !saved.autonomous_mode);
    assert_eq!(
        saved.launch_profile_pool()[0].reasoning.as_deref(),
        Some("xhigh")
    );
    assert!(runtime.issue_monitor_profile_saves.is_empty());
}

#[test]
fn app_runtime_issue_monitor_profiles_set_releases_lane_after_spawn_failure() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs::default();
    seeded.set_launch_profile_pool(vec![pool_profile("codex")]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed pool");
    let before = fs::read(&prefs_path).expect("prefs before enqueue failure");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.blocking_tasks = BlockingTaskSpawner::failing("profile worker unavailable");
    let events = runtime.handle_frontend_event(
        "client-1".into(),
        issue_monitor_profile_reasoning_edit(57880, "high"),
    );
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorProfilesWriteFailed { request_id: 57880, message, .. }
            if message.contains("profile worker unavailable")
    )));
    assert_eq!(fs::read(&prefs_path).expect("unchanged prefs"), before);
    assert!(runtime.issue_monitor_profile_saves.is_empty());

    let queued = queue_issue_monitor_profile_worker(&mut runtime);
    runtime.handle_frontend_event(
        "client-1".into(),
        issue_monitor_profile_reasoning_edit(57881, "xhigh"),
    );
    assert_eq!(queued.lock().expect("next explicit save").len(), 1);
    let events = finish_queued_issue_monitor_profile_save(&mut runtime);
    assert!(events.iter().any(|event| matches!(
        event.event,
        BackendEvent::IssueMonitorProfiles {
            request_id: Some(57881),
            ..
        }
    )));
    assert!(runtime.issue_monitor_profile_saves.is_empty());
}

#[test]
fn app_runtime_issue_monitor_profiles_set_acknowledges_save_with_git_attention() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    fs::write(repo.join(".git"), "gitdir: missing\n").expect("broken Git metadata");
    // The worker's cached diagnosis makes the unrelated snapshot notice
    // deterministic, without depending on a Git subprocess or its deadline.
    gwt_git::repository::git_health::record_metadata_failure(
        &repo,
        "git remote get-url origin",
        "fatal: not a git repository: missing",
    )
    .expect("cached Git attention diagnosis");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs::default();
    seeded.set_launch_profile_pool(vec![pool_profile("claude")]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed pool");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    queue_issue_monitor_profile_worker(&mut runtime);
    let event = serde_json::from_value(serde_json::json!({
        "kind": "issue_monitor_profiles_set",
        "profiles": [{"agent_id": "codex"}], "request_id": 43
    }))
    .expect("candidate editing event");
    let mut events = runtime.handle_frontend_event("client-1".into(), event);
    events.extend(finish_queued_issue_monitor_profile_save(&mut runtime));
    let saved = gwt::load_issue_monitor_prefs(&prefs_path).expect("saved pool");
    assert_eq!(saved.launch_profile_pool()[0].agent_id, "codex");
    assert!(
        events.iter().any(|event| matches!(
            &event.event, BackendEvent::IssueMonitorToast { level, message, .. }
                if level == "error" && message.contains("Git needs attention")
        )),
        "fixture must retain the unrelated Git attention notice: {events:?}"
    );
    assert!(
        events.iter().all(|event| !matches!(
            event.event,
            BackendEvent::IssueMonitorProfilesWriteFailed { .. }
        )),
        "a successful write must not report an unrelated snapshot error: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event.event,
            BackendEvent::IssueMonitorProfiles {
                request_id: Some(43),
                ..
            }
        )),
        "the successful write must acknowledge this request: {events:?}"
    );
}

#[test]
fn app_runtime_issue_monitor_profiles_set_rejects_invalid_edits_without_writing() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs::default();
    seeded.set_launch_profile_pool(vec![pool_profile("claude")]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed pool");
    let project_root = repo.display().to_string();
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    queue_issue_monitor_profile_worker(&mut runtime);
    for payload in [
        serde_json::json!({"profiles": []}),
        serde_json::json!({"profiles": [{"agent_id": "claude", "prefer_for": ["invalid"]}]}),
    ] {
        let before = fs::read(&prefs_path).expect("read prefs");
        let mut payload = payload;
        payload["kind"] = "issue_monitor_profiles_set".into();
        payload["request_id"] = 42.into();
        let event = serde_json::from_value(payload).expect("candidate editing event");
        let mut events = runtime.handle_frontend_event("client-1".into(), event);
        events.extend(finish_queued_issue_monitor_profile_save(&mut runtime));
        assert!(events.iter().any(|event| matches!(
            &event.event, BackendEvent::IssueMonitorToast { level, .. } if level == "error"
        )));
        assert!(events.iter().any(|event| {
            let wire = serde_json::to_value(&event.event).expect("serialize failure");
            wire["kind"] == "issue_monitor_profiles_write_failed"
                && wire["request_id"] == 42
                && wire["project_root"] == project_root
                && !wire["message"].as_str().unwrap_or_default().is_empty()
        }));
        assert_eq!(fs::read(&prefs_path).expect("reload prefs"), before);
    }
}

#[test]
fn app_runtime_issue_monitor_profiles_set_epoch_overflow_is_zero_write_error() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs {
        effect_authority_epoch: u64::MAX,
        ..Default::default()
    };
    seeded.set_launch_profile_pool(vec![pool_profile("claude")]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed pool");
    let before = fs::read(&prefs_path).expect("read prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    queue_issue_monitor_profile_worker(&mut runtime);
    let event = serde_json::from_value(serde_json::json!({
        "kind": "issue_monitor_profiles_set", "profiles": [{"agent_id": "codex"}]
    }))
    .expect("candidate editing event");
    let mut events = runtime.handle_frontend_event("client-1".into(), event);
    events.extend(finish_queued_issue_monitor_profile_save(&mut runtime));
    assert!(events.iter().any(|event| matches!(
        &event.event, BackendEvent::IssueMonitorToast { level, message, .. }
            if level == "error" && message.contains("authority epoch exhausted")
    )));
    assert_eq!(fs::read(&prefs_path).expect("reload prefs"), before);
}

#[test]
fn app_runtime_issue_monitor_profile_save_switches_the_pool_head() {
    // Issue #4079 AC-1: with `[claude, codex]` saved, an Agent Settings save
    // for codex must make codex candidate 1 — and the `launch_profile` mirror
    // the Monitor launches from. The pre-#4079 upsert rewrote the index-1
    // codex entry and left claude launching.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs::default();
    seeded.set_launch_profile_pool(vec![pool_profile("claude"), pool_profile("codex")]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed pool");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let session = sample_ready_agent_launch_wizard_session("tab-1", &repo);
    let request = gwt::LaunchWizardLaunchRequest::Agent(Box::new(
        gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("develop")
            .model("gpt-6-astra")
            .build(),
    ));

    // Saved content must not depend on completing fsync inside the GUI budget.
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
    let events = runtime.save_issue_monitor_profile_from_launch_request(
        session,
        IssueMonitorProfileSaveContext {
            client_id: "client-1".to_string(),
            issue_number: None,
            pool: seeded.launch_profile_pool(),
            sets: None,
        },
        request,
    );

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, level, .. }
                if message == "Issue Monitor settings saved" && level == "info"
        )),
        "profile save must commit successfully: {events:?}"
    );

    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
    let pool = prefs.launch_profile_pool();
    assert_eq!(
        pool.iter()
            .map(|profile| profile.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["codex"],
        "the chosen agent takes candidate 1"
    );
    assert_eq!(pool[0].model.as_deref(), Some("gpt-6-astra"));
    assert_eq!(
        prefs.launch_profile.as_ref().map(|p| p.agent_id.as_str()),
        Some("codex"),
        "the compatibility mirror follows the head"
    );
}

#[test]
fn app_runtime_issue_monitor_configure_issue_previews_the_pool_head_replacement() {
    // Issue #4079 AC-2: with more than one provider in the pool the per-Issue
    // form must say which candidate the save writes, and its preview of the
    // resulting pool summary must be what the Monitor reports afterwards.
    // (Issue #4911: the settings form edits every set instead; this form still
    // switches the head.)
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs::default();
    seeded.set_launch_profile_pool(vec![pool_profile("claude"), pool_profile("codex")]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed pool");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorConfigureIssue {
            issue_number: 3165,
            linked_issue_kind: Some(LinkedIssueKind::Spec),
        },
    );
    let events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::SetFastMode { enabled: true },
        None,
    );
    let view = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::LaunchWizardState {
                wizard: Some(wizard),
            } => Some(wizard.as_ref()),
            _ => None,
        })
        .expect("launch wizard view");
    assert!(
        view.fast_mode,
        "profile editing honors its own Fast preference"
    );
    let impact = view
        .issue_monitor_pool_impact
        .as_ref()
        .expect("Agent Settings must preview its effect on the candidate pool");
    assert_eq!(impact.action, "replace_head");
    assert_eq!(impact.agent_id, "codex");
    assert_eq!(
        impact.replaced_agent_id.as_deref(),
        Some("claude"),
        "the operator must see which candidate is switched out"
    );
    assert!(
        impact.detail.contains(&impact.resulting_summary),
        "the note states the summary the Monitor will report: {impact:?}"
    );
    let previewed_summary = impact.resulting_summary.clone();

    runtime.handle_launch_wizard_action(&runtime.test_context(), LaunchWizardAction::Submit, None);
    wait_for_recorded_event(
        "issue monitor settings runtime resolution",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
        },
    );
    let resolved_event = {
        let mut events = recorded_events.lock().expect("event log");
        events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchWizardRuntimeResolved { .. }
                )
            })
            .map(|index| events.remove(index))
            .expect("runtime resolved event")
    };
    let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
        unreachable!("matched above")
    };
    runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
    runtime.handle_launch_wizard_action(&runtime.test_context(), LaunchWizardAction::Submit, None);
    let saved_events = {
        // Saved content must not depend on completing fsync inside the GUI budget.
        let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
        runtime.handle_launch_wizard_action(
            &runtime.test_context(),
            LaunchWizardAction::Submit,
            None,
        )
    };

    assert!(
        saved_events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, level, .. }
                if message == "Issue Monitor settings saved" && level == "info"
        )),
        "profile preview save must commit successfully: {saved_events:#?}"
    );

    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
    assert!(prefs.launch_profile.as_ref().expect("saved head").fast_mode);
    let saved_summary = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs)
        .status_view()
        .launch_profile_summary;
    assert_eq!(
        saved_summary, previewed_summary,
        "the previewed summary must be the one the Monitor reports after the save"
    );
}

/// Issue #5043 AC-2: an ordinary Resume must deliver an ahead owner branch.
#[test]
fn app_runtime_monitor_plain_issue_resume_receives_delivery_prompt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "plain-issue-delivery",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
        LinkedIssueKind::Issue,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    );
    fs::write(fixture.worktree.join("fix.rs"), "fn fixed() {}\n").expect("write source");
    run_git(&fixture.worktree, &["add", "fix.rs"]);
    run_git(&fixture.worktree, &["commit", "-qm", "undelivered fix"]);

    prepare_monitor_relaunch(
        &mut fixture,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let result = take_monitor_launch_complete("plain-issue-delivery", &fixture.recorded_events);
    let (process, .., session_mode, _, _) = result.as_ref().expect("ordinary Issue Resume");
    assert_eq!(*session_mode, gwt_agent::SessionMode::Resume);
    let prompt = process
        .args
        .iter()
        .find(|argument| {
            argument.contains("verify.run, a pull request, and its merge into develop")
        })
        .unwrap_or_else(|| {
            panic!(
                "Resume must receive the delivery prompt: {:?}",
                process.args
            )
        });
    assert!(prompt.contains("against origin/develop"), "{prompt}");
    assert!(!prompt.contains("close Issue #3165 and finish"), "{prompt}");
}

/// Issue #4222: retain the quota pane and its work, but release its native
/// writer after the real child exits, including an Error hidden by Waiting.
#[test]
fn app_runtime_monitor_quota_hold_clear_resumes_the_same_conversation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    for (case, exit_code, status) in [
        ("quota-clean-exit", 0, WindowProcessStatus::Stopped),
        ("quota-error-exit", 1, WindowProcessStatus::Error),
    ] {
        let mut fixture = monitor_relaunch_fixture(
            temp.path(),
            case,
            MonitorProviderConversationFixture::Present,
            MonitorNativeHolderFixture::ActiveSameConversation,
            false,
        );
        let window_id = fixture.holder_window_id.clone().unwrap();
        fixture
            .runtime
            .active_agent_sessions
            .get_mut(&window_id)
            .unwrap()
            .session_id = fixture.source_session_id.clone();
        fixture.runtime.tabs[0]
            .workspace
            .set_session_id("agent-holder", Some(fixture.source_session_id.clone()));
        let unfinished = fixture.worktree.join("unfinished.txt");
        fs::write(&unfinished, "uncommitted work").unwrap();
        insert_exited_test_pane_runtime(&mut fixture.runtime, &window_id, exit_code);
        fixture
            .runtime
            .handle_runtime_status_with_exit_confirmation(
                window_id.clone(),
                status,
                Some(CODEX_USAGE_LIMIT_SCREEN.to_string()),
                true,
            );
        assert_eq!(
            fixture.runtime.window_status(&window_id),
            Some(WindowProcessStatus::Waiting)
        );
        assert!(fixture
            .runtime
            .active_agent_sessions
            .contains_key(&window_id));
        let gwt::IssueMonitorFailure::ProviderUsageLimit {
            provider,
            resets_at,
            evidence,
            ..
        } = fixture.runtime.provider_quota_holds[&window_id].clone()
        else {
            panic!("quota exit must form a typed hold")
        };

        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
        let mut monitor = test_monitor_with_prefs(
            gwt::IssueMonitorConfig::default(),
            gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
        );
        monitor.complete_active_launch(3165, &window_id);
        let now = Utc::now().to_rfc3339();
        assert_eq!(
            monitor.try_hold_provider_usage_limit(
                3165,
                &window_id,
                &provider,
                "provider usage limit",
                resets_at.as_deref(),
                evidence.map(|evidence| *evidence),
                &now,
            ),
            gwt::IssueMonitorProviderUsageLimitOutcome::Held
        );
        assert!(!monitor.prefs().provider_quota_holds.is_empty());
        monitor.clear_provider_quota_hold("codex", "quota reset", &now);
        assert!(monitor.prefs().provider_quota_holds.is_empty());
        gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).unwrap();

        // Monitor admission is released independently of the retained screen.
        assert_eq!(
            fixture.runtime.window_status(&window_id),
            Some(WindowProcessStatus::Waiting)
        );
        let candidate = fixture
            .runtime
            .issue_monitor_session_by_id(&fixture.source_session_id)
            .unwrap();
        assert_eq!(
            fixture
                .runtime
                .issue_monitor_native_conversation_holder_excluding(&candidate, None),
            None
        );
        let events = prepare_monitor_relaunch(
            &mut fixture,
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        let (prepared_window_id, mut completion) =
            take_monitor_launch_complete_event(case, &fixture.recorded_events);
        assert_monitor_exact_resume(completion.clone(), &fixture);
        // This fixture routes the completion into an in-place restart below.
        // Consume the original placeholder's worker as the GUI handler would.
        fixture
            .runtime
            .initial_launch_workers
            .remove(&prepared_window_id);
        assert!(fixture.runtime.window_lookup.contains_key(&window_id));
        assert!(fixture
            .runtime
            .active_agent_sessions
            .contains_key(&window_id));
        assert_eq!(fs::read_to_string(&unfinished).unwrap(), "uncommitted work");
        assert!(!events.iter().any(|event| matches!(&event.event,
            BackendEvent::IssueMonitorToast { message, .. } if message.contains("held by window")
        )));

        let (spawner, tasks) = BlockingTaskSpawner::queued();
        fixture.runtime.blocking_tasks = spawner;
        // Capture another delivery before restart begins, then apply its worker
        // facts after the reservation appears to exercise GUI revalidation too.
        fixture
            .runtime
            .auto_launch_issue_monitor_delivery_events_for_project(
                &fixture.project_root,
                3165,
                LinkedIssueKind::Spec,
                None,
                gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
            );
        drain_queued_blocking_tasks(&tasks);
        let delayed_delivery = take_issue4803_monitor_preparation(&fixture.recorded_events);

        // An in-place restart reuses this ID before replacing the exited Pane.
        // Its current launch, rather than the old exit receipt, owns the writer.
        fixture
            .runtime
            .initial_launch_workers
            .insert(window_id.clone(), None);
        fixture
            .runtime
            .pending_auto_resume_sources
            .insert(window_id.clone(), fixture.source_session_id.clone());
        fixture.runtime.inflight_launches.insert(
            "restart-reservation".into(),
            (window_id.clone(), Instant::now()),
        );
        assert_eq!(
            fixture
                .runtime
                .live_issue_agent_window("tab-1", 3165, "work/issue-3165"),
            Some(window_id.clone()),
        );
        let windows_before_delivery = fixture.runtime.window_lookup.len();
        fixture
            .runtime
            .handle_issue_monitor_launch_prepared(delayed_delivery);
        assert_eq!(fixture.runtime.issue_monitor_launch_queue.len(), 1);
        assert!(fixture.runtime.pending_launch_delivery_acks.is_empty());
        fixture
            .runtime
            .auto_launch_issue_monitor_delivery_events_for_project(
                &fixture.project_root,
                3165,
                LinkedIssueKind::Spec,
                None,
                gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
            );
        assert_eq!(
            fixture.runtime.issue_monitor_launch_queue.len(),
            1,
            "a reserved restart cannot acknowledge or launch another delivery",
        );
        assert!(fixture.runtime.pending_launch_delivery_acks.is_empty());
        fixture.runtime.advance_issue_monitor_launch_queue_events();
        assert_eq!(fixture.runtime.issue_monitor_launch_queue.len(), 1);
        assert_eq!(fixture.runtime.window_lookup.len(), windows_before_delivery);
        // Fail before spawning a replacement child, so the test can also prove
        // the failed attempt releases its reservation without closing the pane.
        completion.as_mut().unwrap().0.command = temp
            .path()
            .join("missing-restart-agent")
            .display()
            .to_string();
        fixture
            .runtime
            .handle_launch_complete(window_id.clone(), completion);
        assert!(!fixture
            .runtime
            .initial_launch_workers
            .contains_key(&window_id));
        assert!(!fixture
            .runtime
            .pending_auto_resume_sources
            .contains_key(&window_id));
        assert!(fixture
            .runtime
            .pending_launch_completions
            .contains_key(&window_id));
        assert_eq!(
            fixture
                .runtime
                .issue_monitor_native_conversation_holder_excluding(&candidate, None),
            Some(window_id.clone()),
            "the completion worker must retain the writer after consuming the pending source",
        );
        assert_eq!(
            fixture
                .runtime
                .live_issue_agent_window("tab-1", 3165, "work/issue-3165"),
            Some(window_id.clone()),
        );
        assert_eq!(fixture.runtime.issue_monitor_launch_queue.len(), 1);
        assert!(fixture.runtime.pending_launch_delivery_acks.is_empty());
        assert_eq!(fixture.runtime.window_lookup.len(), windows_before_delivery);
        drain_queued_blocking_tasks(&tasks);
        let prepared = take_prepared_agent_launch(&fixture.recorded_events);
        fixture.runtime.handle_agent_launch_prepared(prepared);
        assert!(!fixture
            .runtime
            .pending_launch_completions
            .contains_key(&window_id));
        assert_eq!(
            fixture
                .runtime
                .issue_monitor_native_conversation_holder_excluding(&candidate, None),
            None,
            "a failed restart must release the writer reservation",
        );
        assert!(fixture.runtime.window_lookup.contains_key(&window_id));
        assert!(fixture.runtime.issue_monitor_launch_queue.is_empty());
        assert_eq!(fixture.runtime.issue_monitor_launch_preparations.len(), 1);
        assert!(fixture.runtime.pending_launch_delivery_acks.is_empty());
        assert_eq!(fs::read_to_string(&unfinished).unwrap(), "uncommitted work");
    }
}

/// Waiting and Idle can still own a native writer. In particular a read
/// failure is not a confirmed child exit, even when the hook remains live.
#[test]
fn app_runtime_monitor_live_native_writer_survives_waits_and_read_errors() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "live-statuses",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::ActiveSameConversation,
        false,
    );
    let window_id = fixture.holder_window_id.clone().unwrap();
    let candidate = fixture
        .runtime
        .issue_monitor_session_by_id(&fixture.source_session_id)
        .unwrap();
    for status in [
        WindowProcessStatus::Running,
        WindowProcessStatus::Starting,
        WindowProcessStatus::Idle,
        WindowProcessStatus::Waiting,
        WindowProcessStatus::Interrupted,
    ] {
        fixture
            .runtime
            .window_hook_states
            .insert(window_id.clone(), status);
        assert_eq!(
            fixture
                .runtime
                .issue_monitor_native_conversation_holder_excluding(&candidate, None),
            Some(window_id.clone()),
            "{status:?}"
        );
        assert_eq!(
            fixture
                .runtime
                .live_issue_agent_window("tab-1", 3165, "work/issue-3165"),
            Some(window_id.clone()),
            "{status:?}"
        );
    }
    fixture
        .runtime
        .window_hook_states
        .insert(window_id.clone(), WindowProcessStatus::Waiting);
    fixture
        .runtime
        .window_pty_statuses
        .insert(window_id.clone(), WindowProcessStatus::Error);
    assert_eq!(
        fixture
            .runtime
            .issue_monitor_native_conversation_holder_excluding(&candidate, None),
        Some(window_id.clone())
    );
}
