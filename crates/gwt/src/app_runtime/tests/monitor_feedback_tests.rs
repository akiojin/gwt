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
fn pm_wake_pending_signal_rechecks_subject_before_delivery() {
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
    runtime.pm_wake_events(&repo, &[]);
    runtime.terminal_input_events(&pm_window_id, "draft");
    // Important signals remain held during input; periodic ticks are discarded.
    runtime.pm_wake_events(
        &repo,
        &[pm_wake_inbox_item(42, gwt::MonitorInboxState::NeedsHuman)],
    );
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
            BackendEvent::WorkspaceState { .. }
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
    let _api_key = ScopedEnvVar::unset("OPENAI_API_KEY");
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "facts-prefetch",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
        LinkedIssueKind::Issue,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    );
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    gwt::try_mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root),
        |prefs| {
            let base = prefs.launch_profile_pool()[0].clone();
            let profiles = prefs
                .agent_roster
                .as_ref()
                .unwrap()
                .members
                .iter()
                .filter(|member| member.enabled && member.agent_id.as_deref() == Some("codex"))
                .take(3)
                .map(|member| {
                    let mut profile = base.clone();
                    profile.agent_member_id = Some(member.agent_member_id.clone());
                    profile
                })
                .collect::<Vec<_>>();
            assert_eq!(
                profiles.len(),
                3,
                "registered people selected by this scenario"
            );
            prefs.set_launch_profile_pool(profiles);
            Ok(())
        },
    )
    .expect("select the actual registered people before facts capture");
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
    let mut scheduled = monitor.prefs();
    let members = &scheduled
        .agent_roster
        .as_ref()
        .expect("registered FIFO people")
        .members;
    for delivery in &mut scheduled.pending_launch_deliveries {
        let name = match delivery.issue_number {
            3165 => "Alpha",
            3166 => "Bravo",
            3167 => "Charlie",
            _ => unreachable!("the three actual FIFO deliveries"),
        };
        delivery.agent_member_id = Some(
            members
                .iter()
                .find(|member| member.name == name)
                .expect("the selected registered FIFO person")
                .agent_member_id
                .clone(),
        );
    }
    gwt::save_issue_monitor_prefs(&prefs_path, &scheduled).expect("save durable deliveries");
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
    // Keep the GUI reply pending while the physical owner owns the actual
    // first provider. Its authenticated admission is the durable claim fence.
    assert!(
        !fixture.runtime.pending_launch_feedback_contexts.is_empty(),
        "first materialization not started: preparations={:?}, deliveries={:?}, queued={}",
        fixture.runtime.issue_monitor_launch_preparations,
        fixture.runtime.issue_monitor_launch_deliveries,
        queued.lock().expect("queued workers").len()
    );
    // The extra queued worker carries immutable RPC intent; it cannot run a
    // second read-only facts calculation or claim from the GUI thread.
    run_worker(1);
    let first_completion = take_remote_owner_launch_completion(&fixture.recorded_events);
    let first_window = first_completion.window_id.clone();
    let first_snapshot = match first_completion.result.as_ref().unwrap() {
        crate::app_runtime::RemoteOwnerLaunchOutcome::Runtime(snapshot, _) => snapshot,
        _ => panic!("the actual first owner must retain its physical prepared candidate"),
    };
    let first_pane =
        owner_fixture_block_on(owner.probe(&first_snapshot.id, first_snapshot.incarnation))
            .unwrap();
    assert!(first_pane.pty.try_wait().unwrap().is_none());
    let current = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    let first_claim = current
        .pending_launch_deliveries
        .iter()
        .find(|delivery| delivery.issue_number == 3165)
        .unwrap();
    let materializer_id = owner.client().identity().unwrap().host_instance_id.clone();
    assert_eq!(
        first_claim.materializer_id.as_deref(),
        Some(materializer_id.as_str())
    );
    assert_eq!(first_claim.materializer_pid, Some(std::process::id()));
    assert_eq!(
        first_claim.materializer_window_id.as_deref(),
        Some(first_pane.id.as_str())
    );
    assert!(fixture.runtime.issue_monitor_launch_preparations.is_empty());
    run_queued_monitor_gui_diagnostics(&queued, &[2, 1]);
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
        0,
        "reused readonly facts must not run the removed GUI fallback claim; only the actual owner may validate and claim a materializer"
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
        None,
        "an unauthenticated successor's read-only facts cannot claim the GUI preview"
    );
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    assert_eq!(fixture.runtime.pending_launch_feedback_contexts.len(), 1);
    assert!(
        !fixture.runtime.tracked_window_exists(&expected_window),
        "the refused FIFO head must not consume its preview pane"
    );
    // Settle the immutable rejection on the real owner. The other queued
    // task prepares 3167; it must remain pending throughout this FIFO check.
    run_worker(0);
    let rejection = {
        let mut events = fixture.recorded_events.lock().unwrap();
        let index = events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::IssueMonitorPreflightRejected(_)
                )
            })
            .expect("owner rejection completion");
        events.remove(index)
    };
    let UserEvent::IssueMonitorPreflightRejected(rejection) = fixture
        .runtime
        .accept_project_completion(rejection)
        .expect("current rejection context")
    else {
        unreachable!("selected rejection completion")
    };
    assert!(
        rejection.result.is_ok(),
        "the actual owner must settle its no-PTY refusal: {:?}",
        rejection.result
    );
    fixture
        .runtime
        .handle_monitor_preflight_rejected(*rejection);
    assert!(fixture
        .runtime
        .issue_monitor_launch_preparations
        .iter()
        .any(|key| key.2 == 3167),
        "FIFO head settlement must advance to 3167: preparations={:?}, queue={:?}, deliveries={:?}, workers={}",
        fixture.runtime.issue_monitor_launch_preparations,
        fixture.runtime.issue_monitor_launch_queue.len(),
        fixture.runtime.issue_monitor_launch_deliveries,
        queued.lock().unwrap().len(),
    );
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
    owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::PendingCancel {
            id: first_pane.id.clone(),
            operation_id: first_completion.operation_id,
        },
    ))
    .unwrap();
    assert!(
        first_pane.pty.try_wait().unwrap().is_some(),
        "the exact unaccepted candidate must be physically reaped"
    );
}

