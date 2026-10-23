use super::*;

#[test]
fn monitor_scan_recovery_failure_emits_one_attention_notice() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab-a",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[],
        )],
        Some("tab-a"),
    );
    let mut monitor = test_monitor(gwt::IssueMonitorConfig::default());
    monitor.record_control_commit_error("Issue Monitor authority recovery is blocked: project-state/issue-monitor.shutdown-revoke is malformed");
    let first = runtime.issue_monitor_daemon_status_events(&repo, Box::new(monitor.status_view()));
    assert_eq!(
        first
            .iter()
            .filter(|event| matches!(event.event, BackendEvent::IssueMonitorToast { .. }))
            .count(),
        1
    );
    let repeated =
        runtime.issue_monitor_daemon_status_events(&repo, Box::new(monitor.status_view()));
    assert!(repeated
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::IssueMonitorToast { .. })));
}

#[test]
fn git_health_notice_is_shared_by_pm_and_monitor_until_recovery() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let mut runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab-a",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[],
        )],
        Some("tab-a"),
    );
    let diagnostic = "Git needs attention\nCommand: git rev-parse --git-common-dir\nGit stderr: fatal: broken metadata\nRepair the Git metadata, then retry.";
    let continuation = || super::super::pm::PmWorktreeContinuation::FreshSpawn {
        tab_id: "tab-a".into(),
        project_root: repo.clone(),
    };
    let first = runtime.handle_pm_worktree_prepared(continuation(), Err(diagnostic.into()));
    assert_eq!(
        first
            .iter()
            .filter(|event| matches!(event.event, BackendEvent::IssueMonitorToast { .. }))
            .count(),
        1
    );
    let mut monitor = test_monitor(gwt::IssueMonitorConfig::default());
    monitor.record_scan_error(
        "2026-10-14T00:00:00Z",
        diagnostic.replace("rev-parse --git-common-dir", "remote get-url origin"),
    );
    let repeated =
        runtime.issue_monitor_daemon_status_events(&repo, Box::new(monitor.status_view()));
    assert!(repeated
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::IssueMonitorToast { .. })));
    let repeated = runtime.handle_pm_worktree_prepared(continuation(), Err(diagnostic.into()));
    assert!(repeated
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::IssueMonitorToast { .. })));
    let healthy = test_monitor(gwt::IssueMonitorConfig::default());
    runtime.issue_monitor_daemon_status_events(&repo, Box::new(healthy.status_view()));
    let unchanged = runtime.handle_pm_worktree_prepared(continuation(), Err(diagnostic.into()));
    assert!(
        unchanged
            .iter()
            .all(|event| !matches!(event.event, BackendEvent::IssueMonitorToast { .. })),
        "an empty status is not evidence that Git recovered"
    );
    let mut validated = healthy.status_view();
    validated.last_scan_at = Some((chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339());
    runtime.issue_monitor_daemon_status_events(&repo, Box::new(validated));
    let recurrent = runtime.handle_pm_worktree_prepared(continuation(), Err(diagnostic.into()));
    assert_eq!(
        recurrent
            .iter()
            .filter(|event| matches!(event.event, BackendEvent::IssueMonitorToast { .. }))
            .count(),
        1
    );
}

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
        1,
        "replayed delivery must not start another preparation"
    );
    assert_eq!(runtime.issue_monitor_launch_queue.len(), 1);
    drain_queued_blocking_tasks(&queued);
    assert_eq!(PROBE_CALLS.load(Ordering::SeqCst), 1);
    runtime
        .project_tab_incarnations
        .get_mut("tab-1")
        .expect("project")
        .generation += 1;
    let prepared = take_issue4803_monitor_preparation(&recorded_events);
    assert!(
        runtime
            .handle_issue_monitor_launch_prepared(prepared)
            .is_empty(),
        "a reopened project's old preparation must not be applied"
    );
    assert!(runtime.issue_monitor_launch_queue.is_empty());
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
    run_queued_monitor_gui_diagnostics(&queued, &[2, 0]);
    assert_eq!(
        queued.lock().expect("retry queue").len(),
        1,
        "stale resume must be prepared again"
    );
    assert!(fixture.runtime.pending_launch_feedback_contexts.is_empty());
}

