use super::*;

#[test]
fn agent_bootstrap_spawn_routes_apply_resource_policy_before_release() {
    let source = include_str!("../../project_runtime.rs");
    let direct = source
        .split("pub fn spawn_unbound_pane")
        .nth(1)
        .and_then(|tail| tail.split("pub fn spawn_bound_pane").next())
        .expect("direct spawn route body");
    assert!(
        direct.contains("policy_gate"),
        "direct AgentBootstrap route must consult the launch resource policy"
    );
    assert!(
        direct.contains("new_pending_with_spawn_config"),
        "policy-bearing direct launches must use the PTY start gate"
    );
    assert!(
        direct.contains("new_with_spawn_config"),
        "Shell launches must keep the direct PTY route"
    );
    let apply = direct.find("apply_policy(").expect("direct apply_policy");
    let release = direct.find(".release()").expect("direct release");
    assert!(apply < release, "policy must be applied before release");

    let bound = source
        .split("pub fn spawn_bound_pane")
        .nth(1)
        .and_then(|tail| tail.split("#[cfg(test)]").next())
        .expect("bound spawn route body");
    let apply = bound.find("apply_policy(").expect("bound apply_policy");
    let release = bound.find(".release()").expect("bound release");
    assert!(
        apply < release,
        "bound policy must be applied before release"
    );
}

/// Issue #3942: `setpriority` returns EPERM on hosts where the launcher may
/// not renice the target's process group. Priority tuning is an optimization,
/// so neither spawn route may turn it into `PTY creation failed`.
#[test]
fn agent_resource_policy_failure_never_fails_the_spawn_routes() {
    let source = include_str!("../../project_runtime.rs");
    let direct = source
        .split("pub fn spawn_unbound_pane")
        .nth(1)
        .and_then(|tail| tail.split("pub fn spawn_bound_pane").next())
        .expect("direct spawn route body");
    let bound = source
        .split("pub fn spawn_bound_pane")
        .nth(1)
        .and_then(|tail| tail.split("#[cfg(test)]").next())
        .expect("bound spawn route body");

    for (route, body) in [("direct", direct), ("bound", bound)] {
        let apply = body
            .find("best_effort_apply_policy(")
            .expect("best-effort policy application");
        let statement_end = body[apply..].find(';').expect("apply_policy statement end");
        let statement = &body[apply..apply + statement_end];
        for propagation in ["?", ".unwrap(", ".expect(", "map_err"] {
            assert!(
                !statement.contains(propagation),
                "{route} route must not turn a resource policy failure into a launch failure: \
                 {statement}"
            );
        }
    }

    // The routes cannot re-introduce the failure path even by editing the
    // statement: the helper owns the call and hands no outcome back.
    let helper = source
        .split("fn best_effort_apply_policy(")
        .nth(1)
        .expect("best-effort helper");
    let signature = &helper[..helper.find('{').expect("helper body")];
    assert!(
        !signature.contains("->"),
        "the best-effort helper must not return an outcome the routes could propagate: {signature}"
    );
}

/// Issue #3942 AC-2 / AC-3: a rejected policy is a single warning, never an
/// error that the launch routes can turn into a user-facing "Agent error".
#[test]
fn agent_resource_policy_failure_warns_once_and_keeps_the_launch() {
    let events = capture_tracing_events(|| {
        gwt::project_runtime::note_unapplied_agent_resource_policy_for_test(
            "window-3942",
            Err(gwt_terminal::TerminalError::PtyCreationFailed {
                reason: "apply process policy: setpriority(pgrp 4242, nice 10): \
                     Operation not permitted (os error 1)"
                    .to_string(),
            }),
        );
    });
    let policy_events: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(event.fields.get("message"), Some(message) if message.contains("resource policy"))
        })
        .collect();
    assert_eq!(
        policy_events.len(),
        1,
        "a rejected resource policy must be recorded exactly once: {events:?}"
    );
    let warning = policy_events[0];
    assert_eq!(
        warning.level,
        Level::WARN,
        "a rejected resource policy must stay at warn level: {warning:?}"
    );
    assert_eq!(
        warning.fields.get("window_id").map(String::as_str),
        Some("window-3942"),
        "the warning must name the window: {warning:?}"
    );
    assert!(
        matches!(warning.fields.get("error"), Some(error) if error.contains("setpriority")),
        "the warning must keep the platform reason: {warning:?}"
    );

    let applied = capture_tracing_events(|| {
        gwt::project_runtime::note_unapplied_agent_resource_policy_for_test("window-3942", Ok(()));
    });
    assert!(
        applied.iter().all(|event| !matches!(
            event.fields.get("message"),
            Some(message) if message.contains("resource policy")
        )),
        "an applied resource policy must stay silent: {applied:?}"
    );
}

