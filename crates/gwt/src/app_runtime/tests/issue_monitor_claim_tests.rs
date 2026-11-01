use super::*;
use crate::app_runtime::UpdateDrainObservation;

fn commit_issue_monitor_preparations(
    runtime: &mut AppRuntime,
    queued: &BlockingTestTaskQueue,
    recorded: &Arc<Mutex<Vec<UserEvent>>>,
) -> Vec<OutboundEvent> {
    let mut events = Vec::new();
    // Dispatch facts and the actual owner's launch reply, leaving the separate
    // delivery ACK transaction queued for the caller's phase assertions.
    loop {
        drain_queued_blocking_tasks(queued);
        loop {
            let completion = {
                let mut recorded = recorded.lock().unwrap();
                recorded
                    .iter()
                    .position(|event| {
                        matches!(
                            recorded_project_payload(event),
                            UserEvent::IssueMonitorLaunchPrepared(_)
                                | UserEvent::RemoteOwnerLaunchComplete(_)
                        )
                    })
                    .map(|index| into_recorded_project_payload(recorded.remove(index)))
            };
            match completion {
                Some(UserEvent::IssueMonitorLaunchPrepared(prepared)) => {
                    events.extend(runtime.handle_issue_monitor_launch_prepared(*prepared));
                }
                Some(UserEvent::RemoteOwnerLaunchComplete(completion)) => {
                    events.extend(runtime.handle_remote_owner_launch_complete(*completion));
                }
                None => break,
                _ => unreachable!("matched Monitor launch completion"),
            }
        }
        if runtime.issue_monitor_launch_preparations.is_empty()
            && runtime.pending_launch_completions.is_empty()
        {
            return events;
        }
    }
}

#[test]
fn issue_3809_settings_refresh_uses_the_same_project_background_worker() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&other).unwrap();
    init_repo_with_initial_commit(&repo);
    init_repo_with_initial_commit(&other);
    let tabs = vec![
        sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]),
        sample_project_tab("tab-2", "Other", other.clone(), ProjectKind::Git, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let context = runtime.test_context();
    reset_local_issue_monitor_remote_scan_count();
    runtime.local_issue_monitor_events_for(&context, Some("client-1"), |_| {});
    runtime.local_issue_monitor_events_for(&context, Some("client-1"), |_| {});
    assert_eq!(
        local_issue_monitor_remote_scan_count(),
        0,
        "settings refresh must not reclaim panes without background child observations"
    );
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "the existing in-flight gate coalesces refreshes"
    );
    assert!(runtime
        .project_state_for_root(&repo)
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .contains(&gwt::issue_monitor_prefs_path_for_repo_path(&repo)));
    assert!(
        runtime
            .project_state_for_root(&other)
            .unwrap()
            .issue_monitor_scheduled_scans_in_flight
            .is_empty(),
        "settings refresh must not enqueue unrelated projects"
    );
}

#[test]
#[cfg(any(windows, unix))]
fn issue_3809_scheduled_scan_preserves_child_activity_after_disk_rebase() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let (repo, prefs_path) = seed_scheduled_scan_claim_fixture(&temp, None);
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let session_id = "issue-3809-gui-holder";
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 43,
    };
    let branch = gwt_git::Repository::open(&repo)
        .unwrap()
        .current_branch()
        .unwrap()
        .unwrap();
    seed_defunct_active_owner(
        &sessions_dir,
        &repo,
        &repo,
        &branch,
        owner,
        session_id,
        gwt_agent::AgentStatus::Running,
    );
    let session =
        gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
    let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
        .unwrap()
        .unwrap();
    let started = gwt::process::host_process_start_time(std::process::id()).unwrap();
    gwt_agent::SessionRuntimeState::for_execution_process(
        gwt_agent::AgentStatus::Running,
        &identity,
        1,
        started,
        std::process::id(),
        started,
    )
    .save(&gwt_agent::runtime_state_path(&sessions_dir, session_id))
    .unwrap();

    struct OwnedChild(std::process::Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[cfg(windows)]
    let mut command = gwt_core::process::hidden_command("powershell.exe");
    #[cfg(windows)]
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "$value = 0; while ($true) { $value = ($value + 1) % 1000000 }",
    ]);
    #[cfg(unix)]
    let mut command = gwt_core::process::hidden_command("sh");
    #[cfg(unix)]
    command.args(["-c", "while :; do :; done"]);
    let _child = OwnedChild(
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );

    let before_scan = chrono::Utc::now();
    let now = before_scan.to_rfc3339();
    let old = (before_scan - chrono::Duration::hours(2)).to_rfc3339();
    let window_id = "tab-1::agent-43";
    let mut seeded = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
    );
    seeded.complete_active_launch_at(43, window_id, &old);
    seeded.set_autonomous_phase(43, gwt::AutonomousPhase::Implementing);
    seeded.record_autonomous_heartbeat(43, &old);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded.prefs()).unwrap();
    let snapshot = gwt::IssueMonitorWindowSnapshot {
        project_tab_id: "tab-1".into(),
        observed_at: now.clone(),
        windows: vec![gwt::IssueMonitorWindowObservation {
            monitor_owned: true,
            window_id: window_id.into(),
            issue_number: Some(43),
            status: gwt::WindowState::Running,
            review_dispatch: false,
            hold_reason: None,
            last_output_at: Some(old),
            session_id: Some(session_id.into()),
        }],
    };
    let live_windows = std::collections::BTreeSet::from([window_id.to_string()]);
    let outcome = super::super::run_scheduled_issue_monitor_scan_with_budgets(
        &repo,
        Some("tab-1"),
        Some(&live_windows),
        Some(&snapshot),
        None,
        &now,
        &super::super::default_issue_client_factory(),
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(30),
        &HashSet::new(),
    )
    .expect("scheduled scan");
    super::super::wait_scan_generation_reaper(&prefs_path);
    let ScheduledIssueMonitorScanOutcome::Applied(mut applied) = outcome else {
        panic!("the local scheduled scan must commit");
    };
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    let heartbeat = persisted
        .autonomous_records
        .iter()
        .find(|record| record.issue_number == 43)
        .and_then(|record| record.last_heartbeat.as_deref())
        .expect("the owned child observation must survive the disk rebase");
    assert!(
        chrono::DateTime::parse_from_rfc3339(heartbeat).unwrap()
            >= chrono::DateTime::parse_from_rfc3339(&now).unwrap(),
        "the actual scheduled scan must persist its child activity: {heartbeat} < {now}"
    );
    assert!(persisted
        .launched_issues
        .iter()
        .any(|launch| launch.issue_number == 43 && launch.window_id == window_id));
    assert_eq!(applied.active_issue_numbers(), vec![43]);
    assert_eq!(
        applied.inbox_item(43).unwrap().state,
        gwt::MonitorInboxState::Launched,
        "active child work must never become queued during the scan"
    );
    assert!(applied.take_pending_idle_pane_closes().is_empty());
}

#[test]
fn app_runtime_fallible_control_fallback_error_preserves_project_snapshot() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            max_active_agents: 7,
            ..Default::default()
        },
    )
    .expect("seed Auto continuity");
    let before = fs::read(&prefs_path).expect("read seeded prefs");

    let result = AppRuntime::try_commit_local_issue_monitor_control_with_timeout(
        &repo,
        Duration::from_secs(5),
        |monitor| {
            monitor.set_max_active_agents_override_with_source(
                Some(3),
                gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::Gui,
                "2026-10-22T00:00:00Z",
            );
            Err::<(), _>(std::io::Error::other("machine policy save failed"))
        },
    );

    assert_eq!(
        fs::read(&prefs_path).expect("read prefs after rejected mutation"),
        before,
        "a failed mutation must preserve project bytes and Auto continuity"
    );
    assert!(
        result.is_err(),
        "the failed mutation must reject the transaction"
    );
}

#[test]
fn app_runtime_launch_failed_fallback_lock_timeout_has_zero_commit() {
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
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
                issue_number: 42,
                claimed_at: None,
            }],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed lifecycle prefs");
    let before = fs::read(&prefs_path).expect("read seeded prefs");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(prefs_path.with_extension("lock"))
        .expect("open prefs lock");
    lock.lock_exclusive().expect("hold prefs lock");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_monitor_fallback_commit_timeout =
        super::super::ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT;
    super::super::reset_local_issue_monitor_fallback_commit_count();

    // The prefs lock is held for the whole synchronous call, so the call can
    // only return by giving up on the lock at the fallback commit deadline.
    // Assert that outcome directly instead of timing the call: a wall-clock
    // bound here also measured the fallback projection (Git/cache reads, its
    // own 1 s budget) that runs before the 250 ms commit deadline starts, and
    // failed on a loaded Windows host (SPEC #4740).
    let events = runtime.issue_monitor_launch_failed_result_events(
        42,
        "launch failed",
        Err(
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                "daemon not running".to_string(),
            ),
        ),
    );
    FileExt::unlock(&lock).expect("release prefs lock");

    assert_eq!(
        super::super::local_issue_monitor_fallback_commit_count(),
        0,
        "a lock-timed-out fallback must not commit"
    );
    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, .. }
                if message.contains(gwt_core::operation_deadline::DEADLINE_EXPIRED_MARKER)
                    && message.contains("file lock")
                    && message.contains("operation=issue_monitor_prefs")
        )),
        "the fallback must fail on the prefs lock deadline, not another error: {:?}",
        events
            .iter()
            .filter_map(|event| match &event.event {
                BackendEvent::IssueMonitorToast { message, .. } => Some(message.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(fs::read(&prefs_path).expect("reload prefs"), before);
    assert!(fs::read_dir(prefs_path.parent().expect("prefs parent"))
        .expect("read prefs parent")
        .flatten()
        .all(|entry| !entry
            .file_name()
            .to_string_lossy()
            .starts_with("issue-monitor.json.corrupt-")));
    assert!(events
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::IssueMonitorLaunchFailed { .. })));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, message, issue_number, .. }
            if level == "error"
                && message.contains("local fallback control commit failed")
                && *issue_number == Some(42)
    )));
}

#[test]
fn app_runtime_spawn_fallback_rechecks_new_daemon_fence_before_commit() {
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
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            launching_issues: vec![gwt::IssueMonitorLaunchingIssue {
                issue_number: 42,
                claimed_at: None,
            }],
            effect_authority_epoch: 7,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let before = fs::read(&prefs_path).expect("read seeded prefs");
    let fence = gwt::IssueMonitorAuthorityFence::current_process();
    gwt::persist_issue_monitor_authority_fence(&prefs_path, &fence)
        .expect("daemon establishes fence after publisher Spawn classification");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.issue_monitor_launch_failed_result_events(
        42,
        "launch failed",
        Err(
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                "publisher observed missing endpoint".to_string(),
            ),
        ),
    );

    assert_eq!(fs::read(&prefs_path).expect("reload prefs"), before);
    assert!(events
        .iter()
        .all(|event| !matches!(event.event, BackendEvent::IssueMonitorLaunchFailed { .. })));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, message, .. }
            if level == "error" && message.contains("authority fence appeared")
    )));
}

#[test]
fn app_runtime_lifecycle_recovery_blocked_preserves_corrupt_prefs() {
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
    fs::create_dir_all(prefs_path.parent().expect("prefs parent")).expect("create prefs parent");
    let corrupt = b"{\"enabled\":true";
    fs::write(&prefs_path, corrupt).expect("seed corrupt prefs");
    let quarantine_count = || {
        fs::read_dir(prefs_path.parent().expect("prefs parent"))
            .expect("read prefs directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("issue-monitor.json.corrupt-")
            })
            .count()
    };
    let before_quarantines = quarantine_count();
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let blocked = Err(gwt::runtime_daemon_events::IssueMonitorControlPublishError::RecoveryBlocked);

    let mut events = runtime.issue_monitor_agent_failed_result_events(
        "tab-1::agent-1",
        "agent failed",
        Some(42),
        blocked.clone(),
    );
    events.extend(runtime.issue_monitor_window_closed_result_events("tab-1::agent-1", blocked));

    assert_eq!(fs::read(&prefs_path).expect("read corrupt prefs"), corrupt);
    assert_eq!(quarantine_count(), before_quarantines);
    assert!(!events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
    )));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, message, .. }
            if level == "error" && message.contains(
                gwt::runtime_daemon_events::ISSUE_MONITOR_CONTROL_RECOVERY_BLOCKED_ERROR
            )
    )));
}

#[test]
fn app_runtime_routine_control_fallback_preserves_effect_authority_and_journal() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    gwt::machine_agent_settings::set(
        Some(6),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-22T00:00:00Z",
    )
    .expect("seed machine Manual capacity");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let journal = vec![gwt::PendingIssueMonitorEffect::prepared(
        "release:claim:42:7",
        7,
        gwt::IssueMonitorEffectPayload::ReleaseClaim {
            issue_number: 42,
            claim_id: "stable-claim-42".to_string(),
            owner: "host/session".to_string(),
        },
    )];
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            effect_authority_epoch: 7,
            pending_effects: journal.clone(),
            max_active_agents: 6,
            priority_order: vec![42],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let (proxy, recorded) = AppEventProxy::stub();
    runtime.proxy = proxy;
    let max_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorMaxActiveAgents {
            max_active_agents: Some(3),
        },
    );
    assert!(
        max_events.is_empty(),
        "durable control must leave the GUI thread"
    );
    assert_eq!(tasks.lock().unwrap().len(), 1);
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .max_active_agents,
        6
    );
    assert!(runtime
        .handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::SetIssueMonitorMaxActiveAgents {
                max_active_agents: None
            },
        )
        .is_empty());
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "Auto waits for the preceding Manual save"
    );
    let profile_edit = serde_json::from_value(serde_json::json!({
        "kind": "issue_monitor_profiles_set",
        "profiles": [{"agent_id": "codex", "reasoning": "high"}],
        "request_id": 5026,
    }))
    .expect("profile edit");
    assert!(runtime
        .handle_frontend_event("client-1".to_string(), profile_edit)
        .is_empty());
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "profile save must wait for every preceding Max active request"
    );
    tasks.lock().unwrap().pop().unwrap()();
    let completion = recorded.lock().unwrap().pop().unwrap();
    let UserEvent::IssueMonitorMaxActivePrepared(prepared) = completion else {
        panic!("Max active completion expected");
    };
    let max_events = runtime.issue_monitor_max_active_prepared_events(*prepared);
    assert!(max_events.iter().any(|event| matches!(
        &event.event, BackendEvent::IssueMonitorStatus { status }
            if status.max_active_agents_override == Some(3)
    )));
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "completion starts the next save"
    );
    let reorder_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ReorderIssueMonitorIssues {
            issue_numbers: vec![99, 42],
        },
    );

    assert!(
        !reorder_events.is_empty(),
        "missing daemon forces the atomic GUI fallback writer"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(persisted.max_active_agents, 3);
    let change =
        serde_json::to_value(&persisted).expect("prefs json")["max_active_last_change"].clone();
    assert_eq!(change["source"], "gui");
    assert_eq!(change["max_active_agents_override"], 3);
    chrono::DateTime::parse_from_rfc3339(change["changed_at"].as_str().expect("change time"))
        .expect("RFC3339 change time");
    assert_eq!(persisted.priority_order, vec![99, 42]);
    assert_eq!(persisted.effect_authority_epoch, 7);
    assert_eq!(persisted.pending_effects, journal);

    tasks.lock().unwrap().pop().unwrap()();
    let completion = recorded.lock().unwrap().pop().unwrap();
    let UserEvent::IssueMonitorMaxActivePrepared(prepared) = completion else {
        panic!("Auto completion expected");
    };
    runtime.issue_monitor_max_active_prepared_events(*prepared);
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    assert_eq!(
        persisted.max_active_agents_mode,
        gwt::issue_monitor::IssueMonitorMaxActiveMode::Auto
    );
    let change = persisted.max_active_last_change.unwrap();
    assert_eq!(
        change.source,
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::Gui
    );
    assert_eq!(change.max_active_agents_override, None);
    assert_eq!(persisted.priority_order, vec![99, 42]);
    assert_eq!(persisted.pending_effects, journal);
    assert!(runtime.issue_monitor_max_active_saves.is_empty());
    assert!(tasks.lock().unwrap().is_empty());

    let deferred = recorded
        .lock()
        .unwrap()
        .pop()
        .expect("deferred profile edit");
    assert!(matches!(&deferred, UserEvent::ProjectCompletion { .. }));
    let Some(UserEvent::Frontend {
        client_id,
        client_scope: Some(scope),
        event,
        ..
    }) = runtime.accept_project_completion(deferred)
    else {
        panic!("profile edit must retain its project generation and scope");
    };
    assert!(runtime
        .handle_frontend_event_in_scope(client_id, event, &scope)
        .is_empty());
    assert_eq!(tasks.lock().unwrap().len(), 1);
    {
        let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
        drain_queued_blocking_tasks(&tasks);
    }
    let UserEvent::IssueMonitorProfilesSaved(completion) = recorded
        .lock()
        .unwrap()
        .pop()
        .expect("profile save completion")
    else {
        panic!("profile save completion expected");
    };
    let profile_events = runtime.handle_issue_monitor_profiles_saved(*completion);
    let status = profile_events
        .iter()
        .rev()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("profile completion publishes the current Monitor settings");
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("profile save prefs");
    assert_eq!(persisted.max_active_agents, 3);
    assert_eq!(status.max_active_agents, persisted.max_active_agents);
    assert_eq!(status.max_active_agents_override, None);
    assert_eq!(
        status.max_active_last_change,
        persisted.max_active_last_change
    );
    let change = persisted.max_active_last_change.as_ref().unwrap();
    assert_eq!(
        change.source,
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::Gui
    );
    assert_eq!(change.max_active_agents_override, None);
    assert_eq!(
        persisted.launch_profile_pool()[0].reasoning.as_deref(),
        Some("high")
    );
    assert!(runtime.issue_monitor_profile_saves.is_empty());

    runtime.blocking_tasks = BlockingTaskSpawner::failing("worker unavailable");
    let failed = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorMaxActiveAgents {
            max_active_agents: Some(9),
        },
    );
    assert!(failed.iter().any(|event| matches!(
        &event.event, BackendEvent::IssueMonitorToast { level, message, .. }
            if level == "error" && message.contains("worker unavailable")
    )));
    assert!(runtime.issue_monitor_max_active_saves.is_empty());
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .max_active_last_change
            .unwrap()
            .max_active_agents_override,
        None
    );
}

#[test]
fn app_runtime_max_active_save_orders_reopened_profile_after_old_worker() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut seeded = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        max_active_agents: 6,
        ..Default::default()
    };
    seeded.set_launch_profile_pool(vec![pool_profile("codex")]);
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).unwrap();
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, recorded) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.handle_frontend_event(
        "old-client".into(),
        FrontendEvent::SetIssueMonitorMaxActiveAgents {
            max_active_agents: Some(3),
        },
    );
    let old_context = runtime.test_context();
    runtime
        .project_tab_incarnations
        .get_mut("tab-1")
        .unwrap()
        .generation += 1;
    runtime.refresh_project_state("tab-1");
    assert!(!runtime.project_context_is_current(&old_context));
    let profile_edit = serde_json::from_value(serde_json::json!({
        "kind": "issue_monitor_profiles_set",
        "profiles": [{"agent_id": "codex", "reasoning": "high"}],
        "request_id": 5026,
    }))
    .unwrap();
    runtime.handle_frontend_event("new-client".into(), profile_edit);
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "reopening must not let a profile worker overtake the old Max active save"
    );

    tasks.lock().unwrap().pop().unwrap()();
    let UserEvent::IssueMonitorMaxActivePrepared(prepared) =
        recorded.lock().unwrap().pop().unwrap()
    else {
        panic!("the runtime must receive the old completion to release its lane");
    };
    assert!(runtime
        .issue_monitor_max_active_prepared_events(*prepared)
        .is_empty());
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .max_active_agents,
        3
    );
    assert!(tasks.lock().unwrap().is_empty());

    let deferred = recorded
        .lock()
        .unwrap()
        .pop()
        .expect("new generation profile edit");
    assert!(matches!(&deferred, UserEvent::ProjectCompletion { .. }));
    let Some(UserEvent::Frontend {
        client_id,
        client_scope: Some(scope),
        event,
        ..
    }) = runtime.accept_project_completion(deferred)
    else {
        panic!("the new generation profile request must retain its scope");
    };
    assert_eq!(client_id, "new-client");
    assert!(runtime
        .handle_frontend_event_in_scope(client_id, event, &scope)
        .is_empty());
    assert_eq!(tasks.lock().unwrap().len(), 1);
    {
        let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
        drain_queued_blocking_tasks(&tasks);
    }
    let UserEvent::IssueMonitorProfilesSaved(completion) = recorded.lock().unwrap().pop().unwrap()
    else {
        panic!("profile save completion expected");
    };
    let events = runtime.handle_issue_monitor_profiles_saved(*completion);
    let status = events
        .iter()
        .rev()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("current profile snapshot");
    let saved = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    assert_eq!(saved.max_active_agents, 3);
    assert_eq!(status.max_active_agents, saved.max_active_agents);
    assert_eq!(status.max_active_agents_override, Some(3));
    assert_eq!(status.max_active_last_change, saved.max_active_last_change);
    let change = saved.max_active_last_change.as_ref().unwrap();
    assert_eq!(
        change.source,
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::Gui
    );
    assert_eq!(change.max_active_agents_override, Some(3));
    assert_eq!(
        saved.launch_profile_pool()[0].reasoning.as_deref(),
        Some("high")
    );
    assert!(runtime.issue_monitor_profile_saves.is_empty());
    assert!(tasks.lock().unwrap().is_empty());
}

#[test]
fn app_runtime_allowed_labels_fallback_persists_without_changing_mode() {
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
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    for labels in [
        serde_json::json!(["Server", "backend"]),
        serde_json::json!([]),
    ] {
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "set_issue_monitor_allowed_labels",
            "allowed_labels": labels,
        }))
        .expect("allowed labels event");
        assert!(!runtime
            .handle_frontend_event("client-1".to_string(), event)
            .is_empty());
        let saved = gwt::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        assert!(saved.enabled && saved.autonomous_mode);
        let saved = serde_json::to_value(saved).expect("serialize prefs");
        assert_eq!(
            saved
                .get("allowed_labels")
                .cloned()
                .unwrap_or(serde_json::json!([])),
            labels
        );
    }
}

#[test]
fn app_runtime_allowed_labels_rejection_returns_a_correlated_failure() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            allowed_labels: vec!["Server".to_string()],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(prefs_path.with_extension("lock"))
        .expect("open prefs lock");
    lock.lock_exclusive().expect("hold prefs lock");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_monitor_fallback_commit_timeout = Duration::from_millis(100);
    let event = serde_json::from_value(serde_json::json!({
        "kind": "set_issue_monitor_allowed_labels", "allowed_labels": ["Tools"], "request_id": 41,
    }))
    .expect("label command");
    let events = runtime.handle_frontend_event("client-1".to_string(), event);
    FileExt::unlock(&lock).expect("release prefs lock");
    let failure = events
        .iter()
        .map(|event| serde_json::to_value(&event.event).expect("event wire shape"))
        .find(|event| event["kind"] == "issue_monitor_allowed_labels_write_failed")
        .expect("a rejected save returns a correlated failure to its editor");
    assert_eq!(failure["request_id"], 41);
    assert_eq!(failure["outcome_unknown"], false);
    assert!(events.iter().all(|event| !matches!(
        event.event,
        BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
    )));
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("saved prefs")
            .allowed_labels,
        ["Server"]
    );
}

#[test]
fn app_runtime_allowed_labels_failure_distinguishes_busy_from_unknown() {
    use gwt::runtime_daemon_events::IssueMonitorControlPublishError;
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let context = runtime.project_context("tab-1").expect("project context");
    for (error, outcome_unknown) in [
        (
            IssueMonitorControlPublishError::Busy("admission full".to_string()),
            false,
        ),
        (
            IssueMonitorControlPublishError::OutcomeUnknown("control timed out".to_string()),
            true,
        ),
    ] {
        let events = runtime.issue_monitor_allowed_labels_result_events(
            &context,
            "client-1",
            Err(error),
            vec!["Tools".to_string()],
            Some(41),
        );
        assert!(events.iter().all(|event| matches!(&event.target,
            DispatchTarget::Client(client) if client == "client-1")));
        assert!(events.iter().any(|event| matches!(&event.event,
            BackendEvent::IssueMonitorToast { level, .. } if level == "error")));
        assert!(events.iter().any(|event| matches!(&event.event,
            BackendEvent::IssueMonitorAllowedLabelsWriteFailed { request_id: 41, outcome_unknown: actual }
                if *actual == outcome_unknown)));
        assert!(events.iter().all(|event| !matches!(
            event.event,
            BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
        )));
    }
}

// SPEC #3165 TQ-9: the row's "Add to queue" action is the user's way to put an
// Issue into this terminal's implementation queue. It is the requested feature's
// main direction — "remove" is only its counterpart — so the GUI must reach
// `terminal_queue_push` the same way it reaches the removal, and it must not
// touch another terminal's queue.
#[test]
fn app_runtime_issue_monitor_queue_push_adds_only_to_the_local_terminal_queue() {
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
    let queue = |numbers: &[u64]| gwt::issue_monitor::IssueMonitorTerminalQueue {
        entries: numbers
            .iter()
            .map(
                |number| gwt::issue_monitor::IssueMonitorTerminalQueueEntry {
                    number: *number,
                    queued_at: "2026-09-10T00:00:00Z".to_string(),
                    queued_by: "operator".to_string(),
                    ..Default::default()
                },
            )
            .collect(),
        last_seen_at: None,
    };
    let host = gwt::process::current_hostname();
    let mut seeded = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        max_active_agents: 1,
        ..gwt::IssueMonitorPrefs::default()
    };
    seeded.terminal_queues.insert(host.clone(), queue(&[42]));
    seeded
        .terminal_queues
        .insert(format!("{host}-other"), queue(&[42]));
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorQueuePush {
            issue_numbers: vec![43],
        },
    );

    assert!(!events.is_empty(), "push answers with a refreshed snapshot");
    assert!(
        events.iter().any(|event| {
            event.target == DispatchTarget::Client("client-1".to_string())
                && serde_json::to_value(&event.event).expect("queue result wire shape")
                    == serde_json::json!({
                        "kind": "issue_monitor_queue_result", "operation": "push",
                        "issue_numbers": [43], "error": null,
                    })
        }),
        "queue push must return its committed result to the origin client"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    let numbers = |terminal: &str| {
        persisted.terminal_queues[terminal]
            .entries
            .iter()
            .map(|entry| entry.number)
            .collect::<Vec<_>>()
    };
    assert_eq!(numbers(&host), vec![42, 43]);
    assert_eq!(numbers(&format!("{host}-other")), vec![42]);
    let queued_by = persisted.terminal_queues[&host]
        .entries
        .iter()
        .find(|entry| entry.number == 43)
        .map(|entry| entry.queued_by.clone());
    assert_eq!(
        queued_by.as_deref(),
        Some("operator"),
        "a queue push from the row is the operator's own act"
    );
}

// SPEC #3165 TQ-9 / AC-4: the row's "Remove from queue" action removes the
// Issue from this terminal's queue and leaves other terminals' queues alone.
#[test]
fn app_runtime_issue_monitor_queue_remove_drops_only_the_local_terminal_entry() {
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
    let queue = |numbers: &[u64]| gwt::issue_monitor::IssueMonitorTerminalQueue {
        entries: numbers
            .iter()
            .map(
                |number| gwt::issue_monitor::IssueMonitorTerminalQueueEntry {
                    number: *number,
                    queued_at: "2026-09-10T00:00:00Z".to_string(),
                    queued_by: "operator".to_string(),
                    ..Default::default()
                },
            )
            .collect(),
        last_seen_at: None,
    };
    let host = gwt::process::current_hostname();
    let mut seeded = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        max_active_agents: 1,
        ..gwt::IssueMonitorPrefs::default()
    };
    seeded
        .terminal_queues
        .insert(host.clone(), queue(&[42, 43]));
    seeded
        .terminal_queues
        .insert(format!("{host}-other"), queue(&[42]));
    gwt::save_issue_monitor_prefs(&prefs_path, &seeded).expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::IssueMonitorQueueRemove {
            issue_numbers: vec![42],
        },
    );

    assert!(
        !events.is_empty(),
        "removal answers with a refreshed snapshot"
    );
    assert!(
        events.iter().any(|event| {
            event.target == DispatchTarget::Client("client-1".to_string())
                && serde_json::to_value(&event.event).expect("queue result wire shape")
                    == serde_json::json!({
                        "kind": "issue_monitor_queue_result", "operation": "remove",
                        "issue_numbers": [42], "error": null,
                    })
        }),
        "queue removal must return its committed result to the origin client"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    let numbers = |terminal: &str| {
        persisted.terminal_queues[terminal]
            .entries
            .iter()
            .map(|entry| entry.number)
            .collect::<Vec<_>>()
    };
    assert_eq!(numbers(&host), vec![43]);
    assert_eq!(numbers(&format!("{host}-other")), vec![42]);
}

#[test]
fn app_runtime_issue_monitor_queue_result_reports_daemon_and_local_commit_outcomes() {
    use gwt::runtime_daemon_events::IssueMonitorControlPublishError;
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let context = runtime.test_context();
    for (publication, expected_error) in [
        (Ok(()), None),
        (
            Err(IssueMonitorControlPublishError::Rejected(
                "control rejected".into(),
            )),
            Some("control rejected"),
        ),
        (
            Err(IssueMonitorControlPublishError::OutcomeUnknown(
                "ack timed out".into(),
            )),
            Some("ack timed out"),
        ),
    ] {
        let events = runtime.issue_monitor_control_result_events_with_queue_result(
            &context,
            "client-1",
            publication,
            "queue-move",
            Some(("move", vec![42])),
            |_| panic!("an ACK or uncertain/rejected control must not apply local fallback"),
        );
        assert!(events
            .iter()
            .all(|event| event.target == DispatchTarget::Client("client-1".into())));
        assert!(events.iter().any(|event| matches!(&event.event,
            BackendEvent::IssueMonitorQueueResult { operation, issue_numbers, error }
                if operation == "move" && issue_numbers == &[42] && error.as_deref() == expected_error)));
        assert!(events.iter().all(|event| !matches!(
            event.event,
            BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
        )));
    }

    runtime.issue_monitor_fallback_commit_timeout = Duration::ZERO;
    let events = runtime.issue_monitor_control_result_events_with_queue_result(
        &context,
        "client-1",
        Err(IssueMonitorControlPublishError::TransportUnavailable(
            "daemon unavailable".into(),
        )),
        "queue-push",
        Some(("push", vec![42])),
        |monitor| monitor.terminal_queue_push(&[42], "operator", "2026-10-23T00:00:00Z"),
    );
    let error = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorQueueResult {
                error: Some(error), ..
            } => Some(error),
            _ => None,
        })
        .expect("failed local commit must return its actual error");
    assert!(error.contains("local fallback control commit failed"));
    assert!(error.contains(gwt_core::operation_deadline::DEADLINE_EXPIRED_MARKER));
    assert!(events.iter().any(|event| matches!(&event.event,
        BackendEvent::IssueMonitorToast { message, .. } if message == error)));
}