#[test]
fn issue5596_durable_facts_prefetch_is_bounded_and_keeps_claims_fifo() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "facts-prefetch",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
        LinkedIssueKind::Issue,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    );
    gwt::machine_agent_settings::set(
        Some(3),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-23T00:00:00Z",
    )
    .expect("three canonical machine admission slots for FIFO prefetch");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("read monitor prefs");
    for issue_number in [3166, 3167] {
        prefs.queued_launch_session_strategies.insert(
            issue_number,
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
    }
    let mut monitor = test_monitor_with_prefs(
        gwt::IssueMonitorConfig {
            enabled: true,
            ..Default::default()
        },
        prefs,
    );
    monitor.terminal_queue_push(&[3166, 3167], "operator", &Utc::now().to_rfc3339());
    for issue_number in [3166, 3167] {
        monitor.record_candidate(gwt::IssueMonitorIssue {
            author_login: None,
            number: issue_number,
            title: "fix: prefetched Monitor launch".to_string(),
            labels: vec!["bug".to_string()],
            state: gwt::IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: gwt::IssueMonitorReadiness::Ready,
            updated_at: None,
        });
        assert!(monitor.apply_confirmed_claim(
            issue_number,
            format!("claim-prefetch-{issue_number}"),
            "host/session",
            &format!("effect-prefetch-{issue_number}"),
            &Utc::now().to_rfc3339(),
        ));
    }
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("save durable deliveries");
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    for issue_number in [3165, 3166, 3167] {
        let delivery_id = monitor
            .pending_launch_delivery_id(issue_number)
            .expect("native durable delivery");
        fixture
            .runtime
            .auto_launch_issue_monitor_delivery_events_for_project(
                &fixture.project_root,
                issue_number,
                LinkedIssueKind::Issue,
                Some(delivery_id),
                gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
            );
    }
    assert_eq!(fixture.runtime.issue_monitor_launch_preparations.len(), 1);
    assert_eq!(fixture.runtime.issue_monitor_launch_queue.len(), 2);
    assert!(fixture.runtime.pending_launch_feedback_contexts.is_empty());
    assert_eq!(
        queued.lock().expect("queued facts workers").len(),
        2,
        "the first full worker may overlap only the next durable launch's read-only facts"
    );
    let run_worker = |index| {
        let task = queued.lock().expect("queued worker").remove(index);
        task();
    };
    let take_prefetched = || {
        let mut events = fixture.recorded_events.lock().expect("recorded events");
        let index = events
            .iter()
            .position(|event| matches!(event, UserEvent::IssueMonitorLaunchFactsPrefetched(_)))
            .expect("facts-prefetched completion");
        match events.remove(index) {
            UserEvent::IssueMonitorLaunchFactsPrefetched(prefetched) => *prefetched,
            _ => unreachable!("selected facts-prefetched completion"),
        }
    };
    run_worker(0);
    let prepared = take_issue4803_monitor_preparation(&fixture.recorded_events);
    let auth_probe = fixture.runtime.issue_monitor_provider_auth_probe;
    // The successor's speculative failure must still release a FIFO slot.
    fixture.runtime.issue_monitor_provider_auth_probe =
        |_| gwt::issue_monitor::ProviderAuthState::Unauthenticated;
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared);
    fixture.runtime.issue_monitor_provider_auth_probe = auth_probe;
    // Reap the real first materialization before assertions can unwind.
    // Its PTY preparation is never applied, so this test starts no provider.
    assert!(
        !fixture.runtime.pending_launch_feedback_contexts.is_empty(),
        "first materialization not started: preparations={:?}, deliveries={:?}, queued={}",
        fixture.runtime.issue_monitor_launch_preparations,
        fixture.runtime.issue_monitor_launch_deliveries,
        queued.lock().expect("queued workers").len()
    );
    let (first_window, first_result) =
        take_monitor_launch_complete_event("facts-prefetch", &fixture.recorded_events);
    assert!(first_result.is_ok(), "first launch: {first_result:?}");
    assert!(fixture.runtime.issue_monitor_launch_preparations.is_empty());
    run_queued_monitor_gui_diagnostics(&queued, &[3, 2, 1]);
    assert_eq!(
        queued.lock().unwrap().len(),
        2,
        "an idle claim lane permits the waiting head and one successor to compute"
    );
    run_worker(1);
    let successor = take_prefetched();
    fixture
        .runtime
        .handle_issue_monitor_launch_facts_prefetched(successor.clone());
    assert!(fixture.runtime.issue_monitor_launch_preparations.is_empty());
    assert_eq!(fixture.runtime.pending_launch_feedback_contexts.len(), 1);
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("read unclaimed successors");
    assert!(prefs
        .pending_launch_deliveries
        .iter()
        .filter(|delivery| delivery.issue_number != 3165)
        .all(|delivery| delivery.materializer_window_id.is_none()));
    fixture
        .runtime
        .handle_issue_monitor_launch_facts_prefetched(successor);
    assert_eq!(queued.lock().unwrap().len(), 1, "duplicate result ignored");
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3168,
            LinkedIssueKind::Issue,
            Some("launch:external-prefetch-waiter".to_string()),
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
    assert!(fixture.runtime.issue_monitor_launch_preparations.is_empty());
    assert_eq!(fixture.runtime.issue_monitor_launch_queue.len(), 3);
    assert_eq!(
        queued.lock().unwrap().len(),
        1,
        "a new arrival cannot bypass the waiting head or extend its two-entry prefix"
    );
    fixture.runtime.issue_monitor_launch_queue.pop();
    let first_delivery = fixture.delivery_id.clone().expect("first durable delivery");
    let Some(super::super::IssueMonitorLaunchDeliveryState::Materializing { started_at, .. }) =
        fixture
            .runtime
            .issue_monitor_launch_deliveries
            .get_mut(&first_delivery)
    else {
        panic!("the first delivery must still be materializing");
    };
    // Inject expired Materializing age relative to its recorded start, without waiting.
    *started_at -= super::super::ISSUE_MONITOR_MATERIALIZING_TTL + Duration::from_secs(1);
    // Ordinary expired-delivery recovery may queue, but its tracked pane
    // makes another speculative facts calculation unnecessary.
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Issue,
            Some(first_delivery.clone()),
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
    assert!(fixture.runtime.tracked_window_exists(&first_window));
    assert_eq!(fixture.runtime.issue_monitor_launch_queue.len(), 3);
    let expected_window = combined_window_id(
        "tab-1",
        &fixture
            .runtime
            .tab("tab-1")
            .unwrap()
            .workspace
            .next_window_id_preview(WindowPreset::Agent),
    );
    run_worker(0);
    let head = take_prefetched();
    // The ordinary worker must recheck authentication even for valid facts.
    fixture.runtime.issue_monitor_provider_auth_probe =
        |_| gwt::issue_monitor::ProviderAuthState::Unauthenticated;
    fixture
        .runtime
        .handle_issue_monitor_launch_facts_prefetched(head.clone());
    assert!(fixture
        .runtime
        .issue_monitor_launch_preparations
        .iter()
        .any(|key| key.2 == 3166));
    assert_eq!(queued.lock().unwrap().len(), 1);
    let before = gwt_core::process::thread_git_spawn_count();
    run_worker(0);
    assert_eq!(
        gwt_core::process::thread_git_spawn_count() - before,
        2,
        "reused facts leave the source preflight and current fallback claim's Git validation"
    );
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("read current head claim");
    assert_eq!(
        prefs
            .pending_launch_deliveries
            .iter()
            .find(|delivery| delivery.issue_number == 3166)
            .expect("head delivery")
            .materializer_window_id
            .as_deref(),
        Some(expected_window.as_str()),
        "the head must claim the preview after the first pane was registered"
    );
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    assert_eq!(fixture.runtime.pending_launch_feedback_contexts.len(), 1);
    drain_queued_blocking_tasks(&queued);
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    assert!(fixture
        .runtime
        .issue_monitor_launch_preparations
        .iter()
        .any(|key| key.2 == 3167));
    assert_eq!(
        queued.lock().unwrap().len(),
        1,
        "the expired same-delivery pane must not consume another prefetch slot"
    );
    let materializing = fixture.runtime.issue_monitor_launch_deliveries[&first_delivery].clone();
    for settled in [
        super::super::IssueMonitorLaunchDeliveryState::LaunchedPendingAck {
            window_id: first_window.clone(),
        },
        super::super::IssueMonitorLaunchDeliveryState::Launched {
            window_id: first_window.clone(),
        },
        super::super::IssueMonitorLaunchDeliveryState::LaunchFailed {
            message: "settled delivery".to_string(),
            session_mode: gwt_agent::SessionMode::Normal,
            retryable: true,
            fresh_execution: None,
        },
    ] {
        fixture
            .runtime
            .issue_monitor_launch_deliveries
            .insert(first_delivery.clone(), settled);
        fixture.runtime.advance_issue_monitor_launch_queue_events();
        assert_eq!(
            queued.lock().unwrap().len(),
            1,
            "settled deliveries must not start speculative facts workers"
        );
    }
    fixture
        .runtime
        .issue_monitor_launch_deliveries
        .insert(first_delivery, materializing);
    let before = gwt_core::process::thread_git_spawn_count();
    run_worker(0);
    assert!(
        gwt_core::process::thread_git_spawn_count() - before > 2,
        "unavailable speculation must run normal facts preparation"
    );
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    assert_eq!(
        fixture.runtime.pending_launch_feedback_contexts.len(),
        1,
        "the normal authentication failure cannot publish a successor pane"
    );
    drain_queued_blocking_tasks(&queued);
    let preparations = fixture.runtime.issue_monitor_launch_preparations.clone();
    fixture
        .runtime
        .project_tab_incarnations
        .get_mut("tab-1")
        .unwrap()
        .generation += 1;
    assert!(fixture
        .runtime
        .handle_issue_monitor_launch_facts_prefetched(head)
        .is_empty());
    assert_eq!(
        fixture.runtime.issue_monitor_launch_preparations,
        preparations
    );
    assert!(queued.lock().unwrap().is_empty());
}

