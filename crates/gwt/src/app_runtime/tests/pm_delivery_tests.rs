use super::*;

/// SPEC-3431 FR-111 (T-206): the server-side PM principal gate for pane
/// message delivery — re-verified immediately before the injection. Ordinary
/// sessions, stale registrations, and unknown windows are refused with a
/// typed reply and zero writes; only the live registered PM reaches the
/// authorized injection path.
#[test]
fn authenticated_pm_send_reports_delivered_only_after_exact_target_hook_ack() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    insert_test_pane_runtime(&mut runtime, &pm_window_id);
    runtime.register_pty_writer(&pm_window_id, None);
    let target_window = "tab-1::other-window".to_string();
    insert_test_pane_runtime(&mut runtime, &target_window);
    runtime.register_pty_writer(&target_window, None);

    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let capability = issuer
        .issue(&repo, "pm-session-live")
        .expect("PM capability");
    let grant = issuer.grant_for_test(&capability.token).expect("PM grant");
    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643ae";
    let body = "verify this exact body";
    let body_sha256 = gwt::pm_registry::pm_delivery_prompt_sha256(body);
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    let ack_receipt_path = receipt_path.clone();
    let ack_body_sha256 = body_sha256.clone();
    let (prepared_tx, prepared_rx) = mpsc::sync_channel(1);
    let (release_ack_tx, release_ack_rx) = mpsc::sync_channel(1);
    let ack = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let prepared = gwt::pm_registry::load_pm_delivery_receipts(&ack_receipt_path)
                .unwrap_or_default()
                .iter()
                .any(|receipt| {
                    receipt.operation_id == operation_id
                        && receipt.status == gwt::pm_registry::PmDeliveryReceiptStatus::Prepared
                });
            if prepared {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Prepared receipt was not committed"
            );
            // test-hygiene: allow-short-duration durable cross-process receipt polling; no receipt notification API exists and the separate deadline bounds it
            thread::sleep(Duration::from_millis(10));
        }
        prepared_tx.send(()).expect("announce Prepared receipt");
        release_ack_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("release the exact hook acknowledgement");
        gwt::pm_registry::finish_pm_delivery_receipt(
            &ack_receipt_path,
            operation_id,
            "other-session",
            &ack_body_sha256,
            gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
            None,
        )
        .expect("exact target hook acknowledgement");
    });
    let (responder, mut result, _cancellation) = AgentPmSendResponder::channel();

    assert!(runtime
        .authenticated_pm_pane_send_input_events(
            &issuer,
            "origin-client".to_string(),
            grant,
            operation_id,
            &target_window,
            &format!("{body}\r"),
            Some(responder),
        )
        .is_empty());
    prepared_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("the delivery reached Prepared before its acknowledgement");
    assert!(
        matches!(
            result.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ),
        "delivery must remain pending before the exact target hook acknowledgement"
    );
    assert_eq!(
        gwt::pm_registry::pm_delivery_receipt_for_operation(&receipt_path, operation_id)
            .expect("pending receipt")
            .expect("Prepared operation")
            .status,
        gwt::pm_registry::PmDeliveryReceiptStatus::Prepared
    );
    release_ack_tx
        .send(())
        .expect("acknowledge the pending delivery");
    let result = result.blocking_recv().expect("terminal delivery result");
    ack.join().expect("ack thread");

    assert!(matches!(
        result,
        BackendEvent::PmMessageSendResult {
            status,
            reason: None,
            ..
        } if status == "delivered"
    ));
    assert_eq!(
        gwt::pm_registry::load_pm_delivery_receipts(&receipt_path)
            .expect("delivery receipts")
            .iter()
            .filter(|receipt| receipt.operation_id == operation_id)
            .map(|receipt| receipt.status)
            .collect::<Vec<_>>(),
        vec![
            gwt::pm_registry::PmDeliveryReceiptStatus::Prepared,
            gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
        ]
    );
}