#[test]
fn issue5596_same_project_preparation_overlaps_after_pane_registration() {
    use gwt::project_runtime::{events::OwnerCommand, launch_payload::OwnerLaunchMetadata};
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (root, owner, _) =
        owner_monitor_fixture(temp.path(), 3165, false, &["pane-contract-fixture"]);
    let pause = owner_fixture_block_on(owner.pause_next_agent_launch()).unwrap();
    let id = owner.pane_id("first-materialization");
    let operation_id = "first-monitor-materialization".to_string();
    let client = owner.client();
    let config = super::pane_control_tests::control_launch_config(&root, 3165);
    let mut feedback = issue_monitor_feedback(3165);
    feedback.issue_monitor_project_root = Some(root.clone());
    let first_id = id.clone();
    let first_operation = operation_id.clone();
    let first = thread::spawn(move || {
        owner_fixture_block_on(client.command(OwnerCommand::LaunchAgent {
            id: first_id.clone(),
            operation_id: first_operation,
            tab_id: first_id.split_once("::").unwrap().0.into(),
            config: Box::new(config),
            metadata: Box::new(OwnerLaunchMetadata {
                launch_feedback_context: Some(feedback),
                ..Default::default()
            }),
            cols: 80,
            rows: 24,
        }))
    });
    // test-hygiene: allow-wall-clock-deadline Bounds a real prepared worker/owner IPC hang; the checkpoint establishes ordering.
    let evidence = owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bounds an actual owner worker checkpoint or cancellation observation; the checkpoint establishes ordering.
        tokio::time::timeout(Duration::from_secs(30), pause.wait())
            .await
            .unwrap()
    });
    assert!(evidence.pty.as_ref().unwrap().try_wait().unwrap().is_none());
    let registry =
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(&root))
            .unwrap();
    assert!(
        registry
            .windows
            .iter()
            .any(|row| row.session_id == evidence.session_id),
        "the first actual pane must be registered before sibling preparation"
    );
    assert!(
        !first.is_finished(),
        "the first completion remains pending at its actual worker checkpoint"
    );
    let second = super::pane_control_tests::control_owner_launch(
        &owner,
        "second-materialization",
        super::pane_control_tests::control_launch_config(&root, 3166),
    );
    assert!(
        second.pty.try_wait().unwrap().is_none(),
        "different branches must prepare and materialize while the first completion is pending"
    );
    assert!(!first.is_finished());
    let client = owner.client();
    let cancel = thread::spawn(move || {
        owner_fixture_block_on(client.command(OwnerCommand::PendingCancel { id, operation_id }))
    });
    // test-hygiene: allow-wall-clock-deadline Bounds observing the actual owner cancellation flag; no deadline establishes authority.
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bounds an actual owner worker checkpoint or cancellation observation; the checkpoint establishes ordering.
        tokio::time::timeout(Duration::from_secs(30), async {
            while evidence.current.load(std::sync::atomic::Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    });
    assert!(
        !evidence.current.load(std::sync::atomic::Ordering::Acquire),
        "cancellation must revoke the captured worker incarnation"
    );
    pause.release();
    cancel.join().unwrap().unwrap();
    assert!(
        first.join().unwrap().is_err(),
        "a revoked completion cannot install its physical candidate"
    );
    assert!(
        evidence.pty.as_ref().unwrap().try_wait().unwrap().is_some(),
        "the exact stale candidate must be reaped"
    );
    let third = super::pane_control_tests::control_owner_launch(
        &owner,
        "next-materialization",
        super::pane_control_tests::control_launch_config(&root, 3167),
    );
    assert!(
        third.pty.try_wait().unwrap().is_none(),
        "completion must release its lane without a GUI scheduled tick"
    );
    super::pane_control_tests::control_close(&owner, &second);
    super::pane_control_tests::control_close(&owner, &third);
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
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    let roster =
        gwt::agent_names::read_project_roster(&fixture.project_root, &fixture.runtime.sessions_dir)
            .expect("initialize the persistent roster before GUI capture");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let history_path = gwt::agent_registry::history_path();
    gwt::agent_names::remember(
        &history_path,
        &roster
            .members
            .iter()
            .map(|member| member.name.clone())
            .collect(),
    )
    .unwrap();
    let names_before = fs::read(&history_path).expect("pre-existing member names");
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
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .agent_roster
            .as_ref(),
        Some(&roster),
        "read-only preparation must not consume a member"
    );
    assert_eq!(fs::read(&history_path).unwrap(), names_before);
    // Existing members need no per-launch name allocation. Holding history
    // makes any GUI-side allocation fail immediately and causally.
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
    let prepared_git_spawns = gwt_core::process::thread_git_spawn_count() - before;
    // The immutable request remains queued while this GUI callback runs.
    // Its actual owner may reserve the canonical name once the lock is free.
    fs2::FileExt::unlock(&history).expect("release history for the physical owner");
    drain_queued_blocking_tasks(&queued);
    let completion = take_remote_owner_launch_completion(&fixture.recorded_events);
    fs2::FileExt::lock_exclusive(&history).expect("hold history during readonly GUI completion");
    let before = gwt_core::process::thread_git_spawn_count();
    fixture
        .runtime
        .handle_remote_owner_launch_complete(completion);
    let git_spawns = prepared_git_spawns + gwt_core::process::thread_git_spawn_count() - before;
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
                && window.agent_name.as_deref() == Some(roster.members[0].name.as_str())
        });
    fs2::FileExt::unlock(&history).expect("release name history");
    fixture.runtime.blocking_tasks = previous_spawner;
    assert!(
        named_pane,
        "prepared completion must use the existing canonical member while history is locked: windows={:?}, expected={:?}, details={:?}",
        fixture.runtime.tabs[0].workspace.persisted().windows, roster.members[0], fixture.runtime.window_details
    );
    let window_id = combined_window_id(
        "tab-1",
        &fixture.runtime.tabs[0]
            .workspace
            .persisted()
            .windows
            .iter()
            .find(|window| window.linked_issue_number == Some(3165))
            .unwrap()
            .id,
    );
    let pane = owner_runtime_probe(&fixture.runtime, &window_id);
    let session = pane.session.as_ref().unwrap();
    assert_eq!(
        session.agent_member_id.as_deref(),
        Some(roster.members[0].agent_member_id.as_str())
    );
    assert_eq!(fs::read(&history_path).unwrap(), names_before);
    assert_eq!(
        git_spawns, 0,
        "prepared completion spawned Git on the GUI thread"
    );
    authenticate_monitor_owner_readiness(&owner, &pane);
    close_test_owner_runtime(&mut fixture.runtime, &window_id);
}

#[test]
fn issue5596_prepared_roster_revision_revalidates_before_publishing_a_pane() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let mut fixture = monitor_relaunch_fixture_with_settlement(
        temp.path(),
        "prepared-roster-revision",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
        LinkedIssueKind::Issue,
        gwt::cli::execution_state::ExecutionSettlement::Completed,
    );
    let roster = gwt::agent_names::mutate_project_roster(
        &fixture.project_root,
        &fixture.runtime.sessions_dir,
        gwt::agent_names::AgentRosterAction::Resize { size: 3 },
        None,
    )
    .expect("initialize enough persistent members for the stale preparation");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let profile = prefs.launch_profile_pool()[0].clone();
        prefs.set_launch_profile_pool(
            roster
                .members
                .iter()
                .map(|member| {
                    let mut profile = profile.clone();
                    profile.agent_member_id = Some(member.agent_member_id.clone());
                    profile
                })
                .collect(),
        );
        Ok(())
    })
    .expect("select all three registered people with their saved tuning");
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    let roster = gwt::load_issue_monitor_prefs(&prefs_path)
        .unwrap()
        .agent_roster
        .unwrap();
    let history_path = gwt::agent_registry::history_path();
    let member_names = fs::read(&history_path).expect("pre-existing member names");
    let tab = fixture.runtime.tab_mut("tab-1").unwrap();
    let other = tab.workspace.add_window(WindowPreset::Agent);
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
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .agent_roster
            .as_ref(),
        Some(&roster),
        "read-only preparation must not consume a member"
    );
    // Keep the existing pane and proposed window ID unchanged. Resting the
    // first member changes canonical admission without exhausting capacity.
    let current = gwt::agent_names::mutate_project_roster(
        &fixture.project_root,
        &fixture.runtime.sessions_dir,
        gwt::agent_names::AgentRosterAction::Rest {
            member_id: roster.members[0].agent_member_id.clone(),
        },
        Some(roster.revision),
    )
    .expect("rest the first member after preparation");
    assert!(current.revision > roster.revision);
    assert_eq!(fs::read(&history_path).unwrap(), member_names);
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
        "a stale roster preparation must not publish a pane"
    );
    let revalidation = capture_tracing_events(|| drain_queued_blocking_tasks(&queued));
    assert_eq!(
        revalidation
            .iter()
            .filter(|event| {
                event.fields.get("phase").map(String::as_str) == Some("monitor_gui_revalidate")
            })
            .count(),
        1,
        "revalidate exactly once on a worker"
    );
    assert_eq!(
        fixture
            .recorded_events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, UserEvent::IssueMonitorLaunchPrepared(_)))
            .count(),
        1,
        "the stale roster needs exactly one worker revalidation"
    );
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
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
        "a changed selected person must reprepare before publishing a pane"
    );
    drain_queued_blocking_tasks(&queued);
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    drain_queued_blocking_tasks(&queued);
    let (_, launched, _) = finish_monitor_relaunch_owner(&mut fixture, Vec::new());
    let launched = launched.expect("the physical owner installs the revalidated member");
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
        .expect("revalidated pane has a persistent member name");
    assert_eq!(launched_name, &current.members[1].name);
    assert_eq!(windows.len(), 2);
    let assigned = gwt::load_issue_monitor_prefs(&prefs_path)
        .unwrap()
        .agent_roster
        .unwrap();
    assert!(!assigned.members[0].enabled);
    assert!(assigned.members[0].assignment.is_none());
    assert!(assigned.members[1].enabled);
    assert_eq!(
        assigned.members[1]
            .assignment
            .as_ref()
            .expect("the enabled canonical member was reserved")
            .issue_number,
        Some(3165)
    );
    assert!(fixture.runtime.issue_monitor_launch_preparations.is_empty());
    assert_eq!(fs::read(&history_path).unwrap(), member_names);
    pane_control_tests::control_close(&owner, &launched);
    assert!(launched.pty.try_wait().unwrap().is_some());
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
    Cache::new(issue_cache_root(&fixture.project_root))
        .write_snapshot(&sample_issue_snapshot(
            3166,
            "Foreign producing owner",
            &["bug"],
            "Other work",
            &Utc::now().to_rfc3339(),
        ))
        .unwrap();
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    gwt::try_mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root),
        |prefs| {
            let base = prefs.launch_profile_pool()[0].clone();
            let profiles = prefs
                .agent_roster
                .as_ref()
                .unwrap()
                .members
                .iter()
                .filter(|member| member.enabled && member.agent_id.as_deref() == Some("codex"))
                .take(2)
                .map(|member| {
                    let mut profile = base.clone();
                    profile.agent_member_id = Some(member.agent_member_id.clone());
                    profile
                })
                .collect::<Vec<_>>();
            assert_eq!(
                profiles.len(),
                2,
                "registered people selected by this scenario"
            );
            prefs.set_launch_profile_pool(profiles);
            Ok(())
        },
    )
    .expect("select the actual registered people before facts capture");
    gwt::machine_agent_settings::set(
        Some(2),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        &Utc::now().to_rfc3339(),
    )
    .unwrap();
    // Two workers can capture this preview before either creates a pane.
    let worktree = pane_control_tests::control_launch_config(&fixture.project_root, 3166);
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree.working_dir.unwrap())
        .branch(worktree.branch.unwrap())
        .base_branch(worktree.base_branch.unwrap())
        .skip_permissions(true)
        .build();
    config.linked_issue_number = Some(3166);
    config.explicit_follow_up = true;
    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .unwrap();
    let profiles = prefs.launch_profile_pool();
    let pending_member_id = prefs
        .pending_launch_deliveries
        .iter()
        .find(|delivery| Some(&delivery.delivery_id) == fixture.delivery_id.as_ref())
        .and_then(|delivery| delivery.agent_member_id.as_deref())
        .or_else(|| {
            profiles
                .first()
                .and_then(|profile| profile.agent_member_id.as_deref())
        })
        .expect("the pending launch has its scheduled or primary person");
    let foreign_member_id = profiles
        .iter()
        .find(|profile| {
            profile.agent_id == "codex"
                && profile.agent_member_id.as_deref() != Some(pending_member_id)
                && prefs
                    .agent_roster
                    .as_ref()
                    .unwrap()
                    .members
                    .iter()
                    .any(|member| {
                        Some(&member.agent_member_id) == profile.agent_member_id.as_ref()
                            && member.enabled
                            && member.assignment.is_none()
                    })
        })
        .and_then(|profile| profile.agent_member_id.clone())
        .expect("a distinct registered idle foreign person");
    let foreign_id = owner.pane_id("agent-1");
    let receipt = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::LaunchAgent {
            id: foreign_id.clone(),
            operation_id: "unrelated-preclaim-person".into(),
            tab_id: foreign_id.split_once("::").unwrap().0.into(),
            config: Box::new(config),
            metadata: Box::new(gwt::project_runtime::launch_payload::OwnerLaunchMetadata {
                selected_agent_member_id: Some(foreign_member_id),
                ..Default::default()
            }),
            cols: 80,
            rows: 24,
        },
    ))
    .unwrap();
    let foreign =
        owner_fixture_block_on(owner.probe(&foreign_id, receipt["incarnation"].as_u64().unwrap()))
            .unwrap();
    assert_eq!(foreign.session.as_ref().unwrap().id, receipt["session_id"]);
    assert!(foreign
        .session
        .as_ref()
        .unwrap()
        .execution_binding
        .is_some());
    assert!(
        foreign.grant.is_some(),
        "the production owner must issue the request capability"
    );
    issue5596_project_owner_pane(&mut fixture, &owner, &foreign);
    let foreign_window_id = foreign.id.clone();
    let tab = fixture.runtime.tab_mut("tab-1").unwrap();
    assert!(tab.workspace.set_linked_issue_number("agent-1", Some(3166)));
    tab.workspace
        .set_status("agent-1", WindowProcessStatus::Running);
    let expected_window_id = combined_window_id(
        "tab-1",
        &tab.workspace.next_window_id_preview(WindowPreset::Agent),
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let delivery = prefs
            .pending_launch_deliveries
            .iter_mut()
            .find(|delivery| Some(&delivery.delivery_id) == fixture.delivery_id.as_ref())
            .expect("queued owner A");
        delivery.materializer_id =
            Some(owner.client().identity().unwrap().host_instance_id.clone());
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
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    while !fixture.runtime.issue_monitor_launch_preparations.is_empty() {
        drain_queued_blocking_tasks(&queued);
        fixture
            .runtime
            .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
                &fixture.recorded_events,
            ));
    }
    drain_queued_blocking_tasks(&queued);
    let (window_id, launched, _) = finish_monitor_relaunch_owner(&mut fixture, Vec::new());
    let launched = launched.expect("the exact owner must reprepare a separate physical pane");
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("read reprepared claim");
    let delivery = prefs
        .pending_launch_deliveries
        .iter()
        .find(|delivery| Some(&delivery.delivery_id) == fixture.delivery_id.as_ref())
        .expect("owner A delivery");
    assert_eq!(
        delivery.materializer_window_id.as_deref(),
        Some(launched.id.as_str()),
        "owner A must not reclaim owner B's pane merely because its ID matches an old preview"
    );
    assert_eq!(window_id, expected_window_id);
    let unchanged = owner_fixture_block_on(owner.probe(&foreign.id, foreign.incarnation)).unwrap();
    assert_eq!(
        unchanged.session.as_ref().unwrap().id,
        foreign.session.as_ref().unwrap().id
    );
    assert_eq!(unchanged.pty.process_id(), foreign.pty.process_id());
    assert!(Arc::ptr_eq(&unchanged.pty, &foreign.pty));
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
    pane_control_tests::control_close(&owner, &launched);
    pane_control_tests::control_close(&owner, &foreign);
}

