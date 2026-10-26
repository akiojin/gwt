use super::*;

/// Issue #3633 AC-7: the recurrence guard for the missing spawn subject.
///
/// #3505 was closed while nothing in the production topology started a
/// daemon; #3633 re-registered the identical state weeks later. A supervisor
/// that exists but is never called reproduces that exactly, so this asserts
/// the wiring — every supervision tick that finds an enabled project must ask
/// the supervisor to keep a daemon alive — rather than a spawn detail.
#[test]
fn the_daemon_supervision_tick_keeps_a_runtime_daemon_alive_for_all_open_projects() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    disable_pm_auto_start(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: false,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed disabled prefs");
    runtime.ensure_runtime_daemons_for_open_projects();
    assert_eq!(
        runtime.daemon_supervisor.ensure_attempts(),
        0,
        "a disabled monitor needs no daemon"
    );

    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "disabled Monitor projects still need an owner"
    );

    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled prefs");
    runtime.ensure_runtime_daemons_for_open_projects();
    assert_eq!(
        runtime.daemon_supervisor.ensure_attempts(),
        0,
        "GUI only enqueues ensure"
    );
    runtime.ensure_runtime_daemons_for_open_projects();
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "same project coalesces while queued"
    );
    let task = tasks.lock().unwrap().pop().unwrap();
    task();
    assert_eq!(
        runtime.daemon_supervisor.ensure_attempts(),
        1,
        "the tick must own the daemon for an enabled project"
    );

    // AC-2 in the topology: supervision is "the tick keeps asking", so a
    // second tick has to ask again rather than trust the first answer.
    runtime.ensure_runtime_daemons_for_open_projects();
    let task = tasks.lock().unwrap().pop().unwrap();
    task();
    assert_eq!(runtime.daemon_supervisor.ensure_attempts(), 2);
}

#[test]
fn scheduled_tick_drives_disabled_claim_cleanup_without_scanning() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: false,
            effect_authority_epoch: 8,
            pending_effects: vec![gwt::PendingIssueMonitorEffect::prepared(
                "release:claim-effect-42:8",
                8,
                gwt::IssueMonitorEffectPayload::ReleaseClaim {
                    issue_number: 42,
                    claim_id: "claim-42".to_string(),
                    owner: "host/session".to_string(),
                },
            )],
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed disabled cleanup journal");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    assert!(runtime
        .issue_monitor_scheduled_tick_events_at("2026-08-10T01:00:00Z")
        .is_empty());
    assert_eq!(
        tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        1,
        "disabled monitors still drive durable claim cleanup"
    );
}

#[test]
fn scheduled_tick_is_single_flight_per_canonical_project_scope() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled prefs");
    let tabs = vec![
        sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]),
        sample_project_tab(
            "tab-2",
            "Repo duplicate",
            repo.clone(),
            ProjectKind::Git,
            &[],
        ),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    assert!(runtime
        .issue_monitor_scheduled_tick_events_at("2026-08-10T01:00:00Z")
        .is_empty());
    assert!(runtime
        .issue_monitor_scheduled_tick_events_at("2026-08-10T01:00:01Z")
        .is_empty());

    assert_eq!(
        tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len(),
        1,
        "duplicate tabs and duplicate ticks enqueue one worker"
    );
    assert_eq!(
        runtime
            .project_state_for_root(&repo)
            .unwrap()
            .issue_monitor_scheduled_scans_in_flight
            .len(),
        1
    );
}

#[test]
fn authenticated_pm_scan_now_enqueues_one_exact_project_worker_and_refuses_overlap() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (repo, owner, pm, _) =
        super::managed_launch_tests::managed_owner_fixture(temp.path(), true);
    let foreign_repo = temp.path().join("foreign-repo");
    fs::create_dir_all(&foreign_repo).unwrap();
    init_repo_with_initial_commit(&foreign_repo);
    let foreign_prefs = gwt::issue_monitor_prefs_path_for_repo_path(&foreign_repo);
    gwt::save_issue_monitor_prefs(
        &foreign_prefs,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            ..Default::default()
        },
    )
    .unwrap();
    let foreign_before = fs::read(&foreign_prefs).unwrap();
    let mismatched = owner_scan_reply(
        &owner,
        &pm,
        gwt_core::paths::project_scope_hash(&foreign_repo).to_string(),
    );
    assert_eq!(mismatched["accepted"], false);
    assert_eq!(mismatched["reason"], "project_scope_mismatch");

    gwt::daemon_publisher::publish_issue_monitor_control(
        &repo,
        gwt::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({"enabled":true}),
            std::process::id(),
        ),
    )
    .unwrap();
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bound the real daemon IPC/worker idle barrier; the canonical driver runs independently of the caller executor.
        tokio::time::timeout(
            gwt_core::deadline_budget::HANG_GUARD,
            owner.wait_monitor_idle(),
        )
        .await
        .unwrap()
        .unwrap();
    });
    let scan = owner.pause_next_monitor_scan().unwrap();
    assert_eq!(
        scan.scans_started(),
        0,
        "no scan starts before the exact PM request"
    );
    let scope = gwt_core::paths::project_scope_hash(&repo).to_string();
    let accepted = owner_scan_reply(&owner, &pm, scope.clone());
    assert_eq!(accepted["accepted"], true, "{accepted}");
    assert!(accepted["reason"].is_null());
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Observe arrival at the real canonical scan hook; this is an IPC/worker hang guard, not policy timing.
        tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, scan.wait())
            .await
            .unwrap();
    });
    assert_eq!(
        scan.scans_started(),
        1,
        "one canonical project scan is physically blocked"
    );
    let overlapping = owner_scan_reply(&owner, &pm, scope);
    assert_eq!(overlapping["accepted"], false);
    assert_eq!(overlapping["reason"], "scan_already_in_flight");
    assert_eq!(
        scan.scans_started(),
        1,
        "overlap never creates a second worker"
    );
    assert_eq!(
        fs::read(&foreign_prefs).unwrap(),
        foreign_before,
        "the foreign Monitor is untouched"
    );
    scan.release();
    owner_fixture_block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bound the real daemon IPC/worker idle barrier; the canonical driver runs independently of the caller executor.
        tokio::time::timeout(
            gwt_core::deadline_budget::HANG_GUARD,
            owner.wait_monitor_idle(),
        )
        .await
        .unwrap()
        .unwrap();
    });
}

fn owner_scan_reply(
    owner: &gwt::project_runtime::test_support::ProjectOwnerHarness,
    caller: &gwt::project_runtime::test_support::OwnerRuntimeProbe,
    expected_project_scope: String,
) -> serde_json::Value {
    let client = owner_fixture_block_on(owner.agent_client("scan-client")).unwrap();
    let reply = owner_fixture_block_on(client.request(
        caller.grant.as_ref().unwrap().clone(),
        AgentFrontendRequest::IssueMonitorScanNow {
            expected_project_scope,
        },
    ))
    .unwrap();
    reply
        .events
        .into_iter()
        .find(|event| event["kind"] == "issue_monitor_scan_request_result")
        .expect("actual owner scan request result")
}

