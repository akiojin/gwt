use super::*;

const GROK_PERMISSION: &str = "\x1b[2J\x1b[HRun command?\r\n1. Yes, and don't ask again for anything\r\n2. No\x1b[24;1H1/2 select  Tab next option  Ctrl+O always-approve  Ctrl+C cancel\x1b[?25l";

fn grok_visible_composer(cols: usize) -> String {
    format!(
        "\x1b[2J\x1b[20;1H╭{}╮\x1b[21;1H  ❯ \x1b[22;1H╰{}╯\x1b[24;1HShift+Tab mode  Ctrl+K commands\x1b[21;5H\x1b[?25h",
        "─".repeat(cols - 2), "─".repeat(cols - 2),
    )
}

fn grok_hidden_composer(cols: usize) -> String {
    // Root chrome reserves two left padding columns, the prefix, and one
    // right padding column. Exact-fill omits the native textarea caret.
    format!(
        "\x1b[2J\x1b[20;1H╭{}╮\x1b[21;1H  ❯ {}\x1b[22;1H╰{}╯\x1b[24;1HEnter send  Shift+Tab mode\x1b[?25l",
        "─".repeat(cols - 2), "x".repeat(cols - 5), "─".repeat(cols - 2),
    )
}

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
    pm_delivery_owner_fixture_with_target_role(temp, false)
}

pub(super) fn pm_delivery_owner_fixture_with_target_role(
    temp: &Path,
    pm_role_target: bool,
) -> PmDeliveryFixture {
    pm_delivery_owner_fixture_for_agent(temp, gwt_agent::AgentId::Codex, pm_role_target)
}

fn pm_delivery_owner_fixture_for_agent(
    temp: &Path,
    agent_id: gwt_agent::AgentId,
    pm_role_target: bool,
) -> PmDeliveryFixture {
    use super::pane_control_tests::{
        control_launch_config, control_owner_agent, control_owner_launch, control_register_pm,
    };
    let repo = temp.join("project");
    super::pane_control_tests::control_init_repo(&repo);
    seed_owner_fixture_roster(&repo, 8);
    configure_owner_fixture_tools(&repo, &["pane-contract-fixture", agent_id.command()]);
    let (owner, pm) = control_owner_agent(&repo, "pm", 42);
    if !pm_role_target {
        control_register_pm(&repo, &pm);
    }
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
    let mut config = if pm_role_target {
        let worktree = create_detached_pm_worktree_fixture(&repo);
        gwt::pm_registry::mutate_pm_prefs(
            &gwt::pm_registry::pm_prefs_path_for_repo_path(&repo),
            |prefs| {
                prefs.settings.launch_profile = Some(gwt::pm_registry::PmLaunchProfile {
                    agent_id: agent_id.command().into(),
                    model: None,
                    reasoning: None,
                    version: None,
                    codex_speed: None,
                    fast_mode: None,
                });
            },
        )
        .unwrap();
        AppRuntime::pm_new_launch_config(&repo, &worktree).unwrap()
    } else {
        let location = control_launch_config(&repo, 43);
        let mut config = gwt_agent::AgentLaunchBuilder::new(agent_id.clone())
            .working_dir(location.working_dir.unwrap())
            .branch(location.branch.unwrap())
            .base_branch(location.base_branch.unwrap())
            .build();
        config.linked_issue_number = Some(43);
        config.explicit_follow_up = true;
        config
    };
    config.command = runner.display().to_string();
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
    assert_eq!(target.session.as_ref().unwrap().agent_id, agent_id);
    if pm_role_target {
        // The real PM retires its own authority; pm.stop keeps its live pane.
        // Registering the legacy principal below must not replace a live PM.
        target.with_agent_environment(|| {
            let mut env = gwt::cli::TestEnv::new(repo.clone());
            let code = gwt::cli::dispatch_json_input(
                &mut env,
                "gwtd",
                &serde_json::json!({
                    "schema_version": 1,
                    "operation": "pm.stop",
                    "params": { "project_root": repo },
                })
                .to_string(),
            );
            assert_eq!(code, 0, "pm.stop: {}", String::from_utf8_lossy(&env.stdout));
            let envelope: serde_json::Value = serde_json::from_slice(&env.stdout).unwrap();
            let report: serde_json::Value =
                serde_json::from_str(envelope["output"].as_str().unwrap()).unwrap();
            assert_eq!(
                report["stopped_session_id"],
                target.session.as_ref().unwrap().id
            );
            assert_eq!(report["registration_cleared"], true);
        });
        assert!(
            gwt::pm_registry::load_pm_prefs(&gwt::pm_registry::pm_prefs_path_for_repo_path(&repo))
                .unwrap()
                .registration
                .is_none(),
            "the real pm.stop must clear registration before the legacy principal is registered"
        );
        assert!(
            target.pty.try_wait().unwrap().is_none(),
            "pm.stop retains the protected live PM-role pane"
        );
        control_register_pm(&repo, &pm);
    }
    if agent_id == gwt_agent::AgentId::GrokBuild {
        target
            .pane
            .lock()
            .unwrap()
            .process_bytes(grok_visible_composer(80).as_bytes());
    }
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
    assert_pm_send_waits_for_exact_target_hook_ack(gwt_agent::AgentId::Codex);
}