/// Issue #3608 (AC-1/AC-4/AC-5): the acknowledgement is written by the target
/// Session's own UserPromptSubmit hook, so it routinely lands after the submit
/// retries are exhausted — the live incident recorded `prepared 02:40:38 →
/// ambiguous 02:40:39 → verified 02:40:40`, a delivery that succeeded but was
/// reported as a failure. The operation must spend its whole remaining budget
/// waiting for that acknowledgement, and the durable log must not carry an
/// Ambiguous row for a delivery it reports as delivered.
#[test]
fn authenticated_pm_send_reports_delivered_when_hook_ack_lands_after_submit_retries() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    insert_test_pane_runtime(&mut runtime, &pm_window_id);
    runtime.register_pty_writer(&pm_window_id, None);
    let target_window = "tab-1::other-window".to_string();
    insert_test_pane_runtime(&mut runtime, &target_window);
    runtime.register_pty_writer(&target_window, None);

    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let capability = issuer
        .issue(&repo, "pm-session-live")
        .expect("PM capability");
    let grant = issuer.grant_for_test(&capability.token).expect("PM grant");
    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b6";
    let body = "acknowledged after the submit retries";
    let body_sha256 = gwt::pm_registry::pm_delivery_prompt_sha256(body);
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    let ack_receipt_path = receipt_path.clone();
    let ack_body_sha256 = body_sha256.clone();
    let ack = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let prepared = gwt::pm_registry::load_pm_delivery_receipts(&ack_receipt_path)
                .unwrap_or_default()
                .iter()
                .any(|receipt| {
                    receipt.operation_id == operation_id
                        && receipt.status == gwt::pm_registry::PmDeliveryReceiptStatus::Prepared
                });
            if prepared {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Prepared receipt was not committed"
            );
            // test-hygiene: allow-short-duration durable cross-process receipt polling; no receipt notification API exists and the separate deadline bounds it
            thread::sleep(Duration::from_millis(10));
        }
        // Past the two-attempt submit budget (2 x PANE_SUBMIT_SETTLE), still
        // well inside the operation deadline: exactly the observed incident.
        thread::sleep(Duration::from_millis(1_500));
        gwt::pm_registry::finish_pm_delivery_receipt(
            &ack_receipt_path,
            operation_id,
            "other-session",
            &ack_body_sha256,
            gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
            None,
        )
        .expect("late target hook acknowledgement");
    });
    // Long enough for the late acknowledgement, short enough that the test
    // does not sit on the production acceptance window.
    let (responder, result, _cancellation) =
        AgentPmSendResponder::channel_with_acceptance_window(Duration::from_secs(3));

    assert!(runtime
        .authenticated_pm_pane_send_input_events(
            &issuer,
            "origin-client".to_string(),
            grant,
            operation_id,
            &target_window,
            &format!("{body}\r"),
            Some(responder),
        )
        .is_empty());
    let result = result.blocking_recv().expect("terminal delivery result");
    ack.join().expect("ack thread");

    assert!(
        matches!(
            &result,
            BackendEvent::PmMessageSendResult {
                status,
                reason: None,
                ..
            } if status == "delivered"
        ),
        "a delivery acknowledged inside the operation deadline is delivered, not failed: {result:?}"
    );
    assert_eq!(
        gwt::pm_registry::load_pm_delivery_receipts(&receipt_path)
            .expect("delivery receipts")
            .iter()
            .filter(|receipt| receipt.operation_id == operation_id)
            .map(|receipt| receipt.status)
            .collect::<Vec<_>>(),
        vec![
            gwt::pm_registry::PmDeliveryReceiptStatus::Prepared,
            gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
        ],
        "the durable log must agree with the reported outcome"
    );
}

/// Issue #3608 (AC-2/AC-3): a delivery whose acknowledgement never arrives is
/// reported as its own outcome, distinct from a submit that failed, and it
/// never claims the body is staged — both the body and its submit terminator
/// were written to the pane on this path.
#[test]
fn authenticated_pm_send_reports_unverified_without_target_hook_ack() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    insert_test_pane_runtime(&mut runtime, &pm_window_id);
    runtime.register_pty_writer(&pm_window_id, None);
    let target_window = "tab-1::other-window".to_string();
    insert_test_pane_runtime(&mut runtime, &target_window);
    runtime.register_pty_writer(&target_window, None);
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let capability = issuer
        .issue(&repo, "pm-session-live")
        .expect("PM capability");
    let grant = issuer.grant_for_test(&capability.token).expect("PM grant");
    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643af";
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    // This delivery waits out its whole acceptance window; keep that window the
    // test's own rather than the production one.
    let (responder, result, _cancellation) =
        AgentPmSendResponder::channel_with_acceptance_window(Duration::from_secs(3));

    assert!(runtime
        .authenticated_pm_pane_send_input_events(
            &issuer,
            "origin-client".to_string(),
            grant,
            operation_id,
            &target_window,
            "unacknowledged body\r",
            Some(responder),
        )
        .is_empty());
    let result = result.blocking_recv().expect("terminal delivery result");

    assert!(
        matches!(
            &result,
            BackendEvent::PmMessageSendResult {
                status,
                reason: Some(reason),
                ..
            } if status == "unverified"
                && reason.contains("not acknowledged")
                && reason.contains("do not retry")
                && !reason.contains("staged")
        ),
        "an unacknowledged submit is its own outcome and was never staged: {result:?}"
    );
    assert!(gwt::pm_registry::load_pm_delivery_receipts(&receipt_path)
        .expect("delivery receipts")
        .iter()
        .any(|receipt| {
            receipt.operation_id == operation_id
                && receipt.status == gwt::pm_registry::PmDeliveryReceiptStatus::Ambiguous
        }));
}