#[test]
fn authenticated_scan_now_rejects_non_pm_and_disabled_monitor_without_enqueuing() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (repo, owner, pm, ordinary) =
        super::managed_launch_tests::managed_owner_fixture(temp.path(), false);
    let scope = gwt_core::paths::project_scope_hash(&repo).to_string();
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let original = fs::read(&prefs_path).unwrap();
    let denied = owner_scan_reply(&owner, &ordinary, scope.clone());
    assert_eq!(denied["accepted"], false);
    assert_eq!(denied["reason"], "caller_not_registered_pm");
    assert_eq!(
        fs::read(&prefs_path).unwrap(),
        original,
        "refusal must not enqueue or mutate Monitor state"
    );

    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    prefs.enabled = false;
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
    let disabled_state = fs::read(&prefs_path).unwrap();
    let disabled = owner_scan_reply(&owner, &pm, scope.clone());
    assert_eq!(disabled["accepted"], false);
    assert_eq!(disabled["reason"], "monitor_disabled");
    assert_eq!(fs::read(&prefs_path).unwrap(), disabled_state);

    // Project availability is an immutable owner dispatch fact. Test the
    // existing pure guard with the actual captured PM identity and row.
    let mut window =
        gwt::persistence::load_workspace_state(&gwt::persistence::workspace_state_path(&repo))
            .unwrap()
            .windows
            .into_iter()
            .find(|window| {
                window.session_id.as_deref()
                    == pm.session.as_ref().map(|session| session.id.as_str())
            })
            .unwrap();
    window.id = pm.id.clone();
    let panes = [gwt::project_runtime::agent_dispatch::PaneProjection {
        project_root: repo.clone(),
        tab_id: pm.active_session.as_ref().unwrap().tab_id.clone(),
        window,
        active: pm.active_session.clone(),
    }];
    assert_eq!(
        gwt::project_runtime::agent_dispatch::issue_monitor_scan_permission(
            &repo,
            &panes,
            false,
            pm.grant.as_ref().unwrap().principal(),
            &scope,
        ),
        Err("project_not_open")
    );
    assert_eq!(
        fs::read(&prefs_path).unwrap(),
        disabled_state,
        "all rejected requests leave the canonical Monitor untouched"
    );
}

#[test]
fn authenticated_scan_now_reports_unavailable_daemon_without_enqueuing() {
    let temp = tempdir().unwrap();
    let _home = ScopedGwtHome::set(temp.path());
    let (repo, owner, pm, _) =
        super::managed_launch_tests::managed_owner_fixture(temp.path(), false);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).unwrap();
    prefs.enabled = true;
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
    let before = fs::read(&prefs_path).unwrap();
    let scope = gwt_core::daemon::RuntimeScope::from_project_root(
        &repo,
        gwt_core::daemon::RuntimeTarget::Host,
    )
    .unwrap();
    let endpoint = scope.endpoint_path(&gwt_core::paths::gwt_home());
    let withheld = endpoint.with_extension("withheld-fixture");
    fs::rename(&endpoint, &withheld).unwrap();
    let refused = owner_scan_reply(
        &owner,
        &pm,
        gwt_core::paths::project_scope_hash(&repo).to_string(),
    );
    fs::rename(&withheld, &endpoint).unwrap();
    assert_eq!(refused["accepted"], false);
    assert_eq!(refused["reason"], "monitor_state_unavailable");
    assert_eq!(
        fs::read(&prefs_path).unwrap(),
        before,
        "an unadmitted scan leaves canonical Monitor state untouched"
    );
    let health =
        owner_fixture_block_on(owner.command(gwt::project_runtime::events::OwnerCommand::Health))
            .unwrap();
    assert_eq!(
        health["panes"], 2,
        "failed Monitor admission must preserve both actual owner panes"
    );
    assert_eq!(health["pending_transport"], 0);
}

#[test]
fn scheduled_tick_spawn_failure_is_observable_and_releases_single_flight() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled prefs");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime.blocking_tasks = BlockingTaskSpawner::failing("injected spawn failure");

    let events = runtime.issue_monitor_scheduled_tick_events_at("2026-08-10T01:00:00Z");

    assert!(runtime
        .project_state(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .is_empty());
    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, message, .. }
            if level == "error" && message.contains("injected spawn failure")
    )));
}

#[test]
fn scheduled_scan_completion_defers_projection_until_worker_runs() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let prefs = gwt::IssueMonitorPrefs {
        enabled: true,
        ..gwt::IssueMonitorPrefs::default()
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("prefs");
    let monitor = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), prefs);
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .insert(prefs_path.clone());

    let events = runtime.issue_monitor_scheduled_scan_complete_events(
        &repo,
        &prefs_path,
        "2026-08-10T01:00:00Z",
        Ok(ScheduledIssueMonitorScanOutcome::Applied(Box::new(monitor))),
    );
    assert!(
        events.is_empty(),
        "GUI ingress must not prepare the snapshot"
    );
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "one worker prepares the result"
    );
}

#[test]
fn scheduled_scan_completion_rebases_ephemeral_queue_on_latest_controls() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    gwt::machine_agent_settings::set(
        Some(1),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-22T00:00:00Z",
    )
    .expect("seed machine Manual capacity");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let initial = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        enabled: true,
        max_active_agents: 1,
        ..queued_issue_monitor_prefs(&[43])
    };
    gwt::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed prefs");
    let mut scanned = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), initial);
    gwt::scan_issue_monitor_candidates(
        &mut scanned,
        &[pm_wake_inbox_item(43, gwt::MonitorInboxState::Queued).issue],
        "2026-08-10T01:00:00Z",
    );
    let latest = gwt::IssueMonitorPrefs {
        max_active_agents_mode: gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        enabled: true,
        max_active_agents: 4,
        priority_order: vec![43],
        ..queued_issue_monitor_prefs(&[43])
    };
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    let (proxy, completions) = AppEventProxy::stub();
    runtime.proxy = proxy;
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .insert(prefs_path.clone());

    assert!(runtime
        .issue_monitor_scheduled_scan_complete_events(
            &repo,
            &prefs_path,
            "2026-08-10T01:00:00Z",
            Ok(ScheduledIssueMonitorScanOutcome::Applied(Box::new(scanned))),
        )
        .is_empty());
    tasks.lock().unwrap().remove(0)();
    let UserEvent::IssueMonitorScheduledScanPrepared(stale) =
        into_recorded_project_payload(completions.lock().unwrap().remove(0))
    else {
        panic!("prepared completion")
    };
    // Controls can change after preparation but before the GUI applies it.
    gwt::machine_agent_settings::set(
        Some(4),
        gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
        "2026-10-22T00:01:00Z",
    )
    .expect("concurrent machine control");
    gwt::save_issue_monitor_prefs(&prefs_path, &latest).expect("concurrent controls");
    assert!(runtime
        .issue_monitor_scheduled_scan_prepared_events(*stale)
        .is_empty());
    assert_eq!(
        tasks.lock().unwrap().len(),
        1,
        "stale projection is reprepared"
    );
    tasks.lock().unwrap().remove(0)();
    let UserEvent::IssueMonitorScheduledScanPrepared(current) =
        into_recorded_project_payload(completions.lock().unwrap().remove(0))
    else {
        panic!("reprepared completion")
    };
    let events = runtime.issue_monitor_scheduled_scan_prepared_events(*current);
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("scheduled status");
    assert_eq!(status.queue_len, 1, "the live queue survives completion");
    assert_eq!(
        status.max_active_agents, 4,
        "newer controls win over the worker snapshot"
    );
}

#[test]
fn scheduled_scan_completion_stays_silent_after_disable_or_project_close() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let enabled = gwt::IssueMonitorPrefs {
        enabled: true,
        ..gwt::IssueMonitorPrefs::default()
    };
    let scanned = test_monitor_with_prefs(gwt::IssueMonitorConfig::default(), enabled.clone());
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: false,
            ..enabled.clone()
        },
    )
    .expect("disable during scan");
    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .insert(prefs_path.clone());
    assert!(runtime
        .complete_scheduled_scan_for_test(
            &repo,
            &prefs_path,
            "2026-08-10T01:00:00Z",
            Ok(ScheduledIssueMonitorScanOutcome::Applied(Box::new(
                scanned.clone(),
            ))),
        )
        .is_empty());

    gwt::save_issue_monitor_prefs(&prefs_path, &enabled).expect("re-enable");
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .insert(prefs_path.clone());
    runtime.tabs.clear();
    assert!(runtime
        .complete_scheduled_scan_for_test(
            &repo,
            &prefs_path,
            "2026-08-10T01:01:00Z",
            Ok(ScheduledIssueMonitorScanOutcome::Applied(Box::new(scanned))),
        )
        .is_empty());
}