#[test]
fn app_runtime_local_driver_locked_latest_state_preserves_proposal_fence_result_and_delivery() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // This asserts durable claim results, not filesystem speed under load.
    let operation_now = gwt_core::operation_deadline::now();
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(operation_now);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let preserved = gwt::PendingIssueMonitorEffect::prepared(
        "disarm:99:7",
        7,
        gwt::IssueMonitorEffectPayload::DisarmAutoMerge {
            issue_number: 99,
            pr_number: 199,
            compensates_effect_id: "arm:99:7".to_string(),
        },
    );
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 7,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            pending_effects: vec![preserved.clone()],
            ..queued_issue_monitor_prefs(&[42])
        },
    )
    .expect("seed latest disk authority");
    let loaded = gwt::issue_monitor_worker::LoadedIssueMonitorCandidates {
        linked_prs: Default::default(),
        open_pr_observations: std::collections::BTreeMap::new(),
        open_pr_observed_at: None,
        issues: vec![gwt::IssueMonitorIssue {
            author_login: None,
            number: 42,
            title: "Issue Monitor local proposal".to_string(),
            labels: vec!["bug".to_string()],
            state: gwt::IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: gwt::IssueMonitorReadiness::NotApplicable,
            updated_at: None,
        }],
        source: gwt::IssueMonitorCandidateSource::Live,
        live_error: None,
        readiness_failures: Vec::new(),
        urgent_assignments: Default::default(),
    };
    let now = "2026-07-28T00:00:00Z";
    let mut stale = test_monitor(gwt::IssueMonitorConfig::default());
    // Issue #4045 AC-2: the transaction's outcome is the subject, so pin its
    // budget instead of inheriting the production 250 ms wall-clock bound.
    let _budget = super::super::ScopedLocalIssueMonitorPrefsTimeout::set(
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
    );

    rebase_mutate_and_persist_issue_monitor_state(&prefs_path, &mut stale, |latest| {
        gwt::issue_monitor_worker::scan_loaded_issue_monitor_candidates(
            latest, &loaded, &repo, now,
        );
        latest.set_gui_connected(true);
        // Issue #3528: an outcome is bound to the Issue it was observed for,
        // and a candidate without one is deferred — so the fixture states that
        // #42 was probed this tick and is still open.
        prepare_local_issue_monitor_claim_proposals(
            latest,
            &loaded,
            "windows-host/session",
            now,
            &std::collections::BTreeMap::from([(42, false)]),
        );
    })
    .expect("latest-state transaction commits the prepared claim proposal");

    let prepared = stale
        .pending_effects()
        .iter()
        .find(|effect| {
            matches!(
                effect.payload,
                gwt::IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: 42,
                    ..
                }
            )
        })
        .cloned()
        .expect("latest-state transaction preserves prepared claim proposal");
    assert_eq!(prepared.state, gwt::IssueMonitorEffectState::Prepared);
    assert!(stale
        .pending_effects()
        .iter()
        .any(|effect| effect.effect_id == preserved.effect_id));

    let mut bounded_remote_calls = 0;
    let mut attempting_effect_id = None;
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        operation_now + Duration::from_secs(5),
    );
    drive_local_issue_monitor_claim_effects_with(
        &prefs_path,
        &mut stale,
        |effect, authority_current, _call_now, call_now_text| {
            assert_eq!(effect.state, gwt::IssueMonitorEffectState::Attempting);
            assert!(authority_current);
            assert!(
                gwt_core::operation_deadline::ensure_remaining("test bounded call")
                    .expect("deadline probe")
                    .is_some()
            );
            bounded_remote_calls += 1;
            attempting_effect_id = Some(effect.effect_id.clone());
            let claim = match &effect.payload {
                gwt::IssueMonitorEffectPayload::AcquireClaim {
                    issue_number,
                    claim_id,
                    owner,
                    expires_at,
                    launched_work_id,
                    ..
                } => gwt_github::issue_auto_claim::ClaimComment {
                    comment_id: Some(gwt_github::CommentId(42)),
                    claim_id: claim_id.clone(),
                    owner: owner.clone(),
                    issue_number: *issue_number,
                    status: gwt_github::issue_auto_claim::ClaimStatus::Active,
                    heartbeat_at: call_now_text.to_string(),
                    expires_at: expires_at.clone(),
                    launched_work_id: launched_work_id.clone(),
                },
                other => panic!("unexpected local effect: {other:?}"),
            };
            Ok(LocalIssueMonitorEffectOutcome::Claim(Ok(
                gwt_github::issue_auto_claim::ClaimAcquireOutcome::Acquired(claim),
            )))
        },
    )
    .expect("local driver completes exact claim result");
    assert_eq!(bounded_remote_calls, 1);
    let attempting_effect_id = attempting_effect_id.expect("attempting effect id");
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload exact result");
    assert!(persisted
        .pending_effects
        .iter()
        .any(|effect| effect.effect_id == preserved.effect_id));
    assert!(!persisted
        .pending_effects
        .iter()
        .any(|effect| effect.effect_id == attempting_effect_id));
    assert_eq!(persisted.pending_launch_deliveries.len(), 1);
    assert_eq!(
        persisted.pending_launch_deliveries[0].delivery_id,
        format!("launch:{attempting_effect_id}")
    );
}

/// Issue #4045 AC-3: a persist that outlives the GUI-local prefs budget must
/// not silently keep an in-memory `Prepared` proposal that disk never saw.
/// The delay lands between the mutation and the durable rename — the exact
/// point where a saturated Windows runner's fsync expired the 250 ms budget in
/// run 34047529539 — so the next attempt rebases from disk, finds nothing, and
/// `bounded_remote_calls` stays 0 instead of reaching 1.
#[test]
fn app_runtime_local_driver_slow_persist_does_not_silently_drop_prepared_proposal() {
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
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 7,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..queued_issue_monitor_prefs(&[42])
        },
    )
    .expect("seed disk authority");
    let loaded = gwt::issue_monitor_worker::LoadedIssueMonitorCandidates {
        linked_prs: Default::default(),
        open_pr_observations: std::collections::BTreeMap::new(),
        open_pr_observed_at: None,
        issues: vec![gwt::IssueMonitorIssue {
            author_login: None,
            number: 42,
            title: "Issue Monitor slow persist".to_string(),
            labels: vec!["bug".to_string()],
            state: gwt::IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: gwt::IssueMonitorReadiness::NotApplicable,
            updated_at: None,
        }],
        source: gwt::IssueMonitorCandidateSource::Live,
        live_error: None,
        readiness_failures: Vec::new(),
        urgent_assignments: Default::default(),
    };
    let now = "2026-07-28T00:00:00Z";
    let mut stale = test_monitor(gwt::IssueMonitorConfig::default());
    // Longer than the production budget, well inside the pinned one.
    let persist_delay =
        super::super::ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT + Duration::from_millis(150);
    let _budget = super::super::ScopedLocalIssueMonitorPrefsTimeout::set(
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
    );

    rebase_mutate_and_persist_issue_monitor_state(&prefs_path, &mut stale, |latest| {
        gwt::issue_monitor_worker::scan_loaded_issue_monitor_candidates(
            latest, &loaded, &repo, now,
        );
        latest.set_gui_connected(true);
        // Issue #3528: an outcome is bound to the Issue it was observed for,
        // and a candidate without one is deferred — so the fixture states that
        // #42 was probed this tick and is still open.
        prepare_local_issue_monitor_claim_proposals(
            latest,
            &loaded,
            "windows-host/session",
            now,
            &std::collections::BTreeMap::from([(42, false)]),
        );
        // The proposal exists in memory; the durable rename is what comes next.
        thread::sleep(persist_delay);
    })
    .expect("a persist inside the pinned budget commits the prepared proposal");

    let mut bounded_remote_calls = 0;
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + Duration::from_secs(5),
    );
    drive_local_issue_monitor_claim_effects_with(
        &prefs_path,
        &mut stale,
        |effect, _authority_current, _call_now, call_now_text| {
            bounded_remote_calls += 1;
            let claim = match &effect.payload {
                gwt::IssueMonitorEffectPayload::AcquireClaim {
                    issue_number,
                    claim_id,
                    owner,
                    expires_at,
                    launched_work_id,
                    ..
                } => gwt_github::issue_auto_claim::ClaimComment {
                    comment_id: Some(gwt_github::CommentId(42)),
                    claim_id: claim_id.clone(),
                    owner: owner.clone(),
                    issue_number: *issue_number,
                    status: gwt_github::issue_auto_claim::ClaimStatus::Active,
                    heartbeat_at: call_now_text.to_string(),
                    expires_at: expires_at.clone(),
                    launched_work_id: launched_work_id.clone(),
                },
                other => panic!("unexpected local effect: {other:?}"),
            };
            Ok(LocalIssueMonitorEffectOutcome::Claim(Ok(
                gwt_github::issue_auto_claim::ClaimAcquireOutcome::Acquired(claim),
            )))
        },
    )
    .expect("local driver completes the claim");

    assert_eq!(
        bounded_remote_calls, 1,
        "a slow persist must not silently lose the prepared proposal"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(persisted.pending_launch_deliveries.len(), 1);
}

/// Issue #4045 AC-1: when the durable write expires the budget after the
/// mutation already ran, the helper must report the failure and restore the
/// in-memory state from canonical disk instead of returning the unapplied
/// mutation as if it had won.
#[test]
fn app_runtime_gui_rebase_persist_deadline_expiry_fails_closed_and_restores_disk_state() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let prefs_path = temp.path().join("issue-monitor.json");
    let disk = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        enabled: true,
        max_active_agents: 2,
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &disk).expect("seed prefs");
    let before = fs::read(&prefs_path).expect("read seeded prefs");
    let mut monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), disk.clone());
    let budget = Duration::from_millis(100);
    let _budget = super::super::ScopedLocalIssueMonitorPrefsTimeout::set(budget);
    let persist_delay = budget + Duration::from_millis(150);

    let mutated = super::super::rebase_mutate_and_persist_issue_monitor_state(
        &prefs_path,
        &mut monitor,
        |latest| {
            latest.set_max_active_agents(4);
            thread::sleep(persist_delay);
            true
        },
    );

    let error = mutated.expect_err("an expired persist must fail closed");
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut, "{error}");
    assert_eq!(
        fs::read(&prefs_path).expect("reload prefs"),
        before,
        "an expired persist must be zero-write"
    );
    assert_eq!(
        monitor.prefs().max_active_agents,
        disk.max_active_agents,
        "in-memory state must be restored from canonical disk state"
    );
}

#[test]
fn app_runtime_local_driver_surfaces_remote_claim_failures_in_the_monitor_snapshot() {
    use gwt_github::client::{ApiError, OwnerMutationError};

    for outcome_unknown in [false, true] {
        let temp = tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(temp.path());
        let prefs_path = temp.path().join("issue-monitor.json");
        let prefs = gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        };
        let mut monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs);
        monitor.terminal_queue_push(&[42], "operator", "2026-07-28T00:00:00Z");
        monitor
            .prepare_pending_effect(
                "claim-effect-42",
                gwt::IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: 42,
                    claim_id: "claim-42".to_string(),
                    owner: "host/session".to_string(),
                    heartbeat_at: "2026-08-10T01:00:00Z".to_string(),
                    expires_at: "2026-08-10T01:30:00Z".to_string(),
                    launched_work_id: Some("work/issue-42".to_string()),
                },
            )
            .expect("prepare claim");
        gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist claim");

        drive_local_issue_monitor_claim_effects_with(
            &prefs_path,
            &mut monitor,
            |_effect, _authority_current, _now, _now_text| {
                let source = ApiError::Unexpected(
                    if outcome_unknown {
                        "injected unknown outcome"
                    } else {
                        "injected pre-submit failure"
                    }
                    .to_string(),
                );
                let error = if outcome_unknown {
                    OwnerMutationError::RemoteOutcomeUnknown(source)
                } else {
                    OwnerMutationError::PreSubmit(source)
                };
                Ok(LocalIssueMonitorEffectOutcome::Claim(Err(error)))
            },
        )
        .expect("the journal retains retry/unknown state");

        let error = monitor
            .status_view()
            .last_error
            .expect("remote failure must be operator-visible");
        assert!(
            error.contains(if outcome_unknown {
                "injected unknown outcome"
            } else {
                "injected pre-submit failure"
            }),
            "the exact owner mutation failure remains visible: {error}"
        );
    }
}

#[test]
fn app_runtime_compatibility_claim_driver_defers_while_scheduled_lease_is_held() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    // Issue #3601: `issue_monitor_prefs_path_for_repo_path` resolves the
    // process-global `HOME`, and a parallel test that points `HOME` at its own
    // tempdir moves this fixture into that tempdir. Once that test drops its
    // `TempDir`, the fixture is removed and the reload below silently returns
    // `IssueMonitorPrefs::default()` instead of the persisted Prepared fence.
    // Pinning the thread-local home keeps the fixture owned by this test.
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    assert!(
        prefs_path.starts_with(temp.path()),
        "the durable fence fixture must be owned by this test, not by the process-global \
         home a parallel test may replace and delete: {}",
        prefs_path.display()
    );
    let mut monitor = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    );
    monitor.terminal_queue_push(&[42], "operator", "2026-07-28T00:00:00Z");
    monitor
        .prepare_pending_effect(
            "claim-effect-42",
            gwt::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "claim-42".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-08-11T00:00:00Z".to_string(),
                expires_at: "2026-08-11T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        )
        .expect("prepare claim");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist claim");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let remote_calls = Arc::new(AtomicUsize::new(0));
    runtime.issue_client_factory = Arc::new({
        let remote_calls = Arc::clone(&remote_calls);
        move |_owner, _repo| {
            remote_calls.fetch_add(1, Ordering::SeqCst);
            Err(ApiError::Unexpected(
                "compatibility driver crossed the scheduled lease".to_string(),
            ))
        }
    });
    let scheduled_lease = gwt::try_acquire_issue_monitor_local_fallback_lease(&prefs_path)
        .expect("scheduled worker lease");

    let events = runtime.drive_local_issue_monitor_claim_effects(
        &prefs_path,
        "owner",
        "repo",
        &repo,
        &mut monitor,
    );

    assert!(
        events.is_empty(),
        "the contending driver must defer cleanly"
    );
    assert_eq!(
        remote_calls.load(Ordering::SeqCst),
        0,
        "only the scheduled lease owner may cross the remote mutation boundary"
    );
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("reload deferred effect")
            .pending_effects[0]
            .state,
        gwt::IssueMonitorEffectState::Prepared,
        "deferral must not claim the durable Attempting fence"
    );
    drop(scheduled_lease);
}

#[test]
fn app_runtime_local_claim_result_cannot_revive_candidate_excluded_after_attempt_fence() {
    // Assert the durable claim result, independently of filesystem speed.
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let prefs_path = temp.path().join("issue-monitor.json");
    let mut monitor = test_monitor(gwt::IssueMonitorConfig {
        enabled: true,
        ..gwt::IssueMonitorConfig::default()
    });
    let mut issue = gwt::IssueMonitorIssue {
        author_login: None,
        number: 42,
        title: "Issue 42".to_string(),
        labels: vec!["bug".to_string()],
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: None,
    };
    monitor.terminal_queue_push(&[42], "operator", "2026-07-28T00:00:00Z");
    monitor.record_candidate(issue.clone());
    let key = monitor
        .prepare_pending_effect(
            "claim-effect-42",
            gwt::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "claim-42".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-08-05T10:00:00Z".to_string(),
                expires_at: "2026-08-05T10:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        )
        .expect("prepare claim effect");
    assert!(monitor.mark_pending_effect_attempting(&key));
    let attempting = monitor.pending_effects()[0].clone();

    issue.labels.push("gwt-spec".to_string());
    issue.readiness = gwt::IssueMonitorReadiness::NotReady;
    monitor.record_candidate(issue);
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist excluded state");

    // Assert fenced state transitions independently of host fsync latency.
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
    assert_eq!(
        super::super::commit_local_issue_monitor_effect_result(
            &prefs_path,
            &mut monitor,
            attempting,
            LocalIssueMonitorEffectOutcome::Claim(Ok(
                gwt_github::issue_auto_claim::ClaimAcquireOutcome::Acquired(
                    gwt_github::issue_auto_claim::ClaimComment {
                        comment_id: Some(gwt_github::CommentId(99)),
                        claim_id: "claim-42".to_string(),
                        owner: "host/session".to_string(),
                        issue_number: 42,
                        status: gwt_github::issue_auto_claim::ClaimStatus::Active,
                        heartbeat_at: "2026-08-05T10:00:00Z".to_string(),
                        expires_at: "2026-08-05T10:30:00Z".to_string(),
                        launched_work_id: Some("work/issue-42".to_string()),
                    },
                ),
            )),
            "2026-08-05T10:01:00Z",
        )
        .expect("commit exact local effect result"),
        1
    );

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert!(persisted.launching_issues.is_empty());
    assert!(persisted.pending_launch_deliveries.is_empty());
    assert!(persisted.pending_effects.iter().any(|effect| matches!(
        &effect.payload,
        gwt::IssueMonitorEffectPayload::ReleaseClaim {
            issue_number: 42,
            claim_id,
            owner,
        } if claim_id == "claim-42" && owner == "host/session"
    )));
}

#[test]
fn app_runtime_local_driver_rejects_stale_process_attempting_without_disk_fence() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let prefs_path = temp.path().join("issue-monitor.json");
    let mut stale_effect = gwt::PendingIssueMonitorEffect::prepared(
        "claim:42:stale",
        7,
        gwt::IssueMonitorEffectPayload::AcquireClaim {
            issue_number: 42,
            claim_id: "claim-42-stale".to_string(),
            owner: "stale/session".to_string(),
            heartbeat_at: "2026-07-28T00:00:00Z".to_string(),
            expires_at: "2026-07-28T00:30:00Z".to_string(),
            launched_work_id: Some("work/issue-42".to_string()),
        },
    );
    stale_effect.state = gwt::IssueMonitorEffectState::Attempting;
    let mut stale = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 7,
            pending_effects: vec![stale_effect],
            ..gwt::IssueMonitorPrefs::default()
        },
    );
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: false,
            effect_authority_epoch: 8,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed revoked latest disk authority");
    let mut remote_calls = 0;

    drive_local_issue_monitor_claim_effects_with(
        &prefs_path,
        &mut stale,
        |_effect, _authority_current, _now, _now_text| {
            remote_calls += 1;
            unreachable!("stale process-local Attempting must not cross the remote boundary")
        },
    )
    .expect("revoked disk state is a clean no-op");

    assert_eq!(remote_calls, 0);
    assert!(!stale.config.enabled);
    assert_eq!(stale.effect_authority_epoch(), 8);
    assert!(stale.pending_effects().is_empty());
}

#[test]
fn app_runtime_lifecycle_publish_failure_uses_latest_state_fallback_with_outbox_invariants() {
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
    let journal = gwt::PendingIssueMonitorEffect::prepared(
        "release:claim:99:7",
        7,
        gwt::IssueMonitorEffectPayload::ReleaseClaim {
            issue_number: 99,
            claim_id: "claim-99".to_string(),
            owner: "other/session".to_string(),
        },
    );
    let mut monitor = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            enabled: true,
            effect_authority_epoch: 7,
            pending_effects: vec![journal.clone()],
            ..gwt::IssueMonitorPrefs::default()
        },
    );
    monitor.terminal_queue_push(&[42], "operator", "2026-07-28T00:00:00Z");
    monitor.record_candidate(gwt::IssueMonitorIssue {
        author_login: None,
        number: 42,
        title: "Issue Monitor lifecycle fallback".to_string(),
        labels: vec!["bug".to_string()],
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(
        42,
        "claim-42",
        "host/session",
        "effect-42",
        "2026-07-28T00:00:00Z",
    ));
    assert!(monitor.claim_launch_delivery(
        42,
        "launch:effect-42",
        "gui-a",
        std::process::id(),
        "tab-1::agent-1",
        |_| false,
    ));
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed lifecycle prefs");
    let before = fs::read(&prefs_path).expect("read lifecycle prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_monitor_materializer_id = "gui-a".to_string();
    reset_local_issue_monitor_fallback_commit_count();

    let published = runtime.issue_monitor_launch_failed_result_events_with_delivery(
        Some(&repo),
        42,
        "launch failed",
        Some("launch:effect-42"),
        gwt_agent::SessionMode::Normal,
        Ok(()),
    );
    assert!(published.is_empty());
    assert_eq!(local_issue_monitor_fallback_commit_count(), 0);
    assert_eq!(
        fs::read(&prefs_path).expect("reload published prefs"),
        before
    );

    let fallback = runtime.issue_monitor_launch_failed_result_events_with_delivery(
        Some(&repo),
        42,
        "launch failed",
        Some("launch:effect-42"),
        gwt_agent::SessionMode::Normal,
        Err(
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                "daemon not running".to_string(),
            ),
        ),
    );
    assert!(fallback.iter().any(|event| matches!(
        event.event,
        BackendEvent::IssueMonitorLaunchFailed {
            issue_number: 42,
            ..
        }
    )));
    assert_eq!(
        local_issue_monitor_fallback_commit_count(),
        1,
        "publish failure performs exactly one latest-state fallback transaction",
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload fallback prefs");
    assert_eq!(persisted.effect_authority_epoch, 7);
    assert!(persisted
        .pending_effects
        .iter()
        .any(|effect| effect.effect_id == journal.effect_id));
    assert!(persisted.pending_launch_deliveries.is_empty());
    assert!(persisted
        .failed_issues
        .iter()
        .any(|failed| failed.issue_number == 42 && failed.message == "launch failed"));
}

#[test]
fn app_runtime_recovery_blocked_control_never_recovers_or_mutates_corrupt_prefs() {
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
    fs::create_dir_all(prefs_path.parent().expect("prefs parent")).expect("create prefs parent");
    let corrupt = b"{\"enabled\":true";
    fs::write(&prefs_path, corrupt).expect("seed corrupt prefs");
    let quarantine_count = || {
        fs::read_dir(prefs_path.parent().expect("prefs parent"))
            .expect("read prefs directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("issue-monitor.json.corrupt-")
            })
            .count()
    };
    let before_quarantines = quarantine_count();
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let events = runtime.issue_monitor_control_result_events(
        &runtime.test_context(),
        "client-1",
        Err(gwt::runtime_daemon_events::IssueMonitorControlPublishError::RecoveryBlocked),
        "enabled",
        |monitor| {
            let _ = monitor.set_enabled_with_effect_revocation(false);
        },
    );

    assert_eq!(fs::read(&prefs_path).expect("read corrupt prefs"), corrupt);
    assert_eq!(quarantine_count(), before_quarantines);
    assert!(!events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
    )));
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, message, .. }
            if level == "error"
                && message.contains(
                    gwt::runtime_daemon_events::ISSUE_MONITOR_CONTROL_RECOVERY_BLOCKED_ERROR
                )
    )));
}

#[cfg(unix)]
#[test]
fn app_runtime_issue_monitor_cache_only_control_bounds_origin_probe() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Issue #4045: this test bounds the origin probe, so pin the independent
    // prefs commit budget instead of depending on CI fsync completing in 250 ms.
    let _prefs_budget = super::super::ScopedLocalIssueMonitorPrefsTimeout::set(
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
    );
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
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");

    let fake_bin = temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("create fake bin");
    let fake_git = fake_bin.join("git");
    // Issue #3521: written by a child shell so no fork in a sibling test can
    // inherit a writable descriptor and turn the exec into ETXTBSY.
    gwt_core::test_support::write_executable_script(
        &fake_git,
        r#"#!/bin/sh
if [ "$1" = "rev-parse" ] && [ "$2" = "--path-format=absolute" ]; then
  printf '%s/.git\n' "$PWD"
  exit 0
fi
if [ "$1" = "remote" ] && [ "$2" = "get-url" ] && [ "$3" = "origin" ]; then
  while :; do sleep 1; done
fi
exit 1
"#,
    )
    .expect("write fake git");
    let _path = prepend_tool_parent_to_path(&fake_git);

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.local_issue_monitor_events_with_policy(
        &runtime.test_context(),
        Some("client-1"),
        super::super::IssueMonitorScanPolicy::CacheOnly,
        |monitor| {
            let _ = monitor.set_enabled_with_effect_revocation(true);
        },
    );

    // Issue #5299: the probe never completes on its own, so the deadline error
    // proves the budget was enforced without timing prefs persistence or
    // scheduler delays after the probe was stopped.
    let error = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => status.last_error.as_deref(),
            _ => None,
        })
        .expect("deadline error is operator-visible");
    assert!(
        error.contains(gwt_core::operation_deadline::DEADLINE_EXPIRED_MARKER),
        "unexpected error: {error}"
    );
}

#[test]
fn daemon_monitor_frames_update_each_owner_project() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let active_root = temp.path().join("active");
    let other_root = temp.path().join("other");
    fs::create_dir_all(&active_root).expect("active project");
    fs::create_dir_all(&other_root).expect("other project");
    let tabs = vec![
        sample_project_tab(
            "active",
            "Active",
            active_root.clone(),
            ProjectKind::Git,
            &[],
        ),
        sample_project_tab("other", "Other", other_root.clone(), ProjectKind::Git, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("active"));
    let mut active = test_monitor(gwt::IssueMonitorConfig::default());
    active.set_max_active_agents(4);
    let status = active.status_view();
    let events = runtime.issue_monitor_daemon_status_events(&active_root, Box::new(status.clone()));
    assert!(matches!(
        &events[0].event,
        BackendEvent::IssueMonitorStatus { status: actual } if **actual == status
    ));
    let other = test_monitor(gwt::IssueMonitorConfig::default());
    let other_key = runtime
        .project_context("other")
        .expect("other context")
        .project_key;
    let other_events =
        runtime.issue_monitor_daemon_status_events(&other_root, Box::new(other.status_view()));
    assert!(!other_events.is_empty());
    assert!(other_events
        .iter()
        .all(|event| matches!(&event.target, DispatchTarget::Project(key) if key == &other_key)));
    assert!(runtime
        .issue_monitor_daemon_inbox_events(&other_root, Vec::new())
        .iter()
        .any(
            |event| matches!(event.event, BackendEvent::IssueMonitorInbox { .. })
                && matches!(&event.target, DispatchTarget::Project(key) if key == &other_key)
        ));
    assert!(runtime
        .issue_monitor_daemon_inbox_events(&active_root, Vec::new())
        .iter()
        .any(|event| matches!(event.event, BackendEvent::IssueMonitorInbox { .. })));
}

#[test]
fn issue_5062_daemon_drain_status_includes_other_project_window() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let active_root = temp.path().join("gwt");
    let other_root = temp.path().join("llmlb");
    fs::create_dir_all(&active_root).expect("active project");
    fs::create_dir_all(&other_root).expect("other project");
    init_repo_with_initial_commit(&active_root);
    init_repo_with_initial_commit(&other_root);
    let mut other = sample_project_tab_with_window_at(
        "other",
        "agent-14",
        other_root,
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    other.title = "llmlb".to_string();
    assert!(other
        .workspace
        .set_purpose_title("agent-14", Some("SPEC #821 T012c".to_string())));
    let active = sample_project_tab("active", "gwt", active_root.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![active, other], Some("active"));
    runtime.rebuild_window_lookup();
    let mut monitor = test_monitor(gwt::IssueMonitorConfig::default());
    monitor.set_update_drain(
        gwt::IssueMonitorUpdateDrainReason::Auto,
        "9.110.0",
        "2026-10-10T08:21:17Z",
    );

    let events =
        runtime.issue_monitor_daemon_status_events(&active_root, Box::new(monitor.status_view()));
    let BackendEvent::IssueMonitorStatus { status } = &events[0].event else {
        panic!("status event")
    };
    let blockers = &status.update_drain.as_ref().expect("drain").blocking;
    assert_eq!(blockers, &runtime.update_drain_blockers(&monitor));
    assert_eq!(
        blockers.len(),
        1,
        "another project's lone blocker must not become zero"
    );
    let gwt::update_drain::UpdateBlocker::ActivePane { label, .. } = &blockers[0] else {
        panic!("pane blocker")
    };
    assert!(
        label.contains("llmlb") && label.contains("SPEC #821 T012c"),
        "{label}"
    );
}

#[test]
fn issue_5062_deferred_update_releases_drain_on_the_next_observation() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    let payload = temp.path().join("staged-gwt");
    fs::write(&payload, b"staged payload").unwrap();
    let pending_dir = gwt_core::update::pending_update_dir();
    fs::create_dir_all(&pending_dir).unwrap();
    fs::write(
        pending_dir.join("manifest.json"),
        serde_json::to_vec(&serde_json::json!({
            "version": "9.110.0", "asset_url": "https://example.invalid/update",
            "payload": { "PortableBinary": { "path": payload } },
            "downloaded_at": "2026-10-10T00:00:00Z", "auto_apply_deferred": true,
        }))
        .unwrap(),
    )
    .unwrap();
    let tab = sample_project_tab("active", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("active"));
    let mut monitor = test_monitor(gwt::IssueMonitorConfig::default());
    monitor.set_update_drain_with_origin(
        gwt::IssueMonitorUpdateDrainReason::Auto,
        "9.110.0",
        "2026-10-10T00:00:00Z",
        Some(true),
    );
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &monitor.prefs(),
    )
    .unwrap();
    let context = runtime.project_context("active").unwrap();
    let snapshot = runtime.update_quiescence_snapshot(&monitor);
    let events = runtime.update_drain_observed_events(
        chrono::Utc::now(),
        vec![UpdateDrainObservation {
            context,
            drain: Some((monitor.prefs(), monitor.update_drain().unwrap().clone())),
            snapshot,
        }],
    );
    let prefs =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo)).unwrap();
    assert!(
        prefs.update_drain.is_none(),
        "defer must release admission on the next evaluation"
    );
    assert!(payload.exists() && gwt_core::update::load_pending_update_manifest().is_some());
    assert!(!events.iter().any(|event| matches!(
        event.event,
        BackendEvent::UpdateAutoApply {
            phase: gwt::protocol::UpdateAutoApplyPhase::Applying,
            ..
        }
    )));
    assert!(
        runtime
            .update_drain_observed_events(
                chrono::Utc::now() + chrono::Duration::seconds(15),
                Vec::new(),
            )
            .is_empty(),
        "a deferred manifest with no matching drain must not emit another cancellation"
    );
}

#[test]
fn list_issue_monitor_slow_reader_runs_outside_the_dispatch() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("project");
    fs::create_dir_all(&root).unwrap();
    let tab = sample_project_tab("active", "Project", root, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("active"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let worker_reads = reads.clone();
    let mut watchdog = crate::dispatch_watchdog::DispatchWatchdog::start();
    let started = std::time::Instant::now();
    let guard = watchdog.enter("ListIssueMonitor");
    let events = runtime.list_issue_monitor_events_with_reader(
        &runtime.test_context(),
        "client-1",
        move |_| {
            worker_reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_secs(6));
            Ok(None)
        },
    );
    drop(guard);
    eprintln!(
        "Issue #5537 slow-reader dispatch_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "dispatch must enqueue without entering the reader"
    );
    assert!(events.is_empty());
    assert_eq!(tasks.lock().unwrap().len(), 1);
    // The worker can remain blocked beyond the watchdog threshold after dispatch returns.
    let task = tasks.lock().unwrap().pop().unwrap();
    task();
    drop(watchdog);
    let ledger = temp.path().join(".gwt/logs/errors");
    if ledger.exists() {
        for entry in fs::read_dir(ledger).unwrap() {
            let content = fs::read_to_string(entry.unwrap().path()).unwrap();
            assert!(!content.contains("gui_event_loop_stall"), "{content}");
        }
    }
}

#[test]
fn list_issue_monitor_discards_old_requests_and_hydrates_parallel_clients() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("project");
    fs::create_dir_all(&root).unwrap();
    let tab = sample_project_tab("active", "Project", root, ProjectKind::Git, &[]);
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("active"));
    runtime.persist().unwrap();
    assert!(runtime
        .persist_dispatcher
        .wait_idle(std::time::Duration::from_secs(30)));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let context = runtime.test_context();
    let read = |limit| {
        move |_: &Path| {
            let mut monitor = test_monitor(gwt::IssueMonitorConfig::default());
            monitor.set_max_active_agents(limit);
            Ok(Some(serde_json::to_value(monitor.agent_status()).unwrap()))
        }
    };
    runtime.list_issue_monitor_events_with_reader(&context, "client-a", read(1));
    runtime.list_issue_monitor_events_with_reader(&context, "client-a", read(5));
    runtime.list_issue_monitor_events_with_reader(&context, "client-b", read(2));
    let latest = tasks.lock().unwrap().remove(1);
    latest();
    let latest_events = finish_list_read(&mut runtime, &events);
    assert!(matches!(&latest_events[0].event,
        BackendEvent::IssueMonitorStatus { status } if status.max_active_agents == 5));
    let old = tasks.lock().unwrap().remove(0);
    old();
    assert!(finish_list_read(&mut runtime, &events).is_empty());
    let parallel = tasks.lock().unwrap().remove(0);
    parallel();
    let reply = finish_list_read(&mut runtime, &events);
    assert!(reply.iter().any(|event| matches!(&event.target,
        DispatchTarget::Client(id) if id == "client-b")
        && matches!(&event.event, BackendEvent::IssueMonitorStatus { status } if status.max_active_agents == 5)));
    assert!(
        tasks.lock().unwrap().is_empty(),
        "display updates must not trigger retry starvation"
    );
}

