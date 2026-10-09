use super::*;

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
            effect_authority_epoch: 7,
            pending_effects: journal.clone(),
            max_active_agents: 1,
            priority_order: vec![42],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

    let max_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::SetIssueMonitorMaxActiveAgents {
            max_active_agents: 4,
        },
    );
    let reorder_events = runtime.handle_frontend_event(
        "client-1".to_string(),
        FrontendEvent::ReorderIssueMonitorIssues {
            issue_numbers: vec![99, 42],
        },
    );

    assert!(
        !max_events.is_empty() && !reorder_events.is_empty(),
        "missing daemon forces the atomic GUI fallback writer"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(persisted.max_active_agents, 4);
    assert_eq!(persisted.priority_order, vec![99, 42]);
    assert_eq!(persisted.effect_authority_epoch, 7);
    assert_eq!(persisted.pending_effects, journal);
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
fn app_runtime_local_driver_locked_latest_state_preserves_proposal_fence_result_and_delivery() {
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
        issues: vec![gwt::IssueMonitorIssue {
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
    let mut stale = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());
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
        Instant::now() + Duration::from_secs(5),
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
        issues: vec![gwt::IssueMonitorIssue {
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
    let mut stale = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());
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
        enabled: true,
        max_active_agents: 2,
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &disk).expect("seed prefs");
    let before = fs::read(&prefs_path).expect("read seeded prefs");
    let mut monitor =
        gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), disk.clone());
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
        let mut monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
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
    let mut monitor = gwt::IssueMonitorState::with_prefs(
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
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let prefs_path = temp.path().join("issue-monitor.json");
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        ..gwt::IssueMonitorConfig::default()
    });
    let mut issue = gwt::IssueMonitorIssue {
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

    issue.labels.push("hold".to_string());
    monitor.record_candidate(issue);
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist excluded state");

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
    let mut stale = gwt::IssueMonitorState::with_prefs(
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
    let mut monitor = gwt::IssueMonitorState::with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 7,
            pending_effects: vec![journal.clone()],
            ..gwt::IssueMonitorPrefs::default()
        },
    );
    monitor.terminal_queue_push(&[42], "operator", "2026-07-28T00:00:00Z");
    monitor.record_candidate(gwt::IssueMonitorIssue {
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
  sleep 2
  printf '%s\n' 'https://github.com/owner/repo.git'
  exit 0
fi
exit 1
"#,
    )
    .expect("write fake git");
    let _path = prepend_tool_parent_to_path(&fake_git);

    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let started = Instant::now();

    let events = runtime.local_issue_monitor_events_with_policy(
        &runtime.test_context(),
        Some("client-1"),
        super::super::IssueMonitorScanPolicy::CacheOnly,
        |monitor| {
            let _ = monitor.set_enabled_with_effect_revocation(true);
        },
    );

    assert!(
        started.elapsed() < Duration::from_millis(1_500),
        "cache-only control outlived its origin-probe deadline: {:?}",
        started.elapsed()
    );
    let error = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => status.last_error.as_deref(),
            _ => None,
        })
        .expect("deadline error is operator-visible");
    assert!(error.contains("deadline"), "unexpected error: {error}");
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
    let mut active = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());
    active.set_max_active_agents(4);
    let status = active.status_view();
    let events = runtime.issue_monitor_daemon_status_events(&active_root, Box::new(status.clone()));
    assert!(matches!(
        &events[0].event,
        BackendEvent::IssueMonitorStatus { status: actual } if **actual == status
    ));
    let other = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());
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
fn list_issue_monitor_uses_the_daemon_gui_projection_without_local_scan() {
    let temp = tempdir().expect("tempdir");
    let _home = ScopedGwtHome::set(temp.path());
    // No Git remote: even the old local scan cannot access the network.
    let root = temp.path().join("project");
    fs::create_dir_all(&root).expect("project");
    let tab = sample_project_tab("active", "Project", root.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("active"));
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());
    monitor.set_max_active_agents(5);
    monitor.record_scan_error("2026-09-12T00:00:00Z", "daemon scan failed");
    let agent = serde_json::to_value(monitor.agent_status()).expect("agent status");
    let expected = agent["gui_status"].clone();
    reset_local_issue_monitor_remote_scan_count();

    let events = runtime.list_issue_monitor_events_with_reader(
        &runtime.test_context(),
        "client-1",
        |project_root| {
            assert_eq!(project_root, root);
            Ok(Some(agent))
        },
    );

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
    let mut legacy = serde_json::to_value(
        gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default()).agent_status(),
    )
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
        let events = runtime.list_issue_monitor_events_with_reader(
            &runtime.test_context(),
            "client-1",
            |_| result,
        );
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
        let events = runtime.list_issue_monitor_events_with_reader(
            &runtime.test_context(),
            "client-1",
            |_| Ok(None),
        );
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
fn app_runtime_staged_update_raises_auto_drain_only_when_auto_apply_is_effective() {
    let temp = tempdir().expect("tempdir");
    let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let seed = |auto_apply_updates: Option<bool>| {
        gwt::save_issue_monitor_prefs(
            &prefs_path,
            &gwt::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                auto_apply_updates,
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

    seed(Some(false));
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    assert!(runtime.update_staged_events("9.99.0").is_empty());
    assert!(gwt::load_issue_monitor_prefs(&prefs_path)
        .expect("reload")
        .update_drain
        .is_none());

    seed(None);
    let events = runtime.update_staged_events("9.99.0");
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("drain status broadcast");
    let drain = status.update_drain.as_ref().expect("drain raised");
    assert_eq!(drain.reason, gwt::IssueMonitorUpdateDrainReason::Auto);
    assert_eq!(drain.version, "9.99.0");
    assert!(
        drain.blocking.is_empty(),
        "no agent pane, claim, execution or lease"
    );
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    let persisted_drain = persisted.update_drain.expect("drain persisted");
    assert_eq!(persisted_drain.version, "9.99.0");
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

    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());
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
        "9.99.0",
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
                label: "Sample".to_string(),
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

    let events = runtime.update_staged_events_with("9.99.0", None);
    let log = fs::read_to_string(gwt_core::update::update_log_path()).unwrap_or_default();
    assert!(
        log.lines().any(|line| {
            let entry: serde_json::Value = serde_json::from_str(line).expect("update log JSON");
            entry["stage"] == "drain_started"
                && entry["version"] == "9.99.0"
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
        toasts[0].1.contains("9.99.0") && toasts[0].1.contains("draining"),
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
                auto_apply_updates: None,
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
    let events = runtime.update_staged_events_with("9.99.0", None);
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("drain status broadcast: {events:?}");
    let drain = status.update_drain.as_ref().expect("drain raised");
    assert_eq!(drain.reason, gwt::IssueMonitorUpdateDrainReason::Auto);
    assert_eq!(drain.version, "9.99.0");
    assert_eq!(
        drain.blocking,
        vec![gwt::update_drain::UpdateBlocker::ActivePane {
            window_id: "tab-1::agent-1".to_string(),
            label: "Sample".to_string(),
            state: WindowProcessStatus::Starting,
        }],
        "the status names what the click is waiting for"
    );
    let toasts = update_resume_toasts(&events);
    assert_eq!(toasts.len(), 1, "drain start recorded once: {toasts:?}");
    assert!(
        toasts[0].1.contains("9.99.0") && toasts[0].1.contains("draining"),
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
        Some("9.99.0"),
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
        quiet.update_staged_events_with("9.99.0", None).is_empty(),
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
/// tick as the auto path — attended monitor (`autonomous_mode:false`), the
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
            autonomous_mode: false,
            update_drain: Some(gwt::IssueMonitorUpdateDrain {
                version: "9.99.0".to_string(),
                since: since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                reason: gwt::IssueMonitorUpdateDrainReason::Auto,
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

    assert!(runtime.update_drain_tick_events_at(at(15)).is_empty());
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
    assert!(runtime.update_drain_tick_events_at(at(30)).is_empty());
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
    assert!(runtime.update_drain_tick_events_at(at(45)).is_empty());
    let scheduled = runtime.update_drain_tick_events_at(at(60));
    assert!(
        scheduled.iter().any(|event| matches!(
            &event.event,
            BackendEvent::UpdateAutoApply {
                version,
                phase: gwt::protocol::UpdateAutoApplyPhase::Scheduled,
                grace_secs: Some(60),
            } if version == "9.99.0"
        )),
        "the grace is announced to the CTA: {scheduled:?}"
    );
    assert_eq!(
        drained_events(&user_events),
        0,
        "nothing applies inside the grace"
    );
    let applying = runtime.update_drain_tick_events_at(at(120));
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
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo_with_initial_commit(&repo);
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
        gwt::save_issue_monitor_prefs(
            &prefs_path,
            &gwt::IssueMonitorPrefs {
                enabled: enabled_before_drain,
                autonomous_mode: false,
                max_active_agents: 2,
                update_drain: Some(gwt::IssueMonitorUpdateDrain {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    since: "2026-09-15T00:00:00Z".to_string(),
                    reason: gwt::IssueMonitorUpdateDrainReason::Auto,
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
                        version: "9.99.0".into(),
                        since: since.to_rfc3339(),
                        reason: gwt::IssueMonitorUpdateDrainReason::Auto,
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
    assert!(runtime.update_drain_tick_events_at(at(15)).is_empty());
    runtime.active_tab_id = Some("tab-1".into());
    let scheduled = runtime.update_drain_tick_events_at(at(30));
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
    runtime.update_drain_tick_events_at(at(90));
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
                version: "9.99.0".to_string(),
                since: since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                reason: gwt::IssueMonitorUpdateDrainReason::Auto,
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
    assert!(runtime.update_drain_tick_events_at(at(15)).is_empty());
    assert!(runtime.update_drain_tick_events_at(at(30)).is_empty());
    let notice = runtime.update_drain_tick_events_at(at(1800));
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
    assert_eq!(entry["version"], "9.99.0");
    let blockers: serde_json::Value =
        serde_json::from_str(entry["blockers"].as_str().expect("serialized blockers"))
            .expect("blocker JSON");
    assert_eq!(blockers[0]["window_id"], "tab-1::agent-1");
    assert!(runtime.update_drain_tick_events_at(at(1801)).is_empty());
    assert_eq!(
        fs::read_to_string(&log_path).expect("update log"),
        log,
        "unchanged blockers must not emit a log on every tick"
    );
    assert!(
        toasts[0].1.contains("9.99.0") && toasts[0].1.contains("Sample"),
        "the notice names the blocking pane: {}",
        toasts[0].1
    );
    assert_eq!(drained_events(&user_events), 0);

    // The pane goes Idle: the first quiet tick settles, the second schedules
    // the apply with the cancel grace, and the apply fires once it elapsed.
    runtime
        .window_hook_states
        .insert("tab-1::agent-1".to_string(), WindowProcessStatus::Idle);
    assert!(runtime.update_drain_tick_events_at(at(1815)).is_empty());
    let scheduled = runtime.update_drain_tick_events_at(at(1830));
    assert!(
        scheduled.iter().any(|event| matches!(
            &event.event,
            BackendEvent::UpdateAutoApply {
                version,
                phase: gwt::protocol::UpdateAutoApplyPhase::Scheduled,
                grace_secs: Some(60),
            } if version == "9.99.0"
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
    assert!(runtime.update_drain_tick_events_at(at(1845)).is_empty());
    assert_eq!(drained_events(&user_events), 0);
    let applying = runtime.update_drain_tick_events_at(at(1890));
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
                version: "9.99.0".to_string(),
                since: since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                reason: gwt::IssueMonitorUpdateDrainReason::Auto,
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
    runtime.update_drain_tick_events_at(at(15));
    let scheduled = runtime.update_drain_tick_events_at(at(30));
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
            } if version == "9.99.0"
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
        assert!(runtime.update_drain_tick_events_at(at(secs)).is_empty());
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
            version: "9.99.0".to_string(),
        },
    ] {
        let events = runtime.update_staged_events_with("9.99.0", Some(refusal.clone()));
        let log = fs::read_to_string(gwt_core::update::update_log_path()).unwrap_or_default();
        assert!(
            log.lines().any(|line| {
                let entry: serde_json::Value = serde_json::from_str(line).unwrap();
                entry["stage"] == "pending_refused" && entry["reason"] == refusal.notice("9.99.0")
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
            toasts[0].1.contains("9.99.0") && toasts[0].1.contains("manually"),
            "the notice names the manual fallback: {}",
            toasts[0].1
        );
    }
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
}

#[cfg(unix)]
#[test]
fn app_runtime_issue_monitor_reconciliation_error_survives_rebase_scan() {
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
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig::default());

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
    let mut stale = gwt::IssueMonitorState::with_prefs(
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
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let prefs_path = temp.path().join("issue-monitor.json");
    fs::write(&prefs_path, b"{").expect("seed malformed prefs");
    let mut current = gwt::IssueMonitorState::with_prefs(
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
    let mut monitor = gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
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
    let mut monitor = gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
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
    let prefs_path = temp.path().join("issue-monitor.json");
    let stale_record = issue_monitor_autonomous_record(42, gwt::AutonomousPhase::Implementing, 1);
    let reviewing = issue_monitor_autonomous_record(42, gwt::AutonomousPhase::Reviewing, 2);
    let disk_only = issue_monitor_autonomous_record(99, gwt::AutonomousPhase::Implementing, 3);
    let disk = gwt::IssueMonitorPrefs {
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
    let mut stale = gwt::IssueMonitorState::with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
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
            BackendEvent::WindowCanvasState { workspace } => Some(workspace),
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
    assert_eq!(agent_window.geometry.x, 96.0);
    assert_eq!(agent_window.geometry.y, 96.0);
    assert_eq!(agent_window.geometry.width, 860.0);
    assert_eq!(agent_window.geometry.height, 520.0);
}

#[test]
fn durable_issue_monitor_delivery_materializes_one_window_and_replay_only_acks() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex)
        .save(&sessions_dir)
        .expect("save previous session");
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        ..gwt::IssueMonitorConfig::default()
    });
    monitor.terminal_queue_push(&[3165], "operator", "2026-07-28T00:00:00Z");
    monitor.record_candidate(gwt::IssueMonitorIssue {
        number: 3165,
        title: "SPEC: durable delivery".to_string(),
        labels: vec!["gwt-spec".to_string()],
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::Ready,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(
        3165,
        "claim-3165",
        "host/session",
        "effect-3165",
        "2026-07-28T00:00:00Z",
    ));
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &monitor.prefs(),
    )
    .expect("seed delivery");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

    let first = runtime.auto_launch_issue_monitor_delivery_events(
        &runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    assert!(first
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
    let replay = runtime.auto_launch_issue_monitor_delivery_events(
        &runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    assert!(!replay
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
    let agent_windows = runtime.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(agent_windows, 1);
    let bound_window_id =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
            .expect("load bound delivery")
            .pending_launch_deliveries[0]
            .materializer_window_id
            .clone()
            .expect("bound window id");
    let (ack_spawner, ack_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = ack_spawner;
    assert!(runtime.persist_dispatcher.wait_idle(Duration::from_secs(5)));
    let disk_before_ack = gwt::load_workspace_state(&gwt::workspace_state_path(&repo)).unwrap();
    let writes_before_ack = runtime.persist_dispatcher.enqueued_count();
    let durable_writes_before_ack = runtime.persist_dispatcher.durable_write_count();
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let pending_before_ack = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    let _ = runtime.issue_monitor_launch_completed_delivery_events(
        &repo,
        3165,
        &bound_window_id,
        Some("launch:effect-3165"),
    );
    assert_eq!(
        ack_tasks.lock().unwrap().len(),
        1,
        "delivery ACK must be a worker transaction"
    );
    assert_eq!(
        runtime.persist_dispatcher.enqueued_count(),
        writes_before_ack + 1,
        "the GUI only reserves ordering metadata; the ACK worker authorizes the write"
    );
    let newer_window = runtime
        .tab_mut("tab-1")
        .unwrap()
        .workspace
        .add_window(WindowPreset::Agent, canvas_bounds());
    runtime.register_window("tab-1", &newer_window.id);
    runtime.persist().unwrap();
    assert_eq!(
        runtime.persist_dispatcher.durable_write_count(),
        durable_writes_before_ack,
        "a reserved but unmaterialized ACK must perform zero durable writes"
    );
    assert_eq!(
        gwt::load_workspace_state(&gwt::workspace_state_path(&repo)).unwrap(),
        disk_before_ack,
        "neither the reserved barrier nor newer pending state may write before materialization"
    );
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
        pending_before_ack
    );
    let ack_task = ack_tasks.lock().unwrap().pop().unwrap();
    ack_task();
    assert!(runtime.persist_dispatcher.wait_idle(Duration::from_secs(5)));
    assert!(
        gwt::load_workspace_state(&gwt::workspace_state_path(&repo))
            .unwrap()
            .windows
            .iter()
            .any(|window| window.id == newer_window.id),
        "a delayed ACK must preserve windows added after its snapshot was captured"
    );
    let ack = {
        let mut events = _recorded_events.lock().unwrap();
        let index = events
            .iter()
            .position(|event| matches!(event, UserEvent::IssueMonitorLaunchDeliveryAcknowledged(_)))
            .unwrap();
        let UserEvent::IssueMonitorLaunchDeliveryAcknowledged(ack) = events.remove(index) else {
            unreachable!()
        };
        ack
    };
    runtime.handle_issue_monitor_launch_delivery_ack(*ack);
    assert!(
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(&repo))
            .expect("reload prefs")
            .pending_launch_deliveries
            .is_empty()
    );

    // A same-id replacement cancels the old transaction before it can ACK.
    gwt::save_issue_monitor_prefs(&prefs_path, &pending_before_ack).unwrap();
    runtime.issue_monitor_launch_completed_delivery_events(
        &repo,
        3165,
        &bound_window_id,
        Some("launch:effect-3165"),
    );
    let old_address = runtime.window_lookup[&bound_window_id].clone();
    runtime.register_window(&old_address.tab_id, &old_address.raw_id);
    let writes_before_cancelled_ack = runtime.persist_dispatcher.enqueued_count();
    let durable_writes_before_cancelled_ack = runtime.persist_dispatcher.durable_write_count();
    ack_tasks.lock().unwrap().pop().unwrap()();
    assert!(
        runtime.persist_dispatcher.wait_idle(Duration::from_secs(5)),
        "dropping a stale ACK gate must release the reserved generation"
    );
    assert_eq!(
        runtime.persist_dispatcher.enqueued_count(),
        writes_before_cancelled_ack
    );
    assert_eq!(
        runtime.persist_dispatcher.durable_write_count(),
        durable_writes_before_cancelled_ack
    );
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path).unwrap(),
        pending_before_ack,
        "a stale launch must leave the delivery unacknowledged"
    );
}

#[test]
fn issue_monitor_ack_preserves_reused_failed_window_id() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    assert_ack_preserves_failed_window_replacement(temp.path(), false);
}

#[test]
fn issue_monitor_ack_preserves_failed_window_runtime_installed_after_enqueue() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    assert_ack_preserves_failed_window_replacement(temp.path(), true);
}

/// Issue #4802 AC-2: a launch into a worktree that already has a live agent
/// pane is refused. The Monitor delivery is acknowledged onto the running pane
/// instead, so the Issue ends up bound to one pane rather than accumulating a
/// fresh one per relaunch (#4758 collected sixteen). A finished pane does not
/// block a launch.
#[test]
fn issue_monitor_delivery_into_a_worktree_with_a_live_agent_pane_adopts_it() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (label, pane_status, expect_adoption, pending_identity) in [
        ("running pane", WindowProcessStatus::Running, true, false),
        ("idle pane", WindowProcessStatus::Idle, true, false),
        ("stopped pane", WindowProcessStatus::Stopped, false, false),
        (
            "pre-session Monitor pane",
            WindowProcessStatus::Starting,
            true,
            true,
        ),
    ] {
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo_with_initial_commit(&repo);
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex)
            .save(&sessions_dir)
            .expect("save previous session");
        let now = chrono::Utc::now().to_rfc3339();
        let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
            enabled: true,
            ..gwt::IssueMonitorConfig::default()
        });
        monitor.terminal_queue_push(&[3165], "operator", &now);
        monitor.record_candidate(gwt::IssueMonitorIssue {
            number: 3165,
            title: "SPEC: duplicate launch".to_string(),
            labels: vec!["gwt-spec".to_string()],
            state: gwt::IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: gwt::IssueMonitorReadiness::Ready,
            updated_at: None,
        });
        assert!(monitor.apply_confirmed_claim(
            3165,
            "claim-3165",
            "host/session",
            "effect-3165",
            &now,
        ));
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
        gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed delivery");
        let mut tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Agent],
        );
        let raw_window_id = tab.workspace.persisted().windows[0].id.clone();
        assert!(tab
            .workspace
            .set_linked_issue_number(&raw_window_id, (!pending_identity).then_some(3165)));
        tab.workspace.set_status(&raw_window_id, pane_status);
        let (mut runtime, _recorded_events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        let live_window_id = combined_window_id("tab-1", &raw_window_id);
        if pending_identity {
            let mut feedback = issue_monitor_feedback(3165);
            feedback.issue_monitor_project_root = Some(repo.clone());
            runtime
                .pending_launch_feedback_contexts
                .insert(live_window_id.clone(), feedback);
            let snapshot = runtime
                .issue_monitor_window_snapshot_for_tab("tab-1", "2026-10-06T00:00:00Z")
                .unwrap();
            assert_eq!(
                snapshot.windows[0].issue_number,
                Some(3165),
                "pre-Session identity is observable"
            );
            assert!(snapshot.windows[0].monitor_owned);
        }
        if pane_status == WindowProcessStatus::Stopped {
            runtime
                .window_pty_statuses
                .insert(live_window_id.clone(), pane_status);
        } else {
            runtime
                .window_pty_statuses
                .insert(live_window_id.clone(), WindowProcessStatus::Running);
            runtime
                .window_hook_states
                .insert(live_window_id.clone(), pane_status);
        }
        assert_eq!(runtime.window_status(&live_window_id), Some(pane_status));
        let observation = runtime
            .issue_monitor_window_snapshot_for_tab("tab-1", &chrono::Utc::now().to_rfc3339())
            .unwrap();
        assert_eq!(
            observation.windows[0].monitor_owned, pending_identity,
            "{label}: sessionless manual panes cannot inherit this agent's ambient Monitor route"
        );

        let (ack_spawner, ack_tasks) = BlockingTaskSpawner::queued();
        runtime.blocking_tasks = ack_spawner;
        let mut events = runtime.auto_launch_issue_monitor_delivery_events(
            &runtime.test_context(),
            3165,
            LinkedIssueKind::Spec,
            Some("launch:effect-3165".to_string()),
            gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        );
        events.extend(runtime.finish_queued_delivery_acks(&ack_tasks));
        let agent_windows = runtime.tabs[0]
            .workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == WindowPreset::Agent)
            .count();
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        let reloaded =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs.clone());
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
                reloaded.launched_window_issue(&live_window_id),
                Some(3165),
                "{label}: the Issue is bound to the pane already working it"
            );
        } else {
            assert_eq!(
                agent_windows, 2,
                "{label}: a finished pane does not block a launch"
            );
        }
    }
}

#[test]
fn durable_delivery_fallback_commit_budget_is_an_explicit_runtime_dependency() {
    // Issue #3878: the local fallback commit that claims, marks and ACKs a
    // durable delivery runs under a wall-clock deadline. Production keeps a
    // GUI-thread budget; a test runtime must own its budget explicitly instead
    // of inheriting a host-load-dependent one, and an exhausted budget must
    // leave the delivery replayable rather than half-materialized.
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex)
        .save(&sessions_dir)
        .expect("save previous session");
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        ..gwt::IssueMonitorConfig::default()
    });
    monitor.terminal_queue_push(&[3165], "operator", "2026-07-28T00:00:00Z");
    monitor.record_candidate(gwt::IssueMonitorIssue {
        number: 3165,
        title: "SPEC: budgeted durable delivery".to_string(),
        labels: vec!["gwt-spec".to_string()],
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::Ready,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(
        3165,
        "claim-3165",
        "host/session",
        "effect-3165",
        "2026-07-28T00:00:00Z",
    ));
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed delivery");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
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

    runtime.issue_monitor_fallback_commit_timeout = Duration::ZERO;
    let starved = runtime.auto_launch_issue_monitor_delivery_events(
        &runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    assert!(
        !starved
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "an exhausted fallback commit budget must not materialize a window",
    );
    assert!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("reload prefs")
            .pending_launch_deliveries
            .iter()
            .any(|delivery| delivery.delivery_id == "launch:effect-3165"
                && delivery.materializer_window_id.is_none()),
        "a starved claim must leave the delivery unbound and replayable",
    );

    runtime.issue_monitor_fallback_commit_timeout =
        super::super::TEST_ISSUE_MONITOR_FALLBACK_COMMIT_TIMEOUT;
    let replay = runtime.auto_launch_issue_monitor_delivery_events(
        &runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    assert!(
        replay
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "the same delivery replays into exactly one window once the budget is explicit",
    );
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
}

#[test]
fn durable_issue_monitor_delivery_preserves_live_materializer_and_replays_after_it_stops() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex)
        .save(&sessions_dir)
        .expect("save previous session");
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        ..gwt::IssueMonitorConfig::default()
    });
    monitor.terminal_queue_push(&[3165], "operator", "2026-07-28T00:00:00Z");
    monitor.record_candidate(gwt::IssueMonitorIssue {
        number: 3165,
        title: "SPEC: abandoned durable delivery".to_string(),
        labels: vec!["gwt-spec".to_string()],
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::Ready,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(
        3165,
        "claim-3165",
        "host/session",
        "effect-3165",
        "2026-07-28T00:00:00Z",
    ));
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &monitor.prefs(),
    )
    .expect("seed delivery");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.issue_monitor_launch_deliveries.insert(
        "launch:effect-3165".to_string(),
        super::super::IssueMonitorLaunchDeliveryState::Materializing {
            window_id: "tab-1::missing-materializer".to_string(),
            started_at: Instant::now(),
        },
    );

    let replay = runtime.auto_launch_issue_monitor_delivery_events(
        &runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );

    assert!(
        replay
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "a vanished materializing window must not suppress exact delivery replay",
    );
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

    let live_window_id = match runtime
        .issue_monitor_launch_deliveries
        .get_mut("launch:effect-3165")
        .expect("replayed delivery is materializing")
    {
        super::super::IssueMonitorLaunchDeliveryState::Materializing {
            window_id,
            started_at,
        } => {
            *started_at = Instant::now() - Duration::from_secs(61);
            window_id.clone()
        }
        state => panic!("expected materializing delivery, got {state:?}"),
    };
    insert_test_pane_runtime(&mut runtime, &live_window_id);

    let waiting_replay = runtime.auto_launch_issue_monitor_delivery_events(
        &runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );

    assert!(
        waiting_replay.is_empty(),
        "a materializer with a live PTY must keep its delivery slot while awaiting input",
    );
    assert!(runtime.runtimes.contains_key(&live_window_id));
    assert!(matches!(
        runtime
            .issue_monitor_launch_deliveries
            .get("launch:effect-3165"),
        Some(super::super::IssueMonitorLaunchDeliveryState::Materializing { window_id, .. })
            if window_id == &live_window_id
    ));

    runtime.stop_window_runtime_without_session_projection(&live_window_id);

    let expired_replay = runtime.auto_launch_issue_monitor_delivery_events(
        &runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );

    assert!(
        expired_replay
            .iter()
            .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })),
        "an expired materializing window must be replaced by exact delivery replay",
    );
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
}