#[test]
/// Issue #3528: a completion probe spawns one `gh` per issue, so probing the
/// whole open list burned the scan's deadline on work it could never use. The
/// planner only walks far enough to fill the free claim slots, so the scan
/// must probe exactly that far and no further.
fn completion_probes_stop_once_the_free_claim_slots_are_filled() {
    let candidates = (1..=50).collect::<Vec<u64>>();
    let probed = std::cell::RefCell::new(Vec::new());

    let observed = super::super::claim_candidate_completion_observations(
        1,
        candidates.clone(),
        |issue_number| {
            probed.borrow_mut().push(issue_number);
            Ok::<_, std::convert::Infallible>(false)
        },
    )
    .expect("infallible probe");

    assert_eq!(observed, std::collections::BTreeMap::from([(1, false)]));
    assert_eq!(
        probed.into_inner(),
        vec![1],
        "one free slot must cost exactly one probe, not one per open issue"
    );

    // A completed candidate frees no slot, so the walk continues past it
    // exactly like the claim planner does. Every answer is kept by identity.
    let probed = std::cell::RefCell::new(Vec::new());
    let observed =
        super::super::claim_candidate_completion_observations(1, candidates, |issue_number| {
            probed.borrow_mut().push(issue_number);
            Ok::<_, std::convert::Infallible>(issue_number <= 2)
        })
        .expect("infallible probe");

    assert_eq!(
        observed,
        std::collections::BTreeMap::from([(1, true), (2, true), (3, false)])
    );
    assert_eq!(probed.into_inner(), vec![1, 2, 3]);
}

fn dependency_completion_fixture() -> (
    gwt::IssueMonitorState,
    gwt::issue_monitor_worker::LoadedIssueMonitorCandidates,
) {
    let mut monitor = test_monitor_with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            enabled: true,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..queued_issue_monitor_prefs(&[42])
        },
    );
    monitor.set_gui_connected(true);
    let issues = [42, 44]
        .into_iter()
        .map(|number| gwt::IssueMonitorIssue {
            author_login: None,
            number,
            title: format!("Issue {number}"),
            labels: if number == 44 {
                vec!["gwt-spec".to_string()]
            } else {
                Vec::new()
            },
            state: gwt::IssueMonitorIssueState::Open,
            body: (number == 42).then(|| "Blocked-by: #44".to_string()),
            url: None,
            readiness: gwt::IssueMonitorReadiness::NotApplicable,
            updated_at: Some("2026-10-23T00:00:00Z".to_string()),
        })
        .collect::<Vec<_>>();
    gwt::scan_issue_monitor_candidates(&mut monitor, &issues, "2026-10-23T00:00:00Z");
    let loaded = gwt::issue_monitor_worker::LoadedIssueMonitorCandidates {
        open_pr_observations: BTreeMap::new(),
        open_pr_observed_at: None,
        issues,
        source: gwt::IssueMonitorCandidateSource::Live,
        live_error: None,
        readiness_failures: Vec::new(),
        urgent_assignments: Default::default(),
    };
    (monitor, loaded)
}

#[test]
fn local_claim_observation_rechecks_the_frontier_after_dependency_completion() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (monitor, loaded) = dependency_completion_fixture();
    let probed = Arc::new(Mutex::new(Vec::new()));
    let _hook = super::super::set_local_completion_probe_test_hook({
        let probed = Arc::clone(&probed);
        move |number| {
            probed.lock().expect("probe log").push(number);
            Ok(number == 44)
        }
    });

    let observed =
        super::super::observe_local_claim_candidates(&monitor, &loaded, "example", "repo")
            .expect("completion observations")
            .expect("authorized live scan");

    assert_eq!(probed.lock().expect("probe log").as_slice(), &[44, 42]);
    assert_eq!(observed.1, BTreeMap::from([(42, false)]));
    let blocker = loaded
        .issues
        .iter()
        .find(|issue| issue.number == 44)
        .expect("the probed dependency");
    assert_eq!(
        observed.2,
        BTreeMap::from([(
            44,
            gwt::issue_monitor::IssueMonitorDependencyObservation::new(blocker, Some(true)),
        )])
    );
}

#[test]
fn local_claim_observation_reprobes_pending_claim_dependencies_without_free_capacity() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut monitor, loaded) = dependency_completion_fixture();
    monitor.set_max_active_agents(1);
    let now = "2026-10-23T00:00:00Z";
    assert_eq!(
        monitor.try_prepare_claim_effects_with_probe_confirmed("owner", now, 1, None, |number| {
            Ok::<_, ()>(if number == 44 {
                gwt::issue_monitor::ClaimProbeOutcome::Completed
            } else {
                gwt::issue_monitor::ClaimProbeOutcome::Claimable
            })
        }),
        Ok(1)
    );
    gwt::scan_issue_monitor_candidates(&mut monitor, &loaded.issues, now);
    assert_eq!(monitor.claim_probe_plan(1).0, 0);
    let probed = Arc::new(Mutex::new(Vec::new()));
    let _hook = super::super::set_local_completion_probe_test_hook({
        let probed = Arc::clone(&probed);
        move |number| {
            probed.lock().expect("probe log").push(number);
            Ok(true)
        }
    });

    let observed =
        super::super::observe_local_claim_candidates(&monitor, &loaded, "example", "repo")
            .expect("completion observations")
            .expect("authorized live scan");

    assert_eq!(probed.lock().expect("probe log").as_slice(), &[44]);
    assert!(observed.1.is_empty(), "no slot permits an owner probe");
    let blocker = loaded
        .issues
        .iter()
        .find(|issue| issue.number == 44)
        .expect("the probed dependency");
    assert_eq!(
        observed.2,
        BTreeMap::from([(
            44,
            gwt::issue_monitor::IssueMonitorDependencyObservation::new(blocker, Some(true)),
        )])
    );
    monitor.observe_dependency_completions(&observed.2);
    prepare_local_issue_monitor_claim_proposals(
        &mut monitor,
        &loaded,
        &observed.0,
        now,
        &observed.1,
    );
    assert_eq!(pending_claim_issue_numbers(&monitor.prefs()), vec![42]);
}

#[test]
fn local_claim_observation_discards_the_scan_when_a_dependency_probe_expires() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (monitor, loaded) = dependency_completion_fixture();
    let _hook = super::super::set_local_completion_probe_test_hook(|number| {
        if number != 44 {
            return Ok(false);
        }
        Err(
            gwt::issue_monitor_worker::IssueMonitorCompletionProbeFailure::Deadline(
                gwt::issue_monitor_worker::IssueMonitorScanFailure::new(
                    gwt::issue_monitor_worker::IssueMonitorScanStage::ClaimCompletionReadback,
                    "dependency completion deadline expired",
                ),
            ),
        )
    });

    assert!(
        super::super::observe_local_claim_candidates(&monitor, &loaded, "example", "repo").is_err()
    );
}