#[test]
fn list_issue_monitor_does_not_overwrite_a_newer_daemon_projection() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("project");
    fs::create_dir_all(&root).unwrap();
    let tab = sample_project_tab("active", "Project", root.clone(), ProjectKind::Git, &[]);
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("active"));
    runtime.persist().unwrap();
    assert!(runtime
        .persist_dispatcher
        .wait_idle(std::time::Duration::from_secs(30)));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let monitor = test_monitor(gwt::IssueMonitorConfig::default());
    let old_status = serde_json::to_value(monitor.agent_status()).unwrap();
    runtime.list_issue_monitor_events_with_reader(&runtime.test_context(), "client-1", move |_| {
        Ok(Some(old_status))
    });
    let mut newer = monitor.status_view();
    newer.max_active_agents = 7;
    runtime.issue_monitor_daemon_status_events(&root, Box::new(newer));
    runtime.issue_monitor_daemon_inbox_events(&root, Vec::new());
    let task = tasks.lock().unwrap().remove(0);
    task();
    let reply = finish_list_read(&mut runtime, &events);
    assert!(
        matches!(&reply[0].event, BackendEvent::IssueMonitorStatus { status } if status.max_active_agents == 7)
    );
    assert_eq!(
        runtime
            .project_state(&runtime.test_context())
            .unwrap()
            .issue_monitor_status_cache
            .borrow()
            .as_ref()
            .unwrap()
            .max_active_agents,
        7
    );
    assert!(
        tasks.lock().unwrap().is_empty(),
        "a newer display must hydrate without another read"
    );
    assert!(reply.iter().any(|event| matches!(&event.event, BackendEvent::IssueMonitorInbox { items } if items.is_empty())));
    // An acknowledged control can change prefs before any new display arrives.
    runtime
        .list_issue_monitor_events_with_reader(&runtime.test_context(), "client-1", |_| Ok(None));
    runtime.invalidate_issue_monitor_list(&root);
    let task = tasks.lock().unwrap().remove(0);
    task();
    assert!(finish_list_read(&mut runtime, &events).is_empty());
    let retry = tasks.lock().unwrap().remove(0);
    retry();
    assert!(finish_list_read(&mut runtime, &events)
        .iter()
        .any(|event| matches!(event.event, BackendEvent::IssueMonitorStatus { .. })));
}

fn finish_list_read(
    runtime: &mut AppRuntime,
    events: &Arc<Mutex<Vec<UserEvent>>>,
) -> Vec<OutboundEvent> {
    let event = events.lock().unwrap().remove(0);
    let Some(UserEvent::IssueMonitorListPrepared(prepared)) =
        runtime.accept_project_completion(event)
    else {
        panic!("List completion expected");
    };
    runtime.issue_monitor_list_prepared_events(*prepared)
}

#[test]
fn list_issue_monitor_uses_the_daemon_gui_projection_without_local_scan() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    // No Git remote: even the old local scan cannot access the network.
    let root = temp.path().join("project");
    fs::create_dir_all(&root).expect("project");
    let tab = sample_project_tab("active", "Project", root.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("active"));
    let mut monitor = test_monitor(gwt::IssueMonitorConfig::default());
    monitor.set_max_active_agents(5);
    monitor.record_scan_error("2026-09-12T00:00:00Z", "daemon scan failed");
    let agent = serde_json::to_value(monitor.agent_status()).expect("agent status");
    let expected = agent["gui_status"].clone();
    reset_local_issue_monitor_remote_scan_count();

    let events = runtime.completed_issue_monitor_list_for_test("client-1", move |project_root| {
        assert_eq!(project_root, root);
        Ok(Some(agent))
    });

    assert_eq!(local_issue_monitor_remote_scan_count(), 0);
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0].target, DispatchTarget::Client(id) if id == "client-1"));
    let BackendEvent::IssueMonitorStatus { status } = &events[0].event else {
        panic!("daemon status reply expected");
    };
    assert_eq!(serde_json::to_value(status).expect("GUI status"), expected);
}

#[test]
fn list_issue_monitor_uncertain_and_legacy_daemon_reads_preserve_display() {
    use gwt::runtime_daemon_events::IssueMonitorControlPublishError;

    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("project");
    fs::create_dir_all(&root).expect("project");
    let tab = sample_project_tab("active", "Project", root, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("active"));
    let mut legacy =
        serde_json::to_value(test_monitor(gwt::IssueMonitorConfig::default()).agent_status())
            .expect("legacy status");
    legacy
        .as_object_mut()
        .expect("status object")
        .remove("gui_status");
    reset_local_issue_monitor_remote_scan_count();

    for (result, expects_toast) in [
        (
            Err(IssueMonitorControlPublishError::OutcomeUnknown(
                "status timed out".into(),
            )),
            true,
        ),
        (Ok(Some(serde_json::json!({"gui_status": "invalid"}))), true),
        (Ok(Some(legacy)), false),
    ] {
        let events = runtime.completed_issue_monitor_list_for_test("client-1", move |_| result);
        assert!(events
            .iter()
            .all(|event| matches!(event.event, BackendEvent::IssueMonitorToast { .. })));
        assert_eq!(events.len(), usize::from(expects_toast));
    }
    assert_eq!(local_issue_monitor_remote_scan_count(), 0);
}

#[test]
fn list_issue_monitor_without_daemon_never_scans_remote_for_cold_or_stale_cache() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            terminal_queue_auto_refill: true,
            terminal_queue_auto_refill_limit: 1,
            ..Default::default()
        },
    )
    .expect("enable cached candidate admission");
    let tab = sample_project_tab("active", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("active"));
    for stale in [false, true] {
        if stale {
            Cache::new(issue_cache_root(&repo))
                .write_snapshot(&sample_issue_snapshot(
                    43,
                    "Cached issue",
                    &["bug"],
                    "Body",
                    "2026-07-21T00:00:00Z",
                ))
                .expect("stale cache");
        }
        reset_local_issue_monitor_remote_scan_count();
        let events = runtime.completed_issue_monitor_list_for_test("client-1", |_| Ok(None));
        assert_eq!(
            local_issue_monitor_remote_scan_count(),
            0,
            "List must remain local even when stale={stale}"
        );
        assert!(events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::IssueMonitorStatus { .. })));
        let items = events
            .iter()
            .find_map(|event| match &event.event {
                BackendEvent::IssueMonitorInbox { items } => Some(items),
                _ => None,
            })
            .expect("local inbox");
        assert_eq!(items.len(), usize::from(stale));
    }
}

#[test]
fn issue_monitor_control_error_targets_its_owner_and_drops_ownerless_notifications() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let repo_a = temp.path().join("a");
    let repo_b = temp.path().join("b");
    fs::create_dir_all(&repo_a).unwrap();
    fs::create_dir_all(&repo_b).unwrap();
    let runtime = sample_runtime(
        temp.path(),
        vec![
            sample_project_tab("a", "A", repo_a, ProjectKind::Git, &[]),
            sample_project_tab("b", "B", repo_b.clone(), ProjectKind::Git, &[]),
        ],
        Some("a"),
    );
    let error = || {
        gwt::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(
            "timed out".into(),
        )
    };
    let events = runtime.issue_monitor_control_error_events(
        Some(&repo_b),
        None,
        error(),
        "launch",
        Some(42),
    );
    assert!(matches!(&events[..], [OutboundEvent {
        target: DispatchTarget::Project(key),
        event: BackendEvent::IssueMonitorToast { issue_number: Some(42), .. },
        ..
    }] if Some(key) == runtime.project_key_for_tab("b")));
    assert!(runtime
        .issue_monitor_control_error_events(None, None, error(), "launch", None)
        .is_empty());
}

#[test]
fn issue_monitor_control_error_preserves_the_authoritative_display() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let runtime = sample_runtime(temp.path(), Vec::new(), None);
    let events = runtime.issue_monitor_control_error_events(
        None,
        Some("client-1"),
        gwt::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(
            "control timed out".to_string(),
        ),
        "max-active",
        None,
    );
    assert!(
        events.iter().all(|event| !matches!(
            event.event,
            BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
        )),
        "a control error must not replace live counters with an empty monitor"
    );
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, message, .. }
            if level == "error" && message.contains("control timed out")
    )));
}

#[test]
fn app_runtime_failed_control_commit_never_renders_volatile_kill_switch_state() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            effect_authority_epoch: 7,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(prefs_path.with_extension("lock"))
        .expect("open prefs lock");
    lock.lock_exclusive().expect("hold prefs lock");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_monitor_fallback_commit_timeout =
        super::super::ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT;

    let started = Instant::now();
    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorAutonomousMode { enabled: false },
    );
    FileExt::unlock(&lock).expect("release prefs lock");

    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        events
            .iter()
            .all(|event| !matches!(event.event, BackendEvent::IssueMonitorStatus { .. })),
        "failed transaction must preserve the last authoritative display"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert!(persisted.autonomous_mode);
    assert_eq!(persisted.effect_authority_epoch, 7);
}

/// Issue #3906 AC-3: a staged update raises the `Auto` drain for the staged
/// version when the monitor is unattended and auto-apply is on (the default
/// while autonomous); an explicit `auto_apply_updates:false` keeps the manual
/// button path and raises nothing. Launch ledgers are untouched (#4037).
#[test]
fn app_runtime_staged_update_respects_global_off_over_legacy_project_on() {
    let temp = tempdir().unwrap();
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            autonomous_mode: true,
            ..Default::default()
        },
    )
    .unwrap();
    let legacy_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&legacy_path).unwrap()).unwrap();
    legacy["auto_apply_updates"] = serde_json::json!(true);
    fs::write(&legacy_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let config = gwt_core::paths::gwt_config_path();
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(config, "auto_apply_updates = false\n").unwrap();
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.update_download_in_flight =
        Some(crate::app_runtime::UPDATE_AUTO_APPLY_CLIENT_ID.to_string());
    runtime.update_staged_events_with("9.999.0", None);
    let prefs =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo)).unwrap();
    assert!(
        prefs.update_drain.is_none(),
        "global OFF wins over legacy project ON"
    );
}

#[test]
fn app_runtime_staged_update_raises_auto_drain_only_when_auto_apply_is_effective() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let seed = || {
        gwt::save_issue_monitor_prefs(
            &prefs_path,
            &gwt::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                effect_authority_epoch: 7,
                launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "tab-1::agent-1".to_string(),
                }],
                ..gwt::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
    };

    seed();
    gwt::update_settings::set(Some(false)).unwrap();
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    assert!(runtime.update_staged_events("99.0.0").is_empty());
    assert!(gwt::load_issue_monitor_prefs(&prefs_path)
        .expect("reload")
        .update_drain
        .is_none());

    seed();
    gwt::update_settings::set(Some(true)).unwrap();
    let events = runtime.update_staged_events("99.0.0");
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("drain status broadcast");
    let drain = status.update_drain.as_ref().expect("drain raised");
    assert_eq!(drain.reason, gwt::IssueMonitorUpdateDrainReason::Auto);
    assert_eq!(drain.version, "99.0.0");
    assert!(
        drain.blocking.is_empty(),
        "no agent pane, claim, execution or lease"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    let persisted_drain = persisted.update_drain.expect("drain persisted");
    assert_eq!(persisted_drain.version, "99.0.0");
    assert!(persisted.autonomous_mode);
    assert_eq!(
        persisted.effect_authority_epoch, 7,
        "the drain revokes nothing"
    );
    assert_eq!(
        persisted.launched_issues.len(),
        1,
        "launch ledger untouched"
    );
}

/// Issue #3906 AC-8 / AC-12: the status view names the blockers this
/// process observes — a Running agent pane and a pending `AcquireClaim` —
/// and an Idle pane is not one of them.
#[test]
fn app_runtime_update_drain_blocking_lists_running_agent_panes_and_pending_claims() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
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
    runtime.rebuild_window_lookup();

    let mut monitor = test_monitor(gwt::IssueMonitorConfig::default());
    monitor.terminal_queue_push(&[42], "operator", "2026-07-28T00:00:00Z");
    monitor
        .prepare_pending_effect(
            "claim-effect-42",
            gwt::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "claim-42".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-09-06T01:00:00Z".to_string(),
                expires_at: "2026-09-06T01:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        )
        .expect("prepare claim");
    monitor.set_update_drain(
        gwt::IssueMonitorUpdateDrainReason::Auto,
        "99.0.0",
        "2026-09-06T01:00:00Z",
    );

    // An Agent pane with a live PTY but no hook state yet composes to
    // `Starting` (no agent session bound), which blocks just like Running.
    let blocking = runtime.update_drain_blockers(&monitor);
    assert_eq!(
        blocking,
        vec![
            gwt::update_drain::UpdateBlocker::ActivePane {
                window_id: "tab-1::agent-1".to_string(),
                label: "Repo: Sample".to_string(),
                state: WindowProcessStatus::Starting,
            },
            gwt::update_drain::UpdateBlocker::PendingAcquireClaim { issue_number: 42 },
        ]
    );

    // A hook-reported Idle pane no longer blocks; the claim still does.
    runtime
        .window_hook_states
        .insert("tab-1::agent-1".to_string(), WindowProcessStatus::Idle);
    assert_eq!(
        runtime.update_drain_blockers(&monitor),
        vec![gwt::update_drain::UpdateBlocker::PendingAcquireClaim { issue_number: 42 }]
    );
}

/// Issue #4076 AC-2 (the 2026-09-07 01:52Z incident): staging an update in
/// autonomous mode raises the drain and records it, and sends nothing that
/// could restart gwt — no `ApplyUpdateRestartNow`, no `ApplyUpdateGraceful`,
/// no `ApplyUpdateDrained`. The only automatic route to a restart is the
/// drain tick below.
#[test]
fn app_runtime_staged_update_never_requests_a_restart_by_itself() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, user_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.rebuild_window_lookup();

    let events = runtime.update_staged_events_with("99.0.0", None);
    let log = fs::read_to_string(gwt_core::update::update_log_path()).unwrap_or_default();
    assert!(
        log.lines().any(|line| {
            let entry: serde_json::Value = serde_json::from_str(line).expect("update log JSON");
            entry["stage"] == "drain_started"
                && entry["version"] == "99.0.0"
                && serde_json::from_str::<serde_json::Value>(
                    entry["blockers"].as_str().unwrap_or("null"),
                )
                .expect("blocker JSON")[0]["window_id"]
                    == "tab-1::agent-1"
        }),
        "the update log records the staged version entering drain: {log}"
    );
    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::IssueMonitorStatus { status } if status.update_drain.is_some()
        )),
        "the drain is raised"
    );
    let toasts = update_resume_toasts(&events);
    assert_eq!(toasts.len(), 1, "drain start is recorded once: {toasts:?}");
    assert!(
        toasts[0].1.contains("99.0.0") && toasts[0].1.contains("draining"),
        "notice names the version and the drain: {}",
        toasts[0].1
    );
    assert!(
        user_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty(),
        "staging sends no event-loop request that could restart gwt"
    );
}

/// Issue #5062 AC-2: once a staged update raised the drain, every drain tick
/// past `next_evaluation_at` re-evaluates it and moves the persisted
/// observation forward, even when the blockers are unchanged.
#[test]
fn app_runtime_drain_tick_reevaluates_after_the_staged_update_raised_the_drain() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, user_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.rebuild_window_lookup();
    runtime.update_staged_events_with("99.0.0", None);
    let raised = gwt_core::update::load_update_wait_observation(&repo).expect("observation");
    assert_eq!(raised.reason, "waiting for drain evaluation");
    let next = chrono::DateTime::parse_from_rfc3339(
        raised
            .next_evaluation_at
            .as_deref()
            .expect("next evaluation"),
    )
    .expect("next evaluation time")
    .with_timezone(&chrono::Utc);

    for tick in [next, next + chrono::Duration::seconds(15)] {
        runtime.update_drain_tick_events_at(tick);
        drain_queued_blocking_tasks(&tasks);
        let queued = std::mem::take(&mut *user_events.lock().unwrap());
        for event in queued {
            if let UserEvent::UpdateDrainObserved { now, observations } = event {
                runtime.update_drain_observed_events(now, observations);
            }
        }
        drain_queued_blocking_tasks(&tasks);
        let wait = gwt_core::update::load_update_wait_observation(&repo).expect("observation");
        assert_eq!(wait.stage, "pending_waiting");
        assert_eq!(wait.observed_at, tick.to_rfc3339(), "{wait:?}");
        assert!(wait.reason.contains("tab-1::agent-1"), "{wait:?}");
        assert_eq!(
            wait.next_evaluation_at,
            Some((tick + chrono::Duration::seconds(15)).to_rfc3339())
        );
    }
}

/// Issue #4376 AC-1 / AC-2 / AC-7: a manual Update click (attended monitor,
/// no auto-apply) whose download lands while an agent pane is Running joins
/// the `Auto` drain instead of offering an immediate restart: the hold is
/// raised through the same control (#4037, launches held, ledgers and the
/// monitor settings untouched), nothing is sent that could restart gwt, and
/// the drain start is recorded with the auto path's notice. A quiet host
/// keeps the ready modal's Restart now, exactly as before.
#[test]
fn app_runtime_manual_update_click_with_running_agent_enters_drain_instead_of_applying() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let seed = || {
        gwt::save_issue_monitor_prefs(
            &prefs_path,
            &gwt::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: false,
                effect_authority_epoch: 7,
                launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "tab-1::agent-1".to_string(),
                }],
                ..gwt::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
    };

    // Running agent pane: the click waits.
    seed();
    let payload = temp.path().join("manual-update");
    fs::write(&payload, "payload").unwrap();
    gwt_core::update::persist_pending_update_manifest(&gwt_core::update::PendingUpdateManifest {
        version: "9.99.0".into(),
        asset_url: "https://example.invalid/update".into(),
        payload: gwt_core::update::PreparedPayload::PortableBinary { path: payload },
        downloaded_at: chrono::Utc::now().to_rfc3339(),
        origin: gwt_core::update::PendingUpdateOrigin::ManualLater,
        auto_apply_deferred: false,
    })
    .unwrap();
    gwt::update_settings::set(Some(false)).unwrap();
    assert!(
        !gwt_core::update::load_pending_update_manifest()
            .unwrap()
            .auto_apply_deferred,
        "global OFF preserves the operator defer flag"
    );
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, user_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.rebuild_window_lookup();
    runtime.update_download_in_flight = Some("manual-client".into());
    let events = runtime.update_staged_events_with("99.0.0", None);
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("drain status broadcast: {events:?}");
    let drain = status.update_drain.as_ref().expect("drain raised");
    assert_eq!(drain.reason, gwt::IssueMonitorUpdateDrainReason::Auto);
    assert_eq!(drain.automatic, Some(false));
    assert_eq!(drain.version, "99.0.0");
    assert_eq!(
        drain.blocking,
        vec![gwt::update_drain::UpdateBlocker::ActivePane {
            window_id: "tab-1::agent-1".to_string(),
            label: "Repo: Sample".to_string(),
            state: WindowProcessStatus::Starting,
        }],
        "the status names what the click is waiting for"
    );
    let toasts = update_resume_toasts(&events);
    assert_eq!(toasts.len(), 1, "drain start recorded once: {toasts:?}");
    assert!(
        toasts[0].1.contains("99.0.0") && toasts[0].1.contains("draining"),
        "the auto path's notice is reused: {}",
        toasts[0].1
    );
    assert!(
        user_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty(),
        "a manual click with a running agent never requests a restart"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(
        persisted
            .update_drain
            .as_ref()
            .map(|drain| drain.version.as_str()),
        Some("99.0.0"),
        "the hold is persisted for the Issue Monitor"
    );
    assert!(persisted.enabled, "the drain is a hold, not enabled:false");
    assert!(
        !persisted.autonomous_mode,
        "the attended setting is left alone"
    );
    assert_eq!(
        persisted.effect_authority_epoch, 7,
        "the drain revokes nothing"
    );
    assert_eq!(
        persisted.launched_issues.len(),
        1,
        "launch ledger untouched"
    );

    // Quiet host: no drain, the ready modal's Restart now applies as before.
    seed();
    let quiet_tab = sample_project_tab("tab-2", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut quiet = sample_runtime(temp.path(), vec![quiet_tab], Some("tab-2"));
    assert!(
        quiet.update_staged_events_with("99.0.0", None).is_empty(),
        "a quiet host keeps the manual Restart now path"
    );
    assert!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("reload prefs")
            .update_drain
            .is_none(),
        "no hold is raised when nothing is running"
    );
}

/// Issue #4376 AC-3 / AC-7: the manual-click drain is applied by the same
/// tick as the auto path — a legacy manual-click drain with an autonomous
/// monitor and explicit project auto-apply OFF, the
/// Running pane blocks, the pane going Idle settles over two ticks, the grace
/// is announced, and the apply goes through `ApplyUpdateDrained`, never
/// `ApplyUpdateRestartNow`. Agents are never stopped.
#[test]
fn app_runtime_manual_drain_applies_gracefully_once_quiescent() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let since = chrono::DateTime::parse_from_rfc3339("2026-09-15T00:00:00Z")
        .expect("since")
        .with_timezone(&chrono::Utc);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            update_drain: Some(gwt::IssueMonitorUpdateDrain {
                version: "99.0.0".to_string(),
                since: since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                reason: gwt::IssueMonitorUpdateDrainReason::Auto,
                automatic: None,
                blocking: Vec::new(),
            }),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&prefs_path).unwrap()).unwrap();
    legacy["auto_apply_updates"] = serde_json::json!(false);
    fs::write(&prefs_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let payload = temp.path().join("manual-update");
    fs::write(&payload, "payload").unwrap();
    gwt_core::update::persist_pending_update_manifest(&gwt_core::update::PendingUpdateManifest {
        version: "9.99.0".into(),
        asset_url: "https://example.invalid/update".into(),
        payload: gwt_core::update::PreparedPayload::PortableBinary { path: payload },
        downloaded_at: chrono::Utc::now().to_rfc3339(),
        origin: gwt_core::update::PendingUpdateOrigin::ManualLater,
        auto_apply_deferred: false,
    })
    .unwrap();
    gwt::update_settings::set(Some(false)).unwrap();
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, user_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.rebuild_window_lookup();
    let drained_events = |user_events: &Arc<Mutex<Vec<UserEvent>>>| {
        user_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::ApplyUpdateDrained {
                        automatic: false,
                        ..
                    }
                )
            })
            .count()
    };
    let at = |secs: i64| since + chrono::Duration::seconds(secs);

    assert!(runtime
        .update_drain_tick_events_and_drain_at(at(15))
        .is_empty());
    let early_log = fs::read_to_string(gwt_core::update::update_log_path()).unwrap_or_default();
    assert!(
        early_log.lines().any(|line| {
            let entry: serde_json::Value = serde_json::from_str(line).unwrap();
            entry["stage"] == "pending_waiting"
                && entry["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("agent-1"))
                && entry["next_evaluation_at"] == at(30).to_rfc3339()
        }),
        "waiting must be observable before the warning cadence: {early_log}"
    );
    assert!(runtime
        .update_drain_tick_events_and_drain_at(at(30))
        .is_empty());
    assert_eq!(drained_events(&user_events), 0, "a Running pane blocks");
    assert_eq!(
        runtime
            .window_status("tab-1::agent-1")
            .unwrap_or(WindowProcessStatus::Idle),
        WindowProcessStatus::Starting,
        "the agent pane is never stopped by the drain"
    );

    runtime
        .window_hook_states
        .insert("tab-1::agent-1".to_string(), WindowProcessStatus::Idle);
    assert!(runtime
        .update_drain_tick_events_and_drain_at(at(45))
        .is_empty());
    let scheduled = runtime.update_drain_tick_events_and_drain_at(at(60));
    assert!(
        scheduled.iter().any(|event| matches!(
            &event.event,
            BackendEvent::UpdateAutoApply {
                version,
                phase: gwt::protocol::UpdateAutoApplyPhase::Scheduled,
                grace_secs: Some(60),
            } if version == "99.0.0"
        )),
        "the grace is announced to the CTA: {scheduled:?}"
    );
    assert_eq!(
        drained_events(&user_events),
        0,
        "nothing applies inside the grace"
    );
    let applying = runtime.update_drain_tick_events_and_drain_at(at(120));
    assert!(
        applying.iter().any(|event| matches!(
            &event.event,
            BackendEvent::UpdateAutoApply {
                phase: gwt::protocol::UpdateAutoApplyPhase::Applying,
                ..
            }
        )),
        "the apply is announced: {applying:?}"
    );
    assert_eq!(
        drained_events(&user_events),
        1,
        "exactly one graceful apply"
    );
    assert!(
        user_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .all(|event| !matches!(
                recorded_project_payload(event),
                UserEvent::ApplyUpdateRestartNow { .. }
            )),
        "the drained manual click never uses the Restart-now route"
    );
}

/// Issue #4376 AC-4 / AC-7: after the restart that applied a manually
/// requested update, the Issue Monitor setting from before the drain is in
/// effect again. The drain is a #4037 admission hold layered over the
/// setting — `enabled` is never flipped — so the resume marker records the
/// raised hold per project and the settling bootstrap releases exactly that:
/// a monitor that was enabled comes back enabled, one that was disabled stays
/// disabled, and the drained launches stay attributable.
#[test]
fn app_runtime_restart_after_manual_drain_restores_pre_drain_monitor_setting() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for enabled_before_drain in [true, false] {
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        gwt::machine_agent_settings::set(
            Some(2),
            gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
            "2026-10-22T00:00:00Z",
        )
        .expect("seed machine Manual capacity");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo_with_initial_commit(&repo);
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
        gwt::save_issue_monitor_prefs(
            &prefs_path,
            &gwt::IssueMonitorPrefs {
                max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
                enabled: enabled_before_drain,
                autonomous_mode: false,
                max_active_agents: 2,
                update_drain: Some(gwt::IssueMonitorUpdateDrain {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    since: "2026-09-15T00:00:00Z".to_string(),
                    reason: gwt::IssueMonitorUpdateDrainReason::Auto,
                    automatic: None,
                    blocking: Vec::new(),
                }),
                launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                    issue_number: 4376,
                    window_id: "tab-1::agent-4376".to_string(),
                }],
                ..gwt::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let marker = update_resume_marker_for(&repo, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            runtime.update_resume_projects(),
            marker.projects,
            "the marker written at apply time records the raised hold"
        );
        gwt_core::update::persist_update_resume_marker(&marker).expect("persist marker");

        runtime.bootstrap();

        let restored = gwt::load_issue_monitor_prefs(&prefs_path).expect("prefs after restart");
        assert!(
            restored.update_drain.is_none(),
            "the hold raised by the manual click is released"
        );
        assert_eq!(
            restored.enabled, enabled_before_drain,
            "the pre-drain setting is in effect again (enabled_before_drain={enabled_before_drain})"
        );
        assert!(!restored.autonomous_mode, "attended mode is not promoted");
        assert_eq!(restored.max_active_agents, 2);
        assert_eq!(
            restored
                .launched_issues
                .iter()
                .map(|launch| launch.issue_number)
                .collect::<Vec<_>>(),
            vec![4376],
            "the drained launch stays attributable"
        );
        assert!(
            gwt_core::update::load_update_resume_marker().is_none(),
            "the marker is consumed"
        );
    }
}

/// Issue #4076 AC-2 / AC-5 (#3906 AC-2 / AC-7 / AC-8): the drain tick applies
/// only after two consecutive quiet ticks plus the 60 s cancel grace, through
/// `ApplyUpdateDrained` (which main.rs routes into `ApplyUpdateGraceful`); a
/// Running agent pane keeps it from firing and a long drain is recorded with
/// its blockers without stopping anything.
#[test]
fn update_auto_apply_keeps_project_planners_and_releases_all_matching_drains() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let since = chrono::DateTime::parse_from_rfc3339("2026-09-07T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let repos = [temp.path().join("repo-a"), temp.path().join("repo-b")];
    let tabs = repos
        .iter()
        .enumerate()
        .map(|(index, repo)| {
            fs::create_dir_all(repo).unwrap();
            init_repo_with_initial_commit(repo);
            gwt::save_issue_monitor_prefs(
                &gwt::issue_monitor_prefs_path_for_repo_path(repo),
                &gwt::IssueMonitorPrefs {
                    enabled: true,
                    autonomous_mode: true,
                    update_drain: Some(gwt::IssueMonitorUpdateDrain {
                        version: "99.0.0".into(),
                        since: since.to_rfc3339(),
                        reason: gwt::IssueMonitorUpdateDrainReason::Auto,
                        automatic: None,
                        blocking: Vec::new(),
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
            sample_project_tab(
                &format!("tab-{index}"),
                "Repo",
                repo.clone(),
                ProjectKind::Git,
                &[],
            )
        })
        .collect();
    let (mut runtime, user_events) = sample_runtime_with_events(temp.path(), tabs, Some("tab-0"));
    let at = |secs| since + chrono::Duration::seconds(secs);
    assert!(runtime
        .update_drain_tick_events_and_drain_at(at(15))
        .is_empty());
    runtime.active_tab_id = Some("tab-1".into());
    let scheduled = runtime.update_drain_tick_events_and_drain_at(at(30));
    assert_eq!(
        scheduled
            .iter()
            .filter(|event| matches!(
                event.event,
                BackendEvent::UpdateAutoApply {
                    phase: gwt::protocol::UpdateAutoApplyPhase::Scheduled,
                    ..
                }
            ))
            .count(),
        1,
        "one host announcement for both project planners"
    );
    let context_a = runtime.project_context("tab-0").unwrap();
    let context_b = runtime.project_context("tab-1").unwrap();
    assert_eq!(
        runtime.project_state(&context_a).unwrap().update_auto_apply,
        runtime.project_state(&context_b).unwrap().update_auto_apply,
    );
    assert_ne!(
        runtime.project_state(&context_a).unwrap().update_auto_apply,
        gwt::update_drain::UpdateAutoApplyPlanner::default(),
        "both project planners must retain the quiescence streak",
    );
    runtime.update_drain_tick_events_and_drain_at(at(90));
    assert_eq!(
        user_events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(
                recorded_project_payload(event),
                UserEvent::ApplyUpdateDrained { .. }
            ))
            .count(),
        1,
        "one host apply for the staged version"
    );
    runtime.cancel_update_auto_apply_events();
    for repo in repos {
        assert!(
            gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo),)
                .unwrap()
                .update_drain
                .is_none(),
            "cancel must release every matching project"
        );
    }
}

#[test]
fn app_runtime_update_drain_tick_applies_after_quiescence_and_grace() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let since = chrono::DateTime::parse_from_rfc3339("2026-09-07T00:00:00Z")
        .expect("since")
        .with_timezone(&chrono::Utc);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            update_drain: Some(gwt::IssueMonitorUpdateDrain {
                version: "99.0.0".to_string(),
                since: since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                reason: gwt::IssueMonitorUpdateDrainReason::Auto,
                automatic: None,
                blocking: Vec::new(),
            }),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let (mut runtime, user_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.rebuild_window_lookup();
    let drained_events = |user_events: &Arc<Mutex<Vec<UserEvent>>>| {
        user_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::ApplyUpdateDrained { .. }
                )
            })
            .count()
    };
    let at = |secs: i64| since + chrono::Duration::seconds(secs);

    // Blocked by the Running pane: quiet ticks, nothing sent, until the
    // AC-9 notice cadence (1800 s) is reached — then the blockers are named.
    assert!(runtime
        .update_drain_tick_events_and_drain_at(at(15))
        .is_empty());
    assert!(runtime
        .update_drain_tick_events_and_drain_at(at(30))
        .is_empty());
    let notice = runtime.update_drain_tick_events_and_drain_at(at(1800));
    let toasts = update_resume_toasts(&notice);
    assert_eq!(toasts.len(), 1, "long-drain notice: {toasts:?}");
    assert_eq!(toasts[0].0, "warn");
    assert!(
        toasts[0].1.contains("tab-1::agent-1")
            && toasts[0].1.contains("new launches are held")
            && toasts[0].1.contains("finish"),
        "the bounded notice identifies the blocker, launch hold, and safe next action: {}",
        toasts[0].1
    );
    let log_path = gwt_core::update::update_log_path();
    let log = fs::read_to_string(&log_path).unwrap_or_default();
    let entry = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("update log JSON"))
        .find(|entry| entry["stage"] == "drain_blocked")
        .expect("the bounded warning records drain blockers in the update log");
    assert_eq!(entry["version"], "99.0.0");
    let blockers: serde_json::Value =
        serde_json::from_str(entry["blockers"].as_str().expect("serialized blockers"))
            .expect("blocker JSON");
    assert_eq!(blockers[0]["window_id"], "tab-1::agent-1");
    assert!(runtime
        .update_drain_tick_events_and_drain_at(at(1801))
        .is_empty());
    assert_eq!(
        fs::read_to_string(&log_path).expect("update log"),
        log,
        "unchanged blockers must not emit a log on every tick"
    );
    assert!(
        toasts[0].1.contains("99.0.0") && toasts[0].1.contains("Sample"),
        "the notice names the blocking pane: {}",
        toasts[0].1
    );
    assert_eq!(drained_events(&user_events), 0);

    // The pane goes Idle: the first quiet tick settles, the second schedules
    // the apply with the cancel grace, and the apply fires once it elapsed.
    runtime
        .window_hook_states
        .insert("tab-1::agent-1".to_string(), WindowProcessStatus::Idle);
    assert!(runtime
        .update_drain_tick_events_and_drain_at(at(1815))
        .is_empty());
    let scheduled = runtime.update_drain_tick_events_and_drain_at(at(1830));
    assert!(
        scheduled.iter().any(|event| matches!(
            &event.event,
            BackendEvent::UpdateAutoApply {
                version,
                phase: gwt::protocol::UpdateAutoApplyPhase::Scheduled,
                grace_secs: Some(60),
            } if version == "99.0.0"
        )),
        "the grace is announced to the CTA: {scheduled:?}"
    );
    assert_eq!(
        update_resume_toasts(&scheduled).len(),
        1,
        "the grace is recorded"
    );
    assert_eq!(
        drained_events(&user_events),
        0,
        "nothing applies inside the grace"
    );
    assert!(runtime
        .update_drain_tick_events_and_drain_at(at(1845))
        .is_empty());
    assert_eq!(drained_events(&user_events), 0);
    let applying = runtime.update_drain_tick_events_and_drain_at(at(1890));
    assert!(
        applying.iter().any(|event| matches!(
            &event.event,
            BackendEvent::UpdateAutoApply {
                phase: gwt::protocol::UpdateAutoApplyPhase::Applying,
                ..
            }
        )),
        "the apply is announced: {applying:?}"
    );
    assert_eq!(
        drained_events(&user_events),
        1,
        "exactly one graceful apply request"
    );
    assert!(
        user_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .all(|event| !matches!(
                recorded_project_payload(event),
                UserEvent::ApplyUpdateRestartNow { .. }
            )),
        "the automatic path never uses the Restart-now route"
    );
}