/// Issue #3706 AC-3: Grok waits for its exact accepted-turn receipt.
#[test]
fn issue_3706_grok_pm_send_waits_for_exact_target_hook_ack() {
    assert_pm_send_waits_for_exact_target_hook_ack(gwt_agent::AgentId::GrokBuild);
}

#[test]
fn issue_3706_grok_pm_send_refuses_permission_and_unknown_surfaces_before_body() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _user_home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _grok_home = ScopedEnvVar::set("GROK_HOME", temp.path().join(".grok"));
    let fixture =
        pm_delivery_owner_fixture_for_agent(temp.path(), gwt_agent::AgentId::GrokBuild, false);
    // Upstream Options mode has its own shortcut footer and owns Enter.
    let hidden_root = grok_hidden_composer(80);
    let feedback = format!(
        "{}\x1b[11;1H  ❯ previous prompt\x1b[8;33H┌ Feedback ┐\x1b[11;33H│  Tell us what happened\x1b[15;33H└─────────┘\x1b[11;36H\x1b[?25h",
        grok_visible_composer(205),
    );
    for (surface, cols) in [
        ("\x1b[2J\x1b[Hunknown surface\x1b[21;3H", 80),
        (GROK_PERMISSION, 80),
        (hidden_root.as_str(), 80),
        (feedback.as_str(), 205),
    ] {
        {
            let mut pane = fixture.target.pane.lock().unwrap();
            pane.resize(cols, 24).unwrap();
            pane.process_bytes(surface.as_bytes());
        }
        let operation = uuid::Uuid::new_v4().to_string();
        let (responder, result, cancellation) =
            AgentPmSendResponder::channel_with_acceptance_window(Duration::from_secs(2));
        fixture.send(
            "unsafe-surface".into(),
            &operation,
            &fixture.target.id,
            "a PM notification must not approve anything\r",
            Some(responder),
        );
        assert!(matches!(result.blocking_recv().unwrap(),
            BackendEvent::PmMessageSendResult { status, reason: Some(reason), .. }
                if status == "failed" && reason.contains("Grok composer")));
        assert!(
            !cancellation.cancel(),
            "surface refusal precedes body mutation"
        );
        assert!(fixture
            .target
            .writer
            .as_ref()
            .unwrap()
            .pm_delivery_replay
            .lock()
            .unwrap()
            .is_none());
    }
    // Ordinary keys remain available for a human to recover the focused card.
    fixture
        .target
        .pty
        .write_input(b"\x1b")
        .expect("raw recovery input");
}

