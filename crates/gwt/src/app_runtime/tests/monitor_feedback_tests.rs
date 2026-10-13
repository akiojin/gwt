use super::*;

#[test]
fn pm_wakes_exclude_terminal_and_superseded_escalations() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _) = pm_wake_fixture(&temp);
    let mut active = gwt_agent::Session::new(&repo, "work/active", gwt_agent::AgentId::Codex);
    active.status = gwt_agent::AgentStatus::Idle;
    seed_pm_session_escalation(&repo, &active, "ACTIVE-SUBJECT");
    let mut stopped = gwt_agent::Session::new(&repo, "work/stopped", gwt_agent::AgentId::Codex);
    stopped.status = gwt_agent::AgentStatus::Stopped;
    seed_pm_session_escalation(&repo, &stopped, "STOPPED-SUBJECT");
    let mut superseded = gwt_agent::Session::new(&repo, "work/old", gwt_agent::AgentId::Codex);
    superseded.status = gwt_agent::AgentStatus::Running;
    superseded.linked_issue_number = Some(4274);
    seed_pm_session_escalation(&repo, &superseded, "SUPERSEDED-SUBJECT");
    gwt::cli::execution_state::materialize_at_launch(
        &repo,
        gwt::cli::execution_state::ExecutionOwnerKind::Spec,
        4274,
        "successor-session",
        "gwt-execute",
        false,
    )
    .expect("current owner");
    runtime.pm_wake_decision_at(&repo, &[], "2026-08-18T01:00:00Z");
    let delta = runtime
        .pm_wake_decision_at(
            &repo,
            &[pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)],
            "2026-08-18T01:01:00Z",
        )
        .expect("delta");
    let periodic = runtime
        .pm_periodic_wake_decision_at(&repo, "2026-08-18T01:05:00Z")
        .expect("active blocker");
    for prompt in [delta.delivery_prompt(), periodic.delivery_prompt()] {
        assert!(prompt.contains("ACTIVE-SUBJECT"));
        assert!(!prompt.contains("STOPPED-SUBJECT"), "{prompt}");
        assert!(!prompt.contains("SUPERSEDED-SUBJECT"), "{prompt}");
    }
    active.status = gwt_agent::AgentStatus::Stopped;
    active
        .save(&gwt_core::paths::gwt_sessions_dir())
        .expect("stop last active subject");
    assert!(runtime
        .pm_periodic_wake_decision_at(&repo, "2026-08-18T01:10:00Z")
        .is_none());
    assert_eq!(
        gwt_core::coordination::load_escalation_store(&repo)
            .unwrap()
            .open_escalations()
            .len(),
        3,
        "filtering must preserve unresolved history"
    );
}

#[test]
fn pm_pending_wake_rechecks_subject_before_delivery() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, pm_window_id) = pm_wake_fixture(&temp);
    let _pm_pane = attach_live_pm_pane(&mut runtime, &pm_window_id);
    let mut subject = gwt_agent::Session::new(&repo, "work/subject", gwt_agent::AgentId::Codex);
    subject.status = gwt_agent::AgentStatus::Running;
    seed_pm_session_escalation(&repo, &subject, "PENDING-SUBJECT");
    runtime.terminal_input_events(&pm_window_id, "draft");
    runtime.pm_periodic_wake_events_at(&repo, "2026-08-18T01:00:00Z");
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_wakes[&pm_window_id]
        .delivery_prompt()
        .contains("PENDING-SUBJECT"));
    subject.status = gwt_agent::AgentStatus::Stopped;
    subject
        .save(&gwt_core::paths::gwt_sessions_dir())
        .expect("stop subject while held");
    assert!(
        !runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .pending_pm_wakes[&pm_window_id]
            .delivery_prompt()
            .contains("PENDING-SUBJECT"),
        "a held wake must render current subjects at delivery time"
    );
    runtime.terminal_input_events(&pm_window_id, "\u{0003}");
    drain_pm_wake_delivery_tasks(&mut runtime);
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_wakes
        .is_empty());
}

#[test]
fn issue_4537_hub_sync_does_not_contain_project_workspace() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window(
        "tab-a",
        "shell-a",
        WindowPreset::Shell,
        WindowProcessStatus::Ready,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-a"));
    let events = runtime.frontend_sync_events("hub-client");
    assert!(
        events.iter().all(|event| !matches!(
            event.event,
            BackendEvent::WindowCanvasState { .. }
                | BackendEvent::TerminalSnapshot { .. }
                | BackendEvent::LaunchWizardState { .. }
        )),
        "unbound Hub hydration must contain catalog, never project payloads"
    );
}