/// Issue #4076 AC-2 / AC-5 (#3906 AC-7 / AC-13): cancelling inside the grace
/// releases the drain (the update stays staged for the manual button) and the
/// tick never reschedules that version.
#[test]
fn app_runtime_cancel_update_auto_apply_releases_the_drain_and_stops_the_tick() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let since = chrono::DateTime::parse_from_rfc3339("2026-09-07T00:00:00Z")
        .expect("since")
        .with_timezone(&chrono::Utc);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            update_drain: Some(gwt::IssueMonitorUpdateDrain {
                version: "99.0.0".to_string(),
                since: since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                reason: gwt::IssueMonitorUpdateDrainReason::Auto,
                automatic: None,
                blocking: Vec::new(),
            }),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, user_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let at = |secs: i64| since + chrono::Duration::seconds(secs);
    runtime.update_drain_tick_events_and_drain_at(at(15));
    let scheduled = runtime.update_drain_tick_events_and_drain_at(at(30));
    assert!(scheduled.iter().any(|event| matches!(
        &event.event,
        BackendEvent::UpdateAutoApply {
            phase: gwt::protocol::UpdateAutoApplyPhase::Scheduled,
            ..
        }
    )));

    let events =
        runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::CancelUpdateAutoApply);
    assert!(
        events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::UpdateAutoApply {
                version,
                phase: gwt::protocol::UpdateAutoApplyPhase::Cancelled,
                ..
            } if version == "99.0.0"
        )),
        "the CTA learns about the cancellation: {events:?}"
    );
    assert!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("reload")
            .update_drain
            .is_none(),
        "the hold is released so launches resume"
    );
    let toasts = update_resume_toasts(&events);
    assert_eq!(toasts.len(), 1, "the cancellation is recorded: {toasts:?}");
    assert!(toasts[0].1.contains("cancel"), "{}", toasts[0].1);
    for secs in [45, 60, 120, 600] {
        assert!(runtime
            .update_drain_tick_events_and_drain_at(at(secs))
            .is_empty());
    }
    assert!(
        user_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty(),
        "no apply request after a cancel"
    );
}

/// Issue #4076 AC-4 (#3906 AC-5 / AC-6): an install that needs elevation, or
/// a version whose apply already failed, is never applied unattended — no
/// drain is raised, the manual button path stays, and the notification
/// center says why.
#[test]
fn app_runtime_staged_update_falls_back_to_manual_when_auto_apply_is_refused() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    for refusal in [
        gwt::update_drain::UpdateAutoApplyRefusal::RequiresElevation,
        gwt::update_drain::UpdateAutoApplyRefusal::PreviousFailure {
            version: "99.0.0".to_string(),
        },
    ] {
        let events = runtime.update_staged_events_with("99.0.0", Some(refusal.clone()));
        let log = fs::read_to_string(gwt_core::update::update_log_path()).unwrap_or_default();
        assert!(
            log.lines().any(|line| {
                let entry: serde_json::Value = serde_json::from_str(line).unwrap();
                entry["stage"] == "pending_refused" && entry["reason"] == refusal.notice("99.0.0")
            }),
            "automatic apply refusal must be in the update log: {log}"
        );
        assert!(
            gwt::load_issue_monitor_prefs(&prefs_path)
                .expect("reload")
                .update_drain
                .is_none(),
            "no drain for {refusal:?}"
        );
        let toasts = update_resume_toasts(&events);
        assert_eq!(
            toasts.len(),
            1,
            "one fallback notice for {refusal:?}: {toasts:?}"
        );
        assert_eq!(toasts[0].0, "warn");
        assert!(
            toasts[0].1.contains("99.0.0") && toasts[0].1.contains("manually"),
            "the notice names the manual fallback: {}",
            toasts[0].1
        );
    }
}

#[test]
fn issue_5343_reconnect_restores_staged_cta_and_elevation_notice() {
    let temp = tempdir().unwrap();
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let payload = temp.path().join("update.pkg");
    fs::write(&payload, "prepared installer").unwrap();
    gwt_core::update::persist_pending_update_manifest(&gwt_core::update::PendingUpdateManifest {
        version: "999.0.0".into(),
        asset_url: "https://example.invalid/update.pkg".into(),
        payload: gwt_core::update::PreparedPayload::Installer {
            path: payload,
            kind: gwt_core::update::InstallerKind::MacPkg,
        },
        downloaded_at: chrono::Utc::now().to_rfc3339(),
        origin: gwt_core::update::PendingUpdateOrigin::Automatic,
        auto_apply_deferred: false,
    })
    .unwrap();
    let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
    let events = runtime.frontend_sync_events("browser");
    assert!(
        events.iter().any(|event| matches!(
            event.event,
            BackendEvent::UpdateApplyPendingPersisted { .. }
        )),
        "disk-staged update must restore the manual CTA"
    );
    assert!(
        events.iter().any(|event| matches!(
            event.event,
            BackendEvent::UpdateAutoApply {
                phase: gwt::protocol::UpdateAutoApplyPhase::ElevationRequired,
                ..
            }
        )),
        "CTA must name the elevation requirement"
    );
    assert!(update_resume_toasts(&events)
        .iter()
        .any(|(level, message)| {
            *level == "warn" && message.contains("Administrator privileges required")
        }));
    assert!(gwt_core::update::load_pending_update_manifest().is_some());
}

#[test]
fn app_runtime_enabled_fallback_epoch_overflow_is_zero_write_error() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: u64::MAX,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed exhausted prefs");
    let before = fs::read(&prefs_path).expect("read seeded prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorEnabled { enabled: false },
    );

    assert_eq!(fs::read(&prefs_path).expect("reload prefs"), before);
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, message, .. }
            if level == "error" && message.contains("authority epoch exhausted")
    )));
    assert!(!events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
    )));
}

#[test]
fn app_runtime_autonomous_fallback_epoch_overflow_is_zero_write_error() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            effect_authority_epoch: u64::MAX,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed exhausted prefs");
    let before = fs::read(&prefs_path).expect("read seeded prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorAutonomousMode { enabled: false },
    );

    assert_eq!(fs::read(&prefs_path).expect("reload prefs"), before);
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, message, .. }
            if level == "error" && message.contains("authority epoch exhausted")
    )));
    assert!(!events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorStatus { .. } | BackendEvent::IssueMonitorInbox { .. }
    )));
}

#[test]
fn app_runtime_full_issue_monitor_scan_migrates_legacy_git_failure_and_persists_marker() {
    // Verify migration and durable state, independently of host fsync latency.
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Assert migration and admission outcomes, rather than native Git latency.
    let _projection_timeout = ScopedEnvVar::set(
        "GWT_TEST_ISSUE_MONITOR_FALLBACK_PROJECTION_TIMEOUT_MS",
        "10000",
    );
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let fake_gh = write_fake_gh_issue_list(temp.path());
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &legacy_issue_monitor_failed_prefs(&repo, 43))
        .expect("seed legacy prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    reset_local_issue_monitor_remote_scan_count();
    let events = runtime.local_issue_monitor_events_with_policy(
        &runtime.test_context(),
        Some("client-1"),
        super::super::IssueMonitorScanPolicy::Scan,
        |_| {},
    );
    assert_eq!(
        local_issue_monitor_remote_scan_count(),
        1,
        "the full scan worker policy proves the remote-scan test probe is live"
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(
        status.last_error, None,
        "the stale failure banner is removed"
    );
    assert_eq!(
        status.queue_len, 1,
        "the live open issue returns to the queue"
    );
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 43)
        .expect("live issue row");
    assert_eq!(item.state, gwt::MonitorInboxState::Queued);
    assert_eq!(item.error_message, None);

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload migrated prefs");
    assert_eq!(
        persisted.legacy_git_launch_failure_migration_version,
        gwt::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
    );
    assert!(
        persisted.failed_issues.is_empty(),
        "marker and cleanup are persisted by the final atomic save"
    );
    #[cfg(not(unix))]
    {
        let mut enabled = persisted;
        enabled.enabled = true;
        enabled.max_active_agents_mode = gwt::issue_monitor::IssueMonitorMaxActiveMode::Auto;
        enabled.launch_profile = Some(sample_issue_monitor_launch_profile());
        gwt::save_issue_monitor_prefs(&prefs_path, &enabled).expect("enable Auto scan");
        let observed_at = chrono::Utc::now().timestamp() as u64;
        let measured_root = dunce::canonicalize(&repo).expect("measured project");
        let machine_state = gwt_core::paths::gwt_home().join("machine-state");
        fs::create_dir_all(&machine_state).expect("machine state");
        fs::write(
            machine_state.join("agent-capacity.json"),
            serde_json::to_vec(&serde_json::json!({
                "observed_at": observed_at, "expires_at": observed_at + 30,
                "performance_cores": 8, "gui_cpu_millicores": 0,
                "available_ram_bytes": 80, "per_agent_ram_bytes": 10,
                "free_disk_bytes": 100,
                "targets": {measured_root.join("target").to_string_lossy(): {"bytes": 10, "observed_at": observed_at}},
                "disk_observations": {measured_root.to_string_lossy(): {"available_bytes": 100, "observed_at": observed_at}}
            }))
            .expect("capacity snapshot"),
        )
        .expect("fresh capacity snapshot");
        let capacity = gwt::agent_capacity::project_capacity(&repo, &Default::default(), 0);
        assert!(capacity.measurement_complete, "{}", capacity.reason);
        let probed = Arc::new(Mutex::new(Vec::new()));
        let _hook = super::super::set_local_completion_probe_test_hook({
            let probed = Arc::clone(&probed);
            move |issue_number| {
                probed.lock().expect("probe log").push(issue_number);
                Err(gwt::issue_monitor_worker::IssueMonitorCompletionProbeFailure::Deadline(
                    gwt::issue_monitor_worker::IssueMonitorScanFailure::new(
                        gwt::issue_monitor_worker::IssueMonitorScanStage::ClaimCompletionReadback,
                        "test stops after reaching the Auto admission probe",
                    ),
                ))
            }
        });
        let _events = runtime.local_issue_monitor_events_with_policy(
            &runtime.test_context(),
            Some("client-1"),
            super::super::IssueMonitorScanPolicy::Scan,
            |_| {},
        );
        assert_eq!(
            probed.lock().expect("probe log").as_slice(),
            &[43],
            "Auto admission did not reach the completion probe: {_events:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn app_runtime_issue_monitor_reconciliation_error_survives_rebase_scan() {
    // Assert error provenance, independently of the nominal commit budget.
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
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
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "merge_fail");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
                issue_number: 43,
                window_id: "window-43".to_string(),
            }],
            ..queued_issue_monitor_prefs(&[43])
        },
    )
    .expect("seed launched issue");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.local_issue_monitor_events_with_policy(
        &runtime.test_context(),
        Some("client-1"),
        super::super::IssueMonitorScanPolicy::Scan,
        |_| {},
    );
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    let error = status
        .last_error
        .as_deref()
        .expect("merge reconciliation error");
    assert!(error.contains("merge reconciliation failed"), "{error}");
    assert!(error.contains("gh merged query failed"), "{error}");
    assert_eq!(
        status.active_count, 1,
        "query failure keeps the active slot"
    );

    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    assert_eq!(
        inbox
            .iter()
            .find(|item| item.issue.number == 43)
            .map(|item| item.state),
        Some(gwt::MonitorInboxState::Launched),
        "query failure must not fabricate a merged transition"
    );
}

#[test]
fn issue_monitor_scan_failures_prefer_launch_failure_over_merge_query_error() {
    let mut monitor = test_monitor(gwt::IssueMonitorConfig::default());

    record_issue_monitor_scan_failures(
        &mut monitor,
        "2026-07-27T00:00:00Z",
        Some("issue monitor merge reconciliation failed: gh unavailable".to_string()),
        vec![(43, "agent binary missing".to_string())],
    );

    assert_eq!(
        monitor.status_view().last_error.as_deref(),
        Some("issue #43: agent binary missing"),
        "the later, issue-specific launch failure must remain operator-visible"
    );
}

#[test]
fn app_runtime_full_issue_monitor_cache_fallback_does_not_migrate_legacy_failure() {
    // Verify cache provenance, independently of the prefs commit wall-clock budget.
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(Instant::now());
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
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            43,
            "Cached issue",
            &["bug"],
            "Cached body",
            "2026-07-21T00:00:00Z",
        ))
        .expect("write cache fallback");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &legacy_issue_monitor_failed_prefs(&repo, 43))
        .expect("seed legacy prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.local_issue_monitor_events_with_policy(
        &runtime.test_context(),
        Some("client-1"),
        super::super::IssueMonitorScanPolicy::Scan,
        |_| {},
    );

    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    assert_eq!(
        inbox
            .iter()
            .find(|item| item.issue.number == 43)
            .map(|item| item.state),
        Some(gwt::MonitorInboxState::AgentFailed)
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload cached prefs");
    assert_eq!(persisted.legacy_git_launch_failure_migration_version, 0);
    assert_eq!(persisted.failed_issues.len(), 1);
}

#[test]
fn app_runtime_quick_issue_monitor_snapshot_does_not_migrate_legacy_failure() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let snapshot = sample_issue_snapshot(
        43,
        "Quick cached issue",
        &["bug"],
        "Cached body",
        "2026-07-21T00:00:00Z",
    );
    let cache_root = issue_cache_root(&repo);
    Cache::new(cache_root.clone())
        .write_snapshot(&snapshot)
        .expect("write quick cache");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &legacy_issue_monitor_failed_prefs(&repo, 43))
        .expect("seed legacy prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.quick_issue_monitor_snapshot_events(
        Some("client-1"),
        &repo,
        &cache_root,
        &snapshot,
    );

    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("quick issue monitor inbox");
    assert_eq!(
        inbox
            .iter()
            .find(|item| item.issue.number == 43)
            .map(|item| item.state),
        Some(gwt::MonitorInboxState::AgentFailed)
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload quick prefs");
    assert_eq!(persisted.legacy_git_launch_failure_migration_version, 0);
    assert_eq!(persisted.failed_issues.len(), 1);
}

#[test]
fn app_runtime_agent_failed_after_migration_keeps_new_same_failure() {
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
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            43,
            "Cached issue",
            &["bug"],
            "Cached body",
            "2026-07-21T00:00:00Z",
        ))
        .expect("write cache fallback");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: false,
            legacy_git_launch_failure_migration_version:
                gwt::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed completed migration");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-43".to_string();
    runtime.pending_launch_feedback_contexts.insert(
        window_id.clone(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(43),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: None,
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );
    let failure = legacy_issue_monitor_git_failure(&repo);

    let events = runtime.issue_monitor_agent_failed_result_events(
        &window_id,
        &failure,
        Some(43),
        Err(
            gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                "deterministic local fallback".to_string(),
            ),
        ),
    );
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 43)
        .expect("new failed row");
    assert_eq!(item.state, gwt::MonitorInboxState::AgentFailed);
    assert_eq!(item.error_message.as_deref(), Some(failure.as_str()));

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload failed prefs");
    assert_eq!(
        persisted.legacy_git_launch_failure_migration_version,
        gwt::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
    );
    assert_eq!(persisted.failed_issues.len(), 1);
    assert_eq!(persisted.failed_issues[0].message, failure);
}

#[test]
fn app_runtime_agent_failed_rebases_concurrent_daemon_migration_before_fresh_failure() {
    struct PrefsLockContentionLayer {
        sender: Mutex<Option<mpsc::Sender<()>>>,
    }

    impl<S: Subscriber> Layer<S> for PrefsLockContentionLayer {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = CaptureTracingVisitor::default();
            event.record(&mut visitor);
            if visitor.fields.get("operation").map(String::as_str) != Some("issue_monitor_prefs")
                || visitor.fields.get("error").map(String::as_str) != Some("file lock contended")
            {
                return;
            }
            if let Some(sender) = self.sender.lock().expect("contention sender").take() {
                let _ = sender.send(());
            }
        }
    }

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
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            43,
            "Cached issue",
            &["bug"],
            "Cached body",
            "2026-07-21T00:00:00Z",
        ))
        .expect("write cache fallback");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &legacy_issue_monitor_failed_prefs(&repo, 43))
        .expect("seed legacy prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = "tab-1::agent-43".to_string();
    runtime.pending_launch_feedback_contexts.insert(
        window_id.clone(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(43),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: None,
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );
    let failure = legacy_issue_monitor_git_failure(&repo);

    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(prefs_path.with_extension("lock"))
        .expect("open issue monitor prefs lock");
    lock.lock_exclusive()
        .expect("hold issue monitor prefs lock");
    let (contention_tx, contention_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let writer_window = window_id.clone();
    let writer_failure = failure.clone();
    let writer_repo = repo.clone();
    let writer = thread::spawn(move || {
        // Keep callsite interest dynamic if a subscriber-free sibling registers the WARN first.
        let _interest_guard = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let subscriber = tracing_subscriber::registry().with(PrefsLockContentionLayer {
            sender: Mutex::new(Some(contention_tx)),
        });
        let events = tracing::subscriber::with_default(subscriber, || {
            runtime.issue_monitor_agent_failed_result_events(
                &writer_window,
                &writer_failure,
                Some(43),
                Err(
                    gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                        format!("deterministic local fallback for {}", writer_repo.display()),
                    ),
                ),
            )
        });
        done_tx.send(events).expect("return GUI events");
    });

    let coordinated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gwt_core::test_support::recv_event(&contention_rx, "GUI writer contended on prefs lock");
        assert!(
            matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "GUI writer must not complete while the transaction lock remains held"
        );

        let profile = sample_issue_monitor_launch_profile();
        let reviewing = issue_monitor_autonomous_record(42, gwt::AutonomousPhase::Reviewing, 2);
        let implementing =
            issue_monitor_autonomous_record(99, gwt::AutonomousPhase::Implementing, 1);
        let migrated = gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            enabled: true,
            max_active_agents: 4,
            priority_order: vec![99, 42],
            launch_profile: Some(profile.clone()),
            merged_issues: vec![88],
            autonomous_mode: true,
            autonomous_tuning: gwt::issue_monitor::AutonomousTuning {
                max_attempts: 9,
                ..gwt::issue_monitor::AutonomousTuning::default()
            },
            autonomous_records: vec![reviewing.clone(), implementing.clone()],
            ..gwt::IssueMonitorPrefs::default()
        };
        fs::write(
            &prefs_path,
            serde_json::to_vec_pretty(&migrated).expect("serialize migrated prefs"),
        )
        .expect("commit daemon migration while GUI waits");
        (profile, reviewing, implementing)
    }));
    let unlocked = FileExt::unlock(&lock);
    drop(lock);
    let joined = writer.join();
    let (profile, reviewing, implementing) =
        coordinated.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    unlocked.expect("release issue monitor prefs lock");
    joined.expect("GUI writer thread");
    let events = done_rx
        .try_recv()
        .expect("joined GUI writer returned events after unlock");
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == 43)
        .unwrap_or_else(|| panic!("fresh failed row; events={events:#?}"));
    assert_eq!(item.state, gwt::MonitorInboxState::AgentFailed);
    assert_eq!(item.error_message.as_deref(), Some(failure.as_str()));

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload committed prefs");
    assert_eq!(
        persisted.legacy_git_launch_failure_migration_version,
        gwt::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
        "the stale GUI cannot roll the daemon migration marker back"
    );
    assert_eq!(persisted.failed_issues.len(), 1);
    assert_eq!(persisted.failed_issues[0].message, failure);
    assert!(persisted.enabled, "latest daemon config is preserved");
    assert_eq!(persisted.max_active_agents, 4);
    assert_eq!(persisted.priority_order, vec![99, 42]);
    assert!(persisted.autonomous_mode);
    assert_eq!(persisted.launch_profile, Some(profile));
    assert_eq!(persisted.autonomous_tuning.max_attempts, 9);
    assert_eq!(persisted.merged_issues, vec![88]);
    assert_eq!(
        persisted.autonomous_records,
        vec![reviewing, implementing],
        "the actual GUI final writer must not roll back daemon lifecycle records"
    );
}

#[test]
fn app_runtime_rebase_keeps_equal_marker_disk_only_fresh_failures() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let prefs_path = temp.path().join("issue-monitor.json");
    let fresh_failure = legacy_issue_monitor_git_failure(temp.path());
    let disk = gwt::IssueMonitorPrefs {
        failed_issues: vec![
            gwt::IssueMonitorFailedIssue {
                issue_number: 43,
                message: fresh_failure.clone(),
                window_id: None,
            },
            gwt::IssueMonitorFailedIssue {
                issue_number: 99,
                message: "unrelated failure".to_string(),
                window_id: Some("tab-1::agent-99".to_string()),
            },
        ],
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &disk).expect("seed equal-marker disk failures");
    let mut stale = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs::default(),
    );

    super::super::rebase_mutate_and_persist_issue_monitor_state(&prefs_path, &mut stale, |_| {})
        .expect("GUI rebase commits");

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload committed prefs");
    for prefs in [&persisted, &stale.prefs()] {
        assert_eq!(
            prefs.legacy_git_launch_failure_migration_version,
            gwt::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
        );
        assert_eq!(prefs.failed_issues, disk.failed_issues);
    }
}

#[test]
fn app_runtime_rebase_recovers_malformed_prefs_from_current_state() {
    // This asserts durable recovery, not filesystem speed under load.
    let operation_now = gwt_core::operation_deadline::now();
    let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(operation_now);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let prefs_path = temp.path().join("issue-monitor.json");
    fs::write(&prefs_path, b"{").expect("seed malformed prefs");
    let mut current = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            enabled: true,
            merged_issues: vec![88],
            ..gwt::IssueMonitorPrefs::default()
        },
    );

    super::super::rebase_mutate_and_persist_issue_monitor_state(
        &prefs_path,
        &mut current,
        |monitor| monitor.set_max_active_agents(4),
    )
    .expect("GUI rebase recovers malformed prefs and commits");

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("recovered GUI prefs");
    assert!(persisted.enabled);
    assert_eq!(persisted.max_active_agents, 4);
    assert_eq!(persisted.merged_issues, vec![88]);
    let quarantines = fs::read_dir(temp.path())
        .expect("read prefs directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("issue-monitor.json.corrupt-")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    assert_eq!(quarantines.len(), 1);
    assert_eq!(fs::read(&quarantines[0]).expect("read quarantine"), b"{");
}

#[test]
fn local_fallback_transaction_preserves_the_background_worker_deadline() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let prefs_path = temp.path().join("issue-monitor.json");
    let prefs = gwt::IssueMonitorPrefs {
        enabled: true,
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
    let mut monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs);
    let background_deadline = Instant::now() + Duration::from_secs(5);
    let _deadline =
        gwt_core::operation_deadline::ScopedOperationDeadline::enter(background_deadline);

    let observed =
        super::super::try_rebase_mutate_and_persist_issue_monitor_state_without_authority_fence(
            &prefs_path,
            &mut monitor,
            |_| gwt_core::operation_deadline::current(),
        )
        .expect("fallback transaction");

    assert_eq!(
        observed,
        Some(background_deadline),
        "the GUI-local 250 ms bound must not shorten a background worker deadline"
    );
}

#[test]
fn gui_scan_preparation_does_not_hold_the_prefs_lock() {
    let _subscriber =
        tracing::subscriber::set_default(tracing_subscriber::fmt().with_test_writer().finish());
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _budget = super::super::ScopedLocalIssueMonitorPrefsTimeout::set(
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
    );
    let prefs_path = temp.path().join("issue-monitor.json");
    let prefs = gwt::IssueMonitorPrefs::default();
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
    let mut monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs);

    let prepare = |proposal: &mut gwt::IssueMonitorState| {
        // Observe a deliberately slow read phase without asserting its
        // wall-clock duration. The lock assertion is independent of time.
        let preparing = Instant::now();
        thread::sleep(Duration::from_secs(1));
        tracing::info!(
            preparation_ms = preparing.elapsed().as_secs_f64() * 1000.0,
            "Issue #5447 controlled scan preparation"
        );
        let lock = gwt_core::operation_deadline::NamedFileLock::try_acquire(
            &prefs_path.with_extension("lock"),
            "gui_scan_test",
        );
        proposal.set_enabled(true);
        match lock {
            Ok(lock) => {
                lock.unlock().unwrap();
                true
            }
            Err(_) => false,
        }
    };
    assert!(
        super::super::rebase_mutate_and_persist_issue_monitor_state(
            &prefs_path,
            &mut monitor,
            prepare,
        )
        .unwrap(),
        "observer scan preparation must leave the writer lock available"
    );
    assert!(
        super::super::prepare_and_persist_issue_monitor_scan_without_authority_fence(
            &prefs_path,
            &mut monitor,
            prepare,
        )
        .unwrap(),
        "fallback scan preparation must leave the writer lock available"
    );
    assert!(gwt::load_issue_monitor_prefs(&prefs_path).unwrap().enabled);
}

#[test]
fn gui_scan_rejects_a_snapshot_changed_during_preparation() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _budget = super::super::ScopedLocalIssueMonitorPrefsTimeout::set(
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
    );
    let prefs_path = temp.path().join("issue-monitor.json");
    let prefs = gwt::IssueMonitorPrefs::default();
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
    let mut monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs);

    let error = super::super::rebase_mutate_and_persist_issue_monitor_state(
        &prefs_path,
        &mut monitor,
        |proposal| {
            gwt::try_mutate_issue_monitor_prefs(&prefs_path, |disk| {
                disk.priority_order = vec![5447];
                Ok(())
            })
            .expect("a daemon commit must be able to run during GUI preparation");
            proposal.set_enabled(true);
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    assert!(!persisted.enabled);
    assert_eq!(persisted.priority_order, vec![5447]);
    assert_eq!(monitor.prefs(), persisted);
}

#[test]
fn gui_scan_rechecks_daemon_authority_after_preparation() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _budget = super::super::ScopedLocalIssueMonitorPrefsTimeout::set(
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
    );
    let prefs_path = temp.path().join("issue-monitor.json");
    let prefs = gwt::IssueMonitorPrefs::default();
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
    let before = fs::read(&prefs_path).unwrap();
    let mut monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs);

    let error = super::super::prepare_and_persist_issue_monitor_scan_without_authority_fence(
        &prefs_path,
        &mut monitor,
        |proposal| {
            gwt::persist_issue_monitor_authority_fence(
                &prefs_path,
                &gwt::IssueMonitorAuthorityFence::current_process(),
            )
            .unwrap();
            proposal.set_enabled(true);
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    assert_eq!(fs::read(&prefs_path).unwrap(), before);
    assert!(!monitor.prefs().enabled);
}

#[test]
fn gui_scan_retries_a_concurrent_control_without_waiting_for_the_next_scan() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _budget = super::super::ScopedLocalIssueMonitorPrefsTimeout::set(
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
    );
    let prefs_path = temp.path().join("issue-monitor.json");
    let prefs = gwt::IssueMonitorPrefs {
        launched_issues: vec![gwt::IssueMonitorLaunchedIssue {
            issue_number: 42,
            window_id: "tab-1::agent-42".to_string(),
        }],
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
    let mut monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs);
    let now = "2026-10-15T00:00:00Z";
    monitor.record_window_snapshot(gwt::IssueMonitorWindowSnapshot {
        project_tab_id: "tab-1".to_string(),
        observed_at: now.to_string(),
        windows: Vec::new(),
    });
    let mut attempts = 0;

    super::super::prepare_and_persist_issue_monitor_scan_without_authority_fence(
        &prefs_path,
        &mut monitor,
        |proposal| {
            attempts += 1;
            assert_eq!(
                proposal.classify_idle_windows(&BTreeMap::new(), now).len(),
                1,
                "each attempt must retain the observed canvas for slot recovery",
            );
            if attempts == 1 {
                gwt::try_mutate_issue_monitor_prefs(&prefs_path, |disk| {
                    disk.priority_order = vec![5447];
                    Ok(())
                })
                .unwrap();
            }
            proposal.set_enabled(true);
        },
    )
    .unwrap();

    assert_eq!(attempts, 2);
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    assert_eq!(persisted.priority_order, vec![5447]);
    assert!(persisted.enabled);
    assert_eq!(monitor.prefs(), persisted);
}

#[test]
fn sibling_gui_fallback_transactions_keep_the_250ms_lock_budget_inside_a_longer_scan_deadline() {
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
    let prefs = gwt::IssueMonitorPrefs {
        enabled: true,
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
    let before = fs::read(&prefs_path).expect("read seeded prefs");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(prefs_path.with_extension("lock"))
        .expect("open prefs lock");
    lock.lock_exclusive().expect("hold prefs lock");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_monitor_fallback_commit_timeout =
        super::super::ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT;
    let mut monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs);
    let scan_deadline = Instant::now() + Duration::from_secs(5);
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(scan_deadline);

    let started = Instant::now();
    let rebase_mutated = super::super::rebase_mutate_and_persist_issue_monitor_state(
        &prefs_path,
        &mut monitor,
        |_| true,
    );
    let control = runtime.commit_local_issue_monitor_control_for_project(&repo, |_| ());
    let authorizing = runtime
        .commit_local_issue_monitor_authorizing_control(&runtime.test_context(), |_| {
            Ok::<_, String>(())
        });
    let elapsed = started.elapsed();
    FileExt::unlock(&lock).expect("release prefs lock");

    assert!(
        elapsed < Duration::from_secs(2),
        "GUI fallback transactions outlived their cumulative short lock budget: {elapsed:?}"
    );
    assert!(
        rebase_mutated.is_err(),
        "timed-out rebase must fail closed without running its mutation"
    );
    assert!(
        control.is_err(),
        "timed-out control commit must fail closed"
    );
    assert!(
        authorizing.is_err(),
        "timed-out authorizing commit must fail closed"
    );
    assert_eq!(
        fs::read(&prefs_path).expect("reload prefs"),
        before,
        "timed-out GUI fallback transactions must be zero-write"
    );
}

