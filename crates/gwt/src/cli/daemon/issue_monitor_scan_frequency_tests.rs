//! Real-driver cadence measurements; only remote scan preparation is replaced.

use super::*;

#[test]
fn rejected_disabled_scans_do_not_schedule_themselves() {
    let _env = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let _home = gwt_core::test_support::ScopedGwtHome::set(&home);
    let root = temp.path().join("project");
    fs::create_dir_all(&root).unwrap();
    let scope =
        RuntimeScope::from_project_root(&root, gwt_core::daemon::RuntimeTarget::Host).unwrap();
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&root);
    crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default()).unwrap();
    let rejection_seen = Arc::new(AtomicBool::new(false));
    let rejected = rejection_seen.clone();
    let (observed, mut observations) = mpsc::unbounded_channel();
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let idle_after_rejection = runtime.block_on(async {
        let hub = BroadcastHub::new();
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = spawn_issue_monitor_worker_with_lease(
            scope,
            hub.clone(),
            shutdown.clone(),
            crate::IssueMonitorConfig::default(),
            ISSUE_MONITOR_SCAN_TIMEOUT,
            IssueMonitorWorkerTestHooks {
                scan_override: Some(Arc::new(move |monitor| {
                    let _home = gwt_core::test_support::ScopedGwtHome::set(&home);
                    crate::mutate_issue_monitor_prefs(&prefs_path, |prefs| {
                        prefs.advance_effect_authority_epoch().unwrap();
                    })
                    .unwrap();
                    Ok(monitor)
                })),
                admission_override: Some(Arc::new(Ok)),
                rejected_scan_observer: Some(Arc::new(move |_| {
                    rejected.store(true, Ordering::SeqCst);
                })),
                scan_idle_observer: Some(Arc::new(move |idle| {
                    // Admission may still be joining on the rejection pass.
                    // Wait for real idle; the rejected-scan loop never gets there.
                    if idle && rejection_seen.swap(false, Ordering::SeqCst) {
                        let _ = observed.send(idle);
                    }
                })),
                hold_effects: true,
                ..Default::default()
            },
            None,
        );
        hub.publish_issue_monitor_control(DaemonFrame::Event {
            channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.into(),
            payload: crate::runtime_daemon_events::issue_monitor_payload(
                "control",
                serde_json::json!({"scan_now": {}}),
                std::process::id(),
            ),
        })
        .await
        .unwrap();
        // test-hygiene: allow-wall-clock-deadline The rejection-to-idle channel orders the real driver; this timeout only bounds a broken fixture.
        let observation =
            tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, observations.recv()).await;
        shutdown.request();
        // test-hygiene: allow-wall-clock-deadline Join the real worker before asserting its observed scheduling decision.
        tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, worker)
            .await
            .unwrap()
            .unwrap();
        observation.unwrap()
    });
    assert_eq!(
        idle_after_rejection,
        Some(true),
        "a rejected disabled scan must leave the driver idle instead of scheduling itself"
    );
}

/// Run explicitly with cargo test and --ignored --nocapture; nextest's normal
/// per-test timeout is intentionally shorter than this ten-minute measurement.
#[test]
#[ignore = "ten-minute real-time measurement for Issue #7504"]
fn three_projects_ten_minute_scan_frequency() {
    assert_frequency(&measure_scan_frequency(Duration::from_secs(600)));
}

fn assert_frequency(report: &serde_json::Value) {
    println!("{report}");
    for project in report["projects"].as_array().unwrap().iter().take(2) {
        assert!(project["scan_entries"].as_u64().unwrap() > 0);
        assert!(project["proposal_rejections"].as_u64().unwrap() > 0);
        assert_eq!(project["automatic_scan_entries"], 0);
    }
    assert!(report["projects"][2]["scan_entries"].as_u64().unwrap() > 0);
    assert!(report["projects"][2]["scan_entries"].as_u64().unwrap() <= 50);
    assert!(
        report["projects"][2]["committed_scan_updates"]
            .as_u64()
            .unwrap()
            >= 2
    );
    assert!(report["projects"][2]["last_scan_at"].is_string());
    assert_eq!(report["proposal_commit_failures"], 0);
}