#[test]
fn reopened_project_releases_old_pm_and_scan_worker_gates() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let project_root = temp.path().join("project");
    std::fs::create_dir_all(&project_root).unwrap();
    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab-a",
            "A",
            project_root.clone(),
            ProjectKind::Git,
            &[],
        )],
        Some("tab-a"),
    );
    let context = runtime.project_context("tab-a").unwrap();
    let (proxy, queued) = AppEventProxy::stub();
    let old_worker = proxy.for_project(context);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project_root);
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pending_pm_worktree_preparations
        .insert(project_root.clone());
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .insert(prefs_path.clone());
    runtime
        .project_tab_incarnations
        .get_mut("tab-a")
        .unwrap()
        .generation += 1;
    runtime.refresh_project_state("tab-a");
    assert!(!runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_worktree_preparations
        .contains(&project_root));
    assert!(!runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .contains(&prefs_path));
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .pending_pm_worktree_preparations
        .insert(project_root.clone());
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .insert(prefs_path.clone());
    old_worker.send(UserEvent::IssueMonitorScheduledScanComplete {
        project_root: project_root.clone(),
        prefs_path: prefs_path.clone(),
        now: String::new(),
        outcome: Err("stale".to_string()),
        vanished_window_failures: vec![],
    });
    assert!(runtime
        .accept_project_completion(queued.lock().unwrap().pop().unwrap())
        .is_none());
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .pending_pm_worktree_preparations
        .contains(&project_root));
    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .contains(&prefs_path));
}

#[test]
fn active_work_issue_numbers_include_child_record_session_and_branch_links() {
    let mut session = gwt_agent::Session::new("/repo", "work/shared", gwt_agent::AgentId::Codex);
    session.id = "linked-session".to_string();
    session.linked_issue_number = Some(33);
    let agent = workspace_test_agent_with_conversation(
        "linked-session",
        "2026-09-01T00:00:00Z",
        "conversation",
    );
    let mut child = workspace_test_child("child-work", vec![agent]);
    child.owner = Some("Issue #22".to_string());
    let mut work = workspace_test_work(vec![], vec![child]);
    work.branch = Some("origin/work/shared".to_string());
    work.owner = Some("SPEC #11".to_string());
    let record = serde_json::from_value(serde_json::json!({
        "id": "child-work", "title": "Child", "owner": "Issue #55", "status_category": "idle",
        "created_at": "2026-09-01T00:00:00Z", "updated_at": "2026-09-01T00:00:00Z"
    }))
    .unwrap();
    let mut rows = vec![work];
    super::super::workspace_views::attach_active_work_issue_numbers(
        &mut rows,
        &[record],
        &[session],
        None,
        &std::collections::HashMap::from([("work/shared".to_string(), 44)]),
    );
    assert_eq!(rows[0].linked_issue_numbers, vec![11, 22, 33, 44, 55]);
    // The metadata remains usable without any Knowledge/Issue page loaded.
    assert_eq!(
        serde_json::to_value(&rows[0]).unwrap()["linked_issue_numbers"],
        serde_json::json!([11, 22, 33, 44, 55])
    );
}

#[test]
fn active_work_issue_numbers_include_registry_sessions_beyond_the_display_cap() {
    let repo = tempfile::tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(repo.path());
    init_repo(repo.path());
    let hash = gwt_core::repo_hash::detect_repo_hash(repo.path()).unwrap();
    let sessions = (1..=12)
        .map(|number| {
            let mut session =
                gwt_agent::Session::new(repo.path(), "work/shared", gwt_agent::AgentId::Codex);
            session.id = format!("session-{number}");
            session.repo_hash = Some(hash.as_str().to_string());
            session.linked_issue_number = Some(number);
            session
        })
        .collect::<Vec<_>>();
    let mut rows = vec![workspace_test_work(vec![], vec![])];
    super::super::workspace_views::attach_active_work_issue_numbers(
        &mut rows,
        &[],
        &sessions,
        Some(hash),
        &std::collections::HashMap::new(),
    );
    assert_eq!(rows[0].linked_issue_numbers, (1..=12).collect::<Vec<_>>());
}