#[test]
fn app_runtime_initial_recovery_keeps_legacy_failure_migration_unapplied() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    // Keep durable recovery assertions independent of CI fsync latency.
    let _budget = super::super::ScopedLocalIssueMonitorPrefsTimeout::set(
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
    );
    let prefs_path = temp.path().join("issue-monitor.json");
    fs::write(&prefs_path, b"{").expect("seed malformed prefs");

    let (monitor, ()) =
        super::super::load_mutate_and_persist_issue_monitor_state(&prefs_path, |monitor| {
            monitor.set_enabled(true)
        });

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("recovered GUI prefs");
    assert!(persisted.enabled);
    assert!(monitor.prefs().enabled);
    assert_eq!(
        persisted.legacy_git_launch_failure_migration_version, 0,
        "recovery without a valid in-memory snapshot must wait for a successful live scan"
    );
}

#[test]
fn app_runtime_gui_rebase_uses_latest_disk_config_and_autonomous_records() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    // Rebase assertions depend on a successful commit, not CI fsync latency.
    let _budget = super::super::ScopedLocalIssueMonitorPrefsTimeout::set(
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
    );
    let prefs_path = temp.path().join("issue-monitor.json");
    let stale_record = issue_monitor_autonomous_record(42, gwt::AutonomousPhase::Implementing, 1);
    let reviewing = issue_monitor_autonomous_record(42, gwt::AutonomousPhase::Reviewing, 2);
    let disk_only = issue_monitor_autonomous_record(99, gwt::AutonomousPhase::Implementing, 3);
    let disk = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        enabled: true,
        max_active_agents: 4,
        priority_order: vec![99, 42],
        merged_issues: vec![88],
        autonomous_mode: true,
        autonomous_tuning: gwt::issue_monitor::AutonomousTuning {
            max_attempts: 9,
            ..gwt::issue_monitor::AutonomousTuning::default()
        },
        autonomous_records: vec![reviewing.clone(), disk_only.clone()],
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &disk).expect("seed latest daemon state");
    let mut stale = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            enabled: false,
            max_active_agents: 1,
            priority_order: vec![42],
            merged_issues: vec![77],
            autonomous_mode: false,
            autonomous_records: vec![stale_record],
            ..gwt::IssueMonitorPrefs::default()
        },
    );

    super::super::rebase_mutate_and_persist_issue_monitor_state(&prefs_path, &mut stale, |_| {})
        .expect("GUI rebase commits");

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload GUI rebase");
    for prefs in [&persisted, &stale.prefs()] {
        assert!(prefs.enabled, "latest disk enabled flag wins");
        assert_eq!(prefs.max_active_agents, 4);
        assert_eq!(prefs.priority_order, vec![99, 42]);
        assert!(prefs.autonomous_mode);
        assert_eq!(prefs.autonomous_tuning.max_attempts, 9);
        assert_eq!(prefs.merged_issues, vec![77, 88], "merged state is unioned");
        assert_eq!(
            prefs.autonomous_records,
            vec![reviewing.clone(), disk_only.clone()],
            "GUI observer takes the latest disk record for the same key"
        );
    }
}

#[test]
fn app_runtime_issue_monitor_reorder_persists_and_reorders_cached_inbox() {
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
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let cache = Cache::new(issue_cache_root(&repo));
    for number in [3165, 3166, 3167] {
        cache
            .write_snapshot(&sample_issue_snapshot(
                number,
                &format!("Issue {number}"),
                &["bug"],
                "Issue body",
                "2026-06-23T00:00:00Z",
            ))
            .expect("write issue cache");
    }
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &queued_issue_monitor_prefs(&[3165, 3166, 3167]),
    )
    .expect("seed explicitly queued issues");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ReorderIssueMonitorIssues {
            issue_numbers: vec![3167, 3165, 3166],
        },
    );

    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let visible_numbers: Vec<u64> = inbox.iter().map(|item| item.issue.number).collect();
    assert_eq!(visible_numbers, vec![3167, 3165, 3166]);

    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load issue monitor prefs");
    assert_eq!(prefs.priority_order, vec![3167, 3165, 3166]);
}

#[test]
fn app_runtime_quick_register_issue_creates_cache_entry_without_queue_admission() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let fake_client = Arc::new(FakeIssueClient::new());
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_client_factory = Arc::new({
        let fake_client = Arc::clone(&fake_client);
        move |_owner, _repo| {
            let client: Arc<dyn IssueClient> = fake_client.clone();
            Ok(client)
        }
    });

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::QuickRegisterIssue {
            title: "Investigate Intake registration".to_string(),
            launch: false,
            auto_merge: false,
        },
    );

    assert_eq!(fake_client.call_log(), vec!["create_issue:#1"]);

    let cached = Cache::new(issue_cache_root(&repo))
        .load_entry(IssueNumber(1))
        .expect("quick issue written to cache");
    assert_eq!(cached.snapshot.title, "Investigate Intake registration");
    assert!(cached.snapshot.labels.is_empty());
    for heading in [
        "## Summary",
        "## Background",
        "## Spec Status",
        "## Related SPECs",
        "## Expected Outcome",
        "## Notes",
    ] {
        assert!(
            cached.snapshot.body.contains(heading),
            "quick issue body should contain {heading}: {}",
            cached.snapshot.body
        );
    }
    assert!(
        cached.snapshot.body.contains("compatibility path")
            && cached
                .snapshot
                .body
                .contains("gwt-register-issue remains the primary intake workflow"),
        "quick issue body must describe the withdrawn toolbar path as a compatibility guard: {}",
        cached.snapshot.body
    );

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue registered" && *issue_number == Some(1)
        )
    }));
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    assert!(
        inbox.iter().all(|item| item.issue.number != 1),
        "registration without launch leaves the cached Issue in Backlog"
    );
    let prefs = gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
        .expect("load registration prefs");
    assert!(prefs
        .terminal_queues
        .values()
        .all(|queue| queue.entries.is_empty()));
}

// SPEC #3885 T-033 (FR-022): the "+ New" popover's auto-merge checkbox labels
// the Issue at creation time.
#[test]
fn app_runtime_quick_register_issue_applies_the_auto_merge_label() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let fake_client = Arc::new(FakeIssueClient::new());
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_client_factory = Arc::new({
        let fake_client = Arc::clone(&fake_client);
        move |_owner, _repo| {
            let client: Arc<dyn IssueClient> = fake_client.clone();
            Ok(client)
        }
    });

    runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::QuickRegisterIssue {
            title: "Ship the popover".to_string(),
            launch: false,
            auto_merge: true,
        },
    );

    let cached = Cache::new(issue_cache_root(&repo))
        .load_entry(IssueNumber(1))
        .expect("quick issue written to cache");
    assert_eq!(cached.snapshot.labels, vec!["auto-merge".to_string()]);
}

#[test]
fn app_runtime_quick_register_issue_permission_error_includes_reason_and_fallback() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_client_factory = Arc::new(|_owner, _repo| {
        Ok(Arc::new(PermissionDeniedCreateIssueClient) as Arc<dyn IssueClient>)
    });

    let events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::QuickRegisterIssue {
            title: "Investigate Intake registration".to_string(),
            launch: false,
            auto_merge: false,
        },
    );

    let message = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorToast {
                level,
                message,
                issue_number: None,
                ..
            } if level == "error" => Some(message.as_str()),
            _ => None,
        })
        .expect("permission error toast");
    assert!(
        message.contains("Issues are disabled for this repository"),
        "toast must preserve the GitHub-provided reason: {message}"
    );
    assert!(
        message.contains("Fallback: create the Issue manually on GitHub"),
        "toast must include the FR-011 fallback path: {message}"
    );
}

#[test]
fn app_runtime_issue_monitor_auto_launch_uses_start_with_last_settings() {
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
            max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            max_active_agents: 1,
            ..Default::default()
        },
    )
    .expect("seed positive Monitor pane capacity");
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut previous = gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex);
    previous.model = Some("gpt-5.5".to_string());
    previous.reasoning_level = Some("high".to_string());
    previous.skip_permissions = true;
    previous.save(&sessions_dir).expect("save previous session");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let (spawner, queued) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime.auto_launch_issue_monitor_delivery_events_for_project(
        &repo,
        3165,
        LinkedIssueKind::Spec,
        None,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let events = commit_issue4803_monitor_preparation(&mut runtime, &queued, &recorded_events);

    assert!(events.iter().any(|event| {
        matches!(
            &event.event,
            BackendEvent::IssueMonitorToast { message, issue_number, .. }
                if message == "Issue Monitor launch requested" && *issue_number == Some(3165)
        )
    }));
    assert!(
        runtime
            .project_state(&runtime.test_context())
            .expect("test project state")
            .launch_wizard
            .is_none(),
        "auto launch with last settings must not open the Launch Agent window"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::LaunchWizardState { .. })),
        "silent auto launch must not broadcast LaunchWizardState"
    );
    assert!(
        runtime
            .pending_launch_feedback_contexts
            .values()
            .any(|context| context.issue_monitor_issue_number == Some(3165)),
        "auto launch errors must be wired back to Issue Monitor"
    );
    let workspace = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::WorkspaceState { workspace } => Some(workspace),
            _ => None,
        })
        .expect("workspace broadcast");
    let agent_window = workspace.tabs[0]
        .workspace
        .windows
        .iter()
        .find(|window| window.preset == WindowPreset::Agent)
        .expect("silent auto launch agent window");
    assert_eq!(agent_window.agent_id.as_deref(), Some("codex"));
    assert!(
        runtime.has_pending_agent_launch_worker(&agent_window.id),
        "initial launch must remain registered until its worker completion is applied"
    );
}

#[test]
fn issue_5140_expired_fallback_delivery_does_not_authorize_a_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let mut monitor = test_monitor(gwt::IssueMonitorConfig {
        enabled: true,
        ..gwt::IssueMonitorConfig::default()
    });
    monitor.terminal_queue_push(&[42], "operator", "2000-01-01T00:00:00Z");
    monitor.record_candidate(gwt::IssueMonitorIssue {
        author_login: None,
        number: 42,
        title: "Expired windowless delivery".to_string(),
        labels: Vec::new(),
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(
        42,
        "claim-42",
        "host/session",
        "effect-42",
        "2000-01-01T00:00:00Z",
    ));
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed expired delivery");
    let input = gwt::project_runtime::installation::IssueMonitorLaunchClaimInput {
        project_root: repo.clone(),
        issue_number: 42,
        delivery_id: "launch:effect-42".into(),
        materializer_window_id: "tab-1::agent-1".into(),
        materializer_id: "expired-transport-model".into(),
        materializer_pid: std::process::id(),
        fallback_timeout: gwt_core::deadline_budget::HANG_GUARD,
        snapshot: Some(gwt::IssueMonitorWindowSnapshot {
            project_tab_id: "tab-1".into(),
            observed_at: Utc::now().to_rfc3339(),
            windows: Vec::new(),
        }),
        snapshot_tabs: std::collections::BTreeSet::from(["tab-1".into()]),
    };
    assert!(!input.claim().expect("canonical fallback claim"));
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload committed state");
    assert!(prefs.pending_launch_deliveries.is_empty());
    assert!(prefs.failed_issues.iter().any(|failure| {
        failure.issue_number == 42 && failure.message.contains("launch:effect-42")
    }));
}

#[test]
fn durable_issue_monitor_delivery_materializes_one_window_and_replay_only_acks() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Assert delivery admission and replay outcomes, rather than native Git latency.
    let _projection_timeout = ScopedEnvVar::set(
        "GWT_TEST_ISSUE_MONITOR_FALLBACK_PROJECTION_TIMEOUT_MS",
        "10000",
    );
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "durable-delivery",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    let repo = fixture.project_root.clone();
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let delivery_id = fixture.delivery_id.clone().unwrap();
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    prefs.launch_auto = true;
    prefs.launch_tiers = vec![vec![codex_issue_monitor_launch_profile()]; 3];
    // Higher history is not the selected tier for this fresh launch.
    prefs.record_tier_launch(3165, 2);
    prefs.issue_tiers.get_mut(&3165).unwrap().floor = 0;
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed delivery tiers");
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    let runtime = &mut fixture.runtime;
    let recorded = &fixture.recorded_events;
    let (launch_spawner, launch_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = launch_spawner;
    let mut first = runtime.auto_launch_issue_monitor_delivery_events_for_project(
        &repo,
        3165,
        LinkedIssueKind::Spec,
        Some(delivery_id.clone()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    first.extend(commit_issue_monitor_preparations(
        runtime,
        &launch_tasks,
        recorded,
    ));
    assert!(
        first
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
        "launch must materialize a pane: {first:?}"
    );
    let replay = runtime.auto_launch_issue_monitor_delivery_events_for_project(
        &repo,
        3165,
        LinkedIssueKind::Spec,
        Some(delivery_id.clone()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    assert!(!replay
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })));
    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(agent_windows, 1);
    let window_id = runtime.runtimes.keys().next().unwrap().clone();
    let pane = owner_runtime_probe(runtime, &window_id);
    let session_id = pane.session.as_ref().unwrap().id.clone();
    let child_pid = pane.pty.process_id().expect("actual native child");
    assert_ne!(child_pid, std::process::id());
    assert!(pane.pty.try_wait().unwrap().is_none());
    assert!(pane.pending_fresh_execution.is_some());
    assert_eq!(
        runtime.active_agent_sessions[&window_id].session_id,
        session_id
    );
    assert_eq!(runtime.runtimes[&window_id].child_pid, Some(child_pid));
    let inventory = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    let live_panes = inventory["panes"].as_array().unwrap();
    assert_eq!(
        live_panes.len(),
        1,
        "delivery replay cannot spawn another child"
    );
    assert_eq!(live_panes[0]["session_id"], session_id);
    assert_eq!(live_panes[0]["child_pid"], child_pid);
    let before_ready = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::AcknowledgeMonitorLaunch {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
            issue_number: 3165,
            delivery_id: Some(delivery_id.clone()),
        },
    ))
    .unwrap();
    assert_eq!(before_ready["acknowledged"], false);
    assert_eq!(before_ready["id"], pane.id);
    assert_eq!(before_ready["incarnation"], pane.incarnation);
    drain_queued_blocking_tasks(&launch_tasks);
    let (ack_spawner, ack_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = ack_spawner;
    let census_host_pid = std::process::id();
    let census_host_started_at = gwt::process::host_process_start_time(census_host_pid)
        .expect("the actual census host process start time");
    let census_host_key = format!("{census_host_pid}:{census_host_started_at}");
    let census_window_id = pane.id.clone();
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Join the actual pane1 census before complete prefs baselines; no scheduler delay or field exclusion authorizes the comparison.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                if prefs
                    .monitor_runtime_counts
                    .get(&census_host_key)
                    .is_some_and(|census| {
                        census.host_pid == census_host_pid
                            && census.host_started_at == census_host_started_at
                            && census.counts.len() == 1
                            && census.counts.get(&3165) == Some(&1)
                            && census.windows.len() == 1
                            && census.windows.get(&census_window_id) == Some(&3165)
                    })
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the actual exact-host pane1 census precedes prefs baselines");
    });
    let disk_before_ack = gwt::load_workspace_state(&gwt::workspace_state_path(&repo)).unwrap();
    let pending_before_ack = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    let delivery = &pending_before_ack.pending_launch_deliveries[0];
    assert_eq!(delivery.delivery_id, delivery_id);
    assert_eq!(
        delivery.materializer_window_id.as_deref(),
        Some(pane.id.as_str())
    );
    assert_eq!(
        delivery.materialized_window_id.as_deref(),
        Some(pane.id.as_str())
    );
    assert_eq!(
        delivery.workspace_durable_window_id.as_deref(),
        Some(pane.id.as_str())
    );
    assert!(disk_before_ack.windows.iter().any(|row| {
        row.id == pane.id.split_once("::").unwrap().1
            && row.session_id.as_deref() == Some(session_id.as_str())
    }));
    assert!(!pending_before_ack.launch_confirmations.contains_key(&3165));
    assert!(!pending_before_ack
        .launch_bindings
        .values()
        .any(|issue| *issue == 3165));
    assert_eq!(
        serde_json::to_value(&pending_before_ack).unwrap()["pending_launch_deliveries"][0]
            ["selected_tier"],
        1
    );
    let _ = runtime.issue_monitor_launch_completed_delivery_events(
        &repo,
        3165,
        &window_id,
        Some(&delivery_id),
    );
    assert_eq!(
        ack_tasks.lock().unwrap().len(),
        1,
        "delivery ACK must be a worker transaction"
    );
    let delayed_ack = ack_tasks.lock().unwrap().pop().unwrap();
    assert_eq!(
        gwt::load_workspace_state(&gwt::workspace_state_path(&repo)).unwrap(),
        disk_before_ack
    );
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
        pending_before_ack,
        "a queued GUI ACK cannot authorize an unready owner Session"
    );
    let newer_window = runtime
        .tab_mut("tab-1")
        .unwrap()
        .workspace
        .add_window(WindowPreset::Agent);
    runtime.register_window("tab-1", &newer_window.id);
    let newer_id = combined_window_id("tab-1", &newer_window.id);
    let inspection_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .working_dir(&repo)
        .branch("develop")
        .base_branch("develop")
        .build();
    let newer_owner_id = owner.pane_id(&newer_window.id);
    let newer_receipt = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::LaunchAgent {
            id: newer_owner_id.clone(),
            operation_id: "durable-delivery-newer-inspection".into(),
            tab_id: gwt_core::paths::project_scope_hash(&repo).to_string(),
            config: Box::new(inspection_config.clone()),
            metadata: Box::default(),
            cols: 80,
            rows: 24,
        },
    ))
    .unwrap();
    let newer_probe = owner_fixture_block_on(owner.probe(
        &newer_owner_id,
        newer_receipt["incarnation"].as_u64().unwrap(),
    ));
    if newer_probe.is_err() {
        close_test_owner_runtime(runtime, &window_id);
        runtime.test_project_owners.remove(&repo);
        owner_fixture_block_on(
            Arc::try_unwrap(owner)
                .ok()
                .expect("drained inspection diagnostic owner")
                .shutdown(),
        );
        panic!(
            "actual newer native inspection was not installed: {newer_receipt}; probe={}",
            newer_probe.err().unwrap()
        );
    }
    let newer = newer_probe.unwrap();
    assert_eq!(
        newer.session.as_ref().unwrap().id,
        newer_receipt["session_id"]
    );
    assert!(newer.session.as_ref().unwrap().execution_binding.is_none());
    assert!(newer.grant.is_some());
    let context = runtime.test_context();
    let snapshot = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::Snapshot {
            id: newer.id.clone(),
            incarnation: newer.incarnation,
        },
    ))
    .unwrap();
    runtime.install_remote_owner_projection(
        &context,
        &newer_id,
        owner.client(),
        serde_json::from_value(snapshot).unwrap(),
        newer.session.clone(),
    );
    owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::UpdateLayout {
            layout: runtime.tab("tab-1").unwrap().workspace.persisted().clone(),
        },
    ))
    .unwrap();
    drain_queued_blocking_tasks(&ack_tasks);
    let prepared = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    assert_eq!(
        prepared.pending_launch_deliveries,
        pending_before_ack.pending_launch_deliveries
    );
    assert!(!prepared.launch_confirmations.contains_key(&3165));
    assert!(!prepared
        .launch_bindings
        .values()
        .any(|issue| *issue == 3165));
    authenticate_monitor_owner_readiness(&owner, &pane);
    let ready = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::AcknowledgeMonitorLaunch {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
            issue_number: 3165,
            delivery_id: Some(delivery_id.clone()),
        },
    ))
    .unwrap();
    assert_eq!(ready["acknowledged"], true);
    let disk_after_ready = gwt::load_workspace_state(&gwt::workspace_state_path(&repo)).unwrap();
    let acknowledged = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    delayed_ack();
    assert!(
        gwt::load_workspace_state(&gwt::workspace_state_path(&repo))
            .unwrap()
            .windows
            .iter()
            .any(|window| window.id == newer_window.id),
        "a delayed ACK must preserve windows added after its snapshot was captured"
    );
    let take_ack = || {
        let mut events = recorded.lock().unwrap();
        let index = events
            .iter()
            .position(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::IssueMonitorLaunchDeliveryAcknowledged(_)
                )
            })
            .unwrap();
        let UserEvent::IssueMonitorLaunchDeliveryAcknowledged(ack) =
            into_recorded_project_payload(events.remove(index))
        else {
            unreachable!()
        };
        *ack
    };
    runtime.handle_issue_monitor_launch_delivery_ack(take_ack());
    assert_eq!(
        gwt::load_workspace_state(&gwt::workspace_state_path(&repo)).unwrap(),
        disk_after_ready
    );
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
        acknowledged
    );
    assert_eq!(
        serde_json::to_value(&acknowledged).unwrap()["launch_confirmations"]["3165"]["tier"],
        1
    );
    assert_eq!(
        acknowledged.launch_confirmations[&3165]
            .delivery_id
            .as_deref(),
        Some(delivery_id.as_str())
    );
    assert_eq!(acknowledged.launch_bindings.get(&pane.id), Some(&3165));
    let still_live = owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).unwrap();
    assert_eq!(still_live.session.as_ref().unwrap().id, session_id);
    assert_eq!(still_live.pty.process_id(), Some(child_pid));
    assert!(
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
            .expect("reload prefs")
            .pending_launch_deliveries
            .is_empty()
    );

    // A real same-id replacement rejects the old physical incarnation as well
    // as invalidating the GUI's delayed observation.
    runtime.issue_monitor_launch_completed_delivery_events(
        &repo,
        3165,
        &window_id,
        Some(&delivery_id),
    );
    let stale_ack = ack_tasks.lock().unwrap().pop().unwrap();
    pane_control_tests::control_close(&owner, &pane);
    assert!(pane.pty.try_wait().unwrap().is_some());
    let raw_id = pane.id.split_once("::").unwrap().1;
    // A normal inspection intentionally reuses a live pane on its requested
    // branch/worktree. Reuse the now-drained predecessor worktree, leaving the
    // independently added develop pane live while this exact ID is replaced.
    let mut replacement_config = inspection_config;
    replacement_config.working_dir = Some(pane.session.as_ref().unwrap().worktree_path.clone());
    replacement_config.branch = Some(pane.session.as_ref().unwrap().branch.clone());
    let replacement_receipt = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::LaunchAgent {
            id: owner.pane_id(raw_id),
            operation_id: "durable-delivery-replacement-inspection".into(),
            tab_id: gwt_core::paths::project_scope_hash(&repo).to_string(),
            config: Box::new(replacement_config),
            metadata: Box::default(),
            cols: 80,
            rows: 24,
        },
    ))
    .unwrap();
    let replacement_probe = owner_fixture_block_on(owner.probe(
        &pane.id,
        replacement_receipt["incarnation"].as_u64().unwrap(),
    ));
    if replacement_probe.is_err() {
        close_test_owner_runtime(runtime, &newer_id);
        runtime.runtimes.remove(&window_id);
        runtime.test_project_owners.remove(&repo);
        owner_fixture_block_on(
            Arc::try_unwrap(owner)
                .ok()
                .expect("drained replacement diagnostic owner")
                .shutdown(),
        );
        panic!("actual replacement native inspection was not installed: {replacement_receipt}; probe={}", replacement_probe.err().unwrap());
    }
    let replacement = replacement_probe.unwrap();
    assert_eq!(
        replacement.session.as_ref().unwrap().id,
        replacement_receipt["session_id"]
    );
    assert!(replacement
        .session
        .as_ref()
        .unwrap()
        .execution_binding
        .is_none());
    assert!(replacement.grant.is_some());
    assert_ne!(replacement.incarnation, pane.incarnation);
    assert_ne!(replacement.session.as_ref().unwrap().id, session_id);
    let old_address = runtime.window_lookup[&window_id].clone();
    runtime.register_window(&old_address.tab_id, &old_address.raw_id);
    let snapshot = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::Snapshot {
            id: replacement.id.clone(),
            incarnation: replacement.incarnation,
        },
    ))
    .unwrap();
    runtime.install_remote_owner_projection(
        &context,
        &window_id,
        owner.client(),
        serde_json::from_value(snapshot).unwrap(),
        replacement.session.clone(),
    );
    drain_queued_blocking_tasks(&ack_tasks);
    let mut pending_after_replacement = pending_before_ack.clone();
    pending_after_replacement.agent_roster = gwt::load_issue_monitor_prefs(&prefs_path)
        .unwrap()
        .agent_roster;
    gwt::save_issue_monitor_prefs(&prefs_path, &pending_after_replacement).unwrap();
    pending_after_replacement = owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Join the normal pane0 census after original clone/save and capture whole prefs before either stale ACK; no field is copied or excluded.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                if prefs
                    .monitor_runtime_counts
                    .get(&census_host_key)
                    .is_some_and(|census| {
                        census.host_pid == census_host_pid
                            && census.host_started_at == census_host_started_at
                            && census.counts.is_empty()
                            && census.windows.is_empty()
                    })
                {
                    break prefs;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the actual pane0 census precedes the stale ACK prefs baseline")
    });
    let disk_before_stale_ack =
        gwt::load_workspace_state(&gwt::workspace_state_path(&repo)).unwrap();
    let stale_owner_ack = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::AcknowledgeMonitorLaunch {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
            issue_number: 3165,
            delivery_id: Some(delivery_id),
        },
    ));
    assert!(
        stale_owner_ack.is_err(),
        "the replaced owner incarnation cannot ACK"
    );
    stale_ack();
    assert!(runtime
        .handle_issue_monitor_launch_delivery_ack(take_ack())
        .is_empty());
    assert_eq!(
        gwt::load_workspace_state(&gwt::workspace_state_path(&repo)).unwrap(),
        disk_before_stale_ack
    );
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
        pending_after_replacement,
        "a stale launch must leave the delivery unacknowledged"
    );
    assert_eq!(
        runtime.active_agent_sessions[&window_id].session_id,
        replacement.session.as_ref().unwrap().id
    );
    close_test_owner_runtime(runtime, &window_id);
    close_test_owner_runtime(runtime, &newer_id);
    assert!(replacement.pty.try_wait().unwrap().is_some());
    assert!(newer.pty.try_wait().unwrap().is_some());
    runtime.test_project_owners.remove(&repo);
    owner_fixture_block_on(
        Arc::try_unwrap(owner)
            .ok()
            .expect("drained delivery owner")
            .shutdown(),
    );
}

#[test]
fn issue_monitor_ack_preserves_reused_failed_window_id() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    assert_ack_preserves_failed_window_replacement(temp.path(), false);
}

#[test]
fn issue_monitor_ack_preserves_failed_window_runtime_installed_after_enqueue() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    assert_ack_preserves_failed_window_replacement(temp.path(), true);
}