#[test]
fn issue5596_same_project_preparation_overlaps_after_pane_registration() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "serial-preparation",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
        LinkedIssueKind::Issue,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    );
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    for issue_number in [3165, 3166] {
        fixture
            .runtime
            .auto_launch_issue_monitor_delivery_events_for_project(
                &fixture.project_root,
                issue_number,
                LinkedIssueKind::Issue,
                None,
                gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
            );
    }
    assert_eq!(
        queued.lock().expect("preparation workers").len(),
        1,
        "sibling facts workers must not invalidate each other's preparation"
    );
    drain_queued_blocking_tasks(&queued);
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    fixture.runtime.advance_issue_monitor_launch_queue_events();
    let first_pane_inflight = fixture
        .runtime
        .inflight_launches
        .values()
        .any(|(window_id, _)| {
            fixture
                .runtime
                .pending_launch_feedback_contexts
                .get(window_id)
                .is_some_and(|feedback| feedback.issue_monitor_issue_number == Some(3165))
        });
    let second_preparation_started = fixture
        .runtime
        .issue_monitor_launch_preparations
        .iter()
        .any(|key| key.2 == 3166);
    let (window_id, result) =
        take_monitor_launch_complete_event("serial-preparation", &fixture.recorded_events);
    assert!(result.is_ok(), "first launch: {result:?}");
    run_queued_monitor_gui_diagnostics(&queued, &[2, 1, 0]);
    let next_preparation_count = queued.lock().expect("next preparation").len();
    assert_eq!(
        next_preparation_count, 1,
        "the next facts preparation must start after the first pane is registered"
    );
    assert!(
        first_pane_inflight,
        "the first pane must remain inflight until its completion is applied"
    );
    assert!(
        second_preparation_started,
        "different branches must prepare while the first materialization is running"
    );
    fixture
        .runtime
        .handle_launch_complete(window_id.clone(), result);
    assert_eq!(
        queued.lock().expect("PTY and facts preparation").len(),
        2,
        "the first PTY preparation and next launch facts must overlap"
    );
    assert!(
        fixture
            .runtime
            .issue_monitor_launch_preparations
            .iter()
            .any(|key| key.2 == 3166),
        "the second launch must prepare while the first completion awaits PTY preparation"
    );
    assert!(
        fixture
            .runtime
            .inflight_launches
            .values()
            .any(|(id, _)| id == &window_id),
        "the first pane must remain inflight while its PTY preparation is queued"
    );
    drain_queued_blocking_tasks(&queued);
    let prepared = take_prepared_agent_launch(&fixture.recorded_events);
    // A newer Session supersedes this completion without closing the pane.
    let address = fixture.runtime.window_lookup[&window_id].clone();
    fixture
        .runtime
        .tab_mut(&address.tab_id)
        .unwrap()
        .workspace
        .set_session_id(&address.raw_id, Some("superseding-session".to_string()));
    fixture.runtime.handle_agent_launch_prepared(prepared);
    drain_queued_blocking_tasks(&queued);
    assert!(
        fixture
            .runtime
            .inflight_launches
            .values()
            .all(|(id, _)| id != &window_id),
        "a stale completed worker must release its own materialization lane"
    );
    assert!(
        fixture
            .runtime
            .issue_monitor_launch_preparations
            .iter()
            .any(|key| key.2 == 3166),
        "the second launch must progress on completion without waiting for the scheduled tick"
    );
}