fn issue5596_project_owner_pane(
    fixture: &mut MonitorRelaunchFixture,
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    pane: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
) {
    let raw_id = pane.id.split_once("::").unwrap().1;
    fixture.runtime.tab_mut("tab-1").unwrap().workspace = WindowRegistry::from_persisted(
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(
            &fixture.project_root,
        ))
        .unwrap(),
    );
    fixture.runtime.register_window("tab-1", raw_id);
    let snapshot = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::Snapshot {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
        },
    ))
    .unwrap();
    fixture.runtime.install_remote_owner_projection(
        &fixture.runtime.test_context(),
        &combined_window_id("tab-1", raw_id),
        owner.client(),
        serde_json::from_value(snapshot).unwrap(),
        pane.session.clone(),
    );
}

#[test]
fn issue6096_capacity_renewal_keeps_admission_without_repeating_launch_facts() {
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
    let roster =
        gwt::agent_names::read_project_roster(&fixture.project_root, &fixture.runtime.sessions_dir)
            .expect("initialize the persistent roster before admission");
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    gwt::machine_agent_settings::set(
        Some(3),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-23T00:00:00Z",
    )
    .expect("retain this acceptance's explicit capacity after native owner setup");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let history = gwt::agent_registry::history_path();
    let member_names = fs::read(&history).expect("pre-existing member names");
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
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .agent_roster
            .as_ref(),
        Some(&roster),
        "read-only preparation must not consume a member"
    );
    assert_eq!(fs::read(&history).unwrap(), member_names);
    // The host sampler creates/renews this file independently of launch facts.
    // Issue #6096: a renewal that grants the same slot limit is settled on the
    // GUI by that decision, so a host slower than the ~5s renewal cadence
    // still launches instead of re-evaluating against every new sample.
    let capacity = gwt_core::paths::gwt_home().join("machine-state/agent-capacity.json");
    fs::create_dir_all(capacity.parent().unwrap()).unwrap();
    fs::write(&capacity, b"{}").unwrap();
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared);
    assert!(
        !fixture.runtime.pending_launch_feedback_contexts.is_empty(),
        "a same-limit capacity renewal must reach pane creation, not another preparation"
    );
    assert_eq!(
        fs::read(&history).unwrap(),
        member_names,
        "capacity renewal must reuse the persistent member names"
    );
    assert!(fixture.runtime.issue_monitor_launch_preparations.is_empty());
    drain_queued_blocking_tasks(&queued);
    let (_, launched, _) = finish_monitor_relaunch_owner(&mut fixture, Vec::new());
    let launched = launched.expect("the physical owner accepts the same capacity decision");
    let assigned = gwt::load_issue_monitor_prefs(&prefs_path)
        .unwrap()
        .agent_roster
        .unwrap();
    let member = assigned
        .members
        .iter()
        .find(|member| {
            member
                .assignment
                .as_ref()
                .is_some_and(|assignment| assignment.issue_number == Some(3165))
        })
        .expect("capacity renewal admits one canonical member");
    assert_eq!(member.agent_member_id, roster.members[0].agent_member_id);

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
    pane_control_tests::control_close(&owner, &launched);
    assert!(launched.pty.try_wait().unwrap().is_some());
}

