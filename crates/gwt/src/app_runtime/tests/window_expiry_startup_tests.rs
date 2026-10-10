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
    runtime.runtimes[&window_id]
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
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
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
    let mut seeded = gwt::IssueMonitorState::with_prefs(
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
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-terminal"));
    for (session_id, native_id, linked_issue) in [
        ("session-closed", "native-closed", Some(42)),
        ("session-open", "native-open", None),
    ] {
        let mut session = gwt_agent::Session::new(
            &worktree,
            "work/terminal-restore",
            gwt_agent::AgentId::Codex,
        );
        session.id = session_id.to_string();
        session.agent_session_id = Some(native_id.to_string());
        session.linked_issue_number = linked_issue;
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");
    }

    runtime.bootstrap();
    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
    );

    let windows = runtime.tabs[0].workspace.persisted().windows.clone();
    assert!(
        !windows
            .iter()
            .any(|window| window.session_id.as_deref() == Some("session-closed")),
        "the closed Issue's placeholder is removed before any spawn: {windows:?}"
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
}

// Issue #3927 (SPEC #3340 T-626 / AS-44〜45 / FR-048): a Monitor-owned
// restored window that fails before PTY start is closed only after the
// concrete failure is durably committed as the Issue's `error_message`; a
// manual window that fails the same way keeps its diagnostic pane.
#[test]
fn monitor_owned_pre_pty_launch_failure_is_committed_then_closed_while_manual_is_retained() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let owned_id = combined_window_id("tab-1", "agent-42");
    let manual_id = combined_window_id("tab-1", "agent-7");

    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: owned_id.clone(),
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");

    let mut persisted = empty_workspace_state();
    for raw_id in ["agent-42", "agent-7"] {
        let mut window = sample_window(raw_id, WindowPreset::Codex, WindowProcessStatus::Running);
        window.agent_id = Some("codex".to_string());
        persisted.windows.push(window);
    }
    persisted.next_z_index = 3;
    let tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(persisted),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    for (window_id, raw_id) in [(&owned_id, "agent-42"), (&manual_id, "agent-7")] {
        runtime.window_lookup.insert(
            window_id.clone(),
            crate::app_runtime::WindowAddress {
                tab_id: "tab-1".to_string(),
                raw_id: raw_id.to_string(),
            },
        );
    }
    for (window_id, session_id, linked_issue) in [
        (&owned_id, "session-owned", Some(42)),
        (&manual_id, "session-manual", None),
    ] {
        let mut active = sample_active_agent_session("tab-1", window_id);
        active.session_id = session_id.to_string();
        active.worktree_path = repo.clone();
        runtime
            .active_agent_sessions
            .insert(window_id.clone(), active);
        let mut session =
            gwt_agent::Session::new(&repo, "work/issue-42", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.linked_issue_number = linked_issue;
        session.save(&runtime.sessions_dir).expect("save session");
    }
    let (spawner, finalizers) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let detail = "Prepared continuation no longer matches its owner generation attempt";

    let events = runtime.launch_error_events(owned_id.clone(), detail.to_string(), None);

    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::IssueMonitorLaunchFailed {
                issue_number: 42,
                ..
            }
        )),
        "the failure is committed to the Issue Monitor first: {events:?}"
    );
    assert!(
        !runtime.window_lookup.contains_key(&owned_id),
        "the Monitor-owned pre-PTY failure window is closed after the commit"
    );
    assert!(runtime.tabs[0].workspace.window("agent-42").is_none());
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert!(
        persisted
            .failed_issues
            .iter()
            .any(|failed| failed.issue_number == 42 && failed.message.contains(detail)),
        "the concrete reason is the Issue's durable error_message: {:?}",
        persisted.failed_issues
    );
    assert!(persisted.launched_issues.is_empty(), "the slot is released");
    assert_eq!(
        finalizers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        1
    );

    let manual_events = runtime.launch_error_events(manual_id.clone(), detail.to_string(), None);
    assert!(
        !manual_events
            .iter()
            .any(|event| matches!(&event.event, BackendEvent::IssueMonitorLaunchFailed { .. })),
        "a manual window reports nothing to the Monitor"
    );
    assert!(
        runtime.window_lookup.contains_key(&manual_id),
        "the manual window keeps its diagnostic pane"
    );
    assert_eq!(
        runtime.tabs[0]
            .workspace
            .window("agent-7")
            .map(|window| window.status),
        Some(WindowProcessStatus::Error)
    );
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
    let source = include_str!("../launch.rs");
    let worker = source
        .split("fn spawn_agent_window_async_with_claim")
        .nth(1)
        .and_then(|tail| tail.split("pub(crate) fn close_work").next())
        .expect("async launch worker body");
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
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
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
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
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
        FrontendEvent::StartupAutoResumeReady {
            bounds: canvas_bounds(),
        },
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
    let logs = capture_tracing_events(|| {
        runtime.queue_startup_auto_resume_sessions(&HashSet::new());
        runtime.startup_auto_resume_ready_events(canvas_bounds());
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
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-landed"));
    for id in ["session-empty", "session-diagnostic", "session-error"] {
        save_restore_fixture_session(&runtime.sessions_dir, id, &repo, Some(id), None);
    }
    runtime.window_details.insert(
        combined_window_id("tab-landed", "diagnostic"),
        "The previous launch failed; inspect its diagnostic.".into(),
    );

    runtime.queue_startup_auto_resume_sessions(&HashSet::new());

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
        "only the empty landed placeholder disappears"
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
    assert!(runtime.pending_auto_resume_sources.is_empty());
    assert!(runtime
        .window_details
        .contains_key(&combined_window_id("tab-landed", "diagnostic")));
}

#[test]
fn open_project_empty_landed_cleanup_preserves_same_session_diagnostic_window() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let tab = restore_fixture_tab(
        "tab-landed",
        &repo,
        &[
            ("diagnostic".into(), "session-shared".into()),
            ("empty".into(), "session-shared".into()),
        ],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-landed"));
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
    runtime.restore_open_project_windows("tab-landed");
    let windows = &runtime
        .tab("tab-landed")
        .unwrap()
        .workspace
        .persisted()
        .windows;
    assert_eq!(windows.len(), 1);
    assert_eq!(
        windows[0].id, "diagnostic",
        "remove the exact empty window, not another window sharing its Session"
    );
}

#[test]
fn startup_restore_queues_only_one_session_per_worktree() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let tab = restore_fixture_tab(
        "tab-duplicate",
        &repo,
        &[
            ("agent-a".into(), "session-a".into()),
            ("agent-b".into(), "session-b".into()),
        ],
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-duplicate"));
    for id in ["session-a", "session-b"] {
        save_restore_fixture_session(&runtime.sessions_dir, id, &repo, Some(id), None);
    }
    let logs = capture_tracing_events(|| {
        runtime.queue_startup_auto_resume_sessions(&HashSet::new());
    });
    assert_eq!(runtime.pending_startup_auto_resume_sessions.len(), 1);
    assert!(restore_admission_refusals(&logs)
        .values()
        .any(|reason| reason == "worktree_already_restoring"));
    // Opening the same project while startup restore is queued must not
    // start a second process for that worktree.
    runtime.restore_open_project_windows("tab-duplicate");
    assert!(runtime.pending_auto_resume_sources.is_empty());
}