#[test]
fn issue5596_prepared_fresh_launch_does_not_run_git_on_gui_thread() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "prepared-fresh",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
        LinkedIssueKind::Issue,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    );
    let previous_spawner = fixture.runtime.blocking_tasks.clone();
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Issue,
            None,
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
    drain_queued_blocking_tasks(&queued);
    fixture.runtime.blocking_tasks = previous_spawner;
    // Admission must reserve the name on its worker. Holding the real history
    // file makes any GUI-side reallocation fail immediately and causally.
    let history_path = gwt_core::paths::gwt_project_dir_for_repo_path(&fixture.project_root)
        .join("agent-names.jsonl");
    fs::create_dir_all(history_path.parent().unwrap()).expect("name history directory");
    let history = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&history_path)
        .expect("open name history");
    fs2::FileExt::lock_exclusive(&history).expect("hold name history after admission");
    let before = gwt_core::process::thread_git_spawn_count();
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    let git_spawns = gwt_core::process::thread_git_spawn_count() - before;
    let named_pane = fixture
        .runtime
        .tab("tab-1")
        .unwrap()
        .workspace
        .persisted()
        .windows
        .iter()
        .any(|window| {
            window.linked_issue_number == Some(3165)
                && window
                    .agent_name
                    .as_ref()
                    .is_some_and(|name| !name.is_empty())
        });
    fs2::FileExt::unlock(&history).expect("release name history");
    // A busy history refusal creates no pane or LaunchComplete. Reap only an
    // actual launch before asserting, including on the RED path.
    let result = named_pane
        .then(|| take_monitor_launch_complete("prepared-fresh", &fixture.recorded_events));
    assert!(
        named_pane,
        "prepared completion must publish its worker-reserved name while history is locked"
    );
    assert!(result.as_ref().unwrap().is_ok(), "fresh launch: {result:?}");
    assert_eq!(
        git_spawns, 0,
        "prepared completion spawned Git on the GUI thread"
    );
}

