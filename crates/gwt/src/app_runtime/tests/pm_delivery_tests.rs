use super::*;

// Every resource below is captured from the canonical actor. This fixture
// observes real writer entries and never installs a GUI writer or Session.
pub(super) struct PmDeliveryFixture {
    pub repo: PathBuf,
    pub owner: Arc<gwt::project_runtime::test_support::ProjectOwnerHarness>,
    pub pm: gwt::project_runtime::test_support::OwnerRuntimeProbe,
    pub target: gwt::project_runtime::test_support::OwnerRuntimeProbe,
    clients: Mutex<Vec<gwt::project_runtime::test_support::OwnerAgentClient>>,
}

pub(super) fn pm_delivery_owner_fixture(temp: &Path) -> PmDeliveryFixture {
    use super::pane_control_tests::{
        control_launch_config, control_owner_agent, control_owner_launch, control_register_pm,
    };
    let repo = temp.join("project");
    super::pane_control_tests::control_init_repo(&repo);
    seed_owner_fixture_roster(&repo, 8);
    configure_owner_fixture_tools(&repo, &["pane-contract-fixture", "codex"]);
    let (owner, pm) = control_owner_agent(&repo, "pm", 42);
    control_register_pm(&repo, &pm);
    let runner = temp.join(if cfg!(windows) {
        "pm-codex.cmd"
    } else {
        "pm-codex"
    });
    #[cfg(not(windows))]
    {
        fs::write(&runner, "#!/bin/sh\ncase \"$*\" in *--version*) printf '1.2.3\\n'; exit 0;; *'features list'*) exit 0;; esac\nstty -echo -icanon -isig; exec cat >/dev/null\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    fs::write(&runner, "@echo off\r\nif \"%~1\"==\"--version\" (echo 1.2.3& exit /b 0)\r\nif \"%~1\"==\"features\" exit /b 0\r\npowershell -NoProfile -Command \"while ($null -ne [Console]::ReadLine()) {}\"\r\n").unwrap();
    let location = control_launch_config(&repo, 43);
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(location.working_dir.unwrap())
        .branch(location.branch.unwrap())
        .base_branch(location.base_branch.unwrap())
        .build();
    config.command = runner.display().to_string();
    config.linked_issue_number = Some(43);
    config.explicit_follow_up = true;
    let target = control_owner_launch(&owner, "target", config);
    owner_runtime_status_event(&owner, &target, "Working", "SessionStart");
    let target = owner_fixture_block_on(owner.probe(&target.id, target.incarnation)).unwrap();
    assert!(
        matches!(
            target.status,
            WindowProcessStatus::Running | WindowProcessStatus::Idle | WindowProcessStatus::Waiting
        ),
        "positive PM delivery begins after the actual provider SessionStart: {:?}",
        target.status
    );
    assert_eq!(
        target.session.as_ref().unwrap().agent_id,
        gwt_agent::AgentId::Codex
    );
    PmDeliveryFixture {
        repo,
        owner,
        pm,
        target,
        clients: Mutex::new(Vec::new()),
    }
}

impl PmDeliveryFixture {
    pub fn send(
        &self,
        client: String,
        operation_id: &str,
        id: &str,
        text: &str,
        responder: Option<AgentPmSendResponder>,
    ) -> Vec<serde_json::Value> {
        self.send_as(&self.pm, client, operation_id, id, text, responder)
    }
    pub fn send_as(
        &self,
        caller: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
        client: String,
        operation_id: &str,
        id: &str,
        text: &str,
        responder: Option<AgentPmSendResponder>,
    ) -> Vec<serde_json::Value> {
        let client = owner_fixture_block_on(
            self.owner
                .agent_client(&format!("{client}-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        client.enqueue(
            caller.grant.as_ref().unwrap().clone(),
            AgentFrontendRequest::PmSendInput {
                operation_id: operation_id.into(),
                window_id: id.into(),
                text: text.into(),
                responder,
            },
        );
        let immediate = client.drain().unwrap();
        assert_eq!(immediate.close, None);
        self.clients.lock().unwrap().push(client);
        immediate.events
    }
    pub fn close_target(&self) {
        super::pane_control_tests::control_close(&self.owner, &self.target);
    }
    pub fn close_pm(&self) {
        super::pane_control_tests::control_close(&self.owner, &self.pm);
    }
}

impl Drop for PmDeliveryFixture {
    fn drop(&mut self) {
        let barrier = owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline direct PM replies and the real owner worker must settle before exact process cleanup
            tokio::time::timeout(
                gwt_core::deadline_budget::HANG_GUARD,
                self.owner.agent_dispatch_barrier(),
            )
            .await
        });
        let mut clean = matches!(barrier, Ok(Ok(())));
        for client in self.clients.lock().unwrap().drain(..) {
            clean &= client
                .drain()
                .is_ok_and(|reply| reply.close.is_none() && reply.events.is_empty());
        }
        for pane in [&self.target, &self.pm] {
            if pane.pty.try_wait().is_ok_and(|exit| exit.is_none()) {
                clean &= owner_fixture_block_on(self.owner.command(
                    gwt::project_runtime::events::OwnerCommand::Close {
                        id: pane.id.clone(),
                        incarnation: pane.incarnation,
                    },
                ))
                .is_ok();
            }
        }
        if !thread::panicking() {
            assert!(clean, "PM direct responses must not duplicate on ClientHub and every exact fixture child must reap");
        }
    }
}

/// Issue #5839: ordinary notification delivery and the authenticated PM route
/// must report failure instead of feeding Codex's current Find surface.
#[test]
fn transcript_overlay_reports_failed_pane_and_pm_ack_without_staging_body() {
    use gwt::project_runtime::events::OwnerCommand;
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture(temp.path());
    let target = &fixture.target;
    target
        .pane
        .lock()
        .unwrap()
        .process_bytes(b"\x1b[2J\x1b[H/ T R A N S C R I P T / / /\x1b[21;1HFind: ");
    let ordinary = owner_fixture_block_on(fixture.owner.command(OwnerCommand::SendInput {
        id: target.id.clone(),
        incarnation: target.incarnation,
        text: "watchdog notification\r".into(),
    }));
    assert!(ordinary.is_err_and(|error| error.contains("Transcript")));
    let revision = target.pty.ordinary_input_revision();
    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d79135765839";
    let (responder, result, cancellation) = AgentPmSendResponder::channel();
    fixture.send(
        "client".into(),
        operation_id,
        &target.id,
        "PM notification\r",
        Some(responder),
    );
    assert!(
        matches!(result.blocking_recv().expect("terminal PM result"),
        BackendEvent::PmMessageSendResult { status, reason: Some(reason), .. }
            if status == "failed" && reason.contains("Transcript"))
    );
    assert!(
        !cancellation.cancel(),
        "overlay refusal must precede physical mutation"
    );
    assert_eq!(target.pty.ordinary_input_revision(), revision);
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&fixture.repo);
    let receipt = gwt::pm_registry::pm_delivery_receipt_for_operation(&receipt_path, operation_id)
        .expect("receipt read")
        .expect("durable refusal");
    assert_eq!(
        receipt.status,
        gwt::pm_registry::PmDeliveryReceiptStatus::Refused
    );
    owner_fixture_block_on(fixture.owner.command(OwnerCommand::Input {
        id: target.id.clone(),
        incarnation: target.incarnation,
        data: b"\x1b".to_vec(),
    }))
    .expect("raw recovery keys remain available");
    target
        .pane
        .lock()
        .unwrap()
        .process_bytes(b"\x1b[2J\x1b[Hcodex\r\n> ");
    owner_fixture_block_on(fixture.owner.command(OwnerCommand::SendInput {
        id: target.id.clone(),
        incarnation: target.incarnation,
        text: "recovered notification\r".into(),
    }))
    .expect("ordinary delivery recovers after the overlay closes");
}

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
    let _owner_home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture(temp.path());
    let repo = fixture.repo.clone();
    let target_window = fixture.target.id.clone();
    let target_session_id = fixture.target.session.as_ref().unwrap().id.clone();

    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643ae";
    let body = "verify this exact body";
    let body_sha256 = gwt::pm_registry::pm_delivery_prompt_sha256(body);
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    let ack_receipt_path = receipt_path.clone();
    let ack_body_sha256 = body_sha256.clone();
    let (prepared_tx, prepared_rx) = mpsc::sync_channel(1);
    let (release_ack_tx, release_ack_rx) = mpsc::sync_channel(1);
    let ack = thread::spawn(move || {
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual durable Prepared receipt; the receipt establishes ordering
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
                // test-hygiene: allow-wall-clock-deadline bound an actual owner-worker receipt hang, never assert a scheduling delay
                Instant::now() < deadline,
                "Prepared receipt was not committed"
            );
            // test-hygiene: allow-short-duration durable cross-process receipt polling; no receipt notification API exists and the separate deadline bounds it
            thread::sleep(Duration::from_millis(10));
        }
        prepared_tx.send(()).expect("announce Prepared receipt");
        release_ack_rx
            .recv_timeout(Duration::from_secs(3)) // test-hygiene: allow-wall-clock-deadline hang guard for an actual receipt or explicit ACK-release barrier
            .expect("release the exact hook acknowledgement");
        gwt::pm_registry::finish_pm_delivery_receipt(
            &ack_receipt_path,
            operation_id,
            &target_session_id,
            &ack_body_sha256,
            gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
            None,
        )
        .expect("exact target hook acknowledgement");
    });
    let (responder, mut result, _cancellation) = AgentPmSendResponder::channel();

    assert!(fixture
        .send(
            "origin-client".to_string(),
            operation_id,
            &target_window,
            &format!("{body}\r"),
            Some(responder)
        )
        .is_empty());
    prepared_rx
        .recv_timeout(Duration::from_secs(3)) // test-hygiene: allow-wall-clock-deadline hang guard for an actual receipt or explicit ACK-release barrier
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
    let _owner_home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture(temp.path());
    let repo = fixture.repo.clone();
    let target_window = fixture.target.id.clone();
    let target_session_id = fixture.target.session.as_ref().unwrap().id.clone();

    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b6";
    let body = "acknowledged after the submit retries";
    let body_sha256 = gwt::pm_registry::pm_delivery_prompt_sha256(body);
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    let ack_receipt_path = receipt_path.clone();
    let ack_body_sha256 = body_sha256.clone();
    let ack = thread::spawn(move || {
        // test-hygiene: allow-wall-clock-deadline hang guard for the actual durable Prepared receipt; the receipt establishes ordering
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
                // test-hygiene: allow-wall-clock-deadline bound an actual owner-worker receipt hang, never assert a scheduling delay
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
            &target_session_id,
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

    assert!(fixture
        .send(
            "origin-client".to_string(),
            operation_id,
            &target_window,
            &format!("{body}\r"),
            Some(responder)
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

/// Issue #3897: successful body/submit writes without an exact hook ACK do
/// not prove delivery. Report uncertainty and preserve a later human draft.
#[test]
fn authenticated_pm_send_reports_unconfirmed_without_target_hook_ack() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _owner_home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture(temp.path());
    let repo = fixture.repo.clone();
    let target_window = fixture.target.id.clone();

    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643af";
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    // This delivery waits out its whole acceptance window; keep that window the
    // test's own rather than the production one.
    let (responder, result, _cancellation) =
        AgentPmSendResponder::channel_with_acceptance_window(Duration::from_secs(3));

    assert!(fixture
        .send(
            "origin-client".to_string(),
            operation_id,
            &target_window,
            "unacknowledged body\r",
            Some(responder)
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
            } if status == "unconfirmed"
                && reason.contains("not acknowledged")
                && reason.contains("same operation")
        ),
        "missing acknowledgement must not claim delivery: {result:?}"
    );
    assert!(gwt::pm_registry::load_pm_delivery_receipts(&receipt_path)
        .expect("delivery receipts")
        .iter()
        .any(|receipt| {
            receipt.operation_id == operation_id
                && receipt.status == gwt::pm_registry::PmDeliveryReceiptStatus::Unconfirmed
        }));

    // An intervening human draft revokes submit recovery, even if the old
    // operation has a full-body guard. No new input transaction may start.
    let entry = fixture.target.writer.as_ref().unwrap().clone();
    let original_transaction = entry
        .pm_delivery_replay
        .lock()
        .unwrap()
        .as_ref()
        .expect("full body recovery guard")
        .transaction_revision;
    owner_fixture_block_on(fixture.owner.command(
        gwt::project_runtime::events::OwnerCommand::Input {
            id: fixture.target.id.clone(),
            incarnation: fixture.target.incarnation,
            data: b"human draft".to_vec(),
        },
    ))
    .unwrap();
    let (responder, result, _cancellation) = AgentPmSendResponder::channel();
    fixture.send(
        "origin-client".into(),
        operation_id,
        &target_window,
        "unacknowledged body\r",
        Some(responder),
    );
    assert!(matches!(result.blocking_recv().unwrap(),
        BackendEvent::PmMessageSendResult { status, reason: Some(reason), .. }
            if status == "unconfirmed" && reason.contains("readback-only")));
    assert!(entry.handle.has_unsent_user_input());
    let reservation = entry.handle.reserve_input_transaction().unwrap();
    assert_eq!(
        reservation.previous_transaction_revision(),
        original_transaction,
        "same-operation retry must not submit a later human draft"
    );
}

#[test]
fn authenticated_pm_send_recovers_the_same_operation_after_dropped_submit() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture(temp.path());
    let repo = fixture.repo.clone();
    let target = fixture.target.id.as_str();
    let target_session_id = fixture.target.session.as_ref().unwrap().id.clone();

    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d79135763898";
    let body = "recover an idle composer";
    let (responder, result, _cancellation) =
        AgentPmSendResponder::channel_with_acceptance_window(Duration::from_secs(2));
    fixture.send(
        "client".into(),
        operation_id,
        target,
        &format!("{body}\r"),
        Some(responder),
    );
    assert!(matches!(result.blocking_recv().unwrap(),
        BackendEvent::PmMessageSendResult { status, .. } if status == "unconfirmed"));
    let entry = fixture.target.writer.as_ref().unwrap().clone();
    let original_transaction = entry
        .pm_delivery_replay
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .transaction_revision;
    let path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    let (responder, result, recovery_cancellation) = AgentPmSendResponder::channel();
    fixture.send(
        "client".into(),
        operation_id,
        target,
        &format!("{body}\r"),
        Some(responder),
    );
    assert!(matches!(result.blocking_recv().unwrap(),
        BackendEvent::PmMessageSendResult { status, reason: Some(reason), .. }
            if status == "unconfirmed" && reason.contains("Further calls are readback-only")));
    assert!(
        recovery_cancellation.cancel(),
        "the bounded recovery must reach its input commit"
    );
    assert!(entry.pm_delivery_replay.lock().unwrap().is_none());
    gwt::pm_registry::finish_pm_delivery_receipt(
        &path,
        operation_id,
        &target_session_id,
        &gwt::pm_registry::pm_delivery_prompt_sha256(body),
        gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
        None,
    )
    .unwrap();
    let (responder, result, _cancellation) = AgentPmSendResponder::channel();
    fixture.send(
        "client".into(),
        operation_id,
        target,
        &format!("{body}\r"),
        Some(responder),
    );
    assert!(matches!(result.blocking_recv().unwrap(),
        BackendEvent::PmMessageSendResult { status, .. } if status == "delivered"));
    assert_eq!(
        entry
            .handle
            .reserve_input_transaction()
            .unwrap()
            .previous_transaction_revision(),
        original_transaction + 1,
        "only one bounded replay may claim the original composer"
    );
}

#[test]
fn authenticated_pm_send_rechecks_ambiguous_receipt_when_target_is_gone() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture(temp.path());
    let repo = fixture.repo.clone();
    let target = fixture.target.id.as_str();
    let pm_session_id = fixture.pm.session.as_ref().unwrap().id.clone();
    let target_session_id = fixture.target.session.as_ref().unwrap().id.clone();
    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d79135763897";
    let body = "recover the same operation";
    let path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    let prepared = gwt::pm_registry::PmDeliveryReceipt {
        operation_id: operation_id.into(),
        recorded_at: "2026-10-22T00:00:00Z".into(),
        status: gwt::pm_registry::PmDeliveryReceiptStatus::Prepared,
        principal_session_id: pm_session_id.clone(),
        target_window_id: target.into(),
        target_session_id: target_session_id.clone(),
        body_sha256: gwt::pm_registry::pm_delivery_prompt_sha256(body),
        reason: None,
    };
    gwt::pm_registry::prepare_pm_delivery_receipt(&path, &prepared).unwrap();
    gwt::pm_registry::finish_pm_delivery_receipt(
        &path,
        operation_id,
        &target_session_id,
        &prepared.body_sha256,
        gwt::pm_registry::PmDeliveryReceiptStatus::Ambiguous,
        Some("body_staged_or_unknown"),
    )
    .unwrap();

    let send = || {
        let (responder, result, _cancellation) =
            AgentPmSendResponder::channel_with_acceptance_window(Duration::from_secs(1));
        fixture.send(
            "client".into(),
            operation_id,
            target,
            &format!("{body}\r"),
            Some(responder),
        );
        result.blocking_recv().unwrap()
    };
    assert!(matches!(send(), BackendEvent::PmMessageSendResult {
        status, reason: Some(reason), .. } if status == "unconfirmed" && reason.contains("readback-only")));
    let entry = fixture.target.writer.as_ref().unwrap().clone();
    let reservation = entry.handle.reserve_input_transaction().unwrap();
    assert_eq!(
        reservation.previous_transaction_revision(),
        0,
        "a legacy/partial-write receipt alone must never authorize submit"
    );
    drop(reservation);
    fixture.close_target();
    assert!(matches!(send(), BackendEvent::PmMessageSendResult { status, .. } if status == "lost"));
    assert_eq!(
        gwt::pm_registry::finish_pm_delivery_receipt(
            &path,
            operation_id,
            &target_session_id,
            &prepared.body_sha256,
            gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
            None,
        )
        .unwrap(),
        gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
    );
    assert!(
        matches!(send(), BackendEvent::PmMessageSendResult { status, .. } if status == "delivered")
    );
}