#[test]
fn gwt_input_trace_markers_exclude_payload_lengths_and_raw_errors() {
    for (source_name, source) in [
        (
            "embedded_server.rs",
            include_str!("../../embedded_server.rs"),
        ),
        ("app_runtime/pty_io.rs", include_str!("../pty_io.rs")),
        ("pane_runtime.rs", include_str!("../../pane_runtime.rs")),
    ] {
        for (index, tail) in source
            .split("target: \"gwt_input_trace\"")
            .skip(1)
            .enumerate()
        {
            let marker = tail.split_once(");").map_or(tail, |(marker, _)| marker);
            for forbidden in ["data_len,", "text_len,", "chunk_len", "error =", "%error,"] {
                assert!(
                    !marker.contains(forbidden),
                    "{source_name} gwt_input_trace marker #{index} contains forbidden field {forbidden}: {marker}",
                );
            }
        }
    }
}

#[test]
fn backend_gwt_input_trace_markers_use_stage_local_exact_allowlists() {
    let expected = HashMap::from([
        (
            "app_runtime/pty_io.rs",
            HashMap::from([
                (
                    "event_loop_runtime_missing",
                    vec!["outcome", "stage", "window_id"],
                ),
                (
                    "pty_write",
                    vec!["lock_wait_us", "ok", "stage", "window_id", "write_us"],
                ),
                (
                    "pane_lock_failed",
                    vec!["lock_wait_us", "outcome", "stage", "window_id"],
                ),
                ("registry_lock_poisoned", vec!["stage", "window_id"]),
                (
                    "registry_replacement_barrier_spawn_failed",
                    vec!["outcome", "stage", "window_id"],
                ),
                (
                    "registry_write_poisoned",
                    vec!["outcome", "stage", "window_id"],
                ),
                (
                    "registry_deregister_poisoned",
                    vec!["outcome", "stage", "window_id"],
                ),
                (
                    "close_finalizer_registry_deregister_poisoned",
                    vec!["outcome", "stage", "window_id"],
                ),
            ]),
        ),
        (
            "pane_runtime.rs",
            HashMap::from([(
                "reader_pane_lock",
                vec!["lock_wait_us", "parse_us", "stage", "window_id"],
            )]),
        ),
        (
            "embedded_server.rs",
            HashMap::from([
                ("ws_recv", vec!["client_id", "seq", "stage", "window_id"]),
                (
                    "fast_path_lock_poisoned",
                    vec!["client_id", "seq", "stage", "window_id"],
                ),
                (
                    "fast_path_write",
                    vec![
                        "client_id",
                        "elapsed_ms",
                        "pty_writer_count",
                        "seq",
                        "stage",
                        "window_id",
                        "write_us",
                    ],
                ),
                (
                    "fast_path_write_err",
                    vec!["client_id", "seq", "stage", "window_id"],
                ),
                (
                    "fast_path_miss",
                    vec!["client_id", "seq", "stage", "window_id"],
                ),
                (
                    "ws_dispatch",
                    vec!["client_id", "ok", "seq", "stage", "window_id"],
                ),
            ]),
        ),
    ]);
    for (source_name, source) in [
        (
            "embedded_server.rs",
            include_str!("../../embedded_server.rs"),
        ),
        ("app_runtime/pty_io.rs", include_str!("../pty_io.rs")),
        ("pane_runtime.rs", include_str!("../../pane_runtime.rs")),
    ] {
        let mut actual = HashMap::new();
        for tail in source.split("target: \"gwt_input_trace\"").skip(1) {
            let marker = tail.split_once(");").map_or(tail, |(marker, _)| marker);
            let stage = marker
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("stage = \"")
                        .and_then(|line| line.strip_suffix("\","))
                })
                .expect("gwt_input_trace marker must declare one literal stage");
            let mut fields = marker
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('"') {
                        return None;
                    }
                    let field = line
                        .split_once(" =")
                        .map_or_else(|| line.strip_suffix(','), |(field, _)| Some(field))?;
                    (!field.is_empty()
                        && field
                            .chars()
                            .all(|character| character == '_' || character.is_ascii_alphanumeric()))
                    .then_some(field)
                })
                .collect::<Vec<_>>();
            fields.sort_unstable();
            // A stage can have separate WARN/DEBUG sites; audit every site
            // before collecting stages so one cannot hide another's fields.
            assert_eq!(
                Some(&fields),
                expected[source_name].get(stage),
                "{source_name} gwt_input_trace stage {stage} changed its allowed fields",
            );
            actual.insert(stage, fields);
        }
        assert_eq!(
            actual,
            expected[source_name],
            "{source_name} gwt_input_trace fields changed without an explicit stage-local allowlist update",
        );
    }
}