#[test]
fn issue5596_prepared_name_initial_collision_reprepares_before_publishing_a_pane() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "prepared-name-collision",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
        LinkedIssueKind::Issue,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    );
    let tab = fixture.runtime.tab_mut("tab-1").unwrap();
    let other = tab
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds());
    tab.workspace.set_linked_issue_number(&other.id, Some(3166));
    tab.workspace
        .set_status(&other.id, WindowProcessStatus::Running);
    fixture.runtime.register_window("tab-1", &other.id);
    gwt::machine_agent_settings::set(
        Some(3),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-23T00:00:00Z",
    )
    .expect("capacity for both issues");
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Issue,
            None,
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
    drain_queued_blocking_tasks(&queued);
    let prepared = take_issue4803_monitor_preparation(&fixture.recorded_events);
    let history_path = gwt_core::paths::gwt_project_dir_for_repo_path(&fixture.project_root)
        .join("agent-names.jsonl");
    let history = fs::read_to_string(&history_path).expect("worker-reserved name history");
    let reserved: String = serde_json::from_str(history.lines().last().unwrap()).unwrap();
    // The existing pane and proposed window ID stay the same. Only its name
    // changes after capture, so the initial collision must trigger the retry.
    let tab = fixture.runtime.tab_mut("tab-1").unwrap();
    assert!(tab
        .workspace
        .set_agent_name(&other.id, Some(reserved.clone())));
    let before = gwt_core::process::thread_git_spawn_count();
    assert!(fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared)
        .is_empty());
    assert_eq!(gwt_core::process::thread_git_spawn_count() - before, 0);
    assert_eq!(
        fixture
            .runtime
            .tab("tab-1")
            .unwrap()
            .workspace
            .persisted()
            .windows
            .len(),
        1,
        "a conflicting reserved initial must not publish a pane"
    );
    run_queued_monitor_gui_diagnostics(&queued, &[3, 1, 0]);
    assert_eq!(queued.lock().unwrap().len(), 1, "reprepare on a worker");
    drain_queued_blocking_tasks(&queued);
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    let windows = &fixture
        .runtime
        .tab("tab-1")
        .unwrap()
        .workspace
        .persisted()
        .windows;
    let launched_name = windows
        .iter()
        .find(|window| window.linked_issue_number == Some(3165))
        .and_then(|window| window.agent_name.as_ref())
        .expect("reprepared pane has a reserved name");
    assert_ne!(launched_name.chars().next(), reserved.chars().next());
    assert_eq!(windows.len(), 2);
}

