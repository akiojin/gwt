use super::*;

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

/// SPEC #3165 T-226 / FR-102: provider ownership is a fail-closed preflight;
/// only a present rollout owned by the selected worktree may exact Resume.
#[test]
fn app_runtime_monitor_resume_if_safe_resumes_only_present_provider_conversation() {
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

    for (case_name, availability, exact_resume_expected) in [
        ("present", MonitorProviderConversationFixture::Present, true),
        (
            "missing",
            MonitorProviderConversationFixture::Missing,
            false,
        ),
        (
            "foreign",
            MonitorProviderConversationFixture::Foreign,
            false,
        ),
        (
            "unknown",
            MonitorProviderConversationFixture::Unknown,
            false,
        ),
        (
            "corrupt",
            MonitorProviderConversationFixture::Corrupt,
            false,
        ),
    ] {
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
        let started = Instant::now();
        fixture
            .runtime
            .handle_issue_monitor_launch_prepared(prepared);
        eprintln!("monitor completion {case_name}: {:?}", started.elapsed());
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{case_name} completion blocked the GUI: {:?}",
            started.elapsed()
        );
        let result = take_monitor_launch_complete(case_name, &fixture.recorded_events);
        if exact_resume_expected {
            assert_monitor_exact_resume(result, &fixture);
        } else {
            assert_monitor_fresh_successor(result, &fixture);
        }
    }
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
    let restored =
        gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), receipted.clone());
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
    let monitor =
        gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), settled.clone());
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
        .handle_launch_complete_and_drain(window_id, result);

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
    let events =
        fixture
            .runtime
            .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
                &fixture.recorded_events,
            ));
    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .expect("read rejected delivery");
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

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::ListIssueMonitor);

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

    let saved_events = runtime.handle_launch_wizard_action(
        &runtime.test_context(),
        LaunchWizardAction::Submit,
        None,
    );

    assert!(saved_events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor settings saved" && *issue_number == Some(3165)
        )
    }));
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

#[test]
fn app_runtime_issue_monitor_profiles_set_preserves_sparse_candidate_settings() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
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
    let event = serde_json::from_value(serde_json::json!({
        "kind": "issue_monitor_profiles_set",
        "profiles": [{"agent_id": "codex", "prefer_for": ["kind:feature"]},
                     {"agent_id": "claude"}, {"agent_id": "grok"}]
    }))
    .expect("candidate editing event");
    let events = runtime.handle_frontend_event("client-1".into(), event);
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
    assert_eq!(saved.launch_usage_threshold_percent, 83);
    assert_eq!(saved.max_active_agents, 7);
    assert_eq!(
        saved.effect_authority_epoch,
        seeded.effect_authority_epoch + 1
    );

    // Removing candidates and changing only the threshold retain the remaining
    // candidate's provider-specific configuration and its routing tags.
    let event = serde_json::from_value(serde_json::json!({
        "kind": "issue_monitor_profiles_set",
        "profiles": [{"agent_id": "codex"}], "usage_threshold_percent": 71
    }))
    .expect("candidate removal event");
    runtime.handle_frontend_event("client-1".into(), event);
    let saved = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload reduced pool");
    assert_eq!(saved.launch_profiles, vec![codex]);
    assert_eq!(saved.launch_usage_threshold_percent, 71);
    assert_eq!(saved.max_active_agents, 7);
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
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for payload in [
        serde_json::json!({"profiles": []}),
        serde_json::json!({"profiles": [{"agent_id": "claude", "prefer_for": ["invalid"]}]}),
        serde_json::json!({"profiles": [{"agent_id": "claude"}], "usage_threshold_percent": 0}),
    ] {
        let before = fs::read(&prefs_path).expect("read prefs");
        let mut payload = payload;
        payload["kind"] = "issue_monitor_profiles_set".into();
        let event = serde_json::from_value(payload).expect("candidate editing event");
        let events = runtime.handle_frontend_event("client-1".into(), event);
        assert!(events.iter().any(|event| matches!(
            &event.event, BackendEvent::IssueMonitorToast { level, .. } if level == "error"
        )));
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
    let event = serde_json::from_value(serde_json::json!({
        "kind": "issue_monitor_profiles_set", "profiles": [{"agent_id": "codex"}]
    }))
    .expect("candidate editing event");
    let events = runtime.handle_frontend_event("client-1".into(), event);
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

    runtime.save_issue_monitor_profile_from_launch_request(
        session,
        IssueMonitorProfileSaveContext {
            client_id: "client-1".to_string(),
            issue_number: None,
            pool: seeded.launch_profile_pool(),
            sets: None,
        },
        request,
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
    runtime.handle_launch_wizard_action(&runtime.test_context(), LaunchWizardAction::Submit, None);

    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
    assert!(prefs.launch_profile.as_ref().expect("saved head").fast_mode);
    let saved_summary =
        gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs)
            .status_view()
            .launch_profile_summary;
    assert_eq!(
        saved_summary, previewed_summary,
        "the previewed summary must be the one the Monitor reports after the save"
    );
}