#[test]
fn authenticated_pm_send_replays_verified_receipt_without_a_live_target() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    insert_test_pane_runtime(&mut runtime, &pm_window_id);
    runtime.register_pty_writer(&pm_window_id, None);
    let target_window = "tab-1::other-window";
    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b2";
    let body = "already verified body";
    let body_sha256 = gwt::pm_registry::pm_delivery_prompt_sha256(body);
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    let prepared = gwt::pm_registry::PmDeliveryReceipt {
        operation_id: operation_id.to_string(),
        recorded_at: "2026-08-13T00:00:00Z".to_string(),
        status: gwt::pm_registry::PmDeliveryReceiptStatus::Prepared,
        principal_session_id: "pm-session-live".to_string(),
        target_window_id: target_window.to_string(),
        target_session_id: "other-session".to_string(),
        body_sha256: body_sha256.clone(),
        reason: None,
    };
    gwt::pm_registry::prepare_pm_delivery_receipt(&receipt_path, &prepared)
        .expect("prepare delivered operation");
    gwt::pm_registry::finish_pm_delivery_receipt(
        &receipt_path,
        operation_id,
        "other-session",
        &body_sha256,
        gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
        None,
    )
    .expect("verify delivered operation");
    runtime.active_agent_sessions.remove(target_window);
    runtime.deregister_pty_writer(target_window);
    runtime.runtimes.remove(target_window);

    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let capability = issuer
        .issue(&repo, "pm-session-live")
        .expect("PM capability");
    let grant = issuer.grant_for_test(&capability.token).expect("PM grant");
    let (responder, result, _cancellation) = AgentPmSendResponder::channel();

    assert!(runtime
        .authenticated_pm_pane_send_input_events(
            &issuer,
            "reconnected-origin".to_string(),
            grant,
            operation_id,
            target_window,
            &format!("{body}\r"),
            Some(responder),
        )
        .is_empty());
    assert!(matches!(
        result.blocking_recv().expect("replayed result"),
        BackendEvent::PmMessageSendResult {
            status,
            reason: None,
            ..
        } if status == "delivered"
    ));
    assert_eq!(
        gwt::pm_registry::load_pm_delivery_receipts(&receipt_path)
            .expect("delivery receipts")
            .iter()
            .filter(|receipt| receipt.operation_id == operation_id)
            .count(),
        2,
        "a response-loss replay must not append or re-inject a Verified operation"
    );
}

#[test]
fn concurrent_same_operation_waiters_share_one_prepared_delivery() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    for window_id in [pm_window_id.as_str(), "tab-1::other-window"] {
        insert_test_pane_runtime(&mut runtime, window_id);
        runtime.register_pty_writer(window_id, None);
    }
    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let capability = issuer
        .issue(&repo, "pm-session-live")
        .expect("PM capability");
    let grant = issuer.grant_for_test(&capability.token).expect("PM grant");
    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b3";
    let body = "one prepared delivery";
    let body_sha256 = gwt::pm_registry::pm_delivery_prompt_sha256(body);
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    let (first_responder, mut first_result, _first_cancellation) = AgentPmSendResponder::channel();
    assert!(runtime
        .authenticated_pm_pane_send_input_events(
            &issuer,
            "first-origin".to_string(),
            grant.clone(),
            operation_id,
            "tab-1::other-window",
            &format!("{body}\r"),
            Some(first_responder),
        )
        .is_empty());
    let deadline = Instant::now() + Duration::from_secs(2);
    while !gwt::pm_registry::load_pm_delivery_receipts(&receipt_path)
        .unwrap_or_default()
        .iter()
        .any(|receipt| {
            receipt.operation_id == operation_id
                && receipt.status == gwt::pm_registry::PmDeliveryReceiptStatus::Prepared
        })
    {
        assert!(
            Instant::now() < deadline,
            "Prepared receipt was not committed"
        );
        // test-hygiene: allow-short-duration durable cross-process receipt polling; no receipt notification API exists and the separate deadline bounds it
        thread::sleep(Duration::from_millis(10));
    }
    let (replay_responder, mut replay_result, _replay_cancellation) =
        AgentPmSendResponder::channel();
    assert!(runtime
        .authenticated_pm_pane_send_input_events(
            &issuer,
            "replay-origin".to_string(),
            grant,
            operation_id,
            "tab-1::other-window",
            &format!("{body}\r"),
            Some(replay_responder),
        )
        .is_empty());
    for result in [&mut first_result, &mut replay_result] {
        assert!(
            matches!(
                result.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "both waiters must remain pending before the exact target acknowledgement"
        );
    }
    assert_eq!(
        gwt::pm_registry::pm_delivery_receipt_for_operation(&receipt_path, operation_id)
            .expect("pending receipt")
            .expect("Prepared operation")
            .status,
        gwt::pm_registry::PmDeliveryReceiptStatus::Prepared
    );
    gwt::pm_registry::finish_pm_delivery_receipt(
        &receipt_path,
        operation_id,
        "other-session",
        &body_sha256,
        gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
        None,
    )
    .expect("exact target acknowledgement");

    for result in [first_result, replay_result] {
        assert!(matches!(
            result.blocking_recv().expect("terminal result"),
            BackendEvent::PmMessageSendResult { status, .. } if status == "delivered"
        ));
    }
    let receipts = gwt::pm_registry::load_pm_delivery_receipts(&receipt_path)
        .expect("delivery receipts")
        .into_iter()
        .filter(|receipt| receipt.operation_id == operation_id)
        .collect::<Vec<_>>();
    assert_eq!(
        receipts
            .iter()
            .filter(|receipt| receipt.status == gwt::pm_registry::PmDeliveryReceiptStatus::Prepared)
            .count(),
        1,
        "same-operation replay must join the durable Prepared operation instead of starting a second body"
    );
    assert_eq!(
        receipts.last().map(|receipt| receipt.status),
        Some(gwt::pm_registry::PmDeliveryReceiptStatus::Verified)
    );
}