/// Issue #3528 (SPEC #3200 FR-057): without a saved launch profile the scan
/// can never claim, so it must not spend a single `gh` probe on the frontier.
#[test]
fn scheduled_scan_probes_no_candidate_without_a_launch_profile() {
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
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let (repo, prefs_path) = seed_scheduled_scan_claim_fixture(&temp, None);
    let probed = Arc::new(Mutex::new(Vec::new()));
    let _hook = super::super::set_local_completion_probe_test_hook({
        let probed = Arc::clone(&probed);
        move |issue_number| {
            probed.lock().expect("probe log").push(issue_number);
            Ok(false)
        }
    });

    let outcome = super::super::run_scheduled_issue_monitor_scan_with_budgets(
        &repo,
        Some("tab-1"),
        None,
        None,
        None,
        "2026-09-07T07:00:00Z",
        &super::super::default_issue_client_factory(),
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(30),
        &HashSet::new(),
    )
    .expect("scan");
    super::super::wait_scan_generation_reaper(&prefs_path);
    assert!(matches!(
        outcome,
        ScheduledIssueMonitorScanOutcome::Applied(_)
    ));
    assert!(
        probed.lock().expect("probe log").is_empty(),
        "no launch profile means no claim, so no completion probe"
    );
}

/// Issue #3528 (SPEC #3200 FR-059 / FR-060, #3165 FR-098): a completion probe
/// that hits the observation deadline is fail-closed for the claim proposal —
/// no candidate is claimed on a partial answer — while the read model and the
/// typed stage error still commit under the fresh commit budget.
#[test]
fn scheduled_scan_discards_claim_proposals_when_the_completion_probe_expires() {
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
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let (repo, prefs_path) =
        seed_scheduled_scan_claim_fixture(&temp, Some(sample_issue_monitor_launch_profile()));
    let probed = Arc::new(Mutex::new(Vec::new()));
    let _hook = super::super::set_local_completion_probe_test_hook({
        let probed = Arc::clone(&probed);
        move |issue_number| {
            probed.lock().expect("probe log").push(issue_number);
            Err(
                gwt::issue_monitor_worker::IssueMonitorCompletionProbeFailure::Deadline(
                    gwt::issue_monitor_worker::IssueMonitorScanFailure::new(
                        gwt::issue_monitor_worker::IssueMonitorScanStage::ClaimCompletionReadback,
                        "operation deadline expired during claim-completion-readback",
                    ),
                ),
            )
        }
    });

    let outcome = super::super::run_scheduled_issue_monitor_scan_with_budgets(
        &repo,
        Some("tab-1"),
        None,
        None,
        None,
        "2026-09-07T07:00:00Z",
        &super::super::default_issue_client_factory(),
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(30),
        &HashSet::new(),
    )
    .expect("an expired probe must not fail the commit");
    super::super::wait_scan_generation_reaper(&prefs_path);

    let ScheduledIssueMonitorScanOutcome::Applied(state) = outcome else {
        panic!("the scan holds authority, so it must commit its own findings");
    };
    assert_eq!(probed.lock().expect("probe log").as_slice(), &[43]);
    let status = state.status_view();
    assert!(
        status
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("claim-completion-readback")),
        "the expired probe must be reported as its stage: {:?}",
        status.last_error
    );
    assert_eq!(status.last_scan_at.as_deref(), Some("2026-09-07T07:00:00Z"));
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert!(
        pending_claim_issue_numbers(&persisted).is_empty(),
        "an expired observation must not turn into a claim: {:?}",
        persisted.pending_effects
    );
}

/// Issue #3528 (SPEC #3200 FR-059, #3165 FR-098): an ordinary readback error
/// while the deadline is still valid keeps the #3165 fail-open contract, so a
/// transient `gh` failure never blocks real work.
#[test]
fn scheduled_scan_keeps_fail_open_for_an_ordinary_probe_error() {
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
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let (repo, prefs_path) =
        seed_scheduled_scan_claim_fixture(&temp, Some(sample_issue_monitor_launch_profile()));
    let _hook = super::super::set_local_completion_probe_test_hook(|_| {
        Err(
            gwt::issue_monitor_worker::IssueMonitorCompletionProbeFailure::Operation(
                gwt::issue_monitor_worker::IssueMonitorScanFailure::new(
                    gwt::issue_monitor_worker::IssueMonitorScanStage::ClaimCompletionReadback,
                    "gh merged query failed",
                ),
            ),
        )
    });
    // The fail-open claim proposal is driven through the issue client. The
    // default factory would resolve a token from the fake `gh` and call the
    // real api.github.com, so network latency could burn the scan budget.
    // An offline fake that has never seen #43 rejects the claim pre-submit,
    // which deterministically leaves the proposal pending for a retry.
    let fake_client = Arc::new(FakeIssueClient::new());
    let issue_client_factory: super::super::RuntimeIssueClientFactory = Arc::new({
        let fake_client = Arc::clone(&fake_client);
        move |_owner, _repo| {
            let client: Arc<dyn IssueClient> = fake_client.clone();
            Ok(client)
        }
    });

    let outcome = super::super::run_scheduled_issue_monitor_scan_with_budgets(
        &repo,
        Some("tab-1"),
        None,
        None,
        None,
        "2026-09-07T07:00:00Z",
        &issue_client_factory,
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(30),
        &HashSet::new(),
    )
    .expect("scan");
    super::super::wait_scan_generation_reaper(&prefs_path);
    assert!(matches!(
        outcome,
        ScheduledIssueMonitorScanOutcome::Applied(_)
    ));
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(
        pending_claim_issue_numbers(&persisted),
        vec![43],
        "an ordinary probe error within budget stays fail-open"
    );
    assert!(
        fake_client
            .call_log()
            .iter()
            .any(|call| call == "fetch:#43"),
        "the claim attempt must go through the injected offline client: {:?}",
        fake_client.call_log()
    );
}

/// Issue #3528: the read/probe phase and the commit phase shared one deadline,
/// so a scan slow enough to exhaust it failed at its own commit with
/// `Issue Monitor authority commit check failed: operation deadline expired
/// during file lock` — every tick threw away what it had just learned and the
/// monitor never advanced. An exhausted read phase must degrade its findings,
/// never block the commit.
#[test]
fn scheduled_scan_commits_after_the_read_phase_exhausts_its_budget() {
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
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled prefs");

    let outcome = super::super::run_scheduled_issue_monitor_scan_with_budgets(
        &repo,
        Some("tab-1"),
        None,
        None,
        None,
        "2026-08-12T07:00:00Z",
        &super::super::default_issue_client_factory(),
        std::time::Duration::ZERO,
        std::time::Duration::from_secs(30),
        &HashSet::new(),
    )
    .expect("an exhausted read phase must not fail the commit");
    super::super::wait_scan_generation_reaper(&prefs_path);

    let ScheduledIssueMonitorScanOutcome::Applied(state) = outcome else {
        panic!("the scan holds authority, so it must commit its own findings");
    };
    let status = state.status_view();
    assert!(
        status
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("deadline")),
        "the degraded read phase must be reported, not swallowed: {:?}",
        status.last_error
    );
    assert_eq!(status.last_scan_at.as_deref(), Some("2026-08-12T07:00:00Z"));
}

#[test]
fn generation_recovery_reservation_is_single_flight_and_periodic() {
    let mut state = super::super::ScanGenerationReaperState::default();
    let now = Instant::now();
    let interval = Duration::from_secs(60);
    assert!(state.try_reserve(now), "the first tick starts recovery");
    assert!(
        !state.try_reserve(now + interval),
        "even a later tick cannot queue recovery while its worker is held"
    );
    state.running = false;
    assert!(
        !state.try_reserve(now + interval - Duration::from_secs(1)),
        "completed recovery must not run again on every tick"
    );
    assert!(state.try_reserve(now + interval), "the next period is due");
}