#[test]
fn process_launch_debug_redacts_agent_capability_and_session_identity() {
    let secret = "agent-capability-secret-sentinel";
    let readiness = "continue-readiness-secret-sentinel";
    let launch = ProcessLaunch {
        initial_prompt_file: None,
        command: "docker".to_string(),
        args: vec![
            format!("{}={secret}", gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV),
            format!("{}=session-private", gwt_agent::GWT_SESSION_ID_ENV),
            format!(
                "{}={readiness}",
                gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV
            ),
        ],
        env: HashMap::from([
            (
                gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV.to_string(),
                secret.to_string(),
            ),
            (
                gwt_agent::GWT_SESSION_ID_ENV.to_string(),
                "session-private".to_string(),
            ),
            (
                gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV.to_string(),
                readiness.to_string(),
            ),
        ]),
        remove_env: Vec::new(),
        cwd: None,
        resource_policy: None,
    };

    let debug = format!("{launch:?}");
    assert!(!debug.contains(secret));
    assert!(!debug.contains(readiness));
    assert!(!debug.contains("session-private"));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn recovery_center_project_reload_preserves_other_project_handles() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tabs = vec![
        sample_project_tab(
            "tab-a",
            "A",
            temp.path().join("a"),
            ProjectKind::NonRepo,
            &[],
        ),
        sample_project_tab(
            "tab-b",
            "B",
            temp.path().join("b"),
            ProjectKind::NonRepo,
            &[],
        ),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-a"));
    let a = runtime.project_context("tab-a").unwrap();
    let b = runtime.project_context("tab-b").unwrap();
    runtime
        .project_state_mut(&a)
        .unwrap()
        .recovery_center_handles
        .insert(
            "handle-a".to_string(),
            super::super::RecoveryCenterAction {
                generation: 1,
                board_entry_id: Some("entry-a".to_string()),
            },
        );
    runtime.load_recovery_center_events(&b, "client-b", "load-b");
    let a_reply =
        runtime.open_recovery_center_board_entry_events(&a, "client-a", "open-a", 1, "handle-a");
    assert!(
        matches!(&a_reply[0].event, BackendEvent::RecoveryCenterBoardEntry { board_entry_id: Some(id), .. } if id == "entry-a")
    );
    let b_reply =
        runtime.open_recovery_center_board_entry_events(&b, "client-b", "open-b", 1, "handle-a");
    assert!(matches!(
        &b_reply[0].event,
        BackendEvent::RecoveryCenterBoardEntry {
            board_entry_id: None,
            ..
        }
    ));
}

#[test]
fn recovery_center_projects_only_active_project_records_without_rewriting_sessions() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let _gwt_home = ScopedGwtHome::set(&home);
    let active_repo = temp.path().join("active");
    let foreign_repo = temp.path().join("foreign");
    fs::create_dir_all(&active_repo).expect("active repo");
    fs::create_dir_all(&foreign_repo).expect("foreign repo");
    let tab = sample_project_tab(
        "tab-1",
        "Active",
        active_repo.clone(),
        ProjectKind::Git,
        &[],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let mut active_session =
        gwt_agent::Session::new(&active_repo, "work/issue-1974", gwt_agent::AgentId::Codex);
    active_session.id = "recovery-session-active".to_string();
    active_session
        .save(&runtime.sessions_dir)
        .expect("save active Session");
    let active_session_path = runtime
        .sessions_dir
        .join(format!("{}.toml", active_session.id));
    let session_before = fs::read(&active_session_path).expect("read Session before projection");

    let active_store = RecoveryStore::for_repo(&active_repo, &active_session.id).expect("store");
    let pending = active_store
        .prepare(
            "prepare-pending",
            recovery_center_test_intent(
                &active_session.id,
                "recovery-pending",
                "board-pending",
                BoardWorktreeForm::Ephemeral,
                "pending public body",
            ),
        )
        .expect("prepare pending")
        .record;
    let acknowledged_pending = active_store
        .prepare(
            "prepare-acknowledged",
            recovery_center_test_intent(
                &active_session.id,
                "recovery-acknowledged",
                "board-acknowledged",
                BoardWorktreeForm::BranchBacked,
                "acknowledged public body",
            ),
        )
        .expect("prepare acknowledged")
        .record;
    let acknowledgement = RecoveryAcknowledgement::for_record(
        &acknowledged_pending,
        RecoveryProviderReceipt::new("board-acknowledged").expect("receipt"),
    )
    .expect("acknowledgement");
    active_store
        .acknowledge(
            "recovery-acknowledged",
            acknowledged_pending.revision,
            "ack-acknowledged",
            acknowledgement,
        )
        .expect("acknowledge");
    let conflict_pending = active_store
        .prepare(
            "prepare-conflicted",
            recovery_center_test_intent(
                &active_session.id,
                "recovery-conflicted",
                "board-conflicted",
                BoardWorktreeForm::Unknown,
                "conflicted public body",
            ),
        )
        .expect("prepare conflicted")
        .record;
    active_store
        .mark_conflicted(
            "recovery-conflicted",
            conflict_pending.revision,
            "conflict-conflicted",
            RecoveryConflictKind::StorageUncertain,
        )
        .expect("mark conflicted");

    let mut foreign_session =
        gwt_agent::Session::new(&foreign_repo, "work/foreign", gwt_agent::AgentId::Codex);
    foreign_session.id = "recovery-session-foreign".to_string();
    foreign_session
        .save(&runtime.sessions_dir)
        .expect("save foreign Session");
    RecoveryStore::for_repo(&foreign_repo, &foreign_session.id)
        .expect("foreign store")
        .prepare(
            "prepare-foreign",
            recovery_center_test_intent(
                &foreign_session.id,
                "recovery-foreign",
                "board-foreign",
                BoardWorktreeForm::BranchBacked,
                "foreign private sentinel",
            ),
        )
        .expect("prepare foreign");

    // One unreadable ledger entry must not hide the valid recovery records.
    fs::write(runtime.sessions_dir.join("broken.toml"), "id = [").unwrap();
    let events = runtime.handle_frontend_event(
        "client-recovery".to_string(),
        FrontendEvent::LoadRecoveryCenter {
            request_id: "request-1".to_string(),
        },
    );
    let (generation, items) = events
        .iter()
        .find_map(|outbound| match &outbound.event {
            BackendEvent::RecoveryCenterState {
                request_id,
                generation,
                status: gwt::RecoveryCenterLoadStatus::Ready,
                items,
            } if request_id == "request-1" => Some((*generation, items.clone())),
            _ => None,
        })
        .expect("ready Recovery Center projection");
    assert!(generation > 0);
    assert_eq!(items.len(), 3);
    assert!(items.iter().any(|item| {
        item.state == gwt::RecoveryCenterItemState::Pending
            && item.worktree_form == BoardWorktreeForm::Ephemeral
    }));
    assert!(items.iter().any(|item| {
        item.state == gwt::RecoveryCenterItemState::Acknowledged
            && item.worktree_form == BoardWorktreeForm::BranchBacked
    }));
    assert!(items.iter().any(|item| {
        item.state == gwt::RecoveryCenterItemState::Conflicted
            && item.worktree_form == BoardWorktreeForm::Unknown
    }));
    assert!(items.iter().all(|item| {
        !item.action_handle.contains("recovery-")
            && !item.action_handle.contains(&active_session.id)
            && !item.summary.contains("foreign private sentinel")
    }));
    assert_eq!(
        fs::read(&active_session_path).expect("read Session after projection"),
        session_before,
        "Recovery Center must use Session::load without migration or mutation"
    );
    assert_eq!(pending.state, RecoveryState::Pending);
}

#[test]
fn recovery_center_resolves_board_history_only_for_current_acknowledged_handle() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let _gwt_home = ScopedGwtHome::set(&home);
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let mut session = gwt_agent::Session::new(&repo, "work/issue-1974", gwt_agent::AgentId::Codex);
    session.id = "recovery-action-session".to_string();
    session.save(&runtime.sessions_dir).expect("save Session");
    let store = RecoveryStore::for_repo(&repo, &session.id).expect("store");
    let pending = store
        .prepare(
            "prepare-pending-action",
            recovery_center_test_intent(
                &session.id,
                "recovery-pending-action",
                "board-pending-action",
                BoardWorktreeForm::Ephemeral,
                "pending action",
            ),
        )
        .expect("pending")
        .record;
    let ack_pending = store
        .prepare(
            "prepare-ack-action",
            recovery_center_test_intent(
                &session.id,
                "recovery-ack-action",
                "board-ack-action",
                BoardWorktreeForm::BranchBacked,
                "ack action",
            ),
        )
        .expect("prepare ack")
        .record;
    store
        .acknowledge(
            "recovery-ack-action",
            ack_pending.revision,
            "ack-action",
            RecoveryAcknowledgement::for_record(
                &ack_pending,
                RecoveryProviderReceipt::new("board-ack-action").expect("receipt"),
            )
            .expect("acknowledgement"),
        )
        .expect("acknowledge");

    let load = runtime.handle_frontend_event(
        "client-recovery".to_string(),
        FrontendEvent::LoadRecoveryCenter {
            request_id: "request-load".to_string(),
        },
    );
    let (generation, pending_handle, acknowledged_handle) = load
        .iter()
        .find_map(|outbound| match &outbound.event {
            BackendEvent::RecoveryCenterState {
                generation, items, ..
            } => Some((
                *generation,
                items
                    .iter()
                    .find(|item| item.state == gwt::RecoveryCenterItemState::Pending)?
                    .action_handle
                    .clone(),
                items
                    .iter()
                    .find(|item| item.state == gwt::RecoveryCenterItemState::Acknowledged)?
                    .action_handle
                    .clone(),
            )),
            _ => None,
        })
        .expect("projection handles");

    let open = |runtime: &mut AppRuntime, generation, action_handle: String| {
        runtime.handle_frontend_event(
            "client-recovery".to_string(),
            FrontendEvent::OpenRecoveryCenterBoardEntry {
                request_id: "request-open".to_string(),
                generation,
                action_handle,
            },
        )
    };
    let pending_result = open(&mut runtime, generation, pending_handle);
    assert!(matches!(
        &pending_result[0].event,
        BackendEvent::RecoveryCenterBoardEntry {
            board_entry_id: None,
            ..
        }
    ));
    let acknowledged_result = open(&mut runtime, generation, acknowledged_handle.clone());
    assert!(matches!(
        &acknowledged_result[0].event,
        BackendEvent::RecoveryCenterBoardEntry {
            board_entry_id: Some(entry_id),
            ..
        } if entry_id == "board-ack-action"
    ));
    let stale_result = open(
        &mut runtime,
        generation.saturating_sub(1),
        acknowledged_handle,
    );
    assert!(matches!(
        &stale_result[0].event,
        BackendEvent::RecoveryCenterBoardEntry {
            board_entry_id: None,
            ..
        }
    ));
    assert_eq!(pending.state, RecoveryState::Pending);
}