fn measure_scan_frequency(duration: Duration) -> serde_json::Value {
    let _env = crate::env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let _home = gwt_core::test_support::ScopedGwtHome::set(&home);
    let log_path = temp.path().join("scan-events.jsonl");
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::sync::Mutex::new(fs::File::create(&log_path).unwrap()))
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let scans = Arc::new(std::array::from_fn::<_, 3, _>(|_| AtomicUsize::new(0)));
        let rejections = Arc::new(std::array::from_fn::<_, 3, _>(|_| AtomicUsize::new(0)));
        let committed_scans = Arc::new(std::sync::Mutex::new(std::array::from_fn::<_, 3, _>(
            |_| std::collections::BTreeSet::<String>::new(),
        )));
        let mut workers = Vec::new();
        let mut paths = Vec::new();
        for index in 0..3 {
            let root = temp.path().join(format!("project-{index}"));
            fs::create_dir_all(&root).unwrap();
            let scope = RuntimeScope::from_project_root(
                &root,
                gwt_core::daemon::RuntimeTarget::Host,
            )
            .unwrap();
            let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&root);
            crate::save_issue_monitor_prefs(
                &prefs_path,
                &crate::IssueMonitorPrefs {
                    enabled: index == 2,
                    ..Default::default()
                },
            )
            .unwrap();
            let (scan_counts, rejected, scan_home, scan_prefs, scan_dispatch) = (
                scans.clone(), rejections.clone(), home.clone(), prefs_path.clone(), dispatch.clone(),
            );
            let observed_commits = committed_scans.clone();
            let hub = BroadcastHub::new();
            let shutdown = Arc::new(DaemonShutdown::new());
            let worker = spawn_issue_monitor_worker_with_lease(
                scope,
                hub.clone(),
                shutdown.clone(),
                crate::IssueMonitorConfig::default(),
                ISSUE_MONITOR_SCAN_TIMEOUT,
                IssueMonitorWorkerTestHooks {
                    scan_override: Some(Arc::new(move |mut monitor| {
                        let _home = gwt_core::test_support::ScopedGwtHome::set(&scan_home);
                        tracing::dispatcher::with_default(&scan_dispatch, || {
                            scan_counts[index].fetch_add(1, Ordering::SeqCst);
                            // Model the same one-second remote read before/after.
                            // test-hygiene: allow-wall-clock-deadline This experiment measures the real driver with a fixed simulated remote-read latency.
                            std::thread::sleep(Duration::from_secs(1));
                            if index < 2 {
                                // Each rejected proposal sees a newer direct writer.
                                // Both its write and the driver's rejected commit
                                // retain the real prefs/registry transaction.
                                crate::mutate_issue_monitor_prefs(&scan_prefs, |prefs| {
                                    prefs.advance_effect_authority_epoch().unwrap();
                                })
                                .unwrap();
                            }
                            crate::scan_issue_monitor_candidates(
                                &mut monitor,
                                &[],
                                &gwt_core::timestamp::UtcTimestamp::now().to_rfc3339(),
                            );
                            Ok(monitor)
                        })
                    })),
                    admission_override: Some(Arc::new(Ok)),
                    rejected_scan_observer: Some(Arc::new(move |_| {
                        rejected[index].fetch_add(1, Ordering::SeqCst);
                    })),
                    state_observer: Some(Arc::new(move |monitor| {
                        if let Some(at) = monitor.status_view().last_scan_at {
                            observed_commits.lock().unwrap()[index].insert(at);
                        }
                    })),
                    hold_effects: true,
                    ..Default::default()
                },
                None,
            );
            paths.push(prefs_path);
            workers.push((hub, shutdown, worker));
        }
        let started_at = gwt_core::timestamp::UtcTimestamp::now();
        let started = Instant::now();
        for (hub, _, _) in workers.iter().take(2) {
            hub.publish_issue_monitor_control(DaemonFrame::Event {
                channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.into(),
                payload: crate::runtime_daemon_events::issue_monitor_payload(
                    "control", serde_json::json!({"scan_now": {}}), std::process::id(),
                ),
            })
            .await
            .unwrap();
        }
        // test-hygiene: allow-wall-clock-deadline The requested measurement is a real elapsed observation window, not a scheduling-order assertion.
        tokio::time::sleep_until(tokio::time::Instant::from_std(started + duration)).await;
        let finished_at = gwt_core::timestamp::UtcTimestamp::now();
        let elapsed = started.elapsed();
        let counts = std::array::from_fn::<_, 3, _>(|index| scans[index].load(Ordering::SeqCst));
        let rejected = std::array::from_fn::<_, 3, _>(|index| rejections[index].load(Ordering::SeqCst));
        let committed = committed_scans.lock().unwrap().clone();
        let prefs = paths.iter().map(|path| crate::load_issue_monitor_prefs(path).unwrap()).collect::<Vec<_>>();
        for (_, shutdown, _) in &workers {
            shutdown.request();
        }
        for (_, _, worker) in workers {
            // test-hygiene: allow-wall-clock-deadline Bound teardown of actual blocking scan workers after the measurement has ended.
            tokio::time::timeout(gwt_core::deadline_budget::HANG_GUARD, worker).await.unwrap().unwrap();
        }
        let events = fs::read_to_string(&log_path).unwrap().lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|event| event["timestamp"].as_str()
                .and_then(|at| gwt_core::timestamp::UtcTimestamp::parse_from_rfc3339(at).ok())
                .is_some_and(|at| at >= started_at && at <= finished_at))
            .collect::<Vec<_>>();
        serde_json::json!({
            "scenario": "three-project-rejected-commit-cadence",
            "measurement_ms": elapsed.as_millis(),
            "configured_poll_seconds": crate::IssueMonitorConfig::default().poll_interval_secs,
            "projects": (0..3).map(|index| serde_json::json!({
                "project": index,
                "enabled": index == 2,
                "explicit_scan_requests": usize::from(index < 2),
                "scan_entries": counts[index],
                "automatic_scan_entries": counts[index].saturating_sub(usize::from(index < 2)),
                "proposal_rejections": rejected[index],
                "committed_scan_updates": committed[index].len(),
                "first_committed_scan_at": committed[index].first(),
                "last_committed_scan_at": committed[index].last(),
                "last_scan_at": prefs[index].last_scan_at,
            })).collect::<Vec<_>>(),
            "proposal_commit_failures": events.iter().filter(|event|
                event["fields"]["message"] == "Issues scan proposal commit failed").count(),
            "registry_contention_events": events.iter().filter(|event|
                event["fields"]["operation"] == "shared Agent registry"
                && event["fields"]["message"].as_str().is_some_and(|message|
                    message.starts_with("named file lock acquisition failed or contended"))).count(),
            "measurement_boundary": "real driver, prefs and registry; scan preparation uses empty candidates and one-second simulated remote latency; recurring authority rejection is injected; no GUI or Windows filesystem reproduction; scan entries are counted at the test seam, not stage=total logs",
        })
    })
}