#[test]
fn issue_3706_grok_submit_rechecks_the_current_composer_surface() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _user_home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _grok_home = ScopedEnvVar::set("GROK_HOME", temp.path().join(".grok"));
    let fixture =
        pm_delivery_owner_fixture_for_agent(temp.path(), gwt_agent::AgentId::GrokBuild, false);
    let target = &fixture.target;
    let wrapped = format!(
        "\x1b[2J\x1b[16;1H╭{}╮\x1b[17;1H  ❯ notification\x1b[18;1H    {}\x1b[19;1H    tail\x1b[20;1H╰{}╯\x1b[24;1HEnter send  Shift+Tab mode\x1b[19;9H\x1b[?25h",
        "─".repeat(78), "\u{a0}".repeat(75), "─".repeat(78),
    );
    let exact_fill = grok_hidden_composer(205);
    for (body, cols, surface, accepts_submit) in [
        (
            format!("A{}B", "\u{a0}".repeat(150)),
            80,
            wrapped.as_str(),
            true,
        ),
        ("x".repeat(42), 205, exact_fill.as_str(), true),
        ("PM notification".into(), 80, GROK_PERMISSION, false),
    ] {
        let text = gwt::pm_registry::protected_pm_delivery_prompt(
            "72fc3cd4-ad49-43e3-bf3d-d79135763706",
            &body,
        )
        .expect("one-line PM input must be valid");
        {
            let mut pane = target.pane.lock().unwrap();
            pane.resize(cols, 24).unwrap();
            pane.process_bytes(grok_visible_composer(cols.into()).as_bytes());
        }
        let reservation = Arc::clone(&target.pty).reserve_input_transaction().unwrap();
        let mut writes = Vec::new();
        let submitted = std::cell::Cell::new(false);
        let body_written = std::cell::Cell::new(false);
        let outcome = gwt::project_runtime::agent_dispatch::drive_verified_pane_submit(
            &text,
            false,
            2,
            |bytes| {
                reservation
                    .write_input_authorized(bytes, |commit| {
                        if bytes == b"\r" && body_written.get() {
                            target.pty.ensure_grok_prompt_submit_ready()?;
                        } else {
                            target.pty.ensure_grok_prompt_input_ready()?;
                        }
                        commit(&mut || {})
                    })
                    .map_err(|error| error.to_string())?;
                submitted.set(bytes == b"\r");
                if bytes != b"\r" {
                    body_written.set(true);
                }
                writes.push(bytes.to_vec());
                Ok(())
            },
            |_| {
                // The settle boundary orders the redraw before CR.
                target
                    .pane
                    .lock()
                    .unwrap()
                    .process_bytes(surface.as_bytes());
            },
            || Ok(submitted.get()),
        );
        drop(reservation);
        assert_eq!(submitted.get(), accepts_submit, "{outcome:?}");
        if accepts_submit {
            assert_eq!(
                outcome,
                Ok(
                    gwt::project_runtime::agent_dispatch::VerifiedPaneSubmitOutcome::Verified {
                        submit_attempts: 1
                    }
                )
            );
            assert_eq!(writes.last().unwrap(), b"\r");
        } else {
            assert!(outcome.is_err_and(|error| error.contains("Grok composer")));
            assert_eq!(writes.len(), 1, "permission card cannot receive CR");
        }
    }
}

#[test]
fn issue_3706_grok_submit_recovery_refuses_a_focused_permission_card() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let _user_home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _grok_home = ScopedEnvVar::set("GROK_HOME", temp.path().join(".grok"));
    let fixture =
        pm_delivery_owner_fixture_for_agent(temp.path(), gwt_agent::AgentId::GrokBuild, false);
    let operation = uuid::Uuid::new_v4().to_string();
    let text = "notification with a dropped submission\r";
    let (responder, result, _cancellation) =
        AgentPmSendResponder::channel_with_acceptance_window(Duration::from_secs(2));
    fixture.send(
        "first".into(),
        &operation,
        &fixture.target.id,
        text,
        Some(responder),
    );
    assert!(matches!(result.blocking_recv().unwrap(),
        BackendEvent::PmMessageSendResult { status, .. } if status == "unconfirmed"));
    assert!(fixture
        .target
        .writer
        .as_ref()
        .unwrap()
        .pm_delivery_replay
        .lock()
        .unwrap()
        .is_some());
    fixture
        .target
        .pane
        .lock()
        .unwrap()
        .process_bytes(GROK_PERMISSION.as_bytes());
    let (responder, result, cancellation) = AgentPmSendResponder::channel();
    fixture.send(
        "recovery".into(),
        &operation,
        &fixture.target.id,
        text,
        Some(responder),
    );
    assert!(matches!(result.blocking_recv().unwrap(),
        BackendEvent::PmMessageSendResult { status, reason: Some(reason), .. }
            if status == "unconfirmed" && reason.contains("Grok composer")));
    assert!(
        !cancellation.cancel(),
        "recovery cannot commit CR to a permission card"
    );
}

fn assert_pm_send_waits_for_exact_target_hook_ack(agent_id: gwt_agent::AgentId) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _owner_home = ScopedGwtHome::set(temp.path());
    let fixture = pm_delivery_owner_fixture_for_agent(temp.path(), agent_id, false);
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
    assert_pm_unconfirmed_delivery_preserves_a_later_draft(gwt_agent::AgentId::Codex);
}

#[test]
fn issue_3706_grok_user_input_revokes_hidden_submit_recovery() {
    assert_pm_unconfirmed_delivery_preserves_a_later_draft(gwt_agent::AgentId::GrokBuild);
}