#[test]
fn resumed_agent_window_accepts_terminal_input_and_attachment_staging() {
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
    let window_id = combined_window_id("tab-1", "agent-1");
    let mut session = sample_active_agent_session("tab-1", &window_id);
    session.worktree_path = repo.clone();
    session.agent_project_root = repo.display().to_string();
    runtime
        .active_agent_sessions
        .insert(window_id.clone(), session);
    insert_test_pane_runtime(&mut runtime, &window_id);
    let pane = runtime
        .runtimes
        .get(&window_id)
        .expect("resumed runtime")
        .pane
        .clone();

    runtime.register_pty_writer(&window_id, &pane);

    assert!(
        runtime
            .pty_writers
            .read()
            .expect("PTY writer registry")
            .contains_key(&window_id),
        "a resumed agent window must register its PTY writer so the WebSocket fast path delivers input"
    );

    let terminal_events = runtime.terminal_input_events(&window_id, "mutating prompt\r");
    assert!(
        terminal_events.is_empty(),
        "terminal input into a resumed agent window must reach the PTY without a denial event: {terminal_events:?}"
    );

    let pane_send_events = runtime.pane_send_input_to_window_events(
        "client-1".to_string(),
        &window_id,
        "pane input\r",
    );
    assert!(
        pane_send_events
            .iter()
            .any(|event| matches!(&event.event, BackendEvent::PaneSendResult { ok: true, .. })),
        "pane.write into a resumed agent window must succeed: {pane_send_events:?}"
    );

    let attachment_events =
        runtime.paste_image_events(&window_id, "AQ==", "image/png", Some("pasted.png"));
    assert!(
        attachment_events.is_empty(),
        "attachment staging into a resumed agent window must inject its prompt without denial: {attachment_events:?}"
    );
    assert!(
        repo.join(".gwt").join("drop-files").exists(),
        "attachment bytes must be staged for a resumed agent window"
    );
}