#[test]
fn generation_recovery_releases_its_reservation_when_the_worker_unwinds() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let prefs_path = temp.path().join("issue-monitor.json");
    assert!(super::super::scan_generation_reapers()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(prefs_path.clone())
        .or_default()
        .try_reserve(Instant::now()));
    assert!(std::panic::catch_unwind(|| {
        let _flight = super::super::ScanGenerationReaperFlight(prefs_path.clone());
        panic!("recovery failed");
    })
    .is_err());
    assert!(
        !super::super::scan_generation_reapers()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[&prefs_path]
            .running,
        "panic and a dropped spawn closure must release the in-flight reservation"
    );
}

/// Issue #4975 AC-A3: the scan reads the committed snapshot without joining
/// a concurrent writer's transaction.
#[test]
fn issue_monitor_prefs_snapshot_is_readable_while_its_writer_lock_is_held() {
    let temp = tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let prefs_path = temp.path().join("issue-monitor.json");
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed committed prefs");
    let writer_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(prefs_path.with_extension("lock"))
        .expect("open writer lock");
    writer_lock.lock_exclusive().expect("hold writer lock");
    let (read_tx, read_rx) = mpsc::channel();
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            read_tx
                .send(gwt::load_issue_monitor_prefs(&prefs_path))
                .expect("report snapshot read");
        });
        let result = read_rx.recv_timeout(Duration::from_secs(1));
        FileExt::unlock(&writer_lock).expect("release writer lock");
        result
    });
    assert!(
        matches!(result, Ok(Ok(prefs)) if prefs.enabled),
        "snapshot reads must not wait for the prefs writer"
    );
}

/// Even the daemon-owned tick must return while local recovery is busy.
#[test]
fn scheduled_scan_returns_while_generation_recovery_is_running() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled prefs");
    let daemon = gwt::IssueMonitorAuthorityFence::current_process();
    let (_, _daemon_lease) =
        gwt::establish_issue_monitor_authority_fence(&prefs_path, &daemon, |_| false)
            .expect("live daemon authority");
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let _hook = super::super::ScopedScanGenerationReaperTestHook::set(&prefs_path, move || {
        started_tx.send(()).expect("report recovery started");
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(30))
            .expect("release recovery");
    });
    let (completed_tx, completed_rx) = mpsc::channel();
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            let outcome = super::super::run_scheduled_issue_monitor_scan_with_budgets(
                &repo,
                None,
                None,
                None,
                None,
                "2026-10-14T07:00:00Z",
                &super::super::default_issue_client_factory(),
                Duration::from_secs(60),
                Duration::from_secs(30),
                &HashSet::new(),
            );
            completed_tx.send(outcome).expect("report scan completed");
        });
        started_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("recovery started");
        let result = completed_rx.recv_timeout(Duration::from_secs(1));
        // Release before asserting so the synchronous regression fails cleanly.
        release_tx.send(()).expect("release recovery");
        result
    });
    super::super::wait_scan_generation_reaper(&prefs_path);
    assert!(
        matches!(
            result,
            Ok(Ok(ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon))
        ),
        "the scan waited for generation recovery: {result:?}"
    );
}

/// Issue #3934: the generation reaper used to run once, during bootstrap. A
/// holder that died after that kept its owner unlaunchable until someone
/// restarted the GUI, which is why 45 Issues had to be re-registered under
/// fresh numbers. Periodic background recovery must remain available.
#[test]
fn scheduled_scan_reclaims_a_defunct_generation_in_background() {
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
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3934,
    };
    let session_id = "scan-defunct-holder";
    // The scan worker resolves its own paths from the process-global HOME, so
    // the holder has to be durable where that worker will look for it.
    seed_defunct_active_owner(
        &gwt_core::paths::gwt_sessions_dir(),
        &repo,
        &repo,
        "work/scan-defunct-owner",
        owner,
        session_id,
        gwt_agent::AgentStatus::Interrupted,
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("load ledger")
            .expect("ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Active),
        "the owner starts the scan holding a live-looking generation"
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled prefs");

    let _outcome = super::super::run_scheduled_issue_monitor_scan_with_budgets(
        &repo,
        Some("tab-1"),
        None,
        None,
        None,
        "2026-09-04T07:00:00Z",
        &super::super::default_issue_client_factory(),
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(30),
        &HashSet::new(),
    )
    .expect("scan runs");
    super::super::wait_scan_generation_reaper(&prefs_path);

    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("load ledger")
            .expect("ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
        "the scan must release a generation whose holder is gone"
    );
}

/// Issue #3964 AC-1: in production a live daemon owns the scan, so the GUI
/// worker answered `DeferredToLiveDaemon` before it ever reached the reaper —
/// and the daemon has no reaper. The scan-cadence recovery therefore never ran
/// after startup; 38 stranded generations survived every scan. The reaper is
/// local recovery, not a scan effect, and remains independent of scan authority.
#[test]
fn scheduled_scan_reclaims_a_defunct_generation_even_when_a_live_daemon_owns_the_scan() {
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
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 3964,
    };
    let session_id = "deferred-scan-defunct-holder";
    seed_defunct_active_owner(
        &gwt_core::paths::gwt_sessions_dir(),
        &repo,
        &repo,
        "work/deferred-scan-defunct-owner",
        owner,
        session_id,
        gwt_agent::AgentStatus::Interrupted,
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled prefs");
    let daemon = gwt::IssueMonitorAuthorityFence::current_process();
    let (_, _daemon_lease) =
        gwt::establish_issue_monitor_authority_fence(&prefs_path, &daemon, |_| false)
            .expect("live daemon authority");

    let outcome = super::super::run_scheduled_issue_monitor_scan_with_budgets(
        &repo,
        Some("tab-1"),
        None,
        None,
        None,
        "2026-09-05T07:00:00Z",
        &super::super::default_issue_client_factory(),
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(30),
        &HashSet::new(),
    )
    .expect("scan runs");
    super::super::wait_scan_generation_reaper(&prefs_path);

    assert!(
        matches!(
            outcome,
            ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon
        ),
        "scan authority still belongs to the daemon"
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("load ledger")
            .expect("ledger")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
        "background recovery must run even when the scan defers to the daemon"
    );
}

#[test]
fn scheduled_scan_defers_to_live_daemon_without_remote_io_or_launch() {
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
    let marker = temp.path().join("gh-called");
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let _marker = ScopedEnvVar::set("GWT_FAKE_GH_MARKER", &marker);
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled prefs");
    let daemon = gwt::IssueMonitorAuthorityFence::current_process();
    let (_, daemon_lease) =
        gwt::establish_issue_monitor_authority_fence(&prefs_path, &daemon, |_| false)
            .expect("live daemon authority");
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, recorded) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, scan_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    assert!(runtime
        .issue_monitor_scheduled_tick_events_at("2026-08-10T01:00:00Z")
        .is_empty());
    let (completed_root, completed_prefs, completed_at, outcome) =
        run_scheduled_scan_to_completion(&scan_tasks, &recorded);
    super::super::wait_scan_generation_reaper(&completed_prefs);
    assert!(matches!(
        &outcome,
        Ok(ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon)
    ));
    let events = runtime.complete_scheduled_scan_for_test(
        &completed_root,
        &completed_prefs,
        &completed_at,
        outcome,
    );

    assert!(events.is_empty());
    assert!(!marker.exists(), "live daemon authority skips GitHub I/O");
    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("prefs");
    assert!(persisted.pending_effects.is_empty());
    assert!(persisted.pending_launch_deliveries.is_empty());
    drop(daemon_lease);
}