#[test]
fn authenticated_pm_send_refuses_foreign_project_without_a_durable_receipt() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    insert_test_pane_runtime(&mut runtime, &pm_window_id);
    runtime.register_pty_writer(&pm_window_id, None);

    let foreign = temp.path().join("foreign-repo");
    fs::create_dir_all(&foreign).expect("create foreign repo");
    init_repo(&foreign);
    let mut foreign_tab = sample_project_tab_with_window_at(
        "tab-foreign",
        "agent-foreign",
        foreign,
        WindowPreset::Codex,
        WindowProcessStatus::Running,
    );
    assert!(foreign_tab
        .workspace
        .set_session_id("agent-foreign", Some("foreign-session".to_string())));
    runtime.tabs.push(foreign_tab);
    let foreign_window_id = "tab-foreign::agent-foreign";
    let mut foreign_session = sample_active_agent_session("tab-foreign", foreign_window_id);
    foreign_session.session_id = "foreign-session".to_string();
    runtime
        .active_agent_sessions
        .insert(foreign_window_id.to_string(), foreign_session);
    insert_test_pane_runtime(&mut runtime, foreign_window_id);
    runtime.register_pty_writer(foreign_window_id, None);

    let issuer = crate::embedded_server::AgentCapabilityIssuer::for_test(
        "http://127.0.0.1:43123/internal/hook-live",
        "ws://127.0.0.1:43124/ws",
        "ws://127.0.0.1:43123/internal/pane-ws",
    );
    let capability = issuer
        .issue(&repo, "pm-session-live")
        .expect("PM capability");
    let grant = issuer.grant_for_test(&capability.token).expect("PM grant");
    let oversized_operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b1";
    let (oversized_responder, oversized_result, _oversized_cancellation) =
        AgentPmSendResponder::channel();
    assert!(runtime
        .authenticated_pm_pane_send_input_events(
            &issuer,
            "origin-client".to_string(),
            grant.clone(),
            oversized_operation_id,
            "tab-1::other-window",
            &format!("{}\r", "x".repeat(16 * 1024 + 1)),
            Some(oversized_responder),
        )
        .is_empty());
    assert!(matches!(
        oversized_result
            .blocking_recv()
            .expect("oversized terminal refusal"),
        BackendEvent::PmMessageSendResult {
            status,
            reason: Some(reason),
            ..
        } if status == "failed" && reason.contains("byte limit")
    ));
    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b0";
    let (responder, result, _cancellation) = AgentPmSendResponder::channel();

    assert!(runtime
        .authenticated_pm_pane_send_input_events(
            &issuer,
            "origin-client".to_string(),
            grant,
            operation_id,
            foreign_window_id,
            "must not cross projects\r",
            Some(responder),
        )
        .is_empty());
    assert!(matches!(
        result.blocking_recv().expect("terminal refusal"),
        BackendEvent::PmMessageSendResult {
            status,
            reason: Some(reason),
            ..
        } if status == "failed" && reason.contains("not an authorized live agent pane")
    ));
    assert!(
        gwt::pm_registry::load_pm_delivery_receipts(
            &gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo),
        )
        .expect("delivery receipts")
        .iter()
        .all(|receipt| receipt.operation_id != operation_id),
        "a cross-project refusal must not consume durable receipt storage"
    );
}