#[test]
fn browser_pane_send_input_requires_the_session_owning_project() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut tab = sample_project_tab_with_window_at(
        "b",
        "agent",
        temp.path().join("b"),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    assert!(tab
        .workspace
        .set_session_id("agent", Some("session-b".to_string())));
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("b"));
    let window_id = "b::agent";
    insert_test_pane_runtime(&mut runtime, window_id);
    let b = runtime.project_tab_incarnations["b"].project_key.clone();
    let a = gwt_core::paths::resolve_project_scope(&temp.path().join("a")).hash;
    let refused =
        runtime.pane_send_input_for_project_events("client-a".into(), &a, "session-b", "foreign");
    assert!(matches!(
        &refused[0].event,
        BackendEvent::PaneSendResult { ok: false, .. }
    ));
    let accepted =
        runtime.pane_send_input_for_project_events("client-b".into(), &b, "session-b", "own");
    assert!(matches!(
        &accepted[0].event,
        BackendEvent::PaneSendResult { ok: true, .. }
    ));
}

#[test]
fn pty_writer_registration_preserves_inactive_project_ownership() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tabs = ["a", "b"]
        .into_iter()
        .map(|id| {
            sample_project_tab_with_window_at(
                id,
                "agent",
                temp.path().join(id),
                WindowPreset::Agent,
                WindowProcessStatus::Running,
            )
        })
        .collect();
    let mut runtime = sample_runtime(temp.path(), tabs, Some("b"));
    let window_id = "a::agent";
    insert_test_pane_runtime(&mut runtime, window_id);
    let pane = runtime.runtimes[window_id].pane.clone();

    runtime.register_pty_writer(window_id, &pane);

    let writers = runtime.pty_writers.read().expect("registry");
    assert_eq!(
        writers[window_id].project_key,
        runtime.project_tab_incarnations["a"].project_key
    );
    assert_ne!(
        writers[window_id].project_key,
        runtime.project_tab_incarnations["b"].project_key
    );
    assert!(Arc::ptr_eq(
        &writers[window_id].handle,
        &pane.lock().expect("pane").shared_pty()
    ));
}