#[test]
fn issue6096_slow_admission_launches_after_one_revalidation_despite_rewrites() {
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
        "prefs-churn",
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
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    let capacity = gwt_core::paths::gwt_home().join("machine-state/agent-capacity.json");
    fs::create_dir_all(capacity.parent().unwrap()).unwrap();
    // Issue #6096: model an admission that takes ~15s on a loaded host without
    // waiting for it. In that window the sampler renews the capacity file
    // about every 5s and the daemon rewrites unrelated prefs records. The
    // decision (a free slot, no drain/hold, Issue open) never changes, so the
    // launch must open after at most one re-evaluation, however slow it was.
    let mut renewal = 0_u64;
    let mut slow_evaluation_window = |priority_order: Vec<u64>| {
        for _ in 0..3 {
            renewal += 1;
            let observed_at = u64::try_from(Utc::now().timestamp()).unwrap() + renewal;
            fs::write(
                &capacity,
                serde_json::to_vec(&serde_json::json!({
                    "observed_at": observed_at, "expires_at": observed_at + 30,
                    "performance_cores": 8, "gui_cpu_millicores": renewal * 7,
                    "available_ram_bytes": (64 + renewal) * 1024 * 1024 * 1024,
                    "per_agent_ram_bytes": 512_u64 * 1024 * 1024,
                    "free_disk_bytes": 1_u64 << 40,
                    "inventory": {"sessions": [], "uncertainties": []}
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        prefs.priority_order = priority_order;
        gwt::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
    };
    slow_evaluation_window(vec![9001]);
    assert!(
        fixture
            .runtime
            .handle_issue_monitor_launch_prepared(prepared)
            .is_empty(),
        "unrelated rewrites schedule one worker re-evaluation"
    );
    assert!(fixture.runtime.pending_launch_feedback_contexts.is_empty());
    drain_queued_blocking_tasks(&queued);
    let revalidated = take_issue4803_monitor_preparation(&fixture.recorded_events);
    // The re-evaluation itself is just as slow: everything moves again.
    slow_evaluation_window(vec![9001, 9002]);
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(revalidated);
    assert!(
        !fixture.runtime.pending_launch_feedback_contexts.is_empty(),
        "a slow but unchanged admission decision must reach pane creation"
    );
    assert!(
        !fixture
            .recorded_events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, UserEvent::IssueMonitorLaunchPrepared(_))),
        "no further preparation may be queued once the launch converged"
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
    Cache::new(issue_cache_root(&fixture.project_root))
        .write_snapshot(&sample_issue_snapshot(
            3166,
            "Unrelated producing owner",
            &["bug"],
            "Other work",
            &Utc::now().to_rfc3339(),
        ))
        .unwrap();
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    gwt::try_mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root),
        |prefs| {
            let base = prefs.launch_profile_pool()[0].clone();
            let profiles = prefs
                .agent_roster
                .as_ref()
                .unwrap()
                .members
                .iter()
                .filter(|member| member.enabled && member.agent_id.as_deref() == Some("codex"))
                .take(2)
                .map(|member| {
                    let mut profile = base.clone();
                    profile.agent_member_id = Some(member.agent_member_id.clone());
                    profile
                })
                .collect::<Vec<_>>();
            assert_eq!(
                profiles.len(),
                2,
                "registered people selected by this scenario"
            );
            prefs.set_launch_profile_pool(profiles);
            Ok(())
        },
    )
    .expect("select the actual registered people before facts capture");
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
    let worktree = pane_control_tests::control_launch_config(&fixture.project_root, 3166);
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(worktree.working_dir.unwrap())
        .branch(worktree.branch.unwrap())
        .base_branch(worktree.base_branch.unwrap())
        .skip_permissions(true)
        .build();
    config.linked_issue_number = Some(3166);
    config.explicit_follow_up = true;
    // Keep the pending launch's registered person idle. An unrelated pane
    // must not silently change the source profile this test is revalidating.
    let unrelated_id = owner.pane_id("agent-1");
    let unrelated_member_id = gwt::load_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root),
    )
    .unwrap()
    .agent_roster
    .unwrap()
    .members
    .into_iter()
    .find(|member| member.name == "Bravo")
    .expect("registered unrelated person")
    .agent_member_id;
    let unrelated_receipt = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::LaunchAgent {
            id: unrelated_id.clone(),
            operation_id: "unrelated-registered-agent".into(),
            tab_id: unrelated_id.split_once("::").unwrap().0.into(),
            config: Box::new(config),
            metadata: Box::new(gwt::project_runtime::launch_payload::OwnerLaunchMetadata {
                selected_agent_member_id: Some(unrelated_member_id),
                ..Default::default()
            }),
            cols: 80,
            rows: 24,
        },
    ))
    .unwrap();
    let unrelated = owner_fixture_block_on(owner.probe(
        &unrelated_id,
        unrelated_receipt["incarnation"].as_u64().unwrap(),
    ))
    .unwrap();
    issue5596_project_owner_pane(&mut fixture, &owner, &unrelated);
    // Settle only the installed foreign projection readbacks before measuring revalidation.
    drain_queued_blocking_tasks(&queued);
    let tab = fixture.runtime.tab_mut("tab-1").unwrap();
    let expected_window_id = combined_window_id(
        "tab-1",
        &tab.workspace.next_window_id_preview(WindowPreset::Agent),
    );

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
    run_queued_monitor_gui_diagnostics(&queued, &[1]);
    assert_eq!(
        queued.lock().unwrap().len(),
        1,
        "metadata changes must still revalidate on a worker"
    );
    let before = gwt_core::process::thread_git_spawn_count();
    drain_queued_blocking_tasks(&queued);
    let repeated_git = gwt_core::process::thread_git_spawn_count() - before;
    assert_eq!(repeated_git, 0, "read-only revalidation must reuse Git/base/prompt facts; materializer claim belongs to the physical owner");
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    drain_queued_blocking_tasks(&queued);
    let (window_id, launched, _) = finish_monitor_relaunch_owner(&mut fixture, Vec::new());
    let launched = launched.expect("reused facts must reach the physical owner");
    assert_eq!(window_id, expected_window_id);
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("read refreshed delivery");
    let claim = prefs
        .pending_launch_deliveries
        .iter()
        .find(|delivery| Some(&delivery.delivery_id) == fixture.delivery_id.as_ref())
        .expect("delivery retained");
    assert_eq!(
        claim.materializer_window_id.as_deref(),
        Some(launched.id.as_str()),
        "the reused facts must claim the current proposed pane"
    );
    assert!(fixture.runtime.pending_launch_feedback_contexts.is_empty());
    assert!(unrelated.pty.try_wait().unwrap().is_none());
    pane_control_tests::control_close(&owner, &launched);
    pane_control_tests::control_close(&owner, &unrelated);
}

fn issue5596_native_answer_source(
    fixture: &mut MonitorRelaunchFixture,
) -> (
    Arc<gwt::project_runtime::test_support::ProjectOwnerHarness>,
    String,
    gwt::project_runtime::test_support::OwnerRuntimeProbe,
) {
    let _thread_id = ScopedEnvVar::unset("CODEX_THREAD_ID");
    // The asking provider predates the answer's Monitor delivery. A normal
    // owner launch must not claim or ACK that separate durable delivery.
    let owner = start_monitor_relaunch_owner(fixture, false);
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&fixture.worktree)
        .branch("work/issue-3165")
        .base_branch("develop")
        .model("gpt-5.5")
        .reasoning_level("low")
        .skip_permissions(true)
        .build();
    config.linked_issue_number = Some(3165);
    config.explicit_follow_up = true;
    config.launch_route = gwt_agent::LaunchRoute::Autonomous;
    let pane = pane_control_tests::control_owner_launch(&owner, "answer-holder", config);
    let raw_id = pane.id.split_once("::").unwrap().1;
    fixture.runtime.tab_mut("tab-1").unwrap().workspace = WindowRegistry::from_persisted(
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(
            &fixture.project_root,
        ))
        .unwrap(),
    );
    fixture.runtime.register_window("tab-1", raw_id);
    let window_id = combined_window_id("tab-1", raw_id);
    let context = fixture.runtime.test_context();
    let snapshot = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::Snapshot {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
        },
    ))
    .unwrap();
    fixture.runtime.install_remote_owner_projection(
        &context,
        &window_id,
        owner.client(),
        serde_json::from_value(snapshot).unwrap(),
        pane.session.clone(),
    );
    let session = pane.session.as_ref().expect("actual native asking Session");
    let native_id = format!("native-answer-{}", session.id);
    let rollout =
        PathBuf::from(std::env::var_os("CODEX_HOME").unwrap()).join("sessions/2026/10/27");
    fs::create_dir_all(&rollout).unwrap();
    fs::write(
        rollout.join(format!("rollout-{native_id}.jsonl")),
        serde_json::json!({"type":"session_meta", "payload": {
            "id":native_id, "cwd":session.worktree_path,
        }})
        .to_string(),
    )
    .unwrap();
    let hook = serde_json::json!({
        "hook_event_name":"SessionStart", "session_id":native_id,
        "cwd":session.worktree_path,
    })
    .to_string();
    pane.with_agent_environment(|| {
        let _ready_nonce = pane.pending_fresh_execution.as_ref().map(|pending| {
            ScopedEnvVar::set(
                gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV,
                &pending.readiness_nonce,
            )
        });
        gwt::daemon_runtime::handle_runtime_state("SessionStart", &hook).unwrap();
        gwt::daemon_runtime::handle_forward_for_event("SessionStart", &hook).unwrap();
    });
    if let Some(pending) = &pane.pending_fresh_execution {
        // SessionStart already submitted authenticated readiness. An extra HTTP
        // resend can capture Prepared just before the same bearer becomes Active.
        let ready = owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline Hang guard for the original authenticated owner finalization, not a guessed process delay.
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    let current = owner.probe(&pane.id, pane.incarnation).await.unwrap();
                    if current.pending_fresh_execution.is_none() {
                        break current;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            })
            .await
            .expect("the original SessionStart must commit exact owner readiness")
        });
        let grant = ready.grant.as_ref().unwrap();
        assert!(ready.issuer.grant_is_current(grant));
        assert!(ready.has_capability_bearer(pane.grant.as_ref().unwrap()));
        assert_eq!(
            grant.principal().active_execution_binding(),
            Some(&pending.binding)
        );
        assert!(
            gwt::project_runtime::continuation::fresh_execution_commit_readback_matches(
                &pending.worktree_path,
                pending.owner,
                &pending.session_identity,
            )
        );
    }
    owner_fixture_block_on(owner.command(gwt::project_runtime::events::OwnerCommand::Health))
        .unwrap();
    let source =
        gwt_agent::Session::load(&fixture.sessions_dir.join(format!("{}.toml", session.id)))
            .expect("canonical asking Session after authenticated native SessionStart");
    assert_eq!(source.id, session.id);
    assert_eq!(source.agent_id, gwt_agent::AgentId::Codex);
    assert_eq!(source.exact_resume_session_id(), Some(native_id.as_str()));
    fixture
        .runtime
        .launch_wizard_cache
        .record_session(source.clone());
    fixture.source_session_id = source.id;
    fixture.native_conversation_id = native_id;
    fixture.holder_window_id = Some(window_id.clone());
    if let Some(delivery_id) = &fixture.delivery_id {
        let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
            &fixture.project_root,
        ))
        .unwrap();
        let delivery = prefs
            .pending_launch_deliveries
            .iter()
            .find(|delivery| &delivery.delivery_id == delivery_id)
            .expect("the separate answer delivery must survive the baseline launch");
        assert!(delivery.materializer_window_id.is_none());
        assert!(delivery.materialized_window_id.is_none());
        assert!(delivery.workspace_durable_window_id.is_none());
    }
    let pane = owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).unwrap();
    assert_eq!(
        pane.session.as_ref().unwrap().exact_resume_session_id(),
        Some(fixture.native_conversation_id.as_str()),
        "the actual owner must observe the authenticated native conversation"
    );
    (owner, window_id, pane)
}