#[test]
fn authenticated_pm_send_replays_verified_receipt_without_a_live_target() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _owner_home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture(temp.path());
    let repo = fixture.repo.clone();
    let target_window = fixture.target.id.clone();
    let pm_session_id = fixture.pm.session.as_ref().unwrap().id.clone();
    let target_session_id = fixture.target.session.as_ref().unwrap().id.clone();
    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b2";
    let body = "already verified body";
    let body_sha256 = gwt::pm_registry::pm_delivery_prompt_sha256(body);
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    let prepared = gwt::pm_registry::PmDeliveryReceipt {
        operation_id: operation_id.to_string(),
        recorded_at: "2026-08-13T00:00:00Z".to_string(),
        status: gwt::pm_registry::PmDeliveryReceiptStatus::Prepared,
        principal_session_id: pm_session_id.clone(),
        target_window_id: target_window.to_string(),
        target_session_id: target_session_id.clone(),
        body_sha256: body_sha256.clone(),
        reason: None,
    };
    gwt::pm_registry::prepare_pm_delivery_receipt(&receipt_path, &prepared)
        .expect("prepare delivered operation");
    gwt::pm_registry::finish_pm_delivery_receipt(
        &receipt_path,
        operation_id,
        &target_session_id,
        &body_sha256,
        gwt::pm_registry::PmDeliveryReceiptStatus::Verified,
        None,
    )
    .expect("verify delivered operation");
    fixture.close_target();

    let (responder, result, _cancellation) = AgentPmSendResponder::channel();

    assert!(fixture
        .send(
            "reconnected-origin".to_string(),
            operation_id,
            &target_window,
            &format!("{body}\r"),
            Some(responder)
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
    let _owner_home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture(temp.path());
    let repo = fixture.repo.clone();
    let target = fixture.target.id.as_str();
    let target_session_id = fixture.target.session.as_ref().unwrap().id.clone();

    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b3";
    let body = "one prepared delivery";
    let body_sha256 = gwt::pm_registry::pm_delivery_prompt_sha256(body);
    let receipt_path = gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&repo);
    let (first_responder, mut first_result, _first_cancellation) = AgentPmSendResponder::channel();
    assert!(fixture
        .send(
            "first-origin".to_string(),
            operation_id,
            target,
            &format!("{body}\r"),
            Some(first_responder)
        )
        .is_empty());
    // test-hygiene: allow-wall-clock-deadline hang guard for the actual durable Prepared receipt; the receipt establishes ordering
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
            // test-hygiene: allow-wall-clock-deadline bound an actual owner-worker receipt hang, never assert a scheduling delay
            Instant::now() < deadline,
            "Prepared receipt was not committed"
        );
        // test-hygiene: allow-short-duration durable cross-process receipt polling; no receipt notification API exists and the separate deadline bounds it
        thread::sleep(Duration::from_millis(10));
    }
    let (replay_responder, mut replay_result, _replay_cancellation) =
        AgentPmSendResponder::channel();
    assert!(fixture
        .send(
            "replay-origin".to_string(),
            operation_id,
            target,
            &format!("{body}\r"),
            Some(replay_responder)
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
        &target_session_id,
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
    use super::pane_control_tests::{
        control_close, control_owner_agent, control_seed_foreign_roster,
    };
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture(temp.path());
    let foreign_repo = temp.path().join("foreign");
    control_seed_foreign_roster(&foreign_repo);
    let (foreign_owner, foreign) = control_owner_agent(&foreign_repo, "foreign", 44);
    let oversized_operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b1";
    let (responder, result, cancellation) = AgentPmSendResponder::channel();
    fixture.send(
        "origin".into(),
        oversized_operation_id,
        &fixture.target.id,
        &format!("{}\r", "x".repeat(16 * 1024 + 1)),
        Some(responder),
    );
    assert!(
        matches!(result.blocking_recv().expect("oversized terminal refusal"),
        BackendEvent::PmMessageSendResult { status, reason: Some(reason), .. }
            if status == "failed" && reason.contains("byte limit"))
    );
    assert!(!cancellation.cancel());
    let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b0";
    let revision = foreign.pty.ordinary_input_revision();
    let (responder, result, cancellation) = AgentPmSendResponder::channel();
    fixture.send(
        "origin".into(),
        operation_id,
        &foreign.id,
        "must not cross projects\r",
        Some(responder),
    );
    assert!(matches!(result.blocking_recv().expect("terminal refusal"),
        BackendEvent::PmMessageSendResult { status, reason: Some(reason), .. }
            if status == "failed" && reason.contains("not an authorized live agent pane")));
    assert!(!cancellation.cancel());
    assert_eq!(foreign.pty.ordinary_input_revision(), revision);
    assert!(
        gwt::pm_registry::load_pm_delivery_receipts(
            &gwt::pm_registry::pm_delivery_receipts_path_for_repo_path(&fixture.repo)
        )
        .expect("delivery receipts")
        .iter()
        .all(|receipt| receipt.operation_id != operation_id
            && receipt.operation_id != oversized_operation_id),
        "refusals before admission must not consume durable receipt storage"
    );
    control_close(&foreign_owner, &foreign);
}

#[test]
fn pm_pane_send_gate_refuses_everyone_but_the_live_registered_pm() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture(temp.path());
    let original_transaction = fixture
        .target
        .pty
        .reserve_input_transaction()
        .unwrap()
        .previous_transaction_revision();
    let (responder, result, cancellation) = AgentPmSendResponder::channel();
    fixture.send_as(
        &fixture.target,
        "ordinary".into(),
        &uuid::Uuid::new_v4().to_string(),
        &fixture.target.id,
        "hello\r",
        Some(responder),
    );
    assert!(matches!(result.blocking_recv().unwrap(),
        BackendEvent::PmMessageSendResult { status, reason: Some(reason), .. }
            if status == "failed" && reason.contains("not the live registered PM")));
    assert!(!cancellation.cancel());
    let (responder, result, cancellation) = AgentPmSendResponder::channel();
    fixture.send(
        "unknown".into(),
        &uuid::Uuid::new_v4().to_string(),
        "tab-9::ghost",
        "hello\r",
        Some(responder),
    );
    assert!(matches!(result.blocking_recv().unwrap(),
        BackendEvent::PmMessageSendResult { status, reason: Some(reason), .. }
            if status == "failed" && reason.contains("not an authorized live agent pane")));
    assert!(!cancellation.cancel());
    assert_eq!(
        fixture
            .target
            .pty
            .reserve_input_transaction()
            .unwrap()
            .previous_transaction_revision(),
        original_transaction + 1,
        "refused callers and targets must not reserve a protected write"
    );
    let (responder, result, cancellation) =
        AgentPmSendResponder::channel_with_acceptance_window(Duration::from_secs(2));
    fixture.send(
        "pm".into(),
        &uuid::Uuid::new_v4().to_string(),
        &fixture.target.id,
        "hello\r",
        Some(responder),
    );
    assert!(matches!(result.blocking_recv().unwrap(),
        BackendEvent::PmMessageSendResult { status, .. } if status == "unconfirmed"));
    assert!(
        cancellation.cancel(),
        "the live registered PM reached the actual protected input commit"
    );
    fixture.close_pm();
    let stale = owner_fixture_block_on(fixture.owner.agent_client("stale-pm")).unwrap();
    let reply = owner_fixture_block_on(stale.request(
        fixture.pm.grant.as_ref().unwrap().clone(),
        AgentFrontendRequest::PmSendInput {
            operation_id: uuid::Uuid::new_v4().to_string(),
            window_id: fixture.target.id.clone(),
            text: "must not write\r".into(),
            responder: None,
        },
    ))
    .unwrap();
    assert!(
        reply.close.is_some(),
        "a retired PM capability must fail closed at the authenticated owner"
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