#[test]
fn pty_writer_registration_rejects_a_window_without_project_ownership() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let window_id = "unknown::agent";
    insert_test_pane_runtime(&mut runtime, window_id);
    let pane = runtime.runtimes[window_id].pane.clone();

    runtime.register_pty_writer(window_id, &pane);

    assert!(!runtime
        .pty_writers
        .read()
        .expect("registry")
        .contains_key(window_id));
}

#[test]
fn deregistering_pty_writer_invalidates_the_removed_generation() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().join("repo"),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1";
    insert_test_pane_runtime(&mut runtime, window_id);
    let pane = runtime
        .runtimes
        .get(window_id)
        .expect("runtime")
        .pane
        .clone();
    runtime.register_pty_writer(window_id, &pane);
    let old_generation = runtime
        .pty_writers
        .read()
        .expect("PTY writer registry")
        .get(window_id)
        .map(|entry| Arc::clone(&entry.handle))
        .expect("registered writer");

    runtime.deregister_pty_writer(window_id);

    assert!(
        old_generation.write_input(b"late submit").is_err(),
        "a detached PM worker must not write into a deregistered PTY generation"
    );
}

#[test]
fn deregistering_pty_writer_can_finish_while_a_reserved_submit_is_settling() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().join("repo"),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-1";
    insert_test_pane_runtime(&mut runtime, window_id);
    let pane = runtime
        .runtimes
        .get(window_id)
        .expect("runtime")
        .pane
        .clone();
    runtime.register_pty_writer(window_id, &pane);
    let old_generation = runtime
        .pty_writers
        .read()
        .expect("PTY writer registry")
        .get(window_id)
        .map(|entry| Arc::clone(&entry.handle))
        .expect("registered writer");
    let _reservation = Arc::clone(&old_generation)
        .reserve_input_transaction()
        .expect("reserve protected submit");

    let started = Instant::now();
    runtime.deregister_pty_writer(window_id);

    assert!(
        started.elapsed() < Duration::from_millis(200),
        "registry teardown must not wait for a worker-held reservation"
    );
}