#[test]
fn issue4803_monitor_launch_request_does_not_wait_for_provider_preparation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static PROBE_CALLS: AtomicUsize = AtomicUsize::new(0);
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..Default::default()
        },
    )
    .expect("save monitor profile");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, queued) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    PROBE_CALLS.store(0, Ordering::SeqCst);
    runtime.issue_monitor_provider_auth_probe = |_| {
        PROBE_CALLS.fetch_add(1, Ordering::SeqCst);
        gwt::issue_monitor::ProviderAuthState::Unauthenticated
    };
    for strategy in [
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    ] {
        let started = Instant::now();
        runtime.auto_launch_issue_monitor_delivery_events_for_project(
            &repo,
            4803,
            LinkedIssueKind::Issue,
            None,
            strategy,
        );
        runtime.auto_launch_issue_monitor_delivery_events_for_project(
            &repo,
            4803,
            LinkedIssueKind::Issue,
            None,
            strategy,
        );
        eprintln!("monitor request {strategy:?}: {:?}", started.elapsed());
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "Monitor launch request blocked the GUI for {:?}",
            started.elapsed()
        );
    }
    assert_eq!(
        PROBE_CALLS.load(Ordering::SeqCst),
        0,
        "credential probing must stay queued until the request handler returns"
    );
    assert_eq!(
        queued.lock().expect("queued preparations").len(),
        2,
        "replayed delivery must not start another preparation"
    );
    drain_queued_blocking_tasks(&queued);
    assert_eq!(PROBE_CALLS.load(Ordering::SeqCst), 2);
    runtime
        .project_tab_incarnations
        .get_mut("tab-1")
        .expect("project")
        .generation += 1;
    for _ in 0..2 {
        let prepared = take_issue4803_monitor_preparation(&recorded_events);
        assert!(
            runtime
                .handle_issue_monitor_launch_prepared(prepared)
                .is_empty(),
            "a reopened project's old preparation must not be applied"
        );
    }
    assert!(runtime.issue_monitor_launch_preparations.is_empty());
}

#[test]
fn issue4803_launch_cache_clone_preserves_independent_session_updates() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("sessions dir");
    let session = gwt_agent::Session::new(temp.path(), "develop", gwt_agent::AgentId::Codex);
    session.save(&sessions_dir).expect("save session");
    let mut original = LaunchWizardMemoryCache::load_with_agent_options(&sessions_dir, Vec::new());
    let snapshot = original.clone();
    original.forget_session(&session.id);
    assert!(original.session_by_id(&session.id).is_none());
    assert!(snapshot.session_by_id(&session.id).is_some());
}

#[test]
fn issue4803_monitor_preparation_rechecks_deleted_resume_session() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "deleted-during-preparation",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Spec,
            None,
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
    drain_queued_blocking_tasks(&queued);
    fs::remove_file(
        fixture
            .sessions_dir
            .join(format!("{}.toml", fixture.source_session_id)),
    )
    .expect("delete selected session after preparation");
    let prepared = take_issue4803_monitor_preparation(&fixture.recorded_events);
    assert!(fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared)
        .is_empty());
    assert_eq!(
        queued.lock().expect("retry queue").len(),
        1,
        "stale resume must be prepared again"
    );
    assert!(fixture.runtime.pending_launch_feedback_contexts.is_empty());
}

#[test]
fn runtime_factory_override_gui_rejects_partial_configuration() {
    let _lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _token = ScopedEnvVar::set("GH_TOKEN", "fixture-only-token");
    let _mode = ScopedEnvVar::set("GWT_OWNER_GITHUB_TEST_MODE", "loopback-v1");
    let _rest = ScopedEnvVar::unset("GWT_OWNER_GITHUB_REST_BASE");
    let _graphql = ScopedEnvVar::unset("GWT_OWNER_GITHUB_GRAPHQL_URL");
    let _owner_token = ScopedEnvVar::unset("GWT_OWNER_GITHUB_TOKEN");
    assert!(matches!(
        super::super::default_issue_client_factory()("fixture", "repo"),
        Err(gwt_github::client::ApiError::TestOverrideRejected { .. })
    ));
}