fn assert_expired_launch_ack_cleanup(
    preserve_successor: bool,
    conflicting_owner: bool,
    startup_timeout: bool,
    disabled_monitor: bool,
) {
    use gwt::project_runtime::{
        events::{OwnerCommand, OwnerEvent},
        launch_payload::OwnerLaunchMetadata,
        test_support::OwnerLaunchStage,
    };
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (root, owner, delivery_id) =
        owner_monitor_fixture(temp.path(), 42, true, &["pane-contract-fixture"]);
    if preserve_successor {
        let old = owner_monitor_launch(&root, &owner, 42, "agent", delivery_id.clone());
        super::pane_control_tests::control_close(&owner, &old);
        let successor = super::pane_control_tests::control_owner_launch(
            &owner,
            "agent",
            super::pane_control_tests::control_launch_config(&root, 43),
        );
        assert_eq!(old.id, successor.id);
        assert_ne!(old.incarnation, successor.incarnation);
        assert_ne!(
            old.session.as_ref().unwrap().id,
            successor.session.as_ref().unwrap().id
        );
        let before =
            gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&root))
                .unwrap();
        assert!(
            owner_fixture_block_on(owner.command(OwnerCommand::AcknowledgeMonitorLaunch {
                id: old.id.clone(),
                incarnation: old.incarnation,
                issue_number: 42,
                delivery_id
            }))
            .is_err()
        );
        let current =
            owner_fixture_block_on(owner.probe(&successor.id, successor.incarnation)).unwrap();
        assert!(Arc::ptr_eq(&current.pty, &successor.pty));
        assert!(successor.pty.try_wait().unwrap().is_none());
        let after =
            gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&root))
                .unwrap();
        assert_eq!(after.failed_issues, before.failed_issues);
        assert_eq!(after.pending_effects, before.pending_effects);
        assert_eq!(after.requeue_audit, before.requeue_audit);
        super::pane_control_tests::control_close(&owner, &successor);
        owner_fixture_block_on(owner.shutdown());
        return;
    }
    if startup_timeout {
        let pane = owner_monitor_launch(&root, &owner, 42, "agent", delivery_id);
        let mut events = owner.subscribe();
        let due = pane
            .monitor_startup_at
            .expect("actual owner startup deadline");
        owner_fixture_block_on(owner.check_monitor_deadlines(due)).unwrap();
        let retired = owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline Bound the real owner startup-failure commit and exact physical retirement; the existing deadline is explicitly advanced above.
            tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
                loop {
                    if let OwnerEvent::WindowRemoved {
                        id, incarnation, ..
                    } = events.recv().await.unwrap()
                    {
                        if id == pane.id && incarnation == pane.incarnation {
                            break;
                        }
                    }
                }
            })
            .await
        });
        let remained = owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_ok();
        let finalized = gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir()
                .join(format!("{}.toml", pane.session.as_ref().unwrap().id)),
        )
        .unwrap();
        let monitor = owner_monitor_read(&root, 42);
        if remained {
            super::pane_control_tests::control_close(&owner, &pane);
        }
        owner_fixture_block_on(owner.shutdown());
        assert!(
            retired.is_ok() && !remained,
            "startup timeout must retire its exact actual window and child"
        );
        assert_eq!(finalized.status, gwt_agent::AgentStatus::Stopped);
        assert!(!finalized.restore_window_on_startup);
        assert_eq!(monitor.attempt_count(42), 0);
        assert_eq!(
            monitor.inbox_item(42).unwrap().state,
            gwt::MonitorInboxState::Queued
        );
        assert!(monitor
            .inbox_item(42)
            .unwrap()
            .error_message
            .as_ref()
            .unwrap()
            .contains("startup timed out after 180s"));
        return;
    }
    let checkpoint = owner_fixture_block_on(
        owner.pause_next_agent_launch_at(OwnerLaunchStage::MaterializedBeforeInstall),
    )
    .unwrap();
    let id = owner.pane_id("agent");
    let mut feedback = issue_monitor_feedback(42);
    feedback.issue_monitor_project_root = Some(root.clone());
    feedback.issue_monitor_delivery_id = delivery_id.clone();
    let mut config = super::pane_control_tests::control_launch_config(&root, 42);
    config.launch_route = gwt_agent::LaunchRoute::Autonomous;
    let command = OwnerCommand::LaunchAgent {
        id: id.clone(),
        operation_id: "late-monitor-launch".into(),
        tab_id: id.split_once("::").unwrap().0.into(),
        config: Box::new(config),
        metadata: Box::new(OwnerLaunchMetadata {
            launch_feedback_context: Some(feedback),
            ..Default::default()
        }),
        cols: 80,
        rows: 24,
    };
    let client = owner.client();
    let home = temp.path().to_path_buf();
    let launch = thread::spawn(move || {
        let _home = ScopedGwtHome::set(home);
        owner_fixture_block_on(client.command(command))
    });
    let captured = owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bound the actual materialized Session checkpoint while the real actor launch worker is paused before PTY installation.
        tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, checkpoint.wait())
            .await
            .unwrap()
    });
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&root);
    let expired = if disabled_monitor {
        gwt::daemon_publisher::publish_durable_issue_monitor_control(
            &root,
            gwt::runtime_daemon_events::issue_monitor_payload(
                "control",
                serde_json::json!({"config_set":{"enabled":false}}),
                std::process::id(),
            ),
        )
        .unwrap();
        let disabled = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert!(!disabled.enabled);
        assert!(disabled.pending_launch_deliveries.is_empty());
        Some(disabled)
    } else if conflicting_owner {
        gwt::daemon_publisher::publish_durable_issue_monitor_control(
            &root,
            gwt::runtime_daemon_events::issue_monitor_payload(
                "control",
                serde_json::json!({"launched":{"issue_number":43,"window_id":id}}),
                std::process::id(),
            ),
        )
        .unwrap();
        None
    } else {
        // The real scan commit rebases and expires at real now. Age only this
        // exact fixture delivery so expiry survives that canonical recheck.
        let created_at = (Utc::now() - chrono::Duration::seconds(121)).to_rfc3339();
        gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
            let delivery = prefs
                .pending_launch_deliveries
                .iter_mut()
                .find(|pending| Some(&pending.delivery_id) == delivery_id.as_ref())
                .expect("the exact paused materializer delivery");
            assert_eq!(delivery.issue_number, 42);
            assert_eq!(
                delivery.materializer_window_id.as_deref(),
                Some(id.as_str())
            );
            delivery.created_at = created_at.clone();
            prefs
                .launching_issues
                .iter_mut()
                .find(|launch| launch.issue_number == 42)
                .unwrap()
                .claimed_at = Some(created_at.clone());
            Ok(())
        })
        .unwrap();
        owner_fixture_block_on(
            owner.expire_monitor_windowless_launches_at(Utc::now().to_rfc3339()),
        )
        .unwrap();
        let expired = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert!(
            expired.pending_launch_deliveries.is_empty(),
            "the real daemon expired its windowless delivery"
        );
        assert!(expired.launching_issues.is_empty());
        Some(expired)
    };
    let mut events = owner.subscribe();
    checkpoint.release();
    let receipt = launch.join().unwrap().unwrap();
    let incarnation = receipt["incarnation"].as_u64().unwrap();
    assert_eq!(incarnation, captured.incarnation);
    let acknowledgement =
        owner_fixture_block_on(owner.command(OwnerCommand::AcknowledgeMonitorLaunch {
            id: id.clone(),
            incarnation,
            issue_number: 42,
            delivery_id,
        }));
    if !disabled_monitor {
        assert!(acknowledgement.is_err());
    }
    if !conflicting_owner && !disabled_monitor {
        let _ = owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline Bound actual late-ACK physical cleanup after the canonical daemon has already committed delivery expiry.
            tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
                loop {
                    if let OwnerEvent::WindowRemoved {
                        id: closed,
                        incarnation: closed_incarnation,
                        ..
                    } = events.recv().await.unwrap()
                    {
                        if closed == id && closed_incarnation == incarnation {
                            break;
                        }
                    }
                }
            })
            .await
        });
    }
    let remaining = owner_fixture_block_on(owner.probe(&id, incarnation)).ok();
    let physical_alive = disabled_monitor
        && remaining
            .as_ref()
            .is_some_and(|pane| pane.pty.try_wait().unwrap().is_none());
    let finalized = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir()
            .join(format!("{}.toml", captured.session.as_ref().unwrap().id)),
    )
    .unwrap();
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    if disabled_monitor {
        owner_fixture_block_on(owner.command(OwnerCommand::Close {
            id: id.clone(),
            incarnation,
        }))
        .expect("the exact materialized fixture is closed before its Monitor driver drains");
    } else if let Some(pane) = &remaining {
        super::pane_control_tests::control_close(&owner, pane);
    }
    owner_fixture_block_on(owner.shutdown());
    assert_eq!(
        remaining.is_some(),
        conflicting_owner || disabled_monitor,
        "expiry alone cleans the exact owner; Monitor OFF retains its physical pane: {acknowledgement:?}"
    );
    if disabled_monitor {
        assert!(acknowledgement.is_ok(), "{acknowledgement:?}");
        assert!(
            physical_alive,
            "Monitor OFF must not stop the materialized PTY"
        );
        assert_ne!(finalized.status, gwt_agent::AgentStatus::Stopped);
        assert!(!prefs.enabled);
        assert!(prefs.pending_launch_deliveries.is_empty());
        assert!(prefs.launched_issues.is_empty());
    } else if conflicting_owner {
        assert_eq!(
            prefs.pending_launch_deliveries.len(),
            1,
            "refusal keeps the unconsumed delivery"
        );
        assert!(prefs
            .launched_issues
            .iter()
            .any(|entry| entry.issue_number == 43 && entry.window_id == id));
    } else {
        assert_eq!(finalized.status, gwt_agent::AgentStatus::Stopped);
        assert!(!finalized.restore_window_on_startup);
        let expired = expired.unwrap();
        assert_eq!(prefs.failed_issues, expired.failed_issues);
        assert_eq!(prefs.pending_effects, expired.pending_effects);
        assert_eq!(prefs.requeue_audit, expired.requeue_audit);
        assert!(prefs.pending_launch_deliveries.is_empty());
        assert!(
            prefs.launching_issues.is_empty(),
            "late ACK changed expired launch rows: before={:?}, after={:?}",
            expired.launching_issues,
            prefs.launching_issues
        );
        assert!(prefs.launched_issues.is_empty());
    }
}

#[test]
fn issue_5289_expired_launch_ack_closes_late_window_and_session() {
    assert_expired_launch_ack_cleanup(false, false, false, false);
}

#[test]
fn issue_5289_expired_launch_ack_preserves_successor_runtime() {
    assert_expired_launch_ack_cleanup(true, false, false, false);
}

#[test]
fn issue_5415_refused_duplicate_ack_preserves_the_bound_window() {
    assert_expired_launch_ack_cleanup(false, true, false, false);
}

#[test]
fn issue_5415_post_ack_startup_timeout_requeues_the_exact_window_without_reusing_delivery_id() {
    assert_expired_launch_ack_cleanup(false, false, true, false);
}

#[test]
fn issue_4735_monitor_off_before_ack_preserves_the_materialized_owner() {
    assert_expired_launch_ack_cleanup(false, false, false, true);
}

#[test]
fn issue_5642_consumed_delivery_ack_replay_preserves_the_exact_live_pane() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (root, owner, delivery_id) =
        owner_monitor_fixture(temp.path(), 42, true, &["pane-contract-fixture"]);
    let pane = owner_monitor_launch(&root, &owner, 42, "agent", delivery_id.clone());
    let session_id = pane.session.as_ref().unwrap().id.clone();
    let session_path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    let status_before_ack = gwt_agent::Session::load(&session_path).unwrap().status;
    let acknowledged =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&root)).unwrap();
    assert!(acknowledged.pending_launch_deliveries.is_empty());
    assert_eq!(
        acknowledged.launch_confirmations[&42].delivery_id,
        delivery_id
    );
    // The first real daemon ACK committed but the caller lost its reply.
    owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::AcknowledgeMonitorLaunch {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
            issue_number: 42,
            delivery_id,
        },
    ))
    .unwrap();
    let current = owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).unwrap();
    let session_after_ack = gwt_agent::Session::load(&session_path).unwrap();
    let prefs_after_ack =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&root)).unwrap();
    assert!(
        Arc::ptr_eq(&current.pane, &pane.pane),
        "the exact physical parser remains installed"
    );
    assert!(
        Arc::ptr_eq(&current.pty, &pane.pty),
        "the exact physical PTY remains installed"
    );
    assert_eq!(current.session.as_ref().unwrap().id, session_id);
    assert_eq!(session_after_ack.status, status_before_ack);
    assert_eq!(
        prefs_after_ack.launch_confirmations,
        acknowledged.launch_confirmations
    );
    assert_eq!(
        prefs_after_ack.launched_issues,
        acknowledged.launched_issues
    );
    assert_eq!(
        prefs_after_ack.launch_bindings,
        acknowledged.launch_bindings
    );
    assert_eq!(prefs_after_ack.requeue_audit, acknowledged.requeue_audit);
    assert!(prefs_after_ack.pending_launch_deliveries.is_empty());
    super::pane_control_tests::control_close(&owner, &pane);
    owner_fixture_block_on(owner.shutdown());
}

#[test]
fn issue_5289_launch_worker_finishes_while_pm_prefs_lock_is_held() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("project");
    fs::create_dir_all(&root).unwrap();
    init_repo_with_initial_commit(&root);
    seed_owner_fixture_roster(&root, 8);
    let owner = owner_fixture_block_on(
        gwt::project_runtime::test_support::ProjectOwnerHarness::start(&root),
    )
    .unwrap();
    configure_owner_fixture_start_gate(&owner);
    let config = super::pane_control_tests::control_launch_config(&root, 42);
    let prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(&root);
    gwt::pm_registry::save_pm_prefs(&prefs_path, &Default::default()).unwrap();
    let lock = gwt_core::operation_deadline::NamedFileLock::acquire(
        &prefs_path.with_extension("lock"),
        "pm_prefs",
    )
    .unwrap();
    let (completed, completion) = mpsc::channel();
    let client = owner.client();
    let id = owner.pane_id("agent");
    let command = gwt::project_runtime::events::OwnerCommand::LaunchAgent {
        id: id.clone(),
        operation_id: "pm-prefs-independent".into(),
        tab_id: id.split_once("::").unwrap().0.into(),
        config: Box::new(config),
        metadata: Box::default(),
        cols: 80,
        rows: 24,
    };
    let home = temp.path().to_path_buf();
    let worker = thread::spawn(move || {
        let _home = ScopedGwtHome::set(home);
        completed
            .send(owner_fixture_block_on(client.command(command)))
            .unwrap();
    });
    // test-hygiene: allow-wall-clock-deadline Bound the actual owner launch IPC while the PM preference file lock is deliberately held.
    let completed_while_locked = completion.recv_timeout(gwt_core::deadline_budget::HANG_GUARD);
    lock.unlock().unwrap();
    worker.join().unwrap();
    let receipt = completed_while_locked
        .expect("pm_prefs must not pin actual launch preparation")
        .unwrap();
    let pane =
        owner_fixture_block_on(owner.probe(&id, receipt["incarnation"].as_u64().unwrap())).unwrap();
    assert!(
        pane.pty.try_wait().unwrap().is_none(),
        "the actual launch installs its live child within the budget"
    );
    super::pane_control_tests::control_close(&owner, &pane);
    owner_fixture_block_on(owner.shutdown());
}

/// Issue #4802 AC-2: a launch into a worktree that already has a live agent
/// pane is refused. The Monitor delivery is acknowledged onto the running pane
/// instead, so the Issue ends up bound to one pane rather than accumulating a
/// fresh one per relaunch (#4758 collected sixteen). A finished pane does not
/// block a launch.
#[test]
fn issue_monitor_delivery_into_a_worktree_with_a_live_agent_pane_adopts_it() {
    use gwt::project_runtime::{
        events::{OwnerCommand, OwnerEvent},
        launch_payload::OwnerLaunchMetadata,
    };
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Assert delivery admission and replay outcomes, rather than native Git latency.
    let _projection_timeout = ScopedEnvVar::set(
        "GWT_TEST_ISSUE_MONITOR_FALLBACK_PROJECTION_TIMEOUT_MS",
        "10000",
    );
    for (case, label, pane_status, expect_adoption, pending_identity) in [
        (
            "running",
            "running pane",
            WindowProcessStatus::Running,
            true,
            false,
        ),
        ("idle", "idle pane", WindowProcessStatus::Idle, true, false),
        (
            "stopped",
            "stopped pane",
            WindowProcessStatus::Stopped,
            false,
            false,
        ),
        (
            "pre-session",
            "pre-session Monitor pane",
            WindowProcessStatus::Starting,
            true,
            true,
        ),
    ] {
        let temp = tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path());
        let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
        let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
        let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
        let mut fixture = monitor_relaunch_fixture(
            temp.path(),
            case,
            MonitorProviderConversationFixture::Present,
            MonitorNativeHolderFixture::None,
            true,
        );
        let owner = start_monitor_relaunch_owner(&mut fixture, false);
        let verified = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let repo = fixture.project_root.clone();
            let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
            let delivery_id = fixture.delivery_id.clone().unwrap();
            let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
                .working_dir(&fixture.worktree)
                .branch(git_stdout(&fixture.worktree, &["branch", "--show-current"]))
                .base_branch(git_stdout(&repo, &["branch", "--show-current"]))
                .model("gpt-5.5")
                .reasoning_level("high")
                .skip_permissions(true)
                .build();
            config.linked_issue_number = Some(3165);
            config.explicit_follow_up = true;
            config.launch_route = if pending_identity {
                gwt_agent::LaunchRoute::Autonomous
            } else {
                gwt_agent::LaunchRoute::Manual
            };
            if !expect_adoption {
                // Keep the installed native command and its health probes intact;
                // only this fixture provider's input loop reaches a real exit.
                let (environment, _) = gwt_agent::LaunchEnvironment::from_active_profile(
                    fixture.runtime.profile_config_path.as_ref().unwrap(),
                    gwt_agent::LaunchRuntimeTarget::Host,
                )
                .unwrap()
                .into_parts();
                let bin =
                    std::env::split_paths(std::ffi::OsStr::new(environment.get("PATH").unwrap()))
                        .next()
                        .unwrap();
                assert!(bin.ends_with("fixture-runner-bin"));
                let runner = bin.join(if cfg!(windows) { "codex.cmd" } else { "codex" });
                let original = fs::read_to_string(&runner).unwrap();
                let (input_loop, exit_gate) = if cfg!(windows) {
                    ("[Console]::TreatControlCAsInput = $true; while ($null -ne [Console]::ReadLine()) {}",
                    "[Console]::WriteLine('GWT_ADOPT_EXIT_READY'); $null = [Console]::ReadLine(); exit 1")
                } else {
                    (
                        "while IFS= read -r line; do :; done",
                        "stty -echo; printf 'GWT_ADOPT_EXIT_READY\\n'; IFS= read -r line; exit 1",
                    )
                };
                assert_eq!(original.matches(input_loop).count(), 1);
                fs::write(&runner, original.replacen(input_loop, exit_gate, 1)).unwrap();
            }
            let mut metadata = OwnerLaunchMetadata::default();
            if pending_identity {
                let mut feedback = issue_monitor_feedback(3165);
                feedback.issue_monitor_project_root = Some(repo.clone());
                feedback.issue_monitor_delivery_id = Some(delivery_id.clone());
                feedback.issue_monitor_session_mode = Some(config.session_mode);
                metadata.launch_feedback_context = Some(feedback);
            }
            let owner_id = owner.pane_id("native-holder");
            let receipt = owner_fixture_block_on(owner.command(OwnerCommand::LaunchAgent {
                id: owner_id.clone(),
                operation_id: format!("native-adoption-holder-{case}"),
                tab_id: owner_id.split_once("::").unwrap().0.into(),
                config: Box::new(config),
                metadata: Box::new(metadata),
                cols: 80,
                rows: 24,
            }))
            .unwrap_or_else(|error| {
                let cached = gwt::issue_monitor_worker::load_cached_issue_monitor_candidate(
                    &issue_cache_root(&repo),
                    3165,
                );
                let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                panic!(
                    "the actual native holder is admitted before its delivery replay: {error}; cached={cached:?}; pending={:?}",
                    prefs.pending_launch_deliveries
                );
            });
            let pane = owner_fixture_block_on(
                owner.probe(&owner_id, receipt["incarnation"].as_u64().unwrap()),
            )
            .unwrap();
            assert_eq!(receipt["session_id"], pane.session.as_ref().unwrap().id);
            assert_eq!(
                pane.session.as_ref().unwrap().agent_id,
                gwt_agent::AgentId::Codex
            );
            assert!(pane.pending_fresh_execution.is_some());
            let holder_sid = pane.session.as_ref().unwrap().id.clone();
            let holder_pid = pane.pty.process_id();
            assert!(holder_pid.is_some());
            if !pending_identity {
                authenticate_monitor_owner_readiness(&owner, &pane);
            }
            if expect_adoption && !pending_identity {
                // Live hooks deliberately retain a recoverable native exit. The
                // finished-holder variant instead follows its real DiagnosticExit.
                let mut hook = runtime_hook_state_for_event(
                    if pane_status == WindowProcessStatus::Idle {
                        "idle"
                    } else {
                        "running"
                    },
                    "PostToolUse",
                    &holder_sid,
                );
                hook.project_root = Some(repo.display().to_string());
                hook.branch = Some(pane.session.as_ref().unwrap().branch.clone());
                hook.agent_session_id
                    .clone_from(&pane.session.as_ref().unwrap().agent_session_id);
                hook.occurred_at = Utc::now().to_rfc3339();
                assert!(owner_fixture_block_on(pane.runtime_hook(&hook))
                    .unwrap()
                    .is_success());
            }
            let pane = owner_fixture_block_on(owner.probe(&owner_id, pane.incarnation)).unwrap();
            fixture.runtime.tab_mut("tab-1").unwrap().workspace = WindowRegistry::from_persisted(
                gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(
                    &repo,
                ))
                .unwrap(),
            );
            let raw = owner_id.split_once("::").unwrap().1;
            fixture.runtime.register_window("tab-1", raw);
            let live_window_id = combined_window_id("tab-1", raw);
            let context = fixture.runtime.test_context();
            let owner_identity = owner.client().identity().unwrap().clone();
            let state = fixture.runtime.project_state_mut(&context).unwrap();
            state.project_owner_connection_id = 1;
            state.project_owner_identity = Some(owner_identity.clone());
            let snapshot = owner_fixture_block_on(owner.command(OwnerCommand::Snapshot {
                id: owner_id.clone(),
                incarnation: pane.incarnation,
            }))
            .unwrap();
            fixture.runtime.install_remote_owner_projection(
                &context,
                &live_window_id,
                owner.client(),
                serde_json::from_value(snapshot).unwrap(),
                (!pending_identity).then(|| pane.session.as_ref().unwrap().clone()),
            );
            if !pending_identity {
                fixture
                    .runtime
                    .launch_wizard_cache
                    .record_session(pane.session.as_ref().unwrap().clone());
            } else {
                assert!(
                    fixture
                        .runtime
                        .launch_wizard_cache
                        .session_by_id(&holder_sid)
                        .is_none(),
                    "the GUI has not read the Prepared Session ledger yet"
                );
                assert_eq!(
                    fixture.runtime.active_agent_sessions[&live_window_id].session_id, holder_sid,
                    "the owner snapshot still supplies its exact pending Session identity"
                );
                let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                assert!(!prefs.launch_confirmations.contains_key(&3165),
                "a real Prepared Monitor pane cannot acknowledge before authenticated readiness");
            }
            let mut observed = owner.subscribe();
            if !expect_adoption {
                owner_fixture_block_on(async {
                    // test-hygiene: allow-wall-clock-deadline Actual provider output establishes the exit gate; no scheduler delay is asserted.
                    tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
                        loop {
                            let snapshot = owner
                                .command(OwnerCommand::Snapshot {
                                    id: owner_id.clone(),
                                    incarnation: pane.incarnation,
                                })
                                .await
                                .unwrap();
                            if snapshot["screen"]
                                .as_str()
                                .is_some_and(|screen| screen.contains("GWT_ADOPT_EXIT_READY"))
                            {
                                break;
                            }
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                    owner
                        .command(OwnerCommand::Input {
                            id: owner_id.clone(),
                            incarnation: pane.incarnation,
                            data: if cfg!(windows) { b"exit\r" } else { b"exit\n" }.to_vec(),
                        })
                        .await
                        .unwrap();
                    // test-hygiene: allow-wall-clock-deadline Exact Closed follows the owner's exit-confirmed status and completed process-tree reap; the status alone can precede teardown.
                    tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
                    let mut exit_confirmed = false;
                    loop {
                        let event = observed.recv().await.unwrap();
                        exit_confirmed |= matches!(&event, OwnerEvent::Status { id, incarnation, exit_confirmed: true, .. }
                            if id == &owner_id && *incarnation == pane.incarnation);
                        let closed = matches!(&event, OwnerEvent::Closed { id, incarnation, error: None, .. }
                            if id == &owner_id && *incarnation == pane.incarnation);
                        fixture.runtime.handle_project_owner_event(&context, 1, &owner_identity, event);
                        if closed { break; }
                    }
                    assert!(exit_confirmed, "the exact native provider exit precedes its Closed acknowledgement");
                }).await.unwrap();
                });
                assert!(pane.pty.tree_exit_confirmed());
                assert!(pane.pty.try_wait().unwrap().is_some());
                assert!(!fixture
                    .runtime
                    .issue_monitor_window_is_live(&live_window_id));
                assert!(fixture.runtime.tabs[0]
                    .workspace
                    .persisted()
                    .windows
                    .iter()
                    .any(|window| window.id == raw
                        && window.session_id.as_deref() == Some(holder_sid.as_str())));
            } else if !pending_identity {
                assert_eq!(
                    fixture.runtime.window_status(&live_window_id),
                    Some(pane_status)
                );
            }
            let runtime = &mut fixture.runtime;
            assert!(
                runtime
                    .issue_monitor_window_snapshot_for_tab(
                        "tab-1",
                        &chrono::Utc::now().to_rfc3339()
                    )
                    .is_none(),
                "the GUI cannot republish the native owner's panes under tab aliases"
            );
            let issued_session = pane.session.as_ref().unwrap();
            assert_eq!(
                gwt::cli::execution_state::session_launch_route_from_session(issued_session)
                    == gwt_agent::LaunchRoute::Autonomous,
                pending_identity,
                "{label}: only the actual Monitor launch route owns this observation"
            );
            assert_eq!(
                issued_session.linked_issue_number,
                Some(3165),
                "{label}: actual Issue identity is observable"
            );
            let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
            let physical = inventory["panes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|current| current["id"] == owner_id);
            if expect_adoption {
                let physical = physical.expect("the native owner observes the exact live holder");
                assert_eq!(physical["session_id"], holder_sid);
                assert_eq!(physical["incarnation"], pane.incarnation);
                assert_eq!(physical["child_pid"], holder_pid.unwrap());
            } else {
                assert!(
                    physical.is_none(),
                    "the finished diagnostic row is not a live holder"
                );
            }

            let (ack_spawner, ack_tasks) = BlockingTaskSpawner::queued();
            runtime.blocking_tasks = ack_spawner;
            let mut events = runtime.auto_launch_issue_monitor_delivery_events_for_project(
                &repo,
                3165,
                LinkedIssueKind::Spec,
                Some(delivery_id.clone()),
                gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
            );
            // The facts completion queues the separate adoption RPC. Keep it queued
            // while proving a genuine pre-readiness holder has not acknowledged.
            drain_queued_blocking_tasks(&ack_tasks);
            events.extend(runtime.handle_issue_monitor_launch_prepared(
                take_issue4803_monitor_preparation(&fixture.recorded_events),
            ));
            if pending_identity {
                let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                assert!(!prefs.launch_confirmations.contains_key(&3165));
                let inventory =
                    owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
                assert_eq!(
                    inventory["panes"].as_array().unwrap().len(),
                    1,
                    "the pre-Session replay does not install a second physical pane"
                );
                authenticate_monitor_owner_readiness(&owner, &pane);
            }
            events.extend(commit_issue_monitor_preparations(
                runtime,
                &ack_tasks,
                &fixture.recorded_events,
            ));
            if !expect_adoption {
                let new_id = runtime
                    .runtimes
                    .iter()
                    .find(|(_, projection)| projection.owner_pane_id != owner_id)
                    .map(|(_, projection)| {
                        (projection.owner_pane_id.clone(), projection.incarnation)
                    })
                    .unwrap();
                let new_pane = owner_fixture_block_on(owner.probe(&new_id.0, new_id.1)).unwrap();
                if new_pane.pending_fresh_execution.is_some() {
                    authenticate_monitor_owner_readiness(&owner, &new_pane);
                }
            }
            events.extend(runtime.finish_queued_delivery_acks(&ack_tasks));
            let agent_windows = runtime.tabs[0]
                .workspace
                .persisted()
                .windows
                .iter()
                .filter(|window| window.preset == WindowPreset::Agent)
                .count();
            let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
            if expect_adoption {
                assert!(
                    serde_json::to_value(&prefs).unwrap()["launch_confirmations"]["3165"]["tier"]
                        .is_null(),
                    "adopting a pane must not attribute a newly selected profile to it"
                );
            }
            let reloaded =
                test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs.clone());
            if expect_adoption {
                assert_eq!(agent_windows, 1, "{label}: no second pane is opened");
                assert!(
                    events.iter().any(|event| matches!(
                        &event.event,
                        BackendEvent::IssueMonitorToast { message, .. }
                            if message.contains("did not open a second pane")
                    )),
                    "{label}: the refusal is reported: {events:?}; prefs={prefs:?}"
                );
                assert!(
                    prefs.pending_launch_deliveries.is_empty(),
                    "{label}: the delivery is acknowledged onto the live pane"
                );
                assert_eq!(
                    reloaded.launched_window_issue(&owner_id),
                    Some(3165),
                    "{label}: the Issue is bound to the pane already working it"
                );
                let retained =
                    owner_fixture_block_on(owner.probe(&owner_id, pane.incarnation)).unwrap();
                assert_eq!(retained.session.as_ref().unwrap().id, holder_sid);
                assert_eq!(retained.pty.process_id(), holder_pid);
                assert!(retained.pty.try_wait().unwrap().is_none());
                let inventory =
                    owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
                assert_eq!(
                    inventory["panes"].as_array().unwrap().len(),
                    1,
                    "{label}: no second actual PTY"
                );
                assert!(inventory["pending"].as_array().unwrap().is_empty());
            } else {
                assert_eq!(
                    agent_windows, 2,
                    "{label}: a finished pane does not block a launch"
                );
                assert!(
                    runtime.tabs[0]
                        .workspace
                        .persisted()
                        .windows
                        .iter()
                        .any(|window| window.id == raw
                            && window.session_id.as_deref() == Some(holder_sid.as_str())),
                    "the finished native holder's diagnostic row survives the new launch"
                );
                assert!(pane.pty.tree_exit_confirmed());
            }
        }));
        let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
        let panes = inventory["panes"].as_array().unwrap().clone();
        for pane in panes {
            owner_fixture_block_on(owner.command(OwnerCommand::Close {
                id: pane["id"].as_str().unwrap().into(),
                incarnation: pane["incarnation"].as_u64().unwrap(),
            }))
            .unwrap();
        }
        fixture.runtime.test_project_owners.clear();
        let owner = Arc::try_unwrap(owner)
            .ok()
            .expect("the actual fixture releases its owner handles");
        owner_fixture_block_on(owner.shutdown());
        if let Err(error) = verified {
            std::panic::resume_unwind(error);
        }
    }
}