#[test]
fn terminal_agent_error_invalidates_input_even_when_pane_is_kept_for_diagnostics() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.active_agent_sessions.insert(
        window_id.clone(),
        sample_active_agent_session("tab-1", &window_id),
    );
    insert_test_pane_runtime(&mut runtime, &window_id);
    let prompt_file = tempfile::NamedTempFile::new_in(temp.path())
        .expect("initial prompt")
        .into_temp_path();
    let prompt_path = prompt_file.to_path_buf();
    let window_runtime = runtime.runtimes.get_mut(&window_id).expect("runtime");
    let incarnation = window_runtime.incarnation;
    window_runtime._initial_prompt_file = Some(Arc::new(prompt_file));
    let pane = runtime
        .runtimes
        .get(&window_id)
        .expect("runtime")
        .pane
        .clone();
    runtime.register_pty_writer(&window_id, &pane);
    let generation = runtime
        .pty_writers
        .read()
        .expect("PTY writer registry")
        .get(&window_id)
        .map(|entry| Arc::clone(&entry.handle))
        .expect("registered generation");
    let _ = runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
        "Idle",
        "Stop",
        "session-1",
    ));

    let _ = runtime.handle_runtime_status(
        window_id.clone(),
        WindowProcessStatus::Error,
        Some("PTY process exited".to_string()),
    );

    assert!(
        runtime.runtimes.contains_key(&window_id),
        "the final pane remains available for diagnostics"
    );
    assert!(runtime.recoverable_agent_error_windows.contains(&window_id));
    assert!(!runtime
        .pty_writers
        .read()
        .expect("PTY writer registry")
        .contains_key(&window_id));
    assert!(
        generation.write_input(b"late PM body").is_err(),
        "a delivery prepared before terminal status must not begin a later physical write"
    );
    assert!(
        prompt_path.exists(),
        "an unconfirmed transport error must retain the prompt"
    );
    runtime.handle_runtime_status_event(
        window_id.clone(),
        incarnation,
        WindowProcessStatus::Error,
        Some("PTY process exited".to_string()),
        true,
    );
    assert!(runtime.runtimes.contains_key(&window_id));
    assert!(
        !prompt_path.exists(),
        "confirmed exit must release the prompt even while diagnostics remain"
    );
}