#[test]
fn termination_class_requires_exact_exit_and_readable_bridge_evidence() {
    use gwt::IssueMonitorFailureClass::{Agent, Infrastructure, Unknown};
    let dir = tempdir().unwrap();
    let identity = gwt_agent::SessionExecutionIdentity {
        session_id: "tier-receipt".into(),
        worktree_path: dir.path().into(),
        project_state_root: None,
        repo_hash: Some("repo".into()),
        branch: "work/test".into(),
        agent_id: gwt_agent::AgentId::Codex,
        linked_issue_number: Some(4774),
        execution_binding: gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: "tier-receipt".into(),
            repo_hash: "repo".into(),
            owner_kind: "issue".into(),
            owner_number: 4774,
            capability_generation: 1,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation".into(),
                binding_id: "binding".into(),
                ledger_head_hash: "head".into(),
            },
        },
    };
    let mut session = gwt_agent::Session::new(
        &identity.worktree_path,
        &identity.branch,
        identity.agent_id.clone(),
    );
    session.id = identity.session_id.clone();
    session.repo_hash = identity.repo_hash.clone();
    session.linked_issue_number = identity.linked_issue_number;
    session
        .set_execution_binding(Some(identity.execution_binding.clone()))
        .unwrap();
    assert_eq!(
        gwt_agent::SessionExecutionIdentity::from_session(&session)
            .unwrap()
            .as_ref(),
        Some(&identity)
    );
    session.save(dir.path()).unwrap();
    let path = gwt_agent::runtime_state_path(dir.path(), &identity.session_id);
    let state = gwt_agent::SessionRuntimeState::for_execution_process(
        gwt_agent::AgentStatus::Running,
        &identity,
        7,
        100,
        123,
        101,
    );
    state.save(&path).unwrap();
    let classify = |exit, incarnation| {
        AppRuntime::classify_issue_monitor_termination(
            dir.path(),
            &identity.session_id,
            incarnation,
            exit,
        )
    };
    assert_eq!(classify(false, 7), Unknown);
    assert_eq!(classify(true, 8), Unknown);
    assert_eq!(classify(true, 7), Agent);
    gwt_agent::SessionBridgeObservation::capture(&path, &identity.session_id)
        .unwrap()
        .unwrap()
        .record(gwt_agent::HostBridgeKind::WorkspaceUpdate, true)
        .unwrap();
    assert_eq!(classify(true, 7), Infrastructure);
    fs::write(path.with_extension("bridge-receipt"), "broken json").unwrap();
    assert_eq!(classify(true, 7), Unknown);
    fs::remove_file(path.with_extension("bridge-receipt")).unwrap();
    gwt_agent::SessionRuntimeState::for_execution(gwt_agent::AgentStatus::Stopped, &identity, 7)
        .save(&path)
        .unwrap();
    assert_eq!(classify(true, 7), Unknown);
    // A completed genesis launch need not appear in the Wizard cache. Exercise
    // the real incarnation-fenced event, not only the pure receipt classifier.
    let _home = ScopedGwtHome::set(dir.path());
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    init_repo_without_origin(&project);
    run_git(
        &project,
        &["remote", "add", "origin", "file:///termination-fixture"],
    );
    for fault in [false, true] {
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "agent-1",
            project.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        );
        let mut app = sample_runtime(dir.path(), vec![tab], Some("tab-1"));
        let window_id = "tab-1::agent-1";
        let mut active = sample_active_agent_session("tab-1", window_id);
        active.session_id = identity.session_id.clone();
        active.worktree_path = project.clone();
        active.agent_project_root = project.display().to_string();
        app.active_agent_sessions.insert(window_id.into(), active);
        session.save(&app.sessions_dir).unwrap();
        app.launch_wizard_cache.forget_session(&session.id);
        assert!(app.launch_wizard_cache.session_by_id(&session.id).is_none());
        insert_exited_test_pane_runtime(&mut app, window_id, 1);
        let incarnation = app.runtimes[window_id].incarnation;
        let runtime_path = gwt_agent::runtime_state_path(&app.sessions_dir, &session.id);
        gwt_agent::SessionRuntimeState::for_execution_process(
            gwt_agent::AgentStatus::Running,
            &identity,
            incarnation,
            100,
            123,
            101,
        )
        .save(&runtime_path)
        .unwrap();
        gwt_agent::SessionBridgeObservation::capture(&runtime_path, &session.id)
            .unwrap()
            .unwrap()
            .record(gwt_agent::HostBridgeKind::WorkspaceUpdate, fault)
            .unwrap();
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project);
        let mut prefs = gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            launch_auto: true,
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 4774,
                window_id: window_id.into(),
            }],
            autonomous_records: vec![issue_monitor_autonomous_record(
                4774,
                gwt::AutonomousPhase::Implementing,
                0,
            )],
            ..Default::default()
        };
        prefs.record_tier_launch(4774, 0);
        gwt::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
        app.blocking_tasks = BlockingTaskSpawner::queued().0;
        app.handle_runtime_status_event(
            window_id.into(),
            incarnation,
            WindowProcessStatus::Error,
            Some("Process exited with status 1".into()),
            true,
        );
        let after = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert_eq!(after.autonomous_records[0].attempts, 1);
        assert_eq!(
            after.autonomous_records[0].non_agent_attempts,
            u32::from(fault)
        );
        assert_eq!(after.issue_tiers[&4774].unknown_failures, 0);
    }
}