#[test]
fn scheduled_scan_defer_still_rearms_periodic_wake_for_durable_standing_work() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut monitor = test_monitor_with_prefs(
        gwt::IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..gwt::IssueMonitorConfig::default()
        },
        gwt::load_issue_monitor_prefs(&prefs_path).expect("prefs"),
    );
    gwt::scan_issue_monitor_candidates(
        &mut monitor,
        &[pm_wake_inbox_item(42, gwt::MonitorInboxState::Queued).issue],
        "2026-08-10T00:00:00Z",
    );
    monitor.complete_active_launch(42, "tab-1::other-window");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("standing work");
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 12,
            last_continued_at: Some("2026-08-10T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("quiet loop");
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .insert(prefs_path.clone());

    let events = runtime.complete_scheduled_scan_for_test(
        &repo,
        &prefs_path,
        "2026-08-10T01:00:00Z",
        Ok(ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon),
    );

    assert!(events.is_empty(), "defer adds no observer snapshot");
    assert_eq!(
        gwt::pm_registry::load_pm_loop_state(&loop_path)
            .expect("rearmed loop")
            .last_wake_at
            .as_deref(),
        Some("2026-08-10T01:00:00Z"),
        "standing-work wake is independent of scan authority"
    );
}

#[test]
fn scheduled_scan_reload_error_rearms_periodic_wake_from_the_worker_snapshot() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().expect("tempdir");
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let (repo, mut runtime, _pm_window_id) = pm_wake_fixture(&temp);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    let mut monitor = test_monitor_with_prefs(
        gwt::IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..gwt::IssueMonitorConfig::default()
        },
        gwt::load_issue_monitor_prefs(&prefs_path).expect("prefs"),
    );
    gwt::scan_issue_monitor_candidates(
        &mut monitor,
        &[pm_wake_inbox_item(42, gwt::MonitorInboxState::Queued).issue],
        "2026-08-10T00:00:00Z",
    );
    monitor.complete_active_launch(42, "tab-1::other-window");
    let loop_path = gwt::pm_registry::pm_loop_state_path_for_repo_path(&repo);
    gwt::pm_registry::save_pm_loop_state(
        &loop_path,
        &gwt::pm_registry::PmLoopState {
            consecutive_continuations: 12,
            last_continued_at: Some("2026-08-10T00:00:00Z".to_string()),
            ..gwt::pm_registry::PmLoopState::default()
        },
    )
    .expect("quiet loop");
    fs::write(&prefs_path, b"{").expect("corrupt prefs after worker completion");
    runtime
        .project_state_mut(&runtime.test_context())
        .unwrap()
        .issue_monitor_scheduled_scans_in_flight
        .insert(prefs_path.clone());

    let events = runtime.complete_scheduled_scan_for_test(
        &repo,
        &prefs_path,
        "2026-08-10T01:00:00Z",
        Ok(ScheduledIssueMonitorScanOutcome::Applied(Box::new(monitor))),
    );

    assert!(events.iter().any(|event| matches!(
        &event.event,
        BackendEvent::IssueMonitorToast { level, .. } if level == "error"
    )));
    assert_eq!(
        gwt::pm_registry::load_pm_loop_state(&loop_path)
            .expect("rearmed loop")
            .last_wake_at
            .as_deref(),
        Some("2026-08-10T01:00:00Z"),
        "a malformed disk snapshot must not discard the worker's standing-work wake"
    );
}

#[test]
fn scheduled_scan_discards_scanned_state_when_authority_appears_before_commit() {
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
    let marker = temp.path().join("gh-called");
    let _path = prepend_fake_gh_to_path(&fake_gh);
    let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");
    let _marker = ScopedEnvVar::set("GWT_FAKE_GH_MARKER", &marker);
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..gwt::IssueMonitorPrefs::default()
        },
    )
    .expect("seed enabled prefs");
    let before = fs::read(&prefs_path).expect("prefs bytes before scan");
    let hook_prefs_path = prefs_path.clone();
    set_scheduled_scan_after_lease_before_commit_test_hook(move || {
        gwt::persist_issue_monitor_authority_fence(
            &hook_prefs_path,
            &gwt::IssueMonitorAuthorityFence::current_process(),
        )
        .expect("inject authority after the worker lease pre-check");
    });
    let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
    let (mut runtime, recorded) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let (spawner, scan_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;

    runtime.issue_monitor_scheduled_tick_events_at("2026-08-10T01:00:00Z");
    let (completed_root, completed_prefs, completed_at, outcome) =
        run_scheduled_scan_to_completion(&scan_tasks, &recorded);
    super::super::wait_scan_generation_reaper(&completed_prefs);

    assert!(marker.exists(), "the side-effect-free scan completed first");
    assert!(matches!(
        &outcome,
        Ok(ScheduledIssueMonitorScanOutcome::DeferredToLiveDaemon)
    ));
    assert!(
        runtime
            .complete_scheduled_scan_for_test(
                &completed_root,
                &completed_prefs,
                &completed_at,
                outcome,
            )
            .is_empty()
    );
    assert_eq!(
        fs::read(&prefs_path).expect("prefs after deferred commit"),
        before,
        "authority recheck rejects every scanned/proposed mutation"
    );
}

/// Issue #3505: in the production GUI-only topology, an enabled scheduled
/// tick must advance a live queued issue toward materialization even when no
/// external daemon owns the scan cadence. Refreshing the read model alone is
/// not autonomous launch progress.
#[test]
fn scheduled_tick_advances_autonomous_launch_without_an_external_daemon() {
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
    fs::create_dir_all(&repo).expect("repo");
    init_repo_with_initial_commit(&repo);
    // Issue #3812: pausing the PM must not pause Monitor scan/claim/launch.
    gwt::pm_registry::set_pm_paused(&repo, true).expect("pause PM only");
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
    gwt::save_issue_monitor_prefs(
        &prefs_path,
        &gwt::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            launch_profile: Some(sample_issue_monitor_launch_profile()),
            ..queued_issue_monitor_prefs(&[43])
        },
    )
    .expect("seed enabled autonomous prefs");

    let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
    let (mut runtime, recorded) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
    let fake_client = Arc::new(FakeIssueClient::new());
    let (spawner, scan_tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    fake_client.seed(sample_issue_snapshot(
        43,
        "Refreshed issue",
        &["bug"],
        "Fresh body",
        "2026-04-20T00:00:00Z",
    ));
    runtime.issue_client_factory = Arc::new({
        let fake_client = Arc::clone(&fake_client);
        move |_owner, _repo| {
            let client: Arc<dyn IssueClient> = fake_client.clone();
            Ok(client)
        }
    });

    let events = runtime.issue_monitor_scheduled_tick_events_at("2026-08-10T01:00:00Z");
    assert!(events.is_empty(), "the tick returns before the remote scan");
    let (completed_root, completed_prefs, completed_at, outcome) =
        run_scheduled_scan_to_completion(&scan_tasks, &recorded);
    super::super::wait_scan_generation_reaper(&completed_prefs);
    let events = runtime.complete_scheduled_scan_for_test(
        &completed_root,
        &completed_prefs,
        &completed_at,
        outcome,
    );
    let status = events
        .iter()
        .find_map(|event| match &event.event {
            BackendEvent::IssueMonitorStatus { status } => Some(status),
            _ => None,
        })
        .expect("scheduled status");
    assert_eq!(
        status.total_candidates, 1,
        "the live candidate reached the scheduled snapshot"
    );

    let persisted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    let launch_advanced = !runtime.window_details.is_empty()
        || !persisted.launching_issues.is_empty()
        || !persisted.pending_launch_deliveries.is_empty()
        || !persisted.pending_effects.is_empty();
    assert!(
        launch_advanced,
        "a scheduled tick in the daemon-absent topology must do more than refresh the queue"
    );
}