#[test]
fn issue5596_preclaim_reused_by_another_owner_reprepares_a_new_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    gwt::machine_agent_settings::set(
        Some(2),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-23T00:00:00Z",
    )
    .expect("capacity for both preclaim owners");
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "two-owner-preclaim",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    // Two workers can capture this preview before either creates a pane.
    let foreign = fixture
        .runtime
        .tab_mut("tab-1")
        .unwrap()
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds());
    let foreign_window_id = combined_window_id("tab-1", &foreign.id);
    let tab = fixture.runtime.tab_mut("tab-1").unwrap();
    assert!(tab
        .workspace
        .set_linked_issue_number(&foreign.id, Some(3166)));
    tab.workspace
        .set_status(&foreign.id, WindowProcessStatus::Running);
    let expected_window_id = combined_window_id(
        "tab-1",
        &tab.workspace.next_window_id_preview(WindowPreset::Agent),
    );
    fixture.runtime.register_window("tab-1", &foreign.id);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let delivery = prefs
            .pending_launch_deliveries
            .iter_mut()
            .find(|delivery| Some(&delivery.delivery_id) == fixture.delivery_id.as_ref())
            .expect("queued owner A");
        delivery.materializer_id = Some(fixture.runtime.issue_monitor_materializer_id.clone());
        delivery.materializer_pid = Some(std::process::id());
        delivery.materializer_window_id = Some(foreign_window_id.clone());
        Ok(())
    })
    .expect("seed owner A's stale preclaim before owner B used its preview");
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Spec,
            fixture.delivery_id.clone(),
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
    drain_queued_blocking_tasks(&queued);
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("read reprepared claim");
    let delivery = prefs
        .pending_launch_deliveries
        .iter()
        .find(|delivery| Some(&delivery.delivery_id) == fixture.delivery_id.as_ref())
        .expect("owner A delivery");
    assert_eq!(
        delivery.materializer_window_id.as_deref(),
        Some(expected_window_id.as_str()),
        "owner A must not reclaim owner B's pane merely because its ID matches an old preview"
    );
    assert_eq!(
        fixture
            .runtime
            .tab("tab-1")
            .unwrap()
            .workspace
            .persisted()
            .windows[0]
            .linked_issue_number,
        Some(3166)
    );
    assert!(fixture.runtime.pending_launch_feedback_contexts.is_empty());
}

#[test]
fn issue6096_capacity_renewal_refreshes_admission_without_repeating_launch_facts() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    gwt::machine_agent_settings::set(
        Some(3),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-23T00:00:00Z",
    )
    .expect("manual admission slots independent of host observations");
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "capacity-renewal",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
        LinkedIssueKind::Issue,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    );
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Issue,
            fixture.delivery_id.clone(),
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
    drain_queued_blocking_tasks(&queued);
    let prepared = take_issue4803_monitor_preparation(&fixture.recorded_events);
    let history = gwt_core::paths::gwt_project_dir_for_repo_path(&fixture.project_root)
        .join("agent-names.jsonl");
    let reserved_names = fs::read(&history).expect("worker reserved identity");
    // The host sampler creates/renews this file independently of launch facts.
    let capacity = gwt_core::paths::gwt_home().join("machine-state/agent-capacity.json");
    fs::create_dir_all(capacity.parent().unwrap()).unwrap();
    fs::write(&capacity, b"{}").unwrap();
    assert!(fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared)
        .is_empty());
    let before = gwt_core::process::thread_git_spawn_count();
    drain_queued_blocking_tasks(&queued);
    assert_eq!(
        fs::read(&history).unwrap(),
        reserved_names,
        "capacity renewal must retain the prepared identity"
    );
    assert_eq!(
        gwt_core::process::thread_git_spawn_count() - before,
        1,
        "only final admission may validate Git; claim/base/prompt facts stay prepared"
    );
    // A refreshed completion is queued, without creating a pane on the worker.
    let refreshed = take_issue4803_monitor_preparation(&fixture.recorded_events);
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(refreshed);
    assert!(
        !fixture.runtime.pending_launch_feedback_contexts.is_empty(),
        "refreshed admission must reach pane creation, not another preparation"
    );

    let input = fixture.runtime.capture_issue_monitor_spawn_admission(
        &fixture.project_root,
        3165,
        "work/issue-3165",
    );
    let previous = input.prepare().unwrap();
    gwt::machine_agent_settings::set(
        Some(1),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-23T00:00:00Z",
    )
    .unwrap();
    assert!(
        input
            .refresh(&previous)
            .is_err_and(|error| error == super::super::launch::ISSUE_MONITOR_SPAWN_ADMISSION_STALE),
        "a policy edit while the refresh is queued must retain the original fence"
    );
}