#[test]
fn competing_issue_monitor_subscribers_materialize_one_durable_delivery() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex)
        .save(&sessions_dir)
        .expect("save previous session");
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        ..gwt::IssueMonitorConfig::default()
    });
    monitor.terminal_queue_push(&[3165], "operator", "2026-07-28T00:00:00Z");
    monitor.record_candidate(gwt::IssueMonitorIssue {
        number: 3165,
        title: "SPEC: competing subscribers".to_string(),
        labels: vec!["gwt-spec".to_string()],
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::Ready,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(
        3165,
        "claim-3165",
        "host/session",
        "effect-3165",
        "2026-07-28T00:00:00Z",
    ));
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed delivery");

    let tab_a = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let tab_b = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime_a, _events_a) =
        sample_runtime_with_events(temp.path(), vec![tab_a], Some("tab-1"));
    let (mut runtime_b, _events_b) =
        sample_runtime_with_events(temp.path(), vec![tab_b], Some("tab-1"));
    runtime_a.issue_monitor_materializer_id = "gui-a".to_string();
    runtime_b.issue_monitor_materializer_id = "gui-b".to_string();

    let first = runtime_a.auto_launch_issue_monitor_delivery_events(
        &runtime_a.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let second = runtime_b.auto_launch_issue_monitor_delivery_events(
        &runtime_b.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let non_owner_failure = runtime_b.issue_monitor_launch_failed_delivery_events(
        Some(&repo),
        3165,
        "non-owner validation failed",
        Some("launch:effect-3165"),
    );

    assert!(first
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
    assert!(!second
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
    assert!(!non_owner_failure
        .iter()
        .any(|event| matches!(event.event, BackendEvent::IssueMonitorLaunchFailed { .. })));
    let total_agent_windows = runtime_a.tabs[0]
        .workspace
        .persisted()
        .windows
        .iter()
        .chain(runtime_b.tabs[0].workspace.persisted().windows.iter())
        .filter(|window| window.preset == WindowPreset::Agent)
        .count();
    assert_eq!(total_agent_windows, 1);
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("load claimed delivery");
    assert_eq!(
        persisted.pending_launch_deliveries[0]
            .materializer_id
            .as_deref(),
        Some("gui-a")
    );
}

#[test]
fn durable_issue_monitor_delivery_restart_recovers_only_exact_bound_window() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo_with_initial_commit(&repo);
    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex)
        .save(&sessions_dir)
        .expect("save previous session");
    let mut monitor = gwt::IssueMonitorState::new(gwt::IssueMonitorConfig {
        enabled: true,
        ..gwt::IssueMonitorConfig::default()
    });
    monitor.terminal_queue_push(&[3165], "operator", "2026-07-28T00:00:00Z");
    monitor.record_candidate(gwt::IssueMonitorIssue {
        number: 3165,
        title: "SPEC: durable delivery".to_string(),
        labels: vec!["gwt-spec".to_string()],
        state: gwt::IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::Ready,
        updated_at: None,
    });
    assert!(monitor.apply_confirmed_claim(
        3165,
        "claim-3165",
        "host/session",
        "effect-3165",
        "2026-07-28T00:00:00Z",
    ));
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed delivery");

    let first_tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut first_runtime, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![first_tab], Some("tab-1"));
    let _ = first_runtime.auto_launch_issue_monitor_delivery_events(
        &first_runtime.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    let bound_window_id = gwt::load_issue_monitor_prefs(&prefs_path)
        .expect("load bound delivery")
        .pending_launch_deliveries[0]
        .materializer_window_id
        .clone()
        .expect("bound window id");
    assert!(AppRuntime::mark_issue_monitor_launch_delivery_materialized(
        &repo,
        &first_runtime.issue_monitor_materializer_id,
        first_runtime.issue_monitor_fallback_commit_timeout,
        3165,
        "launch:effect-3165",
        &bound_window_id,
    )
    .expect("mark exact delivery materialized"));
    first_runtime
        .persist_issue_monitor_delivery_workspace(&repo, &bound_window_id)
        .expect("persist exact delivery window");
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .expect("load pre-ACK delivery")
            .pending_launch_deliveries[0]
            .workspace_durable_window_id,
        None,
        "simulate a crash after durable workspace save but before its prefs marker",
    );
    let mut crashed_owner_prefs =
        gwt::load_issue_monitor_prefs(&prefs_path).expect("load crashed owner prefs");
    crashed_owner_prefs.pending_launch_deliveries[0].materializer_pid = Some(2_000_000_000);
    gwt::save_issue_monitor_prefs(&prefs_path, &crashed_owner_prefs)
        .expect("mark the prior materializer process dead");

    let restored_workspace = gwt::load_restored_workspace_state(&repo).expect("restore workspace");
    let restored_tab = ProjectTabRuntime {
        id: "tab-1".to_string(),
        title: "Repo".to_string(),
        project_root: repo.clone(),
        kind: ProjectKind::Git,
        workspace: WindowCanvasState::from_persisted(restored_workspace),
        migration_pending: false,
        main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
    };
    let (mut restarted, _recorded_events) =
        sample_runtime_with_events(temp.path(), vec![restored_tab], Some("tab-1"));
    restarted.issue_monitor_materializer_id = "restarted-gui".to_string();
    let (ack_spawner, ack_tasks) = BlockingTaskSpawner::queued();
    restarted.blocking_tasks = ack_spawner;

    let events = restarted.auto_launch_issue_monitor_delivery_events(
        &restarted.test_context(),
        3165,
        LinkedIssueKind::Spec,
        Some("launch:effect-3165".to_string()),
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
    );
    assert!(!events
        .iter()
        .any(|event| matches!(event.event, BackendEvent::WindowCanvasState { .. })));
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
    restarted.finish_queued_delivery_acks(&ack_tasks);
    let acknowledged = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload ACKed prefs");
    assert!(
        acknowledged.pending_launch_deliveries.is_empty(),
        "restart must ACK the exact saved pane: window={:?}, deliveries={:?}",
        restarted.tabs[0].workspace.persisted().windows,
        acknowledged.pending_launch_deliveries
    );
}