fn assert_pm_unconfirmed_delivery_preserves_a_later_draft(agent_id: gwt_agent::AgentId) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _owner_home = ScopedGwtHome::set(temp.path());
    let _grok_home = ScopedEnvVar::set("GROK_HOME", temp.path().join(".grok"));
    let fixture = pm_delivery_owner_fixture_for_agent(temp.path(), agent_id.clone(), false);
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
    let receipts =
        gwt::pm_registry::load_pm_delivery_receipts(&receipt_path).expect("delivery receipts");
    assert!(
        receipts.iter().any(|receipt| {
            receipt.operation_id == operation_id
                && receipt.status == gwt::pm_registry::PmDeliveryReceiptStatus::Unconfirmed
        }),
        "unconfirmed response must retain its durable receipt: result={result:?}, receipts={receipts:?}"
    );

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
    if agent_id == gwt_agent::AgentId::GrokBuild {
        // A modal can retain this hidden root box/footer. User input still
        // revokes ownership, so it cannot authorize a recovered CR.
        fixture
            .target
            .pane
            .lock()
            .unwrap()
            .process_bytes(grok_hidden_composer(80).as_bytes());
    }
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
    let first_result = result.blocking_recv().unwrap();
    let first_observation = match &first_result {
        BackendEvent::PmMessageSendResult { status, reason, .. } => {
            let detail = reason.as_deref().unwrap_or_default().to_ascii_lowercase();
            serde_json::json!({
                "status": match status.as_str() {
                    "unconfirmed" => "unconfirmed", "delivered" => "delivered",
                    "failed" => "failed", "refused" => "refused", "lost" => "lost",
                    _ => "other_status",
                },
                "receipt_commit_failed": detail.contains("receipt commit failed")
                    || detail.contains("receipt terminal commit failed"),
                "readback_error": detail.contains("receipt")
                    && (detail.contains("unavailable") || detail.contains("replay failed")),
                "readback_only": detail.contains("readback-only"),
            })
        }
        _ => serde_json::json!({"status": "other_event"}),
    };
    assert!(matches!(&first_result,
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
    // One raw snapshot: never lock, repair the receipt tail, or claim current authority.
    let receipt_snapshot = match fs::read_to_string(&path) {
        Ok(raw) => {
            let rows = raw
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(serde_json::from_str::<gwt::pm_registry::PmDeliveryReceipt>)
                .collect::<Result<Vec<_>, _>>();
            match rows {
                Ok(rows) => serde_json::json!({
                    "read": "parsed", "unterminated_tail": !raw.is_empty() && !raw.ends_with('\n'),
                    "matching_rows": rows.iter().filter(|row| row.operation_id == operation_id).count(),
                    "last_status": rows.iter().rev().find(|row| row.operation_id == operation_id).map(|row| row.status),
                }),
                Err(_) => serde_json::json!({
                    "read": "unparseable", "unterminated_tail": !raw.is_empty() && !raw.ends_with('\n'),
                }),
            }
        }
        Err(error) => serde_json::json!({
            "read": "unavailable", "not_found": error.kind() == std::io::ErrorKind::NotFound,
        }),
    };
    let guard_snapshot = entry.pm_delivery_replay.lock().ok().map(|replay| {
        let guard = replay.as_ref();
        serde_json::json!({
            "present": guard.is_some(),
            "same_operation": guard.is_some_and(|guard| guard.operation_id == operation_id),
            "same_target": guard.is_some_and(|guard| guard.target_session_id == target_session_id),
            "explicit_pm_guard": guard.is_some_and(|guard| guard.retained_input.is_none()),
            "same_recorded_transaction": guard.is_some_and(|guard| guard.transaction_revision == original_transaction),
            "ordinary_revision_matches": guard.is_some_and(|guard| entry.handle.ordinary_input_revision() == Some(guard.ordinary_revision)),
            "unsent_user_input": entry.handle.has_unsent_user_input(),
        })
    });
    let (responder, result, recovery_cancellation) = AgentPmSendResponder::channel();
    fixture.send(
        "client".into(),
        operation_id,
        target,
        &format!("{body}\r"),
        Some(responder),
    );
    let recovered_result = result.blocking_recv().unwrap();
    let status_reason = match &recovered_result {
        BackendEvent::PmMessageSendResult { status, reason, .. } => {
            Some((status.as_str(), reason.as_deref()))
        }
        _ => None,
    };
    assert!(
        matches!(&recovered_result,
        BackendEvent::PmMessageSendResult { status, reason: Some(reason), .. }
            if status == "unconfirmed" && reason.contains("Further calls are readback-only")),
        "same-operation submit recovery status/reason: {status_reason:?}; first={first_observation} receipt_snapshot={receipt_snapshot} guard_snapshot={guard_snapshot:?}"
    );
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
        grok_prompt: None,
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
        grok_prompt: None,
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