fn assert_issue5596_unsubmitted_answer_packet_retries(
    close_project: bool,
    revoke_preflight_before_commit: bool,
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
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "unsubmitted-answer",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        false,
    );
    let (owner, source_window_id, source_pane) = issue5596_native_answer_source(&mut fixture);
    let context = fixture.runtime.test_context();
    let identity = owner.client().identity().unwrap().clone();
    let state = fixture.runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = Some(identity.clone());
    let mut owner_events = owner.subscribe();
    owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::Close {
            id: source_pane.id.clone(),
            incarnation: source_pane.incarnation,
        }),
    )
    .expect("close and reap the actual asking provider before exact Resume");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root);
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bounds exact Close/Closed and its physical census publication, not a guessed process delay.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let event = owner_events.recv().await.unwrap();
                let closed = matches!(&event,
                    gwt::project_runtime::events::OwnerEvent::Closed { id, incarnation, error: None, .. }
                    if id == &source_pane.id && *incarnation == source_pane.incarnation);
                fixture.runtime.handle_project_owner_event(&context, 1, &identity, event);
                if closed { break; }
            }
            assert!(source_pane.pty.tree_exit_confirmed());
            assert!(source_pane.pty.try_wait().unwrap().is_some());
            loop {
                owner.wait_monitor_idle().await.unwrap();
                let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                if prefs.monitor_runtime_counts
                    .get(&format!("{}:{}", identity.pid, identity.host_started_at))
                    .is_some_and(|census| !census.counts.contains_key(&3165)
                        && !census.windows.contains_key(&source_pane.id))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
            let inventory = owner.command(
                gwt::project_runtime::events::OwnerCommand::ListPanes,
            ).await.unwrap();
            for phase in ["panes", "pending", "closing", "reconciliation"] {
                assert!(inventory[phase].as_array().unwrap().iter()
                    .all(|row| row["id"] != source_pane.id),
                    "closed asking pane remains owned in {phase}");
            }
        }).await.unwrap();
    });
    assert!(source_pane.pty.tree_exit_confirmed());
    assert!(!fixture
        .runtime
        .active_agent_sessions
        .contains_key(&source_window_id));
    let source = gwt_agent::Session::load(
        &fixture
            .sessions_dir
            .join(format!("{}.toml", fixture.source_session_id)),
    )
    .unwrap();
    assert!(
        !gwt::continuation_requires_new_session(&fixture.project_root, &source).unwrap(),
        "the asking Work must remain nonterminal after its exact provider close"
    );
    seed_resumed_autonomous_handoff(&fixture, "Keep the exact Session");
    assert!(fixture.delivery_id.is_none());
    let pause = close_project.then(|| {
        owner_fixture_block_on(owner.pause_next_agent_launch_at(
            gwt::project_runtime::test_support::OwnerLaunchStage::MaterializedBeforeInstall,
        ))
        .unwrap()
    });
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
    let prepared = take_issue4803_monitor_preparation(&fixture.recorded_events);
    if close_project {
        // A legal unrelated roster edit asks the GUI to revalidate these facts
        // before it can submit the immutable owner launch command.
        let before = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        let roster = before.agent_roster.as_ref().unwrap();
        let spare_index = roster
            .members
            .iter()
            .position(|member| {
                member.enabled
                    && member.assignment.is_none()
                    && source.agent_member_id.as_deref() != Some(member.agent_member_id.as_str())
            })
            .expect("an unassigned spare distinct from the exact asking member");
        let spare = &roster.members[spare_index];
        let name = format!("{} (driver revalidation)", spare.name);
        let source_path = fixture.sessions_dir.join(format!("{}.toml", source.id));
        let source_bytes = fs::read(&source_path).unwrap();
        let renamed = gwt::agent_names::mutate_project_roster(
            &fixture.project_root,
            &fixture.sessions_dir,
            gwt::agent_names::AgentRosterAction::Rename {
                member_id: spare.agent_member_id.clone(),
                name: name.clone(),
            },
            Some(roster.revision),
        )
        .expect("rename only an unrelated spare after facts capture");
        assert!(renamed.revision > roster.revision);
        let mut expected_roster = roster.clone();
        expected_roster.revision = renamed.revision;
        expected_roster.members[spare_index].name = name;
        assert_eq!(
            renamed, expected_roster,
            "revalidation trigger must retain every member, assignment and fixed tool"
        );
        let after = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert_eq!(after.max_active_agents, before.max_active_agents);
        assert_eq!(after.max_active_agents_mode, before.max_active_agents_mode);
        assert_eq!(after.launch_profile, before.launch_profile);
        assert_eq!(after.launch_profiles, before.launch_profiles);
        assert_eq!(
            after.pending_launch_deliveries,
            before.pending_launch_deliveries
        );
        assert_eq!(after.autonomous_handoffs, before.autonomous_handoffs);
        assert_eq!(
            fs::read(&source_path).unwrap(),
            source_bytes,
            "roster revalidation must preserve the full asking Session, SID and selected member"
        );
    }
    let before_submit = (!close_project).then(|| {
        // Capture the raw saved head/pool and full handoff beside the exact
        // closed source. No immutable physical launch command exists yet.
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        let session_bytes =
            fs::read(fixture.sessions_dir.join(format!("{}.toml", source.id))).unwrap();
        let (changed, ()) = gwt::try_mutate_issue_monitor_prefs(&prefs_path, |current| {
            assert_eq!(current.launch_profile, prefs.launch_profile);
            assert_eq!(current.launch_profiles, prefs.launch_profiles);
            let mut pool = current.launch_profile_pool();
            pool.first_mut().expect("saved profile").model =
                Some("changed-while-answer-worker-returned".to_string());
            current.set_launch_profile_pool(pool);
            Ok(())
        })
        .expect("change the canonical profile after facts capture, before owner submit");
        assert_ne!(
            changed.launch_profile, prefs.launch_profile,
            "the stale trigger must change the saved core"
        );
        (changed, session_bytes)
    });
    let mut preparation_events = fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared);
    let mut outcome = "no owner launch completion".to_string();
    let mut completed_window_id = None;
    if close_project {
        let pause = pause.unwrap();
        // test-hygiene: allow-wall-clock-deadline Shares the original checkpoint hang guard across legitimate pre-admission callbacks.
        let checkpoint_deadline = Instant::now() + Duration::from_secs(30);
        let (evidence, launch_worker) = loop {
            let launch_tasks = queued.clone();
            let worker_sessions = fixture.sessions_dir.clone();
            let (launch_finished, mut launch_done) = tokio::sync::oneshot::channel();
            let launch_worker = thread::spawn(move || {
                let home_matches = gwt_core::paths::gwt_sessions_dir() == worker_sessions;
                drain_queued_blocking_tasks(&launch_tasks);
                let _ = launch_finished.send(home_matches);
            });
            let checkpoint = owner_fixture_block_on(async {
                // test-hygiene: allow-wall-clock-deadline Shares the original checkpoint hang guard across legitimate pre-admission callbacks.
                tokio::time::timeout_at(
                    tokio::time::Instant::from_std(checkpoint_deadline),
                    async {
                        tokio::select! {
                            evidence = pause.wait() => Ok(evidence),
                            home_matches = &mut launch_done => Err(home_matches.ok()),
                        }
                    },
                )
                .await
            });
            match checkpoint {
                Ok(Ok(evidence)) => break (evidence, launch_worker),
                stopped => {
                    let worker_finished = matches!(&stopped, Ok(Err(Some(_))));
                    if !worker_finished {
                        pause.release();
                    }
                    let joined = launch_worker.join();
                    if worker_finished && joined.is_ok() {
                        let next = {
                            let mut recorded = fixture.recorded_events.lock().unwrap();
                            let completions = recorded
                                .iter()
                                .enumerate()
                                .filter_map(|(index, event)| {
                                    matches!(
                                        recorded_project_payload(event),
                                        UserEvent::IssueMonitorLaunchPrepared(_)
                                            | UserEvent::RemoteOwnerLaunchComplete(_)
                                            | UserEvent::IssueMonitorPreflightRejected(_)
                                    )
                                    .then_some(index)
                                })
                                .collect::<Vec<_>>();
                            match completions.as_slice() {
                                [index]
                                    if matches!(recorded_project_payload(&recorded[*index]),
                                UserEvent::IssueMonitorLaunchPrepared(prepared)
                                    if prepared.rejection.is_none()) =>
                                {
                                    Some(into_recorded_project_payload(recorded.remove(*index)))
                                }
                                _ => None,
                            }
                        };
                        if let Some(UserEvent::IssueMonitorLaunchPrepared(prepared)) = next {
                            // Dispatch only the existing pre-admission revalidation.
                            // An owner/preflight completion is never refreshed or retried.
                            preparation_events.extend(
                                fixture
                                    .runtime
                                    .handle_issue_monitor_launch_prepared(*prepared),
                            );
                            continue;
                        }
                    }
                    pause.release();
                    let (phase, worker_home_matches) = match stopped {
                        Ok(Err(home_matches)) => {
                            ("worker_finished_before_checkpoint", home_matches)
                        }
                        Err(_) => ("checkpoint_timeout", None),
                        Ok(Ok(_)) => unreachable!("checkpoint evidence handled above"),
                    };
                    let callbacks = fixture.recorded_events.lock().unwrap().iter().filter_map(|event| {
                    match recorded_project_payload(event) {
                        UserEvent::IssueMonitorLaunchPrepared(prepared) => Some(serde_json::json!({
                            "kind":"prepared", "rejection":prepared.rejection.is_some(),
                        })),
                        UserEvent::RemoteOwnerLaunchComplete(completion) => Some(serde_json::json!({
                            "kind":"owner_complete", "error":completion.result.as_ref().err(),
                        })),
                        UserEvent::IssueMonitorPreflightRejected(rejection) => Some(serde_json::json!({
                            "kind":"preflight_rejected", "error":rejection.result.as_ref().err(),
                        })),
                        _ => None,
                    }
                }).collect::<Vec<_>>();
                    let expected_source = serde_json::to_value(&source).unwrap();
                    let current_source = gwt_agent::Session::load(
                        &fixture.sessions_dir.join(format!("{}.toml", source.id)),
                    )
                    .map(|session| serde_json::to_value(session).unwrap());
                    let changed_fields = expected_source
                        .as_object()
                        .unwrap()
                        .keys()
                        .filter(|field| {
                            current_source
                                .as_ref()
                                .ok()
                                .and_then(|current| current.get(*field))
                                != expected_source.get(*field)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    eprintln!("ProjectClose checkpoint not reached: phase={phase}, callbacks={callbacks:?}, source_changed_fields={changed_fields:?}, source_read_error={:?}, caller_home_matches={}, worker_home_matches={worker_home_matches:?}",
                    current_source.as_ref().err().map(|error| error.kind()),
                    gwt_core::paths::gwt_sessions_dir() == fixture.sessions_dir);
                    joined.unwrap();
                    panic!(
                    "ProjectClose must reach its actual pre-PTY write-ahead checkpoint: {phase}"
                );
                }
            }
        };
        assert!(evidence.pty.is_none());
        let checkpoint_delivery = gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .autonomous_handoffs[0]
            .delivery
            .clone();
        let preparation_materialized = evidence.session.is_some();
        let mut launch_worker = Some(launch_worker);
        // A failed preparation also reaches this checkpoint. Let its original
        // error return before ProjectClose can replace it with cancellation.
        let unmaterialized_join = if !preparation_materialized {
            pause.release();
            Some(launch_worker.take().unwrap().join())
        } else {
            None
        };
        fixture.runtime.close_project_tab_events("tab-1");
        let close_tasks = queued.clone();
        let close_worker = thread::spawn(move || drain_queued_blocking_tasks(&close_tasks));
        owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline Bounds cancellation IPC; the owner current flag is the exact cancellation evidence.
            tokio::time::timeout(Duration::from_secs(30), async {
                while evidence.current.load(std::sync::atomic::Ordering::Acquire) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        });
        pause.release();
        let launch_join = match launch_worker {
            Some(worker) => worker.join(),
            None => unmaterialized_join.expect("the unmaterialized worker was joined before close"),
        };
        close_worker.join().unwrap();
        launch_join.unwrap();
        let completion = take_remote_owner_launch_completion(&fixture.recorded_events);
        outcome = format!("{:?}", completion.result.as_ref().err());
        completed_window_id = Some(completion.window_id.clone());
        fixture
            .runtime
            .handle_remote_owner_launch_complete(completion);
        drain_queued_blocking_tasks(&queued);
        assert!(matches!(&checkpoint_delivery,
            gwt::autonomous_handoff::AutonomousHandoffDeliveryState::Attempting { attempt: 1, .. }),
            "the actual owner must persist an unsubmitted write-ahead token before ProjectClose: delivery={checkpoint_delivery:?}; preparation_materialized={preparation_materialized}; owner={outcome}");
    } else {
        // A pre-submit refusal need not produce a launch RPC completion. Drive
        // only real queued GUI/owner callbacks, then inspect canonical retry.
        loop {
            drain_queued_blocking_tasks(&queued);
            let completion = {
                let mut recorded = fixture.recorded_events.lock().unwrap();
                recorded
                    .iter()
                    .position(|event| {
                        matches!(
                            recorded_project_payload(event),
                            UserEvent::IssueMonitorLaunchPrepared(_)
                                | UserEvent::RemoteOwnerLaunchComplete(_)
                                | UserEvent::IssueMonitorPreflightRejected(_)
                        )
                    })
                    .map(|index| into_recorded_project_payload(recorded.remove(index)))
            };
            match completion {
                Some(UserEvent::IssueMonitorLaunchPrepared(prepared)) => {
                    if revoke_preflight_before_commit {
                        let pause = owner_fixture_block_on(owner.pause_next_agent_launch_at(
                            gwt::project_runtime::test_support::OwnerLaunchStage::PreparedBeforeMailbox,
                        ))
                        .unwrap();
                        fixture
                            .runtime
                            .handle_issue_monitor_launch_prepared(*prepared);
                        let tasks = queued.clone();
                        let worker = thread::spawn(move || drain_queued_blocking_tasks(&tasks));
                        let evidence = owner_fixture_block_on(async {
                            // test-hygiene: allow-wall-clock-deadline Bounds arrival at the actual rejection's post-check_current checkpoint, not elapsed ordering.
                            tokio::time::timeout(Duration::from_secs(30), pause.wait())
                                .await
                                .unwrap()
                        });
                        let was_current =
                            evidence.current.load(std::sync::atomic::Ordering::Acquire);
                        let before = fs::read(&prefs_path).unwrap();
                        let before_prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                        let source_path = fixture.sessions_dir.join(format!("{}.toml", source.id));
                        let source_before = fs::read(&source_path).unwrap();
                        let inventory_before = owner_fixture_block_on(
                            owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
                        )
                        .unwrap();
                        evidence
                            .current
                            .store(false, std::sync::atomic::Ordering::Release);
                        pause.release();
                        let joined = worker.join();
                        let rejection = {
                            let mut recorded = fixture.recorded_events.lock().unwrap();
                            recorded
                                .iter()
                                .position(|event| {
                                    matches!(
                                        recorded_project_payload(event),
                                        UserEvent::IssueMonitorPreflightRejected(_)
                                    )
                                })
                                .map(|index| into_recorded_project_payload(recorded.remove(index)))
                        };
                        let after = fs::read(&prefs_path).unwrap();
                        let source_after = fs::read(&source_path).unwrap();
                        let inventory_after = owner_fixture_block_on(
                            owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
                        )
                        .unwrap();
                        fixture.runtime.blocking_tasks = previous_spawner;
                        fixture
                            .runtime
                            .test_project_owners
                            .remove(&fixture.project_root);
                        owner_fixture_block_on(Arc::try_unwrap(owner).ok().unwrap().shutdown());

                        joined.unwrap();
                        assert!(
                            was_current,
                            "rejection must pass check_current before revocation"
                        );
                        assert!(evidence.pty.is_none());
                        assert_eq!(evidence.session_id.as_deref(), Some(source.id.as_str()));
                        assert_eq!(
                            serde_json::to_value(&evidence.session).unwrap(),
                            serde_json::to_value(Some(&source)).unwrap()
                        );
                        let handoff = &before_prefs.autonomous_handoffs[0];
                        assert!(handoff.delivered_at.is_none());
                        let gwt::autonomous_handoff::AutonomousHandoffDeliveryState::Attempting {
                            attempt: 1,
                            target: Some(target),
                            ..
                        } = &handoff.delivery
                        else {
                            panic!("actual rejection must persist its exact unsubmitted Attempting token");
                        };
                        assert_eq!(target.gwt_session_id, source.id);
                        assert_eq!(target.window_id, evidence.id);
                        assert!(target.delivery_id.is_none());
                        assert_eq!(target.materializer_pid, std::process::id());
                        let Some(UserEvent::IssueMonitorPreflightRejected(rejection)) = rejection
                        else {
                            panic!("actual rejection worker must return its owner completion");
                        };
                        assert_eq!(rejection.operation_id, evidence.operation_id);
                        assert!(rejection.delivery_id.is_none());
                        assert!(
                            rejection.result.is_err(),
                            "revoked rejection must refuse settlement: {:?}",
                            rejection.result
                        );
                        assert_eq!(after, before,
                            "revoked rejection must preserve the exact Attempting tuple and prefs bytes");
                        assert_eq!(source_after, source_before);
                        assert_eq!(inventory_after, inventory_before);
                        assert!(inventory_after["panes"].as_array().unwrap().is_empty());
                        assert!(inventory_after["pending"].as_array().unwrap().is_empty());
                        return;
                    }
                    if let Some(expected) = prepared
                        .rejection
                        .as_ref()
                        .filter(|expected| expected.expected_delivery.is_none())
                    {
                        let before = fs::read(&prefs_path).unwrap();
                        let source_path = fixture.sessions_dir.join(format!("{}.toml", source.id));
                        let source_before = fs::read(&source_path).unwrap();
                        let inventory_before = owner_fixture_block_on(
                            owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
                        )
                        .unwrap();
                        let current = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                        let mut unchanged = expected.clone();
                        unchanged.operation_id = uuid::Uuid::new_v4().to_string();
                        unchanged.expected_profile_readset =
                            Some((current.launch_profile, current.launch_profiles));
                        let unchanged_result = owner_fixture_block_on(owner.command(
                            gwt::project_runtime::events::OwnerCommand::RejectMonitorLaunch {
                                rejection: unchanged,
                            },
                        ));
                        assert!(
                            unchanged_result
                                .as_ref()
                                .is_err_and(|error| error.contains("profile change")),
                            "an unchanged raw readset cannot authorize an answer rejection"
                        );
                        let mut changed_source = expected.clone();
                        changed_source.operation_id = uuid::Uuid::new_v4().to_string();
                        changed_source
                            .expected_session
                            .as_mut()
                            .unwrap()
                            .branch
                            .push_str("-foreign-expectation");
                        let changed_result = owner_fixture_block_on(owner.command(
                            gwt::project_runtime::events::OwnerCommand::RejectMonitorLaunch {
                                rejection: changed_source,
                            },
                        ));
                        assert!(
                            changed_result
                                .as_ref()
                                .is_err_and(|error| error.contains("Session changed")),
                            "an answer rejection must retain the original full Session expectation"
                        );
                        assert!(fs::read(&prefs_path).unwrap() == before);
                        assert!(fs::read(&source_path).unwrap() == source_before);
                        let unchanged_inventory = owner_fixture_block_on(
                            owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
                        )
                        .unwrap();
                        assert_eq!(unchanged_inventory["panes"], inventory_before["panes"]);
                        assert_eq!(unchanged_inventory["pending"], inventory_before["pending"]);
                    }
                    preparation_events.extend(
                        fixture
                            .runtime
                            .handle_issue_monitor_launch_prepared(*prepared),
                    );
                }
                Some(UserEvent::RemoteOwnerLaunchComplete(completion)) => {
                    outcome = format!("{:?}", completion.result.as_ref().err());
                    completed_window_id = Some(completion.window_id.clone());
                    preparation_events.extend(
                        fixture
                            .runtime
                            .handle_remote_owner_launch_complete(*completion),
                    );
                }
                Some(UserEvent::IssueMonitorPreflightRejected(rejection)) => {
                    assert!(rejection.delivery_id.is_none());
                    if let Ok(receipt) = rejection.result.as_ref() {
                        assert_eq!(
                            receipt["operation_id"].as_str(),
                            Some(rejection.operation_id.as_str())
                        );
                        assert_eq!(receipt["no_pty"], true);
                        assert_eq!(receipt["no_submit"], true);
                        assert!(receipt["claimed_delivery"].is_null());
                        assert_eq!(
                            receipt["answer_target"]["gwt_session_id"].as_str(),
                            Some(source.id.as_str())
                        );
                    }
                    outcome = format!("{:?}", rejection.result.as_ref().err());
                    preparation_events.extend(
                        fixture
                            .runtime
                            .handle_monitor_preflight_rejected(*rejection),
                    );
                }
                None => break,
                _ => unreachable!("matched Monitor completion"),
            }
        }
    }
    fixture.runtime.blocking_tasks = previous_spawner;
    let inventory = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    // Clean up a real unexpected launch before preserving the original RED.
    if let Some(window_id) =
        completed_window_id.filter(|id| fixture.runtime.runtimes.contains_key(id))
    {
        close_test_owner_runtime(&mut fixture.runtime, &window_id);
    }
    assert!(
        !revoke_preflight_before_commit,
        "revocation regression must reach the actual rejection checkpoint"
    );
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
        "unsubmitted token must retain bounded exact-session retry: {:?}; owner={outcome}; inventory={inventory}; preparation={preparation_events:?}",
        prefs.autonomous_handoffs[0].delivery,
    );
    assert!(prefs.autonomous_handoffs[0].delivered_at.is_none());
    if let Some((before_submit, source_bytes)) = before_submit {
        assert_eq!(
            prefs.pending_launch_deliveries,
            before_submit.pending_launch_deliveries
        );
        assert_eq!(prefs.launch_profiles, before_submit.launch_profiles);
        let mut original_handoff = before_submit.autonomous_handoffs[0].clone();
        original_handoff.delivery = prefs.autonomous_handoffs[0].delivery.clone();
        assert!(
            original_handoff == prefs.autonomous_handoffs[0],
            "settlement changes only the exact unsubmitted delivery state"
        );
        assert!(
            fs::read(fixture.sessions_dir.join(format!("{}.toml", source.id))).unwrap()
                == source_bytes,
            "a refused pre-submit answer preserves its exact closed source Session"
        );
    }
    assert!(
        inventory["panes"].as_array().unwrap().is_empty(),
        "no second physical provider may start after an unsubmitted attempt: {inventory}"
    );
    assert!(fixture.runtime.pending_launch_feedback_contexts.is_empty());
}