#[test]
fn issue5596_unrelated_prefs_and_window_change_reuses_fresh_launch_facts() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    gwt::machine_agent_settings::set(
        Some(3),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-23T00:00:00Z",
    )
    .expect("allow the unrelated pane and the pending launch");
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "reuse-fresh-facts",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
        LinkedIssueKind::Issue,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    );
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Issue,
            fixture.delivery_id.clone(),
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
    drain_queued_blocking_tasks(&queued);
    let prepared = take_issue4803_monitor_preparation(&fixture.recorded_events);
    let tab = fixture.runtime.tab_mut("tab-1").unwrap();
    let unrelated = tab
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds());
    tab.workspace
        .set_linked_issue_number(&unrelated.id, Some(3166));
    tab.workspace
        .set_status(&unrelated.id, WindowProcessStatus::Running);
    let expected_window_id = combined_window_id(
        "tab-1",
        &tab.workspace.next_window_id_preview(WindowPreset::Agent),
    );
    fixture.runtime.register_window("tab-1", &unrelated.id);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        prefs.priority_order.push(3166);
        Ok(())
    })
    .expect("change unrelated queue priority prefs");
    assert!(fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared)
        .is_empty());
    run_queued_monitor_gui_diagnostics(&queued, &[2, 0]);
    assert_eq!(
        queued.lock().unwrap().len(),
        1,
        "metadata changes must still revalidate on a worker"
    );
    let before = gwt_core::process::thread_git_spawn_count();
    drain_queued_blocking_tasks(&queued);
    let repeated_git = gwt_core::process::thread_git_spawn_count() - before;
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("read refreshed delivery");
    let claim = prefs
        .pending_launch_deliveries
        .iter()
        .find(|delivery| Some(&delivery.delivery_id) == fixture.delivery_id.as_ref())
        .expect("delivery retained");
    assert_eq!(
        claim.materializer_window_id.as_deref(),
        Some(expected_window_id.as_str()),
        "the reused facts must claim the current proposed pane"
    );
    // Source preflight, the offline fallback claim, and final pane admission
    // validate the actual origin. Base/hydration/prompt facts remain reused.
    assert_eq!(
        repeated_git, 3,
        "only the current source checks and claim may validate Git"
    );
    assert!(fixture.runtime.pending_launch_feedback_contexts.is_empty());
}

fn assert_issue5596_unsubmitted_answer_packet_retries(close_project: bool) {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "unsubmitted-answer",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    seed_resumed_autonomous_handoff(&fixture, "Keep the exact Session");
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
    let prepared = take_issue4803_monitor_preparation(&fixture.recorded_events);
    assert!(fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared)
        .is_empty());
    drain_queued_blocking_tasks(&queued);
    let prepared = take_issue4803_monitor_preparation(&fixture.recorded_events);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    assert!(
        matches!(
            gwt::load_issue_monitor_prefs(&prefs_path)
                .unwrap()
                .autonomous_handoffs[0]
                .delivery,
            gwt::autonomous_handoff::AutonomousHandoffDeliveryState::Attempting { attempt: 1, .. }
        ),
        "the callback must carry an existing unsubmitted write-ahead token"
    );
    if close_project {
        fixture.runtime.close_project_tab_events("tab-1");
    } else {
        gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
            prefs.launch_profile.as_mut().expect("saved profile").model =
                Some("changed-while-answer-worker-returned".to_string());
            Ok(())
        })
        .expect("change profile after write-ahead");
    }
    assert!(fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared)
        .is_empty());
    drain_queued_blocking_tasks(&queued);
    let prefs =
        gwt::load_issue_monitor_prefs(&prefs_path).expect("load settled unsubmitted attempt");
    assert!(
        matches!(
            prefs.autonomous_handoffs[0].delivery,
            gwt::autonomous_handoff::AutonomousHandoffDeliveryState::RetryBackoff {
                attempt: 1,
                ..
            }
        ),
        "unsubmitted token must retain bounded exact-session retry: {:?}",
        prefs.autonomous_handoffs[0].delivery
    );
    assert!(prefs.autonomous_handoffs[0].delivered_at.is_none());
    assert!(fixture.runtime.pending_launch_feedback_contexts.is_empty());
}

#[test]
fn issue5596_answer_packet_pref_stale_settles_unsubmitted_attempt() {
    assert_issue5596_unsubmitted_answer_packet_retries(false);
}

#[test]
fn issue5596_answer_packet_project_closed_settles_unsubmitted_attempt() {
    assert_issue5596_unsubmitted_answer_packet_retries(true);
}