#[test]
fn pm_pane_send_gate_refuses_everyone_but_the_live_registered_pm() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (_repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    let target_window = "tab-1::other-window".to_string();

    let refusal_of = |events: &[OutboundEvent]| -> String {
        events
            .iter()
            .find_map(|event| match &event.event {
                BackendEvent::PaneSendResult {
                    ok: false,
                    error: Some(error),
                    ..
                } => Some(error.clone()),
                _ => None,
            })
            .expect("a refused send must reply with a typed error")
    };

    // A non-PM session is refused: the self-only world stays intact and the
    // privileged path does not open for it.
    let events = runtime.pm_pane_send_input_events(
        "client-1".to_string(),
        "other-session",
        &target_window,
        "hello\r",
    );
    assert!(
        refusal_of(&events).contains("not the registered PM"),
        "foreign principals must be refused"
    );

    // An unknown window is refused before any principal work.
    let events = runtime.pm_pane_send_input_events(
        "client-1".to_string(),
        "pm-session-live",
        "tab-9::ghost",
        "hello\r",
    );
    assert!(refusal_of(&events).contains("unknown pane"));

    // The live registered PM passes the gate; with no PTY runtime in the
    // harness the write itself reports the missing runtime, which proves the
    // authorized injection path was reached (not a principal refusal).
    let events = runtime.pm_pane_send_input_events(
        "client-1".to_string(),
        "pm-session-live",
        &target_window,
        "hello\r",
    );
    assert!(
        refusal_of(&events).contains("no live runtime"),
        "the live PM must reach the injection path"
    );

    // A stale registration (PM pane gone) is refused at the liveness check.
    runtime.active_agent_sessions.remove(&pm_window_id);
    let events = runtime.pm_pane_send_input_events(
        "client-1".to_string(),
        "pm-session-live",
        &target_window,
        "hello\r",
    );
    assert!(
        refusal_of(&events).contains("no live pane"),
        "a stale PM registration must not deliver"
    );
}

/// SPEC-3864 T-006: install detection must not sit on the startup critical
/// path. The fake `agy` cannot finish its version probe until the test drops
/// a sentinel file *after* `LaunchWizardMemoryCache::load` has returned, so a
/// load that waited for detection would only ever see the probe killed at
/// its deadline (no version). A non-blocking load lets the probe complete
/// and the first wizard access joins the background result.
#[cfg(unix)]
#[test]
fn launch_wizard_memory_cache_load_does_not_block_on_agent_detection() {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).expect("create bin");
    let sentinel = temp.path().join("probe-may-finish");
    let gated_agy = bin.join("agy");
    fs::write(
        &gated_agy,
        format!(
            "#!/bin/sh\nwhile [ ! -e '{}' ]; do sleep 0.05; done\nprintf '9.9.9\\n'\n",
            sentinel.display()
        ),
    )
    .expect("write gated agy");
    fs::set_permissions(&gated_agy, fs::Permissions::from_mode(0o755)).expect("chmod gated agy");
    let _path = prepend_tool_parent_to_path(&gated_agy);
    let _no_custom = ScopedEnvVar::set(gwt_agent::DISABLE_GLOBAL_CUSTOM_AGENTS_ENV, "1");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");

    let cache = LaunchWizardMemoryCache::load(&sessions_dir);
    // Only now may the probe finish: a blocking load would already have hit
    // the probe deadline and lost the version.
    fs::write(&sentinel, b"go").expect("release gated probe");

    let options = cache.agent_options();
    let agy = options
        .iter()
        .find(|option| option.id == "agy")
        .expect("Antigravity option");
    assert!(
        agy.available,
        "background detection must still feed availability"
    );
    assert_eq!(
        agy.installed_version.as_deref(),
        Some("9.9.9"),
        "load must return before the probe completes"
    );
}

#[test]
fn pm_delivery_refuses_self_with_durable_receipt() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    assert_pm_delivery_refused(&temp, false, false);
}

#[test]
fn pm_delivery_refuses_pm_role_with_durable_receipt() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    assert_pm_delivery_refused(&temp, true, false);
}

#[test]
fn pm_delivery_replay_preserves_pending_refusal() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    assert_pm_delivery_refused(&temp, false, true);
}