/// A stopped Monitor has no daemon or scheduled scan to publish measurement
/// changes. The host sampler must own this notification independently.
#[test]
fn issue_monitor_capacity_sampler_notifies_stopped_projects_without_scanning() {
    let main = include_str!("../../main.rs");
    let sampler = main
        .split("let capacity_projects =")
        .nth(1)
        .expect("host sampler")
        .split("runtime_health_poller::spawn_runtime_health_poller")
        .next()
        .unwrap();
    assert!(
        sampler.contains("IssueMonitorCapacityChanged"),
        "capacity observations must notify stopped GUI monitors"
    );
    assert!(
        main.contains("app.issue_monitor_capacity_changed_events(&context, revision, update)"),
        "the event loop must publish the capacity-only overlay"
    );
    assert!(
        sampler.contains("CapacityUpdate::read") && sampler.contains("changed_from"),
        "prepare capacity and suppress unchanged notifications on the sampler worker"
    );
    assert!(
        sampler.contains("let Some(accepted) = accepted")
            && sampler.contains("previous.remove(&root)"),
        "initial status must become ready before a notification is consumed"
    );
    assert!(
        sampler.contains("Some(&accepted)"),
        "worker must preserve the GUI's last accepted Auto limit and Manual mode on read failure"
    );
    assert!(
        !sampler.contains("IssueMonitorScheduledTick"),
        "measurement changes must not trigger scans or PM wakes"
    );
    assert!(
        sampler.contains("refresh_machine_capacity_for_projects(&roots)"),
        "one tick must share its machine observation and target scan budget across projects"
    );
}

#[test]
fn issue_monitor_capacity_updates_stopped_auto_and_manual_without_changing_authority() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let active = temp.path().join("active");
    let other = temp.path().join("other");
    fs::create_dir_all(&active).unwrap();
    fs::create_dir_all(&other).unwrap();
    let tabs = vec![
        sample_project_tab("active", "Active", active, ProjectKind::Git, &[]),
        sample_project_tab("other", "Other", other.clone(), ProjectKind::Git, &[]),
    ];
    let mut runtime = sample_runtime(temp.path(), tabs, Some("active"));
    let (spawner, tasks) = BlockingTaskSpawner::queued();
    runtime.blocking_tasks = spawner;
    let other_key = runtime.project_context("other").unwrap().project_key;
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&other);
    gwt::save_issue_monitor_prefs(&prefs_path, &gwt::IssueMonitorPrefs::default()).unwrap();

    for manual in [None, Some(9)] {
        gwt::machine_agent_settings::set(
            manual,
            gwt::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
            "2026-10-22T00:00:00Z",
        )
        .expect("seed shared mode for the capacity observation");
        let mut durable = test_monitor_with_prefs(
            gwt::IssueMonitorConfig::default(),
            gwt::IssueMonitorPrefs::default(),
        );
        durable.set_max_active_agents_override(manual);
        gwt::save_issue_monitor_prefs(&prefs_path, &durable.prefs()).unwrap();
        let prefs_before = fs::read(&prefs_path).unwrap();
        let mut monitor = test_monitor_with_prefs(
            gwt::IssueMonitorConfig::default(),
            gwt::IssueMonitorPrefs::default(),
        );
        monitor.set_max_active_agents_override(manual);
        let mut expected = monitor.status_view();
        assert!(!expected.enabled);
        expected.queue_len = 17;
        expected.total_candidates = 41;
        expected.last_scan_at = Some("2026-10-01T00:00:00Z".into());
        expected.last_error = Some("daemon-owned diagnostic".into());
        expected.launch_profile_summary = "saved daemon profile".into();
        runtime.issue_monitor_daemon_status_events(&other, Box::new(expected.clone()));

        let fresh = gwt::agent_capacity::AgentCapacity {
            measurement_complete: true,
            machine_budget: Some(5),
            recommended_worker_limit: 4,
            recommended_total_count: 5,
            reason: "real observation".into(),
            ..Default::default()
        };
        let reduced = gwt::agent_capacity::AgentCapacity {
            machine_budget: Some(3),
            recommended_worker_limit: 2,
            recommended_total_count: 3,
            limiting_constraint: "cpu".into(),
            constraints: vec![gwt::agent_capacity::CapacityConstraint {
                resource: "cpu".into(),
                capacity: Some(3),
                binding: true,
                reason: "P cores 8; GUI 5 cores".into(),
            }],
            ..fresh.clone()
        };
        let expired = gwt::agent_capacity::AgentCapacity {
            observed_at: Some(1),
            expires_at: Some(2),
            ..reduced.clone()
        };
        for (observation, auto_limit) in [(fresh, 4), (reduced, 2), (expired, 2)] {
            let events = runtime.issue_monitor_capacity_changed_events_with(&other, |root| {
                assert_eq!(root, other);
                observation.clone()
            });
            assert_eq!(
                events.len(),
                1,
                "no Inbox or PM event on measurement refresh"
            );
            assert_eq!(events[0].target, DispatchTarget::Project(other_key.clone()));
            expected.agent_capacity = observation;
            expected.max_active_agents = manual.unwrap_or(auto_limit);
            let BackendEvent::IssueMonitorStatus { status } = &events[0].event else {
                panic!("capacity update must publish the existing status protocol");
            };
            assert_eq!(
                **status, expected,
                "preserve every noncapacity daemon field"
            );
            assert_eq!(status.max_active_agents_override, manual);
            assert!(
                runtime
                    .issue_monitor_capacity_changed_events_with(&other, |_| {
                        expected.agent_capacity.clone()
                    })
                    .is_empty(),
                "an unchanged capacity must not broadcast the full Monitor status"
            );
            let mut renewed = expected.agent_capacity.clone();
            let observed = renewed.observed_at.unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .saturating_sub(2)
            });
            renewed.observed_at = Some(observed + 1);
            renewed.expires_at = Some(renewed.expires_at.unwrap_or(observed + 30) + 1);
            assert!(
                runtime
                    .issue_monitor_capacity_changed_events_with(&other, |_| renewed)
                    .is_empty(),
                "a timestamp-only renewal must not broadcast the full Monitor status"
            );
        }
        let rows = gwt_core::error_ledger::list_since(None).unwrap();
        let reductions = rows
            .iter()
            .filter(|row| row.message.contains("Auto capacity reduced"))
            .collect::<Vec<_>>();
        assert_eq!(
            reductions.len(),
            1,
            "only Auto reductions are reported once"
        );
        assert_eq!(reductions[0].context["previous_limit"], "4");
        assert_eq!(reductions[0].context["effective_limit"], "2");
        assert_eq!(reductions[0].context["limiting_constraint"], "cpu");
        assert_eq!(fs::read(&prefs_path).unwrap(), prefs_before);
        fs::write(&prefs_path, b"corrupt").unwrap();
        let events = runtime.issue_monitor_capacity_changed_events_with(&other, |_| {
            expected.agent_capacity.clone()
        });
        assert!(
            events.is_empty(),
            "failed prefs read retains unchanged cached mode"
        );
        assert_eq!(fs::read(&prefs_path).unwrap(), b"corrupt");
        fs::write(&prefs_path, prefs_before).unwrap();
    }
    assert!(tasks.lock().unwrap().is_empty(), "no scan, RPC or PM work");
    assert_eq!(runtime.daemon_supervisor.ensure_attempts(), 0);
}

#[test]
fn issue_monitor_capacity_waits_for_the_initial_full_projection() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab",
            "Repo",
            root.clone(),
            ProjectKind::Git,
            &[],
        )],
        Some("tab"),
    );
    assert!(runtime
        .issue_monitor_capacity_changed_events_with(&root, |_| {
            panic!("no observation read before the authoritative initial status")
        })
        .is_empty());
    assert!(runtime
        .issue_monitor_capacity_changed_events_with(&temp.path().join("closed"), |_| {
            panic!("closed project must not publish capacity")
        })
        .is_empty());
}