#[test]
fn issue5596_answer_packet_keeps_the_exact_holder_when_another_issue_pane_is_first() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "answer-exact-holder",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::ActiveSameConversation,
        true,
    );
    seed_resumed_autonomous_handoff(&fixture, "Continue the asking conversation");
    gwt::machine_agent_settings::set(
        Some(3),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-23T00:00:00Z",
    )
    .expect("allow the exact holder and the unrelated live pane");
    let holder_window_id = fixture.holder_window_id.clone().expect("exact holder");
    insert_test_pane_runtime(&mut fixture.runtime, &holder_window_id);
    let tab = fixture.runtime.tab_mut("tab-1").unwrap();
    let other = tab
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds());
    tab.workspace.set_linked_issue_number(&other.id, Some(3165));
    tab.workspace
        .set_status(&other.id, WindowProcessStatus::Running);
    let holder_raw_id = tab.workspace.persisted().windows[0].id.clone();
    assert!(tab
        .workspace
        .set_agent_name(&holder_raw_id, Some("ExistingExactHolder".to_string())));
    let mut persisted = tab.workspace.persisted().clone();
    persisted.windows.swap(0, 1);
    tab.workspace = WindowCanvasState::from_persisted(persisted);
    fixture.runtime.register_window("tab-1", &other.id);
    assert_eq!(
        fixture
            .runtime
            .live_issue_agent_window("tab-1", 3165, "work/issue-3165"),
        Some(combined_window_id("tab-1", &other.id)),
        "generic Issue lookup must differ from the exact native holder"
    );
    let (spawner, queued) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    // Both answer workers encounter a busy canonical history. Their unused
    // name result must not prevent delivery to the existing named holder.
    let history_path = gwt_core::paths::gwt_project_dir_for_repo_path(&fixture.project_root)
        .join("agent-names.jsonl");
    fs::create_dir_all(history_path.parent().unwrap()).expect("name history directory");
    let history = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&history_path)
        .expect("open name history");
    fs2::FileExt::lock_exclusive(&history).expect("hold name history before answer admission");
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Spec,
            fixture.delivery_id.clone(),
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
    let mut preparation_states = Vec::new();
    let preparation_phases = capture_tracing_events(|| {
        for _ in 0..2 {
            drain_queued_blocking_tasks(&queued);
            fixture.runtime.handle_issue_monitor_launch_prepared(
                take_issue4803_monitor_preparation(&fixture.recorded_events),
            );
            let prefs = gwt::load_issue_monitor_prefs(
                &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root),
            )
            .expect("read exact-holder preparation state");
            preparation_states.push((
                fixture
                    .runtime
                    .issue_monitor_launch_deliveries
                    .get(fixture.delivery_id.as_deref().unwrap())
                    .cloned(),
                fixture
                    .runtime
                    .pending_launch_feedback_contexts
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>(),
                queued.lock().expect("queued exact-holder tasks").len(),
                prefs.autonomous_handoffs[0].delivery.clone(),
            ));
        }
    });
    fs2::FileExt::unlock(&history).expect("release name history");
    let windows = &fixture
        .runtime
        .tab("tab-1")
        .unwrap()
        .workspace
        .persisted()
        .windows;
    assert_eq!(windows.len(), 2, "an answer must keep the existing panes");
    assert_eq!(
        windows
            .iter()
            .find(|window| combined_window_id("tab-1", &window.id) == holder_window_id)
            .unwrap()
            .agent_name
            .as_deref(),
        Some("ExistingExactHolder"),
        "an answer must preserve its holder's name despite a busy history"
    );
    assert!(
        matches!(
            fixture.runtime.issue_monitor_launch_deliveries.get(
                fixture.delivery_id.as_deref().expect("durable delivery")
            ),
            Some(super::super::IssueMonitorLaunchDeliveryState::Materializing { window_id, .. })
                if window_id == &holder_window_id
        ),
        "the prepared answer must reach its exact holder instead of retrying against the generic Issue pane; states={preparation_states:?}; phases={preparation_phases:?}"
    );
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

#[test]
fn issue5596_answer_worker_uses_current_autonomous_mode() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _autonomous_execution =
        ScopedEnvVar::unset(gwt::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV);
    let _autonomous_issue = ScopedEnvVar::unset(gwt::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "answer-current-autonomous-mode",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    seed_resumed_autonomous_handoff(&fixture, "Continue the exact Session");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        prefs.autonomous_mode = false;
        Ok(())
    })
    .expect("begin preparation without autonomous mode");
    let previous_spawner = fixture.runtime.blocking_tasks.clone();
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
    assert!(fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ))
        .is_empty());
    run_queued_monitor_gui_diagnostics(&queued, &[2, 1]);
    assert_eq!(
        queued.lock().expect("queued answer worker").len(),
        1,
        "the exact answer must await its write-ahead worker"
    );
    // The second worker accepts a new prefs stamp, so its launch facts must
    // reflect a mode edit made after the first preparation callback.
    gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        prefs.autonomous_mode = true;
        Ok(())
    })
    .expect("enable autonomous mode before answer preparation");
    drain_queued_blocking_tasks(&queued);
    fixture.runtime.blocking_tasks = previous_spawner;
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    let result =
        take_monitor_launch_complete("answer-current-autonomous-mode", &fixture.recorded_events);
    assert_monitor_exact_resume(result, &fixture);
}