#[test]
fn issue5596_answer_packet_pref_stale_settles_unsubmitted_attempt() {
    assert_issue5596_unsubmitted_answer_packet_retries(false, false);
}

#[test]
fn issue5596_answer_packet_project_closed_settles_unsubmitted_attempt() {
    assert_issue5596_unsubmitted_answer_packet_retries(true, false);
}

#[test]
fn issue5596_revoked_answer_rejection_preserves_unsubmitted_attempt() {
    assert_issue5596_unsubmitted_answer_packet_retries(false, true);
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
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "answer-exact-holder",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    let (owner, holder_window_id, holder) = issue5596_native_answer_source(&mut fixture);
    let holder_session = holder.session.as_ref().unwrap();
    let holder_name = holder_session
        .agent_name
        .clone()
        .expect("actual canonical holder name");
    let holder_pid = holder.pty.process_id().unwrap();
    let holder_sid = holder_session.id.clone();
    let delivery_id = fixture
        .delivery_id
        .clone()
        .expect("durable answer delivery");
    seed_resumed_autonomous_handoff(&fixture, "Continue the asking conversation");
    gwt::machine_agent_settings::set(
        Some(3),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-23T00:00:00Z",
    )
    .expect("allow the exact holder and the unrelated live pane");
    let tab = fixture.runtime.tab_mut("tab-1").unwrap();
    let other = tab.workspace.add_window(WindowPreset::Agent);
    tab.workspace.set_linked_issue_number(&other.id, Some(3165));
    tab.workspace
        .set_status(&other.id, WindowProcessStatus::Running);
    let mut persisted = tab.workspace.persisted().clone();
    persisted.windows.swap(0, 1);
    tab.workspace = WindowRegistry::from_persisted(persisted);
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
    // Facts preparation and the physical answer must reuse the canonical
    // existing person while history is locked, without allocating a name.
    let history_path = gwt::agent_registry::history_path();
    fs::create_dir_all(history_path.parent().unwrap()).expect("name history directory");
    let history = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&history_path)
        .expect("open name history");
    let names_before = fs::read(&history_path).unwrap();
    fs2::FileExt::lock_exclusive(&history).expect("hold name history before answer admission");
    let owner_identity = owner.client().identity().unwrap().clone();
    let claim_probe = gwt::project_runtime::installation::IssueMonitorLaunchClaimInput {
        project_root: fixture.project_root.clone(),
        issue_number: 3165,
        delivery_id: delivery_id.clone(),
        materializer_window_id: holder.id.clone(),
        materializer_id: owner_identity.host_instance_id.clone(),
        materializer_pid: owner_identity.pid,
        fallback_timeout:
            gwt::project_runtime::installation::local_issue_monitor_fallback_projection_timeout(),
        snapshot: None,
        snapshot_tabs: Default::default(),
    };
    let claim_before = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        &fixture.project_root,
    ))
    .unwrap();
    let windows_before = owner.monitor_snapshot().unwrap().map(|snapshot| {
        (
            snapshot.project_tab_id,
            snapshot
                .windows
                .into_iter()
                .map(|window| {
                    (
                        window.window_id,
                        window.issue_number,
                        window.status,
                        window.monitor_owned,
                    )
                })
                .collect::<Vec<_>>(),
        )
    });
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &fixture.project_root,
            3165,
            LinkedIssueKind::Spec,
            fixture.delivery_id.clone(),
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
    let preparation_phases =
        capture_tracing_events(|| loop {
            drain_queued_blocking_tasks(&queued);
            if !fixture
                .recorded_events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, UserEvent::IssueMonitorLaunchPrepared(_)))
            {
                break;
            }
            fixture.runtime.handle_issue_monitor_launch_prepared(
                take_issue4803_monitor_preparation(&fixture.recorded_events),
            );
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
        Some(holder_name.as_str()),
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
        "the prepared answer must reach its exact holder instead of retrying against the generic Issue pane; phases={preparation_phases:?}"
    );
    let delivery = {
        let mut events = fixture.recorded_events.lock().unwrap();
        let index = events
            .iter()
            .position(|event| matches!(event, UserEvent::IssueMonitorAnswerDeliveryComplete(_)))
            .expect("actual owner answer completion");
        match events.remove(index) {
            UserEvent::IssueMonitorAnswerDeliveryComplete(delivery) => delivery,
            _ => unreachable!(),
        }
    };
    assert_eq!(delivery.delivery_id.as_deref(), Some(delivery_id.as_str()));
    assert_eq!(delivery.holder_window_id, holder_window_id);
    assert_eq!(delivery.owner_pane_id, holder.id);
    assert_eq!(delivery.runtime_incarnation, holder.incarnation);
    assert!(
        delivery.result.is_ok(),
        "actual answer result: {:?}; pure_preflight_from_before={}; expected_host={owner_identity:?}; expected_window={}; pd_before={:?}; post_error_pd={:?}; windows_before={windows_before:?}; post_error_windows={:?}; post_error_daemon={:?}",
        delivery.result,
        claim_probe.test_preflight_claim(claim_before.clone(), Some(owner_identity.host_started_at)), holder.id,
        claim_before.pending_launch_deliveries.iter().find(|delivery| delivery.delivery_id == delivery_id)
            .map(|delivery| (&delivery.claim_id, &delivery.materializer_id, delivery.materializer_pid, &delivery.materializer_window_id)),
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root)).map(|prefs|
            prefs.pending_launch_deliveries.into_iter().find(|delivery| delivery.delivery_id == delivery_id)
                .map(|delivery| (delivery.claim_id, delivery.materializer_id, delivery.materializer_pid, delivery.materializer_window_id))),
        owner.monitor_snapshot().map(|snapshot| snapshot.map(|snapshot| (snapshot.project_tab_id,
            snapshot.windows.into_iter().map(|window|
                (window.window_id, window.issue_number, window.status, window.monitor_owned)).collect::<Vec<_>>()))),
        gwt::daemon_publisher::read_issue_monitor_status(&fixture.project_root).map(|status| status.map(|status|
            serde_json::json!({"source":status["source"],"last_scan_at":status["last_scan_at"],
                "occupied":status["occupied_slot_count"],"inbox":status["inbox"].as_array().map(|items|
                    items.iter().filter(|item| item["issue_number"] == 3165).map(|item|
                        serde_json::json!({"state":item["state"],"readiness":item["readiness"],
                            "window_id":item["launched_window_id"],"exclusion":item["exclusion_reason"]})).collect::<Vec<_>>())}))),
    );
    assert!(delivery.submitted);
    fixture
        .runtime
        .handle_issue_monitor_answer_delivery_complete(delivery);
    let same = owner_fixture_block_on(owner.probe(&holder.id, holder.incarnation)).unwrap();
    assert_eq!(same.session.as_ref().unwrap().id, holder_sid);
    assert_eq!(same.pty.process_id(), Some(holder_pid));
    assert!(same.pty.try_wait().unwrap().is_none());
    let inventory = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    assert_eq!(
        inventory["panes"].as_array().unwrap().len(),
        1,
        "the generic Issue pane must not cause another physical launch: {inventory}"
    );
    assert_eq!(
        fs::read(&history_path).unwrap(),
        names_before,
        "answer routing must not mint a name"
    );
    close_test_owner_runtime(&mut fixture.runtime, &holder_window_id);
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
    // A completed genesis launch need not appear in the Wizard cache. Its
    // actual owner must classify the real child exit from the canonical Session.
    for fault in [false, true] {
        let case = dir.path().join(if fault {
            "bridge-fault"
        } else {
            "agent-failure"
        });
        fs::create_dir_all(&case).unwrap();
        let _home = ScopedGwtHome::set(&case);
        let (project, owner, delivery) = owner_monitor_fixture_with_prefs(
            &case,
            4774,
            false,
            &["pane-contract-fixture"],
            |prefs| {
                prefs.autonomous_mode = true;
                prefs.launch_auto = true;
                prefs.autonomous_records = vec![issue_monitor_autonomous_record(
                    4774,
                    gwt::AutonomousPhase::Implementing,
                    0,
                )];
                prefs.record_tier_launch(4774, 0);
            },
        );
        let mut config = super::pane_control_tests::control_launch_config(&project, 4774);
        *config.args.last_mut().unwrap() = if cfg!(windows) {
            "$null = [Console]::ReadLine(); exit 1".into()
        } else {
            "stty -echo; IFS= read -r line; exit 1".into()
        };
        let pane =
            owner_monitor_launch_with_config(&project, &owner, 4774, "agent", delivery, config);
        let actual = pane.session.as_ref().unwrap();
        let runtime_path =
            gwt_agent::runtime_state_path(&gwt_core::paths::gwt_sessions_dir(), &actual.id);
        assert!(
            gwt_agent::SessionBridgeObservation::capture(&runtime_path, &actual.id)
                .unwrap()
                .unwrap()
                .record(gwt_agent::HostBridgeKind::WorkspaceUpdate, fault)
                .unwrap()
        );
        assert_eq!(
            gwt_agent::has_unresolved_host_bridge_fault(
                &runtime_path,
                &gwt_agent::SessionRuntimeState::load(&runtime_path).unwrap()
            )
            .unwrap(),
            fault,
            "the exact actual launch must carry the bridge observation"
        );
        let mut app = sample_runtime(
            &case,
            vec![sample_project_tab_with_window_at(
                "tab-1",
                "agent",
                project.clone(),
                WindowPreset::Agent,
                WindowProcessStatus::Running,
            )],
            Some("tab-1"),
        );
        app.sessions_dir = gwt_core::paths::gwt_sessions_dir();
        app.launch_wizard_cache =
            LaunchWizardMemoryCache::load_with_agent_options(&app.sessions_dir, Vec::new());
        app.launch_wizard_cache.forget_session(&actual.id);
        assert!(app.launch_wizard_cache.session_by_id(&actual.id).is_none());
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&project);
        let mut completed = owner.subscribe();
        owner_fixture_block_on(
            owner.command(gwt::project_runtime::events::OwnerCommand::Input {
                id: pane.id.clone(),
                incarnation: pane.incarnation,
                data: b"exit\r\n".to_vec(),
            }),
        )
        .unwrap();
        let closed = owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline Bounds the actual provider exit, exact Session finalizer, and canonical Monitor failure commit; the Closed receipt establishes completion.
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    if let gwt::project_runtime::events::OwnerEvent::Closed {
                        id,
                        incarnation,
                        error,
                        ..
                    } = completed.recv().await.unwrap()
                    {
                        if id == pane.id && incarnation == pane.incarnation {
                            break error;
                        }
                    }
                }
            })
            .await
        });
        let after = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        let reaped = pane.pty.try_wait().unwrap().is_some();
        owner_fixture_block_on(owner.shutdown());
        assert!(
            matches!(closed, Ok(None)),
            "the exact actual provider must finalize cleanly: {closed:?}"
        );
        assert!(reaped);
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
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
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
    fixture
        .runtime
        .handle_issue_monitor_launch_prepared(take_issue4803_monitor_preparation(
            &fixture.recorded_events,
        ));
    let before_admission = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    assert!(
        before_admission["panes"].as_array().unwrap().is_empty(),
        "no physical provider may start before the owner's write-ahead attempt"
    );
    assert!(before_admission["pending"].as_array().unwrap().is_empty());
    assert!(matches!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .autonomous_handoffs[0]
            .delivery,
        gwt::autonomous_handoff::AutonomousHandoffDeliveryState::Pending
    ));
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
    let (window_id, pane, _) = finish_monitor_relaunch_owner(&mut fixture, Vec::new());
    let pane = pane.unwrap();
    assert_owner_monitor_exact_resume(&owner, &pane, &fixture);
    close_test_owner_runtime(&mut fixture.runtime, &window_id);
}