#[test]
fn durable_delivery_fallback_commit_budget_is_an_explicit_runtime_dependency() {
    use gwt::project_runtime::{events::OwnerCommand, installation::IssueMonitorLaunchClaimInput};
    use gwt::runtime_daemon_events::IssueMonitorControlPublishError;
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "budgeted-durable-delivery",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    let owner = start_monitor_relaunch_owner_with_monitor(&mut fixture, false, false);
    eprintln!("budget fallback: actual private owner started");
    let repo = fixture.project_root.clone();
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let delivery_id = fixture.delivery_id.clone().unwrap();
    let issue = fixture.execution_owner.number;
    let identity = owner.client().identity().unwrap().clone();
    let operation_id = format!("monitor-delivery:{delivery_id}");
    let owner_pane_id = owner.pane_id(
        &gwt::project_runtime::continuation::manual_successor_stable_component(
            "project-pane",
            &operation_id,
        ),
    );
    let recorded_events = fixture.recorded_events.clone();
    let execution_owner = fixture.execution_owner;
    let source_session_id = fixture.source_session_id.clone();
    let native_conversation_id = fixture.native_conversation_id.clone();
    let runtime = &mut fixture.runtime;
    let context = runtime.test_context();
    assert_eq!(
        runtime.issue_monitor_fallback_commit_timeout,
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
        "the test fixture must pin its own fallback commit budget",
    );
    assert!(
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT
            > super::super::ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT,
        "the test budget must not inherit the production GUI-thread budget",
    );
    assert!(matches!(
        gwt::load_issue_monitor_authority_fence(&prefs_path).unwrap(),
        gwt::IssueMonitorAuthorityFenceState::Missing
    ));
    let original = fs::read(&prefs_path).unwrap();
    // Keep the real runtime field connected to its production commit helper.
    // The owner has its own budget; GUI canvas events cannot prove a claim.
    runtime.issue_monitor_fallback_commit_timeout = Duration::ZERO;
    let error = runtime
        .commit_local_issue_monitor_control(&context, |monitor| {
            monitor.claim_launch_delivery(
                issue,
                &delivery_id,
                &identity.host_instance_id,
                identity.pid,
                &owner_pane_id,
                gwt::process::is_host_process_alive,
            )
        })
        .expect_err("the runtime's exhausted commit budget must reject its claim");
    assert!(
        error
            .to_string()
            .contains(gwt_core::operation_deadline::DEADLINE_EXPIRED_MARKER),
        "{error}"
    );
    assert_eq!(fs::read(&prefs_path).unwrap(), original);
    let mut claim = IssueMonitorLaunchClaimInput {
        project_root: repo.clone(),
        issue_number: issue,
        delivery_id: delivery_id.clone(),
        materializer_window_id: owner_pane_id.clone(),
        materializer_id: identity.host_instance_id.clone(),
        materializer_pid: identity.pid,
        fallback_timeout: Duration::ZERO,
        snapshot: runtime.issue_monitor_window_snapshot_for_tab("tab-1", &Utc::now().to_rfc3339()),
        snapshot_tabs: std::collections::BTreeSet::from(["tab-1".into()]),
    };
    let unavailable = || {
        IssueMonitorControlPublishError::TransportUnavailable(
            "this physical owner has no Monitor transport".into(),
        )
    };
    let error = claim
        .claim_with_publishers(|_, _| Err(unavailable()), |_| Err(unavailable()))
        .expect_err("an exhausted canonical fallback must reject the same claim");
    assert!(
        error
            .to_string()
            .contains(gwt_core::operation_deadline::DEADLINE_EXPIRED_MARKER),
        "{error}"
    );
    assert_eq!(fs::read(&prefs_path).unwrap(), original);
    assert!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("reload prefs")
            .pending_launch_deliveries
            .iter()
            .any(|delivery| delivery.delivery_id == delivery_id
                && delivery.materializer_window_id.is_none()),
        "a starved claim must leave the delivery unbound and replayable",
    );
    let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
    assert!(inventory["panes"].as_array().unwrap().is_empty());
    assert!(inventory["pending"].as_array().unwrap().is_empty());
    assert!(runtime.tabs[0].workspace.persisted().windows.is_empty());
    runtime.issue_monitor_fallback_commit_timeout =
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT;
    let (_, accepted) = runtime
        .commit_local_issue_monitor_control(&context, |monitor| {
            monitor.claim_launch_delivery(
                issue,
                &delivery_id,
                &identity.host_instance_id,
                identity.pid,
                &owner_pane_id,
                gwt::process::is_host_process_alive,
            )
        })
        .expect("the explicit runtime budget must commit its exact claim");
    assert!(accepted);
    claim.fallback_timeout = runtime.issue_monitor_fallback_commit_timeout;
    assert!(claim
        .claim_with_publishers(|_, _| Err(unavailable()), |_| Err(unavailable()))
        .expect("the same tuple replays through the canonical explicit budget"));
    let claimed = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    let delivery = claimed
        .pending_launch_deliveries
        .iter()
        .find(|delivery| delivery.delivery_id == delivery_id)
        .unwrap();
    assert_eq!(
        delivery.materializer_id.as_deref(),
        Some(identity.host_instance_id.as_str())
    );
    assert_eq!(delivery.materializer_pid, Some(identity.pid));
    assert_eq!(
        delivery.materializer_window_id.as_deref(),
        Some(owner_pane_id.as_str())
    );
    let scope = gwt_core::daemon::RuntimeScope::from_project_root(
        &repo,
        gwt_core::daemon::RuntimeTarget::Host,
    )
    .unwrap();
    let endpoint_path = scope.endpoint_path(&gwt_core::paths::gwt_home());
    let endpoint_bytes = fs::read(&endpoint_path).unwrap();
    eprintln!("budget fallback: positive canonical claim pane={owner_pane_id}");
    let verified = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let checkpoint = owner_fixture_block_on(owner.pause_next_agent_launch_at(
            gwt::project_runtime::test_support::OwnerLaunchStage::MaterializedBeforeInstall,
        ))
        .unwrap();
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = spawner;
        let mut replay = runtime.auto_launch_issue_monitor_delivery_events_for_project(
            &repo,
            issue,
            LinkedIssueKind::Spec,
            Some(delivery_id.clone()),
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        eprintln!("budget fallback: facts queued");
        drain_queued_blocking_tasks(&tasks);
        let prepared = take_issue4803_monitor_preparation(&recorded_events);
        replay.extend(runtime.handle_issue_monitor_launch_prepared(prepared));
        eprintln!("budget fallback: actual owner launch queued");
        let launch_tasks = tasks.clone();
        let home = temp.path().to_path_buf();
        let (finished, finish) = tokio::sync::oneshot::channel();
        let worker = thread::spawn(move || {
            let _home = ScopedGwtHome::set(home);
            drain_queued_blocking_tasks(&launch_tasks);
            let _ = finished.send(());
        });
        let captured = owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline The real launch checkpoint orders the discovery fault before its first Monitor marker publication.
            tokio::time::timeout(Duration::from_secs(30), async {
                tokio::select! {
                    evidence = checkpoint.wait() => Some(evidence),
                    _ = finish => None,
                }
            })
            .await
        });
        eprintln!(
            "budget fallback: checkpoint={:?}",
            captured.as_ref().map(|evidence| evidence
                .as_ref()
                .map(|evidence| (&evidence.id, evidence.incarnation)))
        );
        let corrupted = if matches!(&captured, Ok(Some(_))) {
            fs::write(&endpoint_path, b"{\"pid\":")
        } else {
            Ok(())
        };
        checkpoint.release();
        let joined = worker.join();
        for event in recorded_events.lock().unwrap().iter() {
            if let UserEvent::RemoteOwnerLaunchComplete(completion) =
                recorded_project_payload(event)
            {
                eprintln!(
                    "budget fallback: completion window={} result={:?}",
                    completion.window_id,
                    completion.result.as_ref().map(std::mem::discriminant)
                );
            }
        }
        captured
            .expect("the same preclaimed private owner must materialize its Session")
            .expect("the actual launch worker completed before its Session checkpoint");
        corrupted.expect("inject only the budget fixture's malformed discovery descriptor");
        joined.expect("actual budgeted owner launch worker");
        replay.extend(commit_issue_monitor_preparations(
            runtime,
            &tasks,
            &recorded_events,
        ));
        eprintln!("budget fallback: actual owner launch completion dispatched");
        assert!(replay.iter().any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
        "the same delivery replays into exactly one window once the budget is explicit: {replay:?}");
        assert_eq!(
            runtime.tabs[0]
                .workspace
                .persisted()
                .windows
                .iter()
                .filter(|window| window.preset == WindowPreset::Agent)
                .count(),
            1,
        );
        let alias = runtime
            .runtimes
            .iter()
            .find_map(|(alias, projection)| {
                (projection.owner_pane_id == owner_pane_id).then(|| alias.clone())
            })
            .unwrap_or_else(|| {
                panic!("the exact budgeted owner launch must project its runtime: {replay:?}")
            });
        let pane = owner_runtime_probe(runtime, &alias);
        assert_eq!(pane.id, owner_pane_id);
        let session_id = pane.session.as_ref().unwrap().id.clone();
        let child_pid = pane.pty.process_id().unwrap();
        let still_pending = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        let exact_pending = still_pending
            .pending_launch_deliveries
            .iter()
            .any(|delivery| {
                delivery.delivery_id == delivery_id
                    && delivery.materializer_window_id.as_deref() == Some(owner_pane_id.as_str())
                    && delivery.materializer_id.as_deref()
                        == Some(identity.host_instance_id.as_str())
                    && delivery.materializer_pid == Some(identity.pid)
            });
        let exact_ack =
            still_pending
                .launch_confirmations
                .get(&issue)
                .is_some_and(|confirmation| {
                    confirmation.window_id == pane.id
                        && confirmation.delivery_id.as_deref() == Some(delivery_id.as_str())
                })
                && still_pending.launch_bindings.get(&pane.id) == Some(&issue);
        assert!(
            exact_pending || exact_ack,
            "the same actual owner tuple must remain pending or be exactly ACKed"
        );
        if pane.pending_fresh_execution.is_some() {
            assert!(exact_pending);
            assert!(!still_pending.launch_confirmations.contains_key(&issue));
            authenticate_monitor_owner_readiness(&owner, &pane);
        } else {
            let session = pane.session.as_ref().unwrap();
            assert_eq!(session.id, source_session_id);
            assert_eq!(session.session_mode, gwt_agent::SessionMode::Resume);
            assert_eq!(
                session.exact_resume_session_id(),
                Some(native_conversation_id.as_str())
            );
            let binding = session.execution_binding.as_ref().unwrap();
            let grant = pane.grant.as_ref().unwrap();
            assert_eq!(grant.principal().active_execution_binding(), Some(binding));
            assert!(pane.issuer.grant_is_current(grant));
            assert!(
                gwt::cli::execution_state::current_active_execution_binding_matches(
                    &session.worktree_path,
                    execution_owner,
                    &session.id,
                    &binding.identity,
                )
                .unwrap(),
                "the actual resumed Session must already hold Active authority"
            );
        }
        eprintln!("budget fallback: actual readiness or exact Active resume verified");
        let acknowledged =
            owner_fixture_block_on(owner.command(OwnerCommand::AcknowledgeMonitorLaunch {
                id: pane.id.clone(),
                incarnation: pane.incarnation,
                issue_number: issue,
                delivery_id: Some(delivery_id.clone()),
            }))
            .unwrap();
        assert_eq!(acknowledged["acknowledged"], true);
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert!(prefs.pending_launch_deliveries.is_empty());
        assert_eq!(prefs.launch_bindings.get(&pane.id), Some(&issue));
        assert_eq!(
            prefs.launch_confirmations[&issue].delivery_id.as_deref(),
            Some(delivery_id.as_str())
        );
        let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
        assert_eq!(inventory["panes"].as_array().unwrap().len(), 1);
        assert_eq!(inventory["panes"][0]["session_id"], session_id);
        assert_eq!(inventory["panes"][0]["child_pid"], child_pid);
        assert!(pane.pty.try_wait().unwrap().is_none());
    }));
    if let Err(panic) = &verified {
        eprintln!(
            "budget fallback: caught assertion={:?}",
            panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
        );
    }
    eprintln!(
        "budget fallback: assertion_ok={}; discovery remains unavailable; reading cleanup inventory",
        verified.is_ok()
    );
    let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
    for row in inventory["panes"].as_array().unwrap() {
        if row["id"] == owner_pane_id {
            eprintln!(
                "budget fallback: closing exact incarnation {}",
                row["incarnation"]
            );
            owner_fixture_block_on(owner.command(OwnerCommand::Close {
                id: owner_pane_id.clone(),
                incarnation: row["incarnation"].as_u64().unwrap(),
            }))
            .expect("drain only this exact budgeted owner incarnation");
        }
    }
    fs::write(endpoint_path, endpoint_bytes)
        .expect("restore only the budget fixture's endpoint fault");
    runtime.runtimes.clear();
    runtime.test_project_owners.remove(&repo);
    eprintln!("budget fallback: exact close settled; shutting down owner");
    owner_fixture_block_on(
        Arc::try_unwrap(owner)
            .ok()
            .expect("drained budgeted owner")
            .shutdown(),
    );
    verified.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[test]
fn issue_5642_materializing_replay_preserves_the_unconsumed_initial_launch() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "initial-worker-lifetime",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    let delivery_id = fixture.delivery_id.clone().expect("durable delivery");
    prepare_monitor_relaunch(
        &mut fixture,
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
    );
    // Keep the genuine owner's successful reply queued. The GUI's immutable
    // pending operation still owns its placeholder until that reply is applied.
    let completion = take_remote_owner_launch_completion(&fixture.recorded_events);
    let (owner_id, incarnation, session_id, child_pid) = match &completion.result {
        Ok(super::super::RemoteOwnerLaunchOutcome::Runtime(snapshot, _)) => (
            snapshot.id.clone(),
            snapshot.incarnation,
            snapshot
                .session_id
                .clone()
                .expect("actual owner Session ID"),
            snapshot.child_pid.expect("actual native child"),
        ),
        Err(error) => panic!("initial actual owner launch refused: {error}"),
        Ok(super::super::RemoteOwnerLaunchOutcome::MonitorFailureClosed { detail }) => {
            panic!("initial actual Monitor launch closed: {detail}")
        }
        Ok(super::super::RemoteOwnerLaunchOutcome::Failed { failure, .. }) => {
            panic!("initial actual owner launch failed: {}", failure.detail)
        }
        _ => panic!("initial actual owner launch returned a terminal outcome"),
    };
    let pane = owner_fixture_block_on(owner.probe(&owner_id, incarnation)).unwrap();
    assert_eq!(pane.session.as_ref().unwrap().id, session_id);
    assert_eq!(pane.pty.process_id(), Some(child_pid));
    assert!(pane.pty.try_wait().unwrap().is_none());
    assert!(
        fixture.runtime.runtimes.is_empty(),
        "the GUI reply remains unconsumed"
    );
    let window_id = match fixture
        .runtime
        .issue_monitor_launch_deliveries
        .get_mut(&delivery_id)
        .expect("materializing delivery")
    {
        super::super::IssueMonitorLaunchDeliveryState::Materializing {
            window_id,
            started_at,
        } => {
            // test-hygiene: allow-wall-clock-deadline inject an elapsed TTL without sleeping; worker completion determines replay ordering.
            *started_at = Instant::now() - Duration::from_secs(61);
            window_id.clone()
        }
        state => panic!("expected materializing delivery, got {state:?}"),
    };
    let generation = fixture.runtime.window_lifecycle_generations.lock().unwrap()[&window_id];
    let pending = &fixture.runtime.pending_launch_completions[&window_id];
    let operation_id = pending.operation_id.clone();
    let current = pending.current.clone();
    let inflight = fixture.runtime.inflight_launches.clone();
    let feedback = fixture.runtime.pending_launch_feedback_contexts[&window_id]
        .issue_monitor_delivery_id
        .clone();
    fixture.runtime.auto_launch_issue_monitor_delivery_events(
        &fixture.runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some(delivery_id),
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
    );
    let generation_after = fixture
        .runtime
        .window_lifecycle_generations
        .lock()
        .unwrap()
        .get(&window_id)
        .copied();
    let inflight_after = fixture.runtime.inflight_launches.clone();
    let feedback_after = fixture
        .runtime
        .pending_launch_feedback_contexts
        .get(&window_id)
        .and_then(|feedback| feedback.issue_monitor_delivery_id.clone());
    let agent_windows = fixture.runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    let replay_operation = fixture
        .runtime
        .pending_launch_completions
        .get(&window_id)
        .map(|pending| pending.operation_id.clone());
    let current_before_refresh = current.load(std::sync::atomic::Ordering::Acquire);
    let inventory = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    let initial_worker_before_refresh = fixture
        .runtime
        .pending_launch_completions
        .contains_key(&window_id);
    let tab_id = fixture.runtime.tabs[0].id.clone();
    fixture.runtime.refresh_project_tab_incarnation(&tab_id);
    let initial_worker_after_refresh = fixture
        .runtime
        .pending_launch_completions
        .contains_key(&window_id);
    let (spawner, cleanup_tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .handle_remote_owner_launch_complete(completion);
    fixture
        .runtime
        .close_window_after_issue_monitor_finalize_events(&window_id);
    drain_queued_blocking_tasks(&cleanup_tasks);
    let surviving = owner_fixture_block_on(owner.probe(&owner_id, incarnation)).ok();
    if let Some(surviving) = &surviving {
        pane_control_tests::control_close(&owner, surviving);
    }
    fixture
        .runtime
        .test_project_owners
        .remove(&fixture.project_root);
    owner_fixture_block_on(
        Arc::try_unwrap(owner)
            .ok()
            .expect("drained pending owner")
            .shutdown(),
    );

    assert_eq!(
        generation_after,
        Some(generation),
        "a queued initial completion still owns its original window generation"
    );
    assert_eq!(
        inflight_after, inflight,
        "the original launch remains in flight"
    );
    assert_eq!(feedback_after, feedback);
    assert_eq!(agent_windows, 1);
    assert_eq!(replay_operation, Some(operation_id));
    assert!(current_before_refresh);
    assert!(!current.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(inventory["panes"].as_array().unwrap().len(), 1);
    assert_eq!(inventory["panes"][0]["session_id"], session_id);
    assert_eq!(inventory["panes"][0]["child_pid"], child_pid);
    assert!(initial_worker_before_refresh);
    assert!(
        !initial_worker_after_refresh,
        "project incarnation refresh must retire its initial launch worker tracking"
    );
    assert!(fixture.runtime.initial_launch_workers.is_empty());
    assert!(
        surviving.is_none(),
        "a stale GUI reply must cancel only its actual pending owner operation"
    );
    assert!(pane.pty.try_wait().unwrap().is_some());
}

#[test]
fn durable_issue_monitor_delivery_preserves_live_materializer_and_replays_after_it_stops() {
    use gwt::project_runtime::{
        client::ProjectRuntimeClient,
        events::{OwnerCommand, OwnerEvent},
    };
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(root) = std::env::var_os("GWT_TEST_LIVE_MATERIALIZER_ROOT") {
        // A different real Host must release its GUI lifetime and exit before
        // another materializer can acquire this durable delivery.
        let root = PathBuf::from(root);
        let _home = ScopedGwtHome::set(PathBuf::from(
            std::env::var_os("GWT_TEST_LIVE_MATERIALIZER_HOME").unwrap(),
        ));
        let owner = owner_fixture_block_on(
            gwt::project_runtime::test_support::ProjectOwnerHarness::start_with_monitor(&root),
        )
        .unwrap();
        configure_owner_fixture_start_gate(&owner);
        owner_fixture_block_on(owner.set_issue_client_factory(Arc::new(|_, _| {
            Ok(Arc::new(FakeIssueClient::new()) as Arc<dyn IssueClient>)
        })))
        .unwrap();
        owner_fixture_block_on(owner.wait_monitor_idle()).unwrap();
        let mut observed = owner.subscribe();
        fs::write(root.join("live-materializer-ready"), b"ready").unwrap();
        let mut stop = String::new();
        std::io::stdin().read_line(&mut stop).unwrap();
        let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
        let panes = inventory["panes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                owner_fixture_block_on(owner.probe(
                    row["id"].as_str().unwrap(),
                    row["incarnation"].as_u64().unwrap(),
                ))
                .unwrap()
            })
            .collect::<Vec<_>>();
        let identities = panes.iter().map(|pane| serde_json::json!({
            "id": pane.id, "incarnation": pane.incarnation,
            "session_id": pane.session.as_ref().unwrap().id,
            "child_pid": pane.pty.process_id().unwrap(),
            "child_started_at": gwt::process::host_process_start_time(pane.pty.process_id().unwrap()).unwrap(),
        })).collect::<Vec<_>>();
        owner_fixture_block_on(owner.shutdown());
        for pane in &panes {
            assert!(
                pane.pty.tree_exit_confirmed(),
                "GuiLifetime must reap its captured tree"
            );
            assert!(pane.pty.try_wait().unwrap().is_some());
        }
        let mut closed = Vec::new();
        while let Ok(event) = observed.try_recv() {
            if matches!(
                &event,
                OwnerEvent::Status { .. } | OwnerEvent::Closed { .. }
            ) {
                closed.push(event);
            }
        }
        assert!(panes
            .iter()
            .all(|pane| closed.iter().any(|event| matches!(event,
            OwnerEvent::Closed { id, incarnation, error: None, .. }
                if id == &pane.id && *incarnation == pane.incarnation))));
        fs::write(
            root.join("live-materializer-drained.json"),
            serde_json::to_vec(&serde_json::json!({ "panes": identities, "events": closed }))
                .unwrap(),
        )
        .unwrap();
        return;
    }
    // Assert delivery admission and replay outcomes, rather than native Git latency.
    let _projection_timeout = ScopedEnvVar::set(
        "GWT_TEST_ISSUE_MONITOR_FALLBACK_PROJECTION_TIMEOUT_MS",
        "10000",
    );
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "live-materializer",
        MonitorProviderConversationFixture::Missing,
        MonitorNativeHolderFixture::None,
        true,
    );
    // Reuse the installed-provider/profile setup before any physical launch.
    // The empty private owner relinquishes discovery before the real child binds.
    let bootstrap = start_monitor_relaunch_owner_with_monitor(&mut fixture, false, false);
    fixture.runtime.test_project_owners.clear();
    owner_fixture_block_on(Arc::try_unwrap(bootstrap).ok().unwrap().shutdown());
    let repo = fixture.project_root.clone();
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let delivery_id = fixture.delivery_id.clone().unwrap();
    let mut child = gwt_core::process::hidden_command(std::env::current_exe().unwrap())
        .args([
            "app_runtime::tests::issue_monitor_claim_tests::durable_issue_monitor_delivery_preserves_live_materializer_and_replays_after_it_stops",
            "--exact", "--nocapture",
        ])
        .env("GWT_TEST_LIVE_MATERIALIZER_ROOT", &repo)
        .env("GWT_TEST_LIVE_MATERIALIZER_HOME", temp.path())
        .env("HOME", temp.path()).env("USERPROFILE", temp.path())
        .stdin(std::process::Stdio::piped())
        .spawn().unwrap();
    let host_pid = child.id();
    let live = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let waiting_since = Instant::now();
        while !repo.join("live-materializer-ready").exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "the actual materializer failed to bind"
            );
            // test-hygiene: allow-wall-clock-deadline The real child publishes readiness after binding its authenticated transport.
            assert!(waiting_since.elapsed() < gwt_core::deadline_budget::HANG_GUARD);
            thread::sleep(Duration::from_millis(100));
        }
        let client = owner_fixture_block_on(ProjectRuntimeClient::from_current_project(
            &repo,
            &gwt_core::paths::gwt_home(),
        ))
        .unwrap();
        let identity = client.identity().unwrap().clone();
        assert_eq!(identity.pid, host_pid);
        assert_ne!(identity.pid, std::process::id());
        let context = fixture.runtime.test_context();
        let state = fixture.runtime.project_state_mut(&context).unwrap();
        state.project_owner_connection_id = 1;
        state.project_owner_identity = Some(identity.clone());
        fixture.runtime.issue_monitor_launch_deliveries.insert(
            delivery_id.clone(),
            super::super::IssueMonitorLaunchDeliveryState::Materializing {
                window_id: "tab-1::missing-materializer".into(),
                started_at: Instant::now(),
            },
        );
        let mut replay = prepare_monitor_relaunch(
            &mut fixture,
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        let completion = take_remote_owner_launch_completion(&fixture.recorded_events);
        assert!(
            matches!(
                &completion.result,
                Ok(super::super::RemoteOwnerLaunchOutcome::Runtime(..))
            ),
            "the real native materializer must be accepted: {:?}",
            completion.result.as_ref().map(std::mem::discriminant)
        );
        let window_id = completion.window_id.clone();
        replay.extend(
            fixture
                .runtime
                .handle_remote_owner_launch_complete(completion),
        );
        assert!(
            replay
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "a vanished materializing window must not suppress exact delivery replay"
        );
        assert_eq!(
            fixture.runtime.tabs[0]
                .workspace
                .persisted()
                .windows
                .iter()
                .filter(|window| window.preset == WindowPreset::Agent)
                .count(),
            1
        );
        assert!(
            !fixture
                .runtime
                .initial_launch_workers
                .contains_key(&window_id),
            "the actual completion retires its exact initial worker"
        );
        let projection = &fixture.runtime.runtimes[&window_id];
        let owner_id = projection.owner_pane_id.clone();
        let incarnation = projection.incarnation;
        let snapshot = owner_fixture_block_on(client.command(OwnerCommand::Snapshot {
            id: owner_id.clone(),
            incarnation,
        }))
        .unwrap();
        let session_id = snapshot["session_id"].as_str().unwrap().to_string();
        let child_pid = u32::try_from(snapshot["child_pid"].as_u64().unwrap()).unwrap();
        let child_started_at = gwt::process::host_process_start_time(child_pid).unwrap();
        assert!(!snapshot["exit_confirmed"].as_bool().unwrap());
        let claimed = owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline The owner commits its canonical layout marker after the launch reply; compare replay at that durable phase.
            tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, async {
                loop {
                    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
                    if prefs.pending_launch_deliveries.iter().any(|delivery| {
                        delivery.delivery_id == delivery_id
                            && delivery.workspace_durable_window_id.as_deref()
                                == Some(owner_id.as_str())
                    }) {
                        break prefs;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("the actual materializer must commit its exact durable layout marker")
        });
        let delivery = claimed
            .pending_launch_deliveries
            .iter()
            .find(|delivery| delivery.delivery_id == delivery_id)
            .unwrap();
        assert_eq!(
            delivery.materializer_id.as_deref(),
            Some(identity.host_instance_id.as_str())
        );
        assert_eq!(delivery.materializer_pid, Some(host_pid));
        assert_eq!(
            delivery.materializer_window_id.as_deref(),
            Some(owner_id.as_str())
        );
        assert_eq!(
            delivery.materialized_window_id.as_deref(),
            Some(owner_id.as_str())
        );
        assert_eq!(
            delivery.workspace_durable_window_id.as_deref(),
            Some(owner_id.as_str())
        );
        assert!(
            !claimed.launch_confirmations.contains_key(&3165),
            "the Prepared native child has not acknowledged readiness"
        );
        match fixture
            .runtime
            .issue_monitor_launch_deliveries
            .get_mut(&delivery_id)
            .unwrap()
        {
            super::super::IssueMonitorLaunchDeliveryState::Materializing {
                window_id: live_id,
                started_at,
            } => {
                assert_eq!(live_id, &window_id);
                *started_at = Instant::now() - Duration::from_secs(61);
            }
            state => panic!("expected materializing delivery, got {state:?}"),
        }
        let waiting_replay = fixture.runtime.auto_launch_issue_monitor_delivery_events(
            &context,
            3165,
            LinkedIssueKind::Spec,
            Some(delivery_id.clone()),
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        assert!(
            waiting_replay.is_empty(),
            "a materializer with a live PTY must keep its delivery slot while awaiting input"
        );
        assert!(fixture.runtime.runtimes.contains_key(&window_id));
        assert!(
            matches!(fixture.runtime.issue_monitor_launch_deliveries.get(&delivery_id),
            Some(super::super::IssueMonitorLaunchDeliveryState::Materializing { window_id: live_id, .. })
                if live_id == &window_id)
        );
        let inventory = owner_fixture_block_on(client.command(OwnerCommand::ListPanes)).unwrap();
        assert_eq!(inventory["panes"].as_array().unwrap().len(), 1);
        assert!(inventory["pending"].as_array().unwrap().is_empty());
        assert_eq!(inventory["panes"][0]["session_id"], session_id);
        assert_eq!(inventory["panes"][0]["child_pid"], child_pid);
        assert_eq!(inventory["panes"][0]["incarnation"], incarnation);
        assert_eq!(
            gwt::load_issue_monitor_prefs(&prefs_path)
                .unwrap()
                .pending_launch_deliveries,
            claimed.pending_launch_deliveries
        );
        (
            context,
            identity,
            window_id,
            owner_id,
            incarnation,
            session_id,
            child_pid,
            child_started_at,
        )
    }));
    // EOF requests normal GuiLifetime shutdown even if a phase assertion failed.
    drop(child.stdin.take());
    let status = child.wait().unwrap();
    let (
        context,
        identity,
        live_window_id,
        owner_id,
        incarnation,
        session_id,
        child_pid,
        child_started_at,
    ) = match live {
        Ok(live) => live,
        Err(error) => std::panic::resume_unwind(error),
    };
    assert!(
        status.success(),
        "the actual materializer must finish its GUI lifetime"
    );
    assert!(!gwt::process::is_host_process_alive(host_pid));
    assert!(!gwt::process::exact_pty_process_tree_is_alive(
        child_pid,
        child_started_at
    ));
    let drained: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.join("live-materializer-drained.json")).unwrap())
            .unwrap();
    assert_eq!(drained["panes"].as_array().unwrap().len(), 1);
    assert_eq!(drained["panes"][0]["id"], owner_id);
    assert_eq!(drained["panes"][0]["incarnation"], incarnation);
    assert_eq!(drained["panes"][0]["session_id"], session_id);
    assert_eq!(drained["panes"][0]["child_pid"], child_pid);
    assert_eq!(drained["panes"][0]["child_started_at"], child_started_at);
    for event in serde_json::from_value::<Vec<OwnerEvent>>(drained["events"].clone()).unwrap() {
        fixture
            .runtime
            .handle_project_owner_event(&context, 1, &identity, event);
    }
    assert!(!fixture
        .runtime
        .issue_monitor_window_is_live(&live_window_id));
    assert!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .pending_launch_deliveries
            .iter()
            .any(|delivery| delivery.delivery_id == delivery_id
                && delivery.materializer_id.as_deref() == Some(identity.host_instance_id.as_str())),
        "GuiLifetime drain preserves the same unacknowledged durable delivery"
    );
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    let replacement = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_ne!(owner.client().identity().unwrap(), &identity);
        let state = fixture.runtime.project_state_mut(&context).unwrap();
        state.project_owner_connection_id = 2;
        state.project_owner_identity = owner.client().identity().cloned();
        let replay = prepare_monitor_relaunch(
            &mut fixture,
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        let (_, successor, expired_replay) = finish_monitor_relaunch_owner(&mut fixture, replay);
        let successor = successor.expect("the reaped materializer permits exact delivery replay");
        assert!(
            expired_replay
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "an expired materializing window must be replaced by exact delivery replay"
        );
        assert_eq!(
            fixture.runtime.tabs[0]
                .workspace
                .persisted()
                .windows
                .iter()
                .filter(|window| window.preset == WindowPreset::Agent)
                .count(),
            1
        );
        let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
        assert_eq!(inventory["panes"].as_array().unwrap().len(), 1);
        assert_eq!(
            inventory["panes"][0]["session_id"],
            successor.session.as_ref().unwrap().id
        );
        assert_eq!(
            inventory["panes"][0]["child_pid"],
            successor.pty.process_id().unwrap()
        );
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert!(prefs
            .pending_launch_deliveries
            .iter()
            .any(|delivery| delivery.delivery_id == delivery_id
                && delivery.materializer_id.as_deref()
                    == Some(owner.client().identity().unwrap().host_instance_id.as_str())
                && delivery.materializer_window_id.as_deref() == Some(successor.id.as_str())));
        if successor.pending_fresh_execution.is_some() {
            authenticate_monitor_owner_readiness(&owner, &successor);
        }
    }));
    let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
    for pane in inventory["panes"].as_array().unwrap() {
        owner_fixture_block_on(owner.command(OwnerCommand::Close {
            id: pane["id"].as_str().unwrap().into(),
            incarnation: pane["incarnation"].as_u64().unwrap(),
        }))
        .unwrap();
    }
    fixture.runtime.runtimes.clear();
    fixture.runtime.test_project_owners.clear();
    owner_fixture_block_on(Arc::try_unwrap(owner).ok().unwrap().shutdown());
    if let Err(error) = replacement {
        std::panic::resume_unwind(error);
    }
}

#[test]
fn competing_issue_monitor_subscribers_materialize_one_durable_delivery() {
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
        "competing-subscribers",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    let repo = fixture.project_root.clone();
    let delivery_id = fixture.delivery_id.clone().unwrap();
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    let tab_b = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime_b, events_b) =
        sample_runtime_with_events(&temp.path().join("gui-b"), vec![tab_b], Some("tab-1"));
    runtime_b.sessions_dir = fixture.sessions_dir.clone();
    runtime_b.profile_config_path = fixture.runtime.profile_config_path.clone();
    runtime_b.launch_wizard_cache = fixture.runtime.launch_wizard_cache.clone();
    runtime_b
        .test_project_owners
        .insert(repo.clone(), owner.clone());
    let runtime_a = &mut fixture.runtime;
    runtime_a.issue_monitor_materializer_id = "gui-a".to_string();
    runtime_b.issue_monitor_materializer_id = "gui-b".to_string();
    let (spawner_a, tasks_a) = BlockingTaskSpawner::queued();
    let (spawner_b, tasks_b) = BlockingTaskSpawner::queued();
    runtime_a.blocking_tasks = spawner_a;
    runtime_b.blocking_tasks = spawner_b;
    let mut first = runtime_a.auto_launch_issue_monitor_delivery_events(
        &runtime_a.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some(delivery_id.clone()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    first.extend(commit_issue_monitor_preparations(
        runtime_a,
        &tasks_a,
        &fixture.recorded_events,
    ));
    let window_id = runtime_a
        .runtimes
        .keys()
        .next()
        .expect("first owner projection")
        .clone();
    let pane = owner_runtime_probe(runtime_a, &window_id);
    let session = pane.session.as_ref().unwrap();
    let session_id = session.id.clone();
    let child_pid = pane.pty.process_id().unwrap();
    let pending_readiness = pane.pending_fresh_execution.is_some();
    if !pending_readiness {
        let binding = session
            .execution_binding
            .as_ref()
            .expect("actual producing Session");
        assert_eq!(binding.session_id, session_id);
        assert_eq!(
            gwt::cli::execution_state::current_execution_binding(
                &session.worktree_path,
                fixture.execution_owner,
            )
            .unwrap(),
            Some(binding.identity.clone())
        );
        assert_eq!(
            gwt::cli::execution_state::diagnose_owner(
                &session.worktree_path,
                fixture.execution_owner,
            )
            .ecr_status,
            Some(gwt::cli::execution_state::ExecutionControlStatus::Active)
        );
        assert!(pane
            .grant
            .as_ref()
            .unwrap()
            .principal()
            .authorizes_producing_mutation());
    }
    let before_ready = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::AcknowledgeMonitorLaunch {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
            issue_number: 3165,
            delivery_id: Some(delivery_id.clone()),
        },
    ))
    .unwrap();
    assert_eq!(before_ready["acknowledged"], !pending_readiness);
    let first_claim = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    let first_inventory = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    let mut second = runtime_b.auto_launch_issue_monitor_delivery_events(
        &runtime_b.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some(delivery_id.clone()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    second.extend(commit_issue_monitor_preparations(
        &mut runtime_b,
        &tasks_b,
        &events_b,
    ));
    let second_inventory = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    let non_owner_failure = runtime_b.issue_monitor_launch_failed_delivery_events(
        Some(&repo),
        3165,
        "non-owner validation failed",
        Some(&delivery_id),
    );
    drain_queued_blocking_tasks(&tasks_a);
    drain_queued_blocking_tasks(&tasks_b);
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("load claimed delivery");
    let inventory = owner_fixture_block_on(
        owner.command(gwt::project_runtime::events::OwnerCommand::ListPanes),
    )
    .unwrap();
    let second_is_same_owner = runtime_b.runtimes.values().all(|projection| {
        projection.owner_pane_id == pane.id
            && projection.incarnation == pane.incarnation
            && projection.child_pid == Some(child_pid)
            && owner.client().identity() == Some(&projection.owner_identity)
    }) && runtime_b
        .active_agent_sessions
        .values()
        .all(|active| active.session_id == session_id);
    let physical_windows = runtime_a
        .runtimes
        .values()
        .chain(runtime_b.runtimes.values())
        .map(|projection| (projection.owner_pane_id.clone(), projection.incarnation))
        .collect::<HashSet<_>>();
    if pending_readiness {
        authenticate_monitor_owner_readiness(&owner, &pane);
    }
    for physical in inventory["panes"].as_array().unwrap() {
        owner_fixture_block_on(
            owner.command(gwt::project_runtime::events::OwnerCommand::Close {
                id: physical["id"].as_str().unwrap().into(),
                incarnation: physical["incarnation"].as_u64().unwrap(),
            }),
        )
        .unwrap();
    }
    runtime_a.runtimes.clear();
    runtime_b.runtimes.clear();
    runtime_a.test_project_owners.remove(&repo);
    runtime_b.test_project_owners.remove(&repo);
    let materializer_id = owner.client().identity().unwrap().host_instance_id.clone();
    owner_fixture_block_on(
        Arc::try_unwrap(owner)
            .ok()
            .expect("drained shared owner")
            .shutdown(),
    );
    assert!(first
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })));
    assert!(
        second_is_same_owner,
        "a competing GUI cannot install an independent runtime: {second:?}"
    );
    assert!(!non_owner_failure
        .iter()
        .any(|event| matches!(event.event, BackendEvent::IssueMonitorLaunchFailed { .. })));
    assert_eq!(physical_windows.len(), 1);
    assert_eq!(first_inventory["panes"].as_array().unwrap().len(), 1);
    assert_eq!(second_inventory["panes"].as_array().unwrap().len(), 1,
        "a second subscriber must preserve the accepted producer: events={second:?}; before={first_inventory:?}; after={second_inventory:?}");
    assert_eq!(inventory["panes"].as_array().unwrap().len(), 1);
    assert_eq!(inventory["panes"][0]["session_id"], session_id);
    assert_eq!(inventory["panes"][0]["child_pid"], child_pid);
    assert_eq!(
        persisted.pending_launch_deliveries,
        first_claim.pending_launch_deliveries
    );
    assert_eq!(
        persisted.launch_confirmations,
        first_claim.launch_confirmations
    );
    assert_eq!(persisted.launch_bindings, first_claim.launch_bindings);
    assert!(!persisted
        .failed_issues
        .iter()
        .any(|failed| failed.issue_number == 3165));
    if pending_readiness {
        assert_eq!(
            persisted.pending_launch_deliveries[0]
                .materializer_id
                .as_deref(),
            Some(materializer_id.as_str())
        );
        assert_eq!(
            persisted.pending_launch_deliveries[0]
                .materializer_window_id
                .as_deref(),
            Some(pane.id.as_str())
        );
        assert_eq!(
            persisted.pending_launch_deliveries[0].delivery_id,
            delivery_id
        );
    } else {
        assert!(persisted.pending_launch_deliveries.is_empty());
        assert_eq!(persisted.launch_confirmations[&3165].window_id, pane.id);
        assert_eq!(
            persisted.launch_confirmations[&3165].delivery_id.as_deref(),
            Some(delivery_id.as_str())
        );
        assert_eq!(persisted.launch_bindings.get(&pane.id), Some(&3165));
    }
    assert!(pane.pty.try_wait().unwrap().is_some());
}