#[test]
fn blocking_task_spawner_propagates_scoped_gwt_home() {
    let home = tempdir().expect("isolated gwt home");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let expected = home.path().join(".gwt");
    let spawner = BlockingTaskSpawner::thread();
    let (observed_tx, observed_rx) = mpsc::channel();

    spawner.spawn(move || {
        observed_tx
            .send(gwt_core::paths::gwt_home())
            .expect("report worker gwt home");
    });

    assert_eq!(
        observed_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker reports gwt home"),
        expected
    );
}

#[test]
fn blocking_task_spawner_returns_before_a_stalled_task_finishes() {
    let spawner = BlockingTaskSpawner::thread();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let (spawn_returned_tx, spawn_returned_rx) = mpsc::channel();

    let caller = thread::spawn(move || {
        spawner.spawn(move || {
            started_tx.send(()).expect("signal stalled task start");
            release_rx.recv().expect("release stalled task");
            finished_tx
                .send(())
                .expect("signal stalled task completion");
        });
        spawn_returned_tx.send(()).expect("signal spawn returned");
    });

    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("stalled task starts");
    let returned_before_release = spawn_returned_rx
        .recv_timeout(Duration::from_secs(2))
        .is_ok();
    let finished_before_release = finished_rx.try_recv();
    release_tx.send(()).expect("release stalled task");
    caller.join().expect("join spawn caller");

    assert!(
        returned_before_release,
        "spawn must return while the task is blocked"
    );
    assert!(matches!(
        finished_before_release,
        Err(mpsc::TryRecvError::Empty)
    ));
    finished_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("stalled task eventually completes");
}

#[test]
fn app_runtime_rejects_removed_legacy_memo_window_creation() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events =
        runtime.create_window_events(&runtime.test_context(), WindowPreset::Memo, canvas_bounds());

    assert!(events.is_empty());
    assert!(runtime.window_lookup.is_empty());
    assert!(runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .is_empty());
}

#[test]
fn app_runtime_work_singleton_reuses_create_before_hydration() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let first_events =
        runtime.create_window_events(&runtime.test_context(), WindowPreset::Work, canvas_bounds());
    let first = runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows
        .first()
        .expect("first Work window")
        .clone();

    let repeated_events =
        runtime.create_window_events(&runtime.test_context(), WindowPreset::Work, canvas_bounds());
    let windows = &runtime
        .tab("tab-1")
        .expect("tab")
        .workspace
        .persisted()
        .windows;

    assert_eq!(
        windows.len(),
        1,
        "repeated Work create must reuse the window"
    );
    assert_eq!(windows[0].id, first.id);
    assert_eq!(windows[0].preset, WindowPreset::Work);
    assert!(
        windows[0].z_index > first.z_index,
        "reuse must raise the existing Work window"
    );
    assert_eq!(runtime.window_lookup.len(), 1);
    assert!(
        runtime.runtimes.is_empty(),
        "Work reuse must not start a PTY"
    );
    assert!(first_events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
    assert!(repeated_events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
    assert!(
        !repeated_events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })),
        "reuse must not re-enter the runtime start/status path"
    );

    assert!(runtime
        .persist_dispatcher
        .wait_idle(std::time::Duration::from_secs(5)));
    let persisted =
        load_workspace_state(&workspace_state_path(&repo)).expect("persisted Work singleton state");
    assert_eq!(persisted.windows.len(), 1);
    assert_eq!(persisted.windows[0].id, first.id);
}