#[test]
fn app_runtime_issue_monitor_pending_launch_error_marks_issue_row_failed() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _gh_lock = fake_gh_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let _projection_timeout = ScopedEnvVar::set(
        "GWT_TEST_ISSUE_MONITOR_FALLBACK_PROJECTION_TIMEOUT_MS",
        "10000",
    );
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    init_repo(&repo);
    let fake_bin = temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("create fake tool directory");
    let fake_gh = write_fake_gh_issue_list(&fake_bin);
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");
    Cache::new(issue_cache_root(&repo))
        .write_snapshot(&sample_issue_snapshot(
            42,
            "Issue Monitor pending launch failure",
            &["bug"],
            "Issue body",
            "2026-06-23T00:00:00Z",
        ))
        .expect("write issue cache");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(&repo),
        &gwt::IssueMonitorPrefs {
            enabled: true,
            max_active_agents: 5,
            ..queued_issue_monitor_prefs(&[42])
        },
    )
    .expect("save issue monitor prefs");
    let tab = sample_project_tab_with_window_at(
        "tab-1",
        "agent-1",
        repo.clone(),
        WindowPreset::Agent,
        WindowProcessStatus::Running,
    );
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let window_id = combined_window_id("tab-1", "agent-1");
    runtime.pending_launch_feedback_contexts.insert(
        window_id.clone(),
        LaunchFeedbackContext {
            client_id: "__issue_monitor__".to_string(),
            title: "Issue Monitor".to_string(),
            issue_monitor_issue_number: Some(42),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: Some(repo),
            issue_monitor_session_mode: None,
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
    );

    let events = runtime.handle_runtime_status_with_exit_confirmation(
        window_id,
        WindowProcessStatus::Error,
        Some("Stop-block hit an error".to_string()),
        true,
    );

    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("issue monitor status");
    assert_eq!(status.state, "error");
    assert_eq!(
        status.last_error.as_deref(),
        Some("issue #42: Stop-block hit an error")
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
        .find(|item| item.issue.number == 42)
        .expect("failed issue row");
    assert_eq!(item.state, gwt::MonitorInboxState::AgentFailed);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
    assert!(runtime.tabs[0].workspace.persisted().windows.is_empty());
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

    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    fs::write(
        repo.join("docker-compose.yml"),
        "services:\n  app:\n    image: alpine:3.20\n",
    )
    .expect("compose");
    run_git(&repo, &["add", "docker-compose.yml"]);
    run_git(&repo, &["commit", "-qm", "compose"]);
    run_git(&repo, &["push", "origin", "develop"]);

    let sessions_dir = temp.path().join("sessions");
    fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    let mut previous =
        gwt_agent::Session::new(&repo, "feature/spec-3170", gwt_agent::AgentId::Codex);
    previous.model = Some("gpt-5.5".to_string());
    previous.reasoning_level = Some("xhigh".to_string());
    previous.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
    previous.docker_service = None;
    previous.save(&sessions_dir).expect("save previous session");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded_events) =
        sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    runtime.agent_capability_issuer =
        Some(crate::embedded_server::AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:43123/internal/hook-live",
            "ws://127.0.0.1:43124/ws",
            "ws://127.0.0.1:43123/internal/pane-ws",
        ));
    let profiles = runtime.issue_monitor_previous_profiles(&runtime.tabs[0].project_root);
    let repo_profile = profiles.repo_local().expect("repo-local last settings");
    assert_eq!(
        repo_profile.runtime_target,
        gwt_agent::LaunchRuntimeTarget::Host
    );

    let _events = runtime.auto_launch_issue_monitor_request_events_for_project(
        &repo,
        3165,
        LinkedIssueKind::Spec,
    );

    wait_for_recorded_event(
        "issue monitor last settings runtime launch",
        &recorded_events,
        |events| {
            events.iter().any(|event| {
                matches!(
                    recorded_project_payload(event),
                    UserEvent::LaunchComplete { .. }
                )
            })
        },
    );
    let result = {
        let events = recorded_events.lock().expect("event log");
        events
            .iter()
            .find_map(|event| match recorded_project_payload(event) {
                UserEvent::LaunchComplete { result, .. } => Some(result.clone()),
                _ => None,
            })
            .expect("launch complete")
    };
    let Ok((process, _, _, _, _, _, _, _, runtime_target, _, _, _)) = *result else {
        panic!("Issue Monitor auto launch failed: {result:?}");
    };
    assert_eq!(runtime_target, gwt_agent::LaunchRuntimeTarget::Host);
    let payload = launch_payload_fragments(&process);
    assert!(
        payload.iter().any(|fragment| fragment.contains(
            "This prompt was generated by Issue Monitor. It is not a statement, approval, or visual confirmation by a human user."
        )),
        "Issue Monitor auto launch must identify the generated prompt as non-human provenance: {payload:?}"
    );
    assert_eq!(
        payload
            .iter()
            .filter(|fragment| fragment.contains("$gwt-execute #3165"))
            .count(),
        1,
        "Issue Monitor auto launch must pass the generated prompt to the agent exactly once: {payload:?}"
    );
}