#[test]
fn durable_issue_monitor_delivery_restart_recovers_only_exact_bound_window() {
    use gwt::project_runtime::events::OwnerCommand;
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "durable-delivery-restart",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    // This recovery models a lost GUI materializer without a live Monitor
    // authority. Keep a real physical owner and the original allowed fallback.
    let owner = start_monitor_relaunch_owner_with_monitor(&mut fixture, false, false);
    eprintln!("restart fallback: actual private owner started");
    let repo = fixture.project_root.clone();
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    assert!(matches!(
        gwt::load_issue_monitor_authority_fence(&prefs_path).unwrap(),
        gwt::IssueMonitorAuthorityFenceState::Missing
    ));
    let delivery_id = fixture.delivery_id.clone().unwrap();
    let issue = fixture.execution_owner.number;
    let identity = owner.client().identity().unwrap().clone();
    let operation_id = format!("monitor-delivery:{delivery_id}");
    let expected_id = owner.pane_id(
        &gwt::project_runtime::continuation::manual_successor_stable_component(
            "project-pane",
            &operation_id,
        ),
    );
    let claim = gwt::project_runtime::installation::IssueMonitorLaunchClaimInput {
        project_root: repo.clone(),
        issue_number: issue,
        delivery_id: delivery_id.clone(),
        materializer_window_id: expected_id.clone(),
        materializer_id: identity.host_instance_id,
        materializer_pid: identity.pid,
        fallback_timeout: fixture.runtime.issue_monitor_fallback_commit_timeout,
        snapshot: fixture
            .runtime
            .issue_monitor_window_snapshot_for_tab("tab-1", &Utc::now().to_rfc3339()),
        snapshot_tabs: std::collections::BTreeSet::from(["tab-1".into()]),
    };
    let unavailable = || {
        gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
            "the original GUI claimed while no Monitor transport was available".into(),
        )
    };
    assert!(claim
        .claim_with_publishers(|_, _| Err(unavailable()), |_| Err(unavailable()))
        .unwrap());
    let scope = gwt_core::daemon::RuntimeScope::from_project_root(
        &repo,
        gwt_core::daemon::RuntimeTarget::Host,
    )
    .unwrap();
    let endpoint_path = scope.endpoint_path(&gwt_core::paths::gwt_home());
    let endpoint_bytes = fs::read(&endpoint_path).unwrap();
    let checkpoint = owner_fixture_block_on(owner.pause_next_agent_launch_at(
        gwt::project_runtime::test_support::OwnerLaunchStage::MaterializedBeforeInstall,
    ))
    .unwrap();
    let (spawner, launch_tasks) = BlockingTaskSpawner::queued();
    fixture.runtime.blocking_tasks = spawner;
    fixture
        .runtime
        .auto_launch_issue_monitor_delivery_events_for_project(
            &repo,
            issue,
            LinkedIssueKind::Spec,
            Some(delivery_id.clone()),
            gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
        );
    drain_queued_blocking_tasks(&launch_tasks);
    let prepared = take_issue4803_monitor_preparation(&fixture.recorded_events);
    let events = fixture
        .runtime
        .handle_issue_monitor_launch_prepared(prepared);
    eprintln!("restart fallback: exact preclaim pane={expected_id}; actual owner launch queued");
    let home = temp.path().to_path_buf();
    let (finished, finish) = tokio::sync::oneshot::channel();
    let worker = thread::spawn(move || {
        let _home = ScopedGwtHome::set(home);
        drain_queued_blocking_tasks(&launch_tasks);
        let _ = finished.send(());
    });
    let captured = owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bounds the actual admitted Session before its first physical/Monitor publication; the checkpoint establishes ordering.
        tokio::time::timeout(Duration::from_secs(30), async {
            tokio::select! {
                evidence = checkpoint.wait() => Some(evidence),
                _ = finish => None,
            }
        })
        .await
    });
    eprintln!(
        "restart fallback: checkpoint={:?}",
        captured.as_ref().map(|evidence| evidence
            .as_ref()
            .map(|evidence| (&evidence.id, evidence.incarnation)))
    );
    let corrupted = if matches!(&captured, Ok(Some(_))) {
        fs::write(&endpoint_path, b"{\"pid\":")
    } else {
        Ok(())
    };
    checkpoint.release();
    let joined = worker.join();
    for event in fixture.recorded_events.lock().unwrap().iter() {
        if let UserEvent::RemoteOwnerLaunchComplete(completion) = recorded_project_payload(event) {
            eprintln!(
                "restart fallback: completion window={} result={:?}",
                completion.window_id,
                completion.result.as_ref().map(std::mem::discriminant)
            );
        }
    }
    captured
        .expect("the exact preclaimed private owner must materialize its Session")
        .expect("the actual restart launch worker completed before its Session checkpoint");
    corrupted.expect("inject this fixture's malformed discovery descriptor");
    joined.expect("actual owner launch worker");
    let (_first_alias, pane, _) = finish_monitor_relaunch_owner(&mut fixture, events);
    let pane = match pane {
        Ok(pane) => pane,
        Err(error) => {
            fs::write(&endpoint_path, &endpoint_bytes)
                .expect("restore the discovery fault before draining a refused launch");
            let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
            for row in inventory["panes"].as_array().unwrap() {
                if row["id"] == expected_id {
                    owner_fixture_block_on(owner.command(OwnerCommand::Close {
                        id: expected_id.clone(),
                        incarnation: row["incarnation"].as_u64().unwrap(),
                    }))
                    .expect("drain only this refused restart launch's exact incarnation");
                }
            }
            fixture.runtime.runtimes.clear();
            fixture.runtime.test_project_owners.remove(&repo);
            owner_fixture_block_on(
                Arc::try_unwrap(owner)
                    .ok()
                    .expect("drained refused restart owner")
                    .shutdown(),
            );
            panic!("actual private owner native Session refused: {error}");
        }
    };
    assert_eq!(pane.id, expected_id);
    let session_id = pane.session.as_ref().unwrap().id.clone();
    let child_pid = pane.pty.process_id().unwrap();
    let verified = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eprintln!("restart fallback: actual Session/PID captured; beforeReady marker/ACK");
        let before_ready =
            owner_fixture_block_on(owner.command(OwnerCommand::AcknowledgeMonitorLaunch {
                id: pane.id.clone(),
                incarnation: pane.incarnation,
                issue_number: issue,
                delivery_id: Some(delivery_id.clone()),
            }))
            .unwrap();
        assert_eq!(before_ready["acknowledged"], false);
        eprintln!("restart fallback: beforeReady marker committed without ACK");
        let durable = gwt::load_workspace_state(&gwt::workspace_state_path(&repo)).unwrap();
        let raw = pane.id.split_once("::").unwrap().1;
        assert!(durable.windows.iter().any(|row| {
            row.id == raw && row.session_id.as_deref() == Some(session_id.as_str())
        }));
        // Reproduce only the interrupted durable-marker write. The real owner,
        // exact claim tuple, native Session and child all survive the GUI restart.
        gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
            let delivery = prefs
                .pending_launch_deliveries
                .iter_mut()
                .find(|delivery| delivery.delivery_id == delivery_id)
                .unwrap();
            assert_eq!(
                delivery.materializer_window_id.as_deref(),
                Some(pane.id.as_str())
            );
            assert_eq!(delivery.materializer_pid, Some(std::process::id()));
            delivery.workspace_durable_window_id = None;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            gwt::load_issue_monitor_prefs(&prefs_path)
                .expect("load pre-ACK delivery")
                .pending_launch_deliveries[0]
                .workspace_durable_window_id,
            None,
            "simulate a crash after durable workspace save but before its prefs marker",
        );
        fixture.runtime.runtimes.clear();
        let restored_workspace =
            gwt::load_restored_workspace_state(&repo).expect("restore workspace");
        let restored_tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WindowRegistry::from_persisted(restored_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let (mut restarted, recorded_events) =
            sample_runtime_with_events(temp.path(), vec![restored_tab], Some("tab-1"));
        restarted.sessions_dir = fixture.sessions_dir.clone();
        restarted.issue_monitor_materializer_id = "restarted-gui".to_string();
        let context = restarted.test_context();
        let identity = owner.client().identity().unwrap().clone();
        let state = restarted.project_state_mut(&context).unwrap();
        state.project_owner_connection_id = 1;
        state.project_owner_identity = Some(identity);
        let alias = combined_window_id("tab-1", raw);
        let snapshot = owner_fixture_block_on(owner.command(OwnerCommand::Snapshot {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
        }))
        .unwrap();
        restarted.install_remote_owner_projection(
            &context,
            &alias,
            owner.client(),
            serde_json::from_value(snapshot).unwrap(),
            pane.session.clone(),
        );
        let (ack_spawner, ack_tasks) = BlockingTaskSpawner::queued();
        restarted.blocking_tasks = ack_spawner;

        // Issue #3642: replay must ACK the exact saved pane despite endpoint corruption.
        let mut events = restarted.auto_launch_issue_monitor_delivery_events(
            &restarted.test_context(),
            issue,
            LinkedIssueKind::Spec,
            Some(delivery_id.clone()),
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        events.extend(commit_issue_monitor_preparations(
            &mut restarted,
            &ack_tasks,
            &recorded_events,
        ));
        assert!(!events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })));
        assert_eq!(
            restarted.tabs[0]
                .workspace
                .persisted()
                .windows
                .iter()
                .filter(|window| window.preset == WindowPreset::Agent)
                .count(),
            1
        );
        let mut ack_events = restarted.finish_queued_delivery_acks(&ack_tasks);
        let still_pending = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert!(still_pending
            .pending_launch_deliveries
            .iter()
            .any(|delivery| {
                delivery.delivery_id == delivery_id
                    && delivery.materializer_window_id.as_deref() == Some(pane.id.as_str())
            }));
        assert!(!still_pending.launch_confirmations.contains_key(&issue));
        authenticate_monitor_owner_readiness(&owner, &pane);
        eprintln!("restart fallback: actual readiness authenticated");
        let ready = owner_fixture_block_on(owner.command(OwnerCommand::AcknowledgeMonitorLaunch {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
            issue_number: issue,
            delivery_id: Some(delivery_id.clone()),
        }))
        .unwrap();
        assert_eq!(ready["acknowledged"], true);
        ack_events.extend(restarted.issue_monitor_launch_completed_delivery_events(
            &repo,
            issue,
            &alias,
            Some(&delivery_id),
        ));
        ack_events.extend(restarted.finish_queued_delivery_acks(&ack_tasks));
        assert!(
            !events.iter().chain(&ack_events).any(|event| matches!(
                &event.event,
                BackendEvent::IssueMonitorToast { level, .. } if level == "error"
            )),
            "error events cannot prove delivery ACK success: {events:?} {ack_events:?}"
        );
        let acknowledged = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload ACKed prefs");
        assert!(
            acknowledged.pending_launch_deliveries.is_empty(),
            "restart must ACK the exact saved pane: window={:?}, deliveries={:?}",
            restarted.tabs[0].workspace.persisted().windows,
            acknowledged.pending_launch_deliveries
        );
        assert_eq!(acknowledged.launch_bindings.get(&pane.id), Some(&issue));
        assert_eq!(
            acknowledged.launch_confirmations[&issue]
                .delivery_id
                .as_deref(),
            Some(delivery_id.as_str())
        );
        let exact = owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).unwrap();
        assert_eq!(exact.session.as_ref().unwrap().id, session_id);
        assert_eq!(exact.pty.process_id(), Some(child_pid));
        assert!(exact.pty.try_wait().unwrap().is_none());
        assert_eq!(restarted.runtimes[&alias].incarnation, pane.incarnation);
        let inventory = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes)).unwrap();
        assert_eq!(
            inventory["panes"].as_array().unwrap().len(),
            1,
            "restart replay cannot create a second PTY"
        );
        assert_eq!(inventory["panes"][0]["session_id"], session_id);
        assert_eq!(inventory["panes"][0]["child_pid"], child_pid);
        restarted.runtimes.clear();
    }));
    if let Err(panic) = &verified {
        eprintln!(
            "restart fallback: caught assertion={:?}",
            panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
        );
    }
    eprintln!(
        "restart fallback: discovery remains unavailable; closing exact incarnation {}",
        pane.incarnation
    );
    let closed = owner_fixture_block_on(owner.command(OwnerCommand::Close {
        id: pane.id.clone(),
        incarnation: pane.incarnation,
    }));
    fs::write(endpoint_path, endpoint_bytes).expect("restore only this fixture's endpoint fault");
    let reaped = pane.pty.try_wait();
    fixture.runtime.runtimes.clear();
    fixture.runtime.test_project_owners.remove(&repo);
    eprintln!("restart fallback: close_result={closed:?}; shutting down owner");
    owner_fixture_block_on(
        Arc::try_unwrap(owner)
            .ok()
            .expect("drained restarted delivery owner")
            .shutdown(),
    );
    verified.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    closed.expect("close the exact surviving native child");
    assert!(reaped.unwrap().is_some());
}

#[test]
fn issue_5642_delivery_replay_does_not_adopt_another_issue_reusing_its_window_id() {
    assert_delivery_replay_preserves_foreign_window(true);
}

#[test]
fn issue_5642_unmaterialized_delivery_replay_does_not_close_a_reused_window_id() {
    assert_delivery_replay_preserves_foreign_window(false);
}

fn assert_delivery_replay_preserves_foreign_window(materialized: bool) {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo_with_initial_commit(&repo);
    let mut tab = sample_project_tab(
        "tab-1",
        "Repo",
        repo.clone(),
        ProjectKind::Git,
        &[WindowPreset::Agent],
    );
    let raw_id = tab.workspace.persisted().windows[0].id.clone();
    tab.workspace.set_linked_issue_number(&raw_id, Some(43));
    let window_id = combined_window_id("tab-1", &raw_id);
    let (mut runtime, _) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let mut monitor = test_monitor(gwt::IssueMonitorConfig {
        enabled: true,
        max_active: 3,
        ..Default::default()
    });
    let now = Utc::now().to_rfc3339();
    monitor.terminal_queue_push(&[42], "operator", &now);
    monitor.record_candidate(gwt::IssueMonitorIssue {
        author_login: None,
        number: 42,
        title: "Replayed launch reservation".into(),
        labels: Vec::new(),
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(42, "claim-42", "host/session", "effect-42", &now));
    let delivery_id = "launch:effect-42";
    assert!(monitor.claim_launch_delivery(
        42,
        delivery_id,
        &runtime.issue_monitor_materializer_id,
        std::process::id(),
        &window_id,
        |_| true,
    ));
    if materialized {
        assert!(monitor.mark_launch_delivery_materialized(
            42,
            delivery_id,
            &runtime.issue_monitor_materializer_id,
            &window_id,
        ));
    }
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).unwrap();
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.auto_launch_issue_monitor_delivery_events(
        &runtime.test_context(),
        42,
        LinkedIssueKind::Issue,
        Some(delivery_id.into()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    runtime.finish_queued_delivery_acks(&tasks);

    assert!(
        runtime.tracked_window_exists(&window_id),
        "replaying an old reservation must preserve another Issue's replacement pane"
    );
    assert_eq!(
        runtime.tabs[0]
            .workspace
            .window(&raw_id)
            .unwrap()
            .linked_issue_number,
        Some(43),
    );
    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    let monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs);
    assert_ne!(
        monitor.launched_window_issue(&window_id),
        Some(42),
        "a reused ID does not prove that the pane belongs to the old launch"
    );
}

#[test]
fn app_runtime_issue_monitor_pending_launch_error_marks_issue_row_failed() {
    use gwt::project_runtime::events::OwnerCommand;
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "pending-launch-error",
        MonitorProviderConversationFixture::Present,
        MonitorNativeHolderFixture::None,
        true,
    );
    gwt::try_mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&fixture.project_root),
        |prefs| {
            prefs.enabled = true;
            Ok(())
        },
    )
    .unwrap();
    let (owner, window_id, pane, _) = materialize_monitor_relaunch_at_owner(
        &mut fixture,
        gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
    );
    let issue = fixture.execution_owner.number;
    let root = fixture.project_root.clone();
    let context = fixture.runtime.test_context();
    let identity = owner.client().identity().unwrap().clone();
    let state = fixture.runtime.project_state_mut(&context).unwrap();
    state.project_owner_connection_id = 1;
    state.project_owner_identity = Some(identity.clone());
    let mut source = owner.subscribe();
    let observed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert!(pane.pending_fresh_execution.is_some());
        assert!(pane.pty.try_wait().unwrap().is_none());
        let pending =
            gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&root))
                .unwrap();
        assert!(!pending.launch_confirmations.contains_key(&issue));
        assert!(pending.pending_launch_deliveries.iter().any(|delivery| {
            Some(&delivery.delivery_id) == fixture.delivery_id.as_ref()
                && delivery.materializer_window_id.as_deref() == Some(pane.id.as_str())
        }));
        owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline Bounds the real reconciler's census publication; exact pane membership, not elapsed time, establishes completion.
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    let prefs = gwt::load_issue_monitor_prefs(
                        &gwt::issue_monitor_prefs_path_for_repo_path(&root),
                    )
                    .unwrap();
                    if prefs.monitor_runtime_counts.values().any(|census| {
                        census.host_pid == std::process::id()
                            && census.windows.get(&pane.id) == Some(&issue)
                    }) {
                        break;
                    }
                    owner.command(OwnerCommand::Health).await.unwrap();
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("actual pending child must be counted before injecting its failure");
        });
        let session = pane.session.as_ref().unwrap();
        let mut hook = runtime_hook_state("error", &session.id);
        hook.project_root = Some(root.display().to_string());
        hook.branch = Some(session.branch.clone());
        hook.agent_session_id.clone_from(&session.agent_session_id);
        hook.message = Some("Stop-block hit an error".into());
        hook.occurred_at = Utc::now().to_rfc3339();
        assert_eq!(
            owner_fixture_block_on(pane.runtime_hook(&hook)).unwrap(),
            reqwest::StatusCode::NO_CONTENT
        );
        owner_fixture_block_on(async {
            // test-hygiene: allow-wall-clock-deadline Bounds the actual owner's durable failure publication; exact Issue/message readback establishes completion.
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    let prefs = gwt::load_issue_monitor_prefs(
                        &gwt::issue_monitor_prefs_path_for_repo_path(&root),
                    )
                    .unwrap();
                    if prefs.failed_issues.iter().any(|failed| {
                        failed.issue_number == issue && failed.message == "Stop-block hit an error"
                    }) {
                        break;
                    }
                    owner.command(OwnerCommand::Health).await.unwrap();
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("actual pending launch failure must commit before GUI readback");
        });
        let monitor = owner_monitor_read(&root, issue);
        fixture.runtime.issue_monitor_daemon_inbox_events(
            &root,
            monitor.inbox_view_at(&Utc::now().to_rfc3339()),
        )
    }));
    let closed = owner_fixture_block_on(owner.command(OwnerCommand::Close {
        id: pane.id.clone(),
        incarnation: pane.incarnation,
    }));
    let settled = owner_fixture_block_on(owner.settled_self_closes());
    let retired = owner_fixture_block_on(owner.command(OwnerCommand::ListPanes));
    let exact_runtime_absent =
        owner_fixture_block_on(owner.probe(&pane.id, pane.incarnation)).is_err();
    let session_absent = matches!(
        gwt_agent::inspect_session_path(
            &fixture
                .sessions_dir
                .join(format!("{}.toml", pane.session.as_ref().unwrap().id))
        ),
        gwt_agent::SessionPathState::Missing
    );
    let reaped = pane.pty.try_wait();
    let tree_reaped = pane.pty.tree_exit_confirmed();
    while let Ok(event) = source.try_recv() {
        fixture
            .runtime
            .handle_project_owner_event(&context, 1, &identity, event);
    }
    fixture.runtime.test_project_owners.remove(&root);
    owner_fixture_block_on(
        Arc::try_unwrap(owner)
            .ok()
            .expect("drained pending error owner")
            .shutdown(),
    );
    let mut events = observed.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    settled.expect("all exact pending-error retirements settled");
    let retired = retired.expect("actual owner retirement inventory");
    for phase in ["panes", "pending", "closing", "reconciliation"] {
        assert!(
            retired[phase]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| { row["id"] != pane.id || row["incarnation"] != pane.incarnation }),
            "failed pending child still owns {phase}: {retired}; close={closed:?}"
        );
    }
    assert!(exact_runtime_absent && session_absent && tree_reaped,
        "an already retired child needs exact Session, owner and tree absence proof; close={closed:?}; inventory={retired}");
    assert!(reaped.unwrap().is_some());
    // Failure history precedes physical retirement. Owner shutdown drains the
    // runtime reconciler, so only this fresh snapshot can prove the slot is free.
    let monitor = owner_monitor_read(&root, issue);
    events.extend(
        fixture
            .runtime
            .issue_monitor_daemon_status_events(&root, Box::new(monitor.status_view())),
    );
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(status.state, "idle");
    assert_eq!(
        status.last_error.as_deref(),
        Some(format!("issue #{issue}: Stop-block hit an error").as_str())
    );
    let inbox = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorInbox { items } => Some(items),
            _ => None,
        })
        .expect("issue monitor inbox");
    let item = inbox
        .iter()
        .find(|item| item.issue.number == issue)
        .expect("failed issue row");
    assert_eq!(item.state, gwt::MonitorInboxState::AgentFailed);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
    assert!(!fixture.runtime.runtimes.contains_key(&window_id));
    assert!(fixture.runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .is_empty());
}

#[test]
fn app_runtime_issue_monitor_auto_launch_uses_last_settings_runtime_target() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let codex_home = temp.path().join("codex-home");
    fs::create_dir_all(&codex_home).expect("create Codex home");
    let _codex_home = ScopedEnvVar::set("CODEX_HOME", &codex_home);
    let _session_id = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);
    let _session_runtime = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
    let _ready_nonce = ScopedEnvVar::unset(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
    let _forward_url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
    let _forward_token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
    let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);

    let mut fixture = monitor_relaunch_fixture(
        temp.path(),
        "last-settings-runtime",
        MonitorProviderConversationFixture::Missing,
        MonitorNativeHolderFixture::None,
        false,
    );
    let repo = fixture.project_root.clone();
    fs::write(
        repo.join("docker-compose.yml"),
        "services:\n  app:\n    image: alpine:3.20\n",
    )
    .expect("compose");
    run_git(&repo, &["add", "docker-compose.yml"]);
    run_git(&repo, &["commit", "-qm", "compose"]);
    run_git(&repo, &["push", "origin", "develop"]);

    run_git(&fixture.worktree, &["merge", "--ff-only", "develop"]);
    let mut previous = gwt_agent::Session::load(
        &fixture
            .sessions_dir
            .join(format!("{}.toml", fixture.source_session_id)),
    )
    .unwrap();
    previous.model = Some("gpt-5.5".to_string());
    previous.reasoning_level = Some("xhigh".to_string());
    previous.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
    previous.docker_service = None;
    // The original last-settings fixture supplied no resumable provider
    // conversation. Keep it a Fresh prompt acceptance rather than a Resume
    // input-delivery acceptance merely because the shared fixture has history.
    previous.agent_session_id = None;
    previous
        .save(&fixture.sessions_dir)
        .expect("save previous session");
    seed_owner_fixture_roster(&repo, 1);
    gwt::try_mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        |prefs| {
            // Declare the native member's tool without installing a Monitor
            // profile override: this acceptance must choose the saved Session.
            prefs.agent_roster.as_mut().unwrap().members[0].agent_id = Some("codex".into());
            prefs.launch_profile = None;
            prefs.launch_profiles.clear();
            Ok(())
        },
    )
    .unwrap();
    fixture.runtime.launch_wizard_cache = LaunchWizardMemoryCache::load_with_agent_options(
        &fixture.sessions_dir,
        sample_agent_options(),
    );
    let capture_path = temp.path().join("last-settings-native.txt");
    pin_monitor_fixture_agents(
        &fixture,
        &temp.path().join("last-settings-runtime"),
        &[(
            "GWT_TEST_MONITOR_NATIVE_CAPTURE",
            capture_path.to_str().unwrap(),
        )],
    );
    let owner = start_monitor_relaunch_owner(&mut fixture, false);
    let runtime = &mut fixture.runtime;
    let profiles = runtime.issue_monitor_previous_profiles(&runtime.tabs[0].project_root);
    let repo_profile = profiles.repo_local().expect("repo-local last settings");
    assert_eq!(
        repo_profile.runtime_target,
        gwt_agent::LaunchRuntimeTarget::Host
    );

    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let mut events = runtime.auto_launch_issue_monitor_request_events_for_project(
        &repo,
        3165,
        LinkedIssueKind::Spec,
    );
    events.extend(commit_issue_monitor_preparations(
        runtime,
        &tasks,
        &fixture.recorded_events,
    ));
    let window_id = runtime
        .runtimes
        .keys()
        .next()
        .unwrap_or_else(|| panic!("actual last-settings owner launch did not install: {events:?}"))
        .clone();
    let pane = owner_runtime_probe(runtime, &window_id);
    let session_id = pane.session.as_ref().unwrap().id.clone();
    let runtime_target = pane.session.as_ref().unwrap().runtime_target;
    let child_pid = pane
        .pty
        .process_id()
        .expect("actual last-settings native child");
    assert_ne!(child_pid, std::process::id());
    assert_eq!(runtime.runtimes[&window_id].child_pid, Some(child_pid));
    let captured = owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bound the real native provider's atomic argument readback; its SID establishes completion.
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match fs::read_to_string(&capture_path) {
                    Ok(record) => return Ok(record),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.to_string()),
                }
                if let Some(exit) = pane.pty.try_wait().map_err(|error| error.to_string())? {
                    return Err(format!("actual provider exited before readback: {exit:?}"));
                }
                owner
                    .command(gwt::project_runtime::events::OwnerCommand::Health)
                    .await?;
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| Err("actual provider readback timed out".into()))
    });
    let screen =
        gwt_core::process_console::redact_line(&pane.pane.lock().unwrap().screen().contents());
    let close = owner_fixture_block_on(owner.command(
        gwt::project_runtime::events::OwnerCommand::Close {
            id: pane.id.clone(),
            incarnation: pane.incarnation,
        },
    ));
    runtime.runtimes.remove(&window_id);
    runtime.test_project_owners.remove(&repo);
    owner_fixture_block_on(
        Arc::try_unwrap(owner)
            .ok()
            .expect("drained last-settings owner")
            .shutdown(),
    );
    let captured = captured
        .unwrap_or_else(|error| panic!("{error}; close={close:?}; provider screen={screen}"));
    close.unwrap();
    let (captured_sid, captured) = captured.split_once('\n').unwrap();
    let (_, captured) = captured.split_once('\n').unwrap();
    let (_, prompt) = captured
        .split_once("\n__GWT_MONITOR_PROMPT_CONTENT__\n")
        .unwrap();
    assert_eq!(captured_sid, session_id);
    assert!(pane.pty.try_wait().unwrap().is_some());
    assert_eq!(runtime_target, gwt_agent::LaunchRuntimeTarget::Host);
    assert!(
        prompt.contains(
            "This prompt was generated by Issue Monitor. It is not a statement, approval, or visual confirmation by a human user."
        ),
        "Issue Monitor auto launch must identify the generated prompt as non-human provenance: {prompt}"
    );
    assert_eq!(
        prompt.matches("$gwt-execute #3165").count(),
        1,
        "Issue Monitor auto launch must pass the generated prompt to the agent exactly once: {prompt}"
    );
}
#[test]
fn terminal_convergence_tick_defers_durable_update_reads_and_coalesces_scans() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().join("repo"),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let now = chrono::Utc::now();

    assert!(runtime.update_drain_tick_events_at(now).is_empty());
    assert!(runtime.update_drain_tick_events_at(now).is_empty());
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "durable update observations must run on one queued worker, not the GUI dispatch"
    );
    assert!(runtime.window_lookup.contains_key("tab-1::agent-1"));
}

#[test]
fn update_drain_worker_snapshot_rechecks_a_pane_that_became_busy() {
    use crate::app_runtime::UpdateDrainObservation;
    let temp = tempdir().unwrap();
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        temp.path().join("repo"),
        WindowPreset::Agent,
        WindowProcessStatus::Idle,
    );
    let (mut runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let context = runtime.project_context("tab-1").unwrap();
    let now = chrono::Utc::now();
    let observe = || UpdateDrainObservation {
        context: context.clone(),
        drain: Some((
            gwt::IssueMonitorPrefs::default(),
            gwt::IssueMonitorUpdateDrain {
                version: "99.0.0".into(),
                since: now.to_rfc3339(),
                reason: gwt::IssueMonitorUpdateDrainReason::Auto,
                automatic: None,
                blocking: Vec::new(),
            },
        )),
        snapshot: gwt::update_drain::UpdateQuiescenceSnapshot {
            panes: Vec::new(),
            pending_acquire_claims: Vec::new(),
            active_executions: Vec::new(),
            held_verification_leases: Vec::new(),
        },
    };
    assert!(runtime
        .update_drain_observed_events(now, vec![observe()])
        .is_empty());
    runtime
        .window_hook_states
        .insert("tab-1::agent-1".into(), WindowProcessStatus::Running);
    let outbound =
        runtime.update_drain_observed_events(now + chrono::Duration::seconds(15), vec![observe()]);
    assert!(!outbound
        .iter()
        .any(|event| matches!(event.event, BackendEvent::UpdateAutoApply { .. })));
    assert!(!events
        .lock()
        .unwrap()
        .iter()
        .any(|event| matches!(event, UserEvent::ApplyUpdateDrained { .. })));
    drain_queued_blocking_tasks(&tasks);
}