#[test]
fn issue_monitor_capacity_overlay_rejects_observations_before_control_or_reopen() {
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab",
            "Repo",
            root.clone(),
            ProjectKind::Git,
            &[],
        )],
        Some("tab"),
    );
    let (context, revision, ready) = runtime.capacity_observation_contexts().pop().unwrap();
    assert!(ready.is_none());
    let mut initial = test_monitor(gwt::IssueMonitorConfig::default()).status_view();
    initial.max_active_agents = 7;
    initial.max_active_agents_override = None;
    let state = runtime.project_state(&context).unwrap();
    state
        .issue_monitor_status_cache
        .replace(Some(initial.clone()));
    assert_eq!(
        runtime
            .capacity_observation_contexts()
            .pop()
            .unwrap()
            .2
            .unwrap()
            .limits,
        Some((7, None))
    );
    let mut measured = initial.clone();
    measured.max_active_agents = 9;
    measured.max_active_agents_override = Some(9);
    let update = gwt::agent_capacity::CapacityUpdate::from_status(&measured);
    runtime.invalidate_issue_monitor_list(&root);
    assert!(runtime
        .issue_monitor_capacity_changed_events(&context, revision, update.clone())
        .is_empty());
    assert_eq!(
        state.issue_monitor_status_cache.borrow().as_ref(),
        Some(&initial)
    );
    let revision = (
        state.issue_monitor_list_revision.get(),
        state.issue_monitor_status_revision.get(),
    );
    let mut replaced = context.clone();
    replaced.generation += 1;
    assert!(runtime
        .issue_monitor_capacity_changed_events(&replaced, revision, update.clone())
        .is_empty());
    assert_eq!(
        runtime
            .issue_monitor_capacity_changed_events(&context, revision, update)
            .len(),
        1
    );
    assert_eq!(
        state.issue_monitor_status_cache.borrow().as_ref(),
        Some(&measured)
    );
}

#[test]
fn issue_monitor_capacity_overlay_preserves_newer_daemon_limit_and_accepts_next_observation() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _gwt_home = ScopedGwtHome::set(temp.path());
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab",
            "Repo",
            root.clone(),
            ProjectKind::Git,
            &[],
        )],
        Some("tab"),
    );
    let mut initial = test_monitor(gwt::IssueMonitorConfig::default()).status_view();
    initial.max_active_agents = 7;
    initial.max_active_agents_override = None;
    initial.agent_capacity = gwt::agent_capacity::AgentCapacity {
        measurement_complete: true,
        recommended_worker_limit: 7,
        ..Default::default()
    };
    runtime.issue_monitor_daemon_status_events(&root, Box::new(initial.clone()));
    let (context, revision, accepted) = runtime.capacity_observation_contexts().pop().unwrap();
    let old_update =
        gwt::agent_capacity::CapacityUpdate::read_with(&root, accepted.as_ref(), |_| {
            gwt::agent_capacity::AgentCapacity {
                measurement_complete: false,
                reason: "pending observation".into(),
                ..initial.agent_capacity.clone()
            }
        });
    assert_eq!(old_update.limits, Some((7, None)));

    let mut newer = initial;
    newer.max_active_agents = 4;
    newer.agent_capacity.recommended_worker_limit = 4;
    runtime.issue_monitor_daemon_status_events(&root, Box::new(newer));
    runtime.issue_monitor_capacity_changed_events(&context, revision, old_update);
    let state = runtime.project_state(&context).unwrap();
    assert_eq!(
        state
            .issue_monitor_status_cache
            .borrow()
            .as_ref()
            .unwrap()
            .max_active_agents,
        4,
        "an old incomplete worker observation must not restore the prior Auto limit"
    );

    let (context, revision, accepted) = runtime.capacity_observation_contexts().pop().unwrap();
    let next_update =
        gwt::agent_capacity::CapacityUpdate::read_with(&root, accepted.as_ref(), |_| {
            gwt::agent_capacity::AgentCapacity {
                measurement_complete: true,
                recommended_worker_limit: 5,
                reason: "next observation".into(),
                ..Default::default()
            }
        });
    assert_eq!(
        runtime
            .issue_monitor_capacity_changed_events(&context, revision, next_update)
            .len(),
        1,
        "the next worker observation must still be published"
    );
    assert_eq!(
        state
            .issue_monitor_status_cache
            .borrow()
            .as_ref()
            .unwrap()
            .max_active_agents,
        5
    );
}

#[test]
fn issue_monitor_capacity_uses_saved_manual_authority_when_old_daemon_omits_it() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempdir().unwrap();
    let _home = ScopedEnvVar::set("HOME", temp.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let runtime = sample_runtime(
        temp.path(),
        vec![sample_project_tab(
            "tab",
            "Repo",
            root.clone(),
            ProjectKind::Git,
            &[],
        )],
        Some("tab"),
    );
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&root);
    fs::create_dir_all(prefs_path.parent().unwrap()).unwrap();
    let legacy = br#"{"enabled":false,"max_active_agents":9,"priority_order":[]}"#;
    fs::write(&prefs_path, legacy).unwrap();
    assert_eq!(
        gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .max_active_agents_mode,
        gwt::issue_monitor::IssueMonitorMaxActiveMode::Manual,
        "fixture must be valid legacy Manual prefs"
    );
    // A live daemon from before this feature has no override field even when
    // its persisted positive value is an explicit Manual limit.
    let mut wire =
        serde_json::to_value(test_monitor(gwt::IssueMonitorConfig::default()).status_view())
            .unwrap();
    wire.as_object_mut()
        .unwrap()
        .remove("max_active_agents_override");
    wire.as_object_mut().unwrap().remove("agent_capacity");
    let mut expected: gwt::IssueMonitorStatusView = serde_json::from_value(wire).unwrap();
    expected.queue_len = 13;
    let mut old_daemon_status = expected.clone();
    runtime.issue_monitor_daemon_status_events(&root, Box::new(expected.clone()));
    let capacity = gwt::agent_capacity::AgentCapacity {
        measurement_complete: true,
        recommended_worker_limit: 4,
        ..Default::default()
    };
    let events = runtime.issue_monitor_capacity_changed_events_with(&root, |_| capacity.clone());
    expected.agent_capacity = capacity;
    expected.max_active_agents = 9;
    expected.max_active_agents_override = Some(9);
    let BackendEvent::IssueMonitorStatus { status } = &events[0].event else {
        panic!("status")
    };
    assert_eq!(
        **status, expected,
        "durable Manual authority must survive an old daemon"
    );
    assert_eq!(fs::read(&prefs_path).unwrap(), legacy);
    // A failed preference read must not erase the last valid Manual choice.
    fs::write(&prefs_path, b"corrupt").unwrap();
    // A repeated old wire frame must not erase the last valid Manual authority
    // before the failed prefs read falls back to that same cached projection.
    old_daemon_status.queue_len = 19;
    old_daemon_status.last_error = Some("latest daemon diagnostic".into());
    runtime.issue_monitor_daemon_status_events(&root, Box::new(old_daemon_status));
    expected.queue_len = 19;
    expected.last_error = Some("latest daemon diagnostic".into());
    let expired = gwt::agent_capacity::AgentCapacity {
        observed_at: Some(1),
        expires_at: Some(2),
        ..expected.agent_capacity.clone()
    };
    let events = runtime.issue_monitor_capacity_changed_events_with(&root, |_| expired.clone());
    expected.agent_capacity = expired;
    let BackendEvent::IssueMonitorStatus { status } = &events[0].event else {
        panic!("status")
    };
    assert_eq!(**status, expected);
    assert_eq!(fs::read(&prefs_path).unwrap(), b"corrupt");
}
