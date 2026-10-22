//! Bounded queue admission independent of the full maintenance scan.

use crate::issue_monitor_worker::{IssueMonitorScanFailure, IssueMonitorScanStage};
use crate::{IssueMonitorIssue, IssueMonitorPrefs, IssueMonitorState};

pub(super) fn spawn(
    scope: gwt_core::daemon::RuntimeScope,
    mut monitor: IssueMonitorState,
    gui_connected: bool,
    deadline: std::time::Instant,
    test_hooks: super::IssueMonitorWorkerTestHooks,
) -> tokio::task::JoinHandle<Result<IssueMonitorState, IssueMonitorScanFailure>> {
    tokio::task::spawn_blocking(move || {
        #[cfg(test)]
        if let Some(admission) = &test_hooks.admission_override {
            return admission(monitor);
        }
        #[cfg(not(test))]
        let _ = test_hooks;
        // Leave time for the driver's durable proposal commit. A failed
        // observation defers only that row instead of exhausting this deadline.
        let observation_deadline = deadline
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap_or(deadline);
        let _deadline =
            gwt_core::operation_deadline::ScopedOperationDeadline::enter(observation_deadline);
        let _timing = crate::issue_monitor_worker::IssueMonitorScanTiming::start("launch-decision");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let prefs = crate::issue_monitor::load_issue_monitor_prefs_for_running_version(&prefs_path)
            .map_err(|error| {
                IssueMonitorScanFailure::new(
                    IssueMonitorScanStage::CandidateLoad,
                    error.to_string(),
                )
            })?;
        let (owner, repo) =
            crate::issue_monitor_worker::github_remote_owner_and_repo(&scope.project_root)
                .map_err(|error| {
                    IssueMonitorScanFailure::new(
                        IssueMonitorScanStage::RemoteResolution,
                        error.to_string(),
                    )
                })?;
        monitor.set_source_repository(Some((&owner, &repo)));
        prepare_with(
            monitor,
            &prefs,
            gui_connected,
            &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            |issue| {
                crate::issue_monitor_worker::try_refresh_issue_monitor_candidate(
                    &scope.project_root,
                    &owner,
                    &repo,
                    issue,
                )
            },
            |issue| {
                crate::issue_monitor_worker::try_issue_completed_by_merged_pr(&owner, &repo, issue)
            },
        )
    })
}

fn prepare_with(
    mut monitor: IssueMonitorState,
    prefs: &IssueMonitorPrefs,
    gui_connected: bool,
    now: &str,
    mut refresh: impl FnMut(&IssueMonitorIssue) -> Result<IssueMonitorIssue, IssueMonitorScanFailure>,
    mut completed: impl FnMut(&IssueMonitorIssue) -> Result<bool, IssueMonitorScanFailure>,
) -> Result<IssueMonitorState, IssueMonitorScanFailure> {
    monitor.rebase_daemon_driver_prefs(prefs);
    monitor.set_gui_connected(gui_connected);
    let active_cap = if monitor.has_launch_profile() {
        monitor.effective_max_active_agents()
    } else {
        0
    };
    let (_, frontier) = monitor.claim_probe_plan(active_cap);
    let candidates = frontier
        .into_iter()
        .filter_map(|number| {
            monitor
                .inbox_item(number)
                .map(|item| (number, item.issue.clone()))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    monitor.try_prepare_claim_effects_with_probe_confirmed(
        &crate::process::current_claim_owner(), now, active_cap, None,
        |number| {
            let observed = (|| {
                crate::issue_monitor_worker::ensure_scan_deadline(IssueMonitorScanStage::CandidateLoad)?;
                let Some(previous) = candidates.get(&number) else { return Ok(None); };
                let current = refresh(previous)?;
                // Match targeted refresh's canonical empty-body representation.
                let mut expected = previous.clone();
                expected.body = expected.body.filter(|body| !body.is_empty());
                // Only the full scan evaluates changed launch prerequisites.
                // Comment-resident artifacts can change at the same Issue revision.
                if previous.updated_at.is_none() || current != expected {
                    return Ok(None);
                }
                completed(&current).map(Some)
            })();
            Ok::<_, std::convert::Infallible>(match observed {
                Ok(Some(completed)) => crate::issue_monitor::ClaimProbeOutcome::from_completed(completed),
                Ok(None) => crate::issue_monitor::ClaimProbeOutcome::Deferred,
                Err(failure) => {
                    tracing::warn!(issue = number, error = %failure, "immediate admission deferred a candidate");
                    crate::issue_monitor::ClaimProbeOutcome::Deferred
                }
            })
        },
    ).unwrap_or_else(|never| match never {});
    Ok(monitor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{mpsc, Arc, Mutex},
        time::{Duration, Instant},
    };

    fn queued_monitor() -> IssueMonitorState {
        let profile = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("work/issue-42")
            .build();
        let mut monitor = IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 1,
                max_active_agents_mode: crate::issue_monitor::IssueMonitorMaxActiveMode::Manual,
                launch_profile: Some(crate::IssueMonitorLaunchProfile::from(&profile)),
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.set_source_repository(Some(("example", "repo")));
        monitor.terminal_queue_push(&[42], "operator", "2026-10-16T00:00:00Z");
        monitor.record_candidate(IssueMonitorIssue {
            author_login: None,
            number: 42,
            title: "queued work".to_string(),
            labels: vec!["gwt-queued".to_string()],
            state: crate::IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: crate::IssueMonitorReadiness::default(),
            updated_at: Some("2026-10-16T00:00:00Z".to_string()),
        });
        monitor
    }

    fn fixture_scope(root: std::path::PathBuf) -> gwt_core::daemon::RuntimeScope {
        std::fs::create_dir_all(&root).unwrap();
        for args in [
            vec!["init"],
            vec![
                "remote",
                "add",
                "origin",
                "https://github.com/example/repo.git",
            ],
        ] {
            assert!(gwt_core::process::hidden_command("git")
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap()
                .status
                .success());
        }
        gwt_core::daemon::RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            root,
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn issue_5557_admission_commits_before_a_blocked_full_scan_finishes() {
        assert_admission_while_scan_blocked(true).await;
    }

    #[tokio::test]
    async fn issue_5644_poll_admits_without_scan_now_while_maintenance_is_blocked() {
        assert_admission_while_scan_blocked(false).await;
    }

    #[allow(clippy::await_holding_lock)] // the worker and fresh HOME share one test environment
    async fn assert_admission_while_scan_blocked(send_scan_now: bool) {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _prefs_budget = super::super::pin_prefs_hang_guard();
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let scope = fixture_scope(temp.path().join("repo"));
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let monitor = queued_monitor();
        let candidate = monitor.inbox_item(42).unwrap().issue.clone();
        crate::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).unwrap();
        let scan_started = Arc::new(tokio::sync::Notify::new());
        let started_hook = Arc::clone(&scan_started);
        let scan_finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished_hook = Arc::clone(&scan_finished);
        let (release, blocked) = mpsc::channel::<()>();
        let blocked = Mutex::new(blocked);
        let hooks = super::super::IssueMonitorWorkerTestHooks {
            scan_override: Some(Arc::new(move |monitor| {
                started_hook.notify_one();
                let _ = blocked
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(60));
                finished_hook.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(monitor)
            })),
            admission_override: Some(Arc::new(move |mut monitor| {
                // Model an already observed queue row; remote refresh/probe are
                // deterministic here, while control reception and commit are real.
                monitor.set_source_repository(Some(("example", "repo")));
                monitor.record_candidate(candidate.clone());
                let prefs = monitor.prefs();
                prepare_with(
                    monitor,
                    &prefs,
                    true,
                    "2026-10-16T00:00:01Z",
                    |issue| Ok(issue.clone()),
                    |_| Ok(false),
                )
            })),
            effect_override: Some(Arc::new(|effect, execution_now| {
                let crate::IssueMonitorEffectPayload::AcquireClaim {
                    issue_number,
                    claim_id,
                    owner,
                    launched_work_id,
                    ..
                } = &effect.payload
                else {
                    panic!("the fixture only executes its admission claim");
                };
                super::super::IssueMonitorEffectOutcome::Claim(Ok(
                    gwt_github::issue_auto_claim::ClaimAcquireOutcome::Acquired(
                        gwt_github::issue_auto_claim::ClaimComment {
                            comment_id: Some(gwt_github::CommentId(99)),
                            claim_id: claim_id.clone(),
                            owner: owner.clone(),
                            issue_number: *issue_number,
                            status: gwt_github::issue_auto_claim::ClaimStatus::Active,
                            heartbeat_at: execution_now.to_string(),
                            expires_at: (chrono::DateTime::parse_from_rfc3339(execution_now)
                                .unwrap()
                                + chrono::Duration::minutes(30))
                            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                            launched_work_id: launched_work_id.clone(),
                        },
                    ),
                ))
            })),
            ..Default::default()
        };
        let hub = super::super::BroadcastHub::new();
        let _materializer = hub.acquire_issue_monitor_materializer();
        let mut statuses = hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL);
        let shutdown = Arc::new(super::super::DaemonShutdown::new());
        let worker = super::super::spawn_issue_monitor_worker_with_lease(
            scope,
            hub.clone(),
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 1,
                ..Default::default()
            },
            Duration::from_secs(if send_scan_now { 30 } else { 5 }),
            hooks,
            None,
        );
        let full_scan_started =
            tokio::time::timeout(Duration::from_secs(30), scan_started.notified())
                .await
                .is_ok();
        let before_claims = crate::load_issue_monitor_prefs(&prefs_path)
            .unwrap()
            .pending_effects
            .len();
        let started = Instant::now();
        let control = if send_scan_now {
            Some(
                hub.publish_issue_monitor_control(gwt_core::daemon::DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload: crate::runtime_daemon_events::issue_monitor_payload(
                        "control",
                        serde_json::json!({"scan_now": {}}),
                        u32::MAX,
                    ),
                })
                .await,
            )
        } else {
            None
        };
        let launch_dispatched = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let gwt_core::daemon::DaemonFrame::Event { payload, .. } =
                    statuses.recv().await.unwrap()
                {
                    if payload.get("event").and_then(serde_json::Value::as_str)
                        == Some("launch_request")
                        && payload
                            .pointer("/payload/issue_number")
                            .and_then(serde_json::Value::as_u64)
                            == Some(42)
                    {
                        break;
                    }
                }
            }
        })
        .await
        .is_ok();
        let maintenance_pending_at_dispatch =
            !scan_finished.load(std::sync::atomic::Ordering::SeqCst);
        let dispatch_elapsed = started.elapsed();
        let dispatched = crate::load_issue_monitor_prefs(&prefs_path).unwrap();
        let _ = release.send(());
        shutdown.request();
        tokio::time::timeout(Duration::from_secs(30), worker)
            .await
            .unwrap()
            .unwrap();
        println!(
            "{}",
            serde_json::json!({
                "scenario": "blocked-maintenance-dispatch", "before_claims": before_claims,
                "dispatch_ms": dispatch_elapsed.as_millis(), "dispatch_before_release": launch_dispatched,
                "maintenance_pending": maintenance_pending_at_dispatch,
                "durable_deliveries": dispatched.pending_launch_deliveries.len(),
            })
        );
        assert!(full_scan_started);
        assert!(
            control.is_none_or(|result| result.is_ok()),
            "ScanNow must reach the live control lane"
        );
        assert!(maintenance_pending_at_dispatch);
        assert!(
            launch_dispatched,
            "fresh admission must dispatch a launch request before maintenance is released"
        );
        assert!(
            dispatched
                .pending_launch_deliveries
                .iter()
                .any(|delivery| delivery.issue_number == 42
                    && delivery.branch_name == "work/issue-42")
        );
    }

    #[test]
    fn issue_5557_admission_defers_a_changed_candidate_to_the_full_scan() {
        let monitor = queued_monitor();
        let proposal = prepare_with(
            monitor.clone(),
            &monitor.prefs(),
            true,
            "2026-10-16T00:00:01Z",
            |issue| {
                Ok(IssueMonitorIssue {
                    updated_at: Some("2026-10-16T00:00:01Z".to_string()),
                    ..issue.clone()
                })
            },
            |_| panic!("an unvalidated issue revision cannot authorize admission"),
        )
        .unwrap();
        assert!(proposal.pending_effects().is_empty());
    }

    #[test]
    fn admission_defers_changed_readiness_at_the_same_issue_revision() {
        let mut monitor = queued_monitor();
        let mut issue = monitor.inbox_item(42).unwrap().issue.clone();
        issue.labels.push("gwt-spec".into());
        issue.readiness = crate::IssueMonitorReadiness::ReadyWithOpenTasks;
        issue.body = Some(String::new());
        monitor.record_candidate(issue);
        let mut probes = 0;
        let proposal = prepare_with(
            monitor.clone(),
            &monitor.prefs(),
            true,
            "2026-10-16T00:00:01Z",
            |issue| {
                // Comment-resident artifacts can change without a new Issue timestamp.
                Ok(IssueMonitorIssue {
                    readiness: crate::IssueMonitorReadiness::NotReady,
                    body: None,
                    ..issue.clone()
                })
            },
            |_| {
                probes += 1;
                Ok(false)
            },
        )
        .unwrap();
        assert!(proposal.pending_effects().is_empty());
        assert_eq!(probes, 0, "changed prerequisites need a full scan first");

        let unchanged = prepare_with(
            monitor.clone(),
            &monitor.prefs(),
            true,
            "2026-10-16T00:00:01Z",
            // The live loader keeps Some(""); targeted refresh returns None.
            |issue| {
                Ok(IssueMonitorIssue {
                    body: None,
                    ..issue.clone()
                })
            },
            |_| Ok(false),
        )
        .unwrap();
        assert_eq!(unchanged.pending_effects().len(), 1);
    }

    #[test]
    fn issue_5557_scan_cadence_and_deadline_stay_within_one_minute() {
        assert!(crate::IssueMonitorConfig::default().poll_interval_secs <= 30);
        assert!(
            super::super::issue_monitor_scan_budget(&queued_monitor(), Duration::from_secs(180),)
                <= Duration::from_secs(60)
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // fresh HOME isolates the real driver and prefs
    async fn issue_5557_failed_scan_retries_before_the_long_poll() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _prefs_budget = super::super::pin_prefs_hang_guard();
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let scope = fixture_scope(temp.path().join("repo"));
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        crate::save_issue_monitor_prefs(&prefs_path, &queued_monitor().prefs()).unwrap();
        let scan_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hook_calls = Arc::clone(&scan_calls);
        let second_scan = Arc::new(tokio::sync::Notify::new());
        let hook_second_scan = Arc::clone(&second_scan);
        let first_started = Arc::new(Mutex::new(None::<Instant>));
        let hook_first_started = Arc::clone(&first_started);
        let hooks = super::super::IssueMonitorWorkerTestHooks {
            scan_override: Some(Arc::new(move |monitor| {
                if hook_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    *hook_first_started.lock().unwrap() = Some(Instant::now());
                    Err(IssueMonitorScanFailure::new(
                        IssueMonitorScanStage::CandidateLoad,
                        "fixture first scan failed",
                    ))
                } else {
                    hook_second_scan.notify_one();
                    Ok(monitor)
                }
            })),
            hold_effects: true,
            ..Default::default()
        };
        let hub = super::super::BroadcastHub::new();
        let shutdown = Arc::new(super::super::DaemonShutdown::new());
        let worker = super::super::spawn_issue_monitor_worker_with_lease(
            scope,
            hub,
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 300,
                ..Default::default()
            },
            Duration::from_secs(30),
            hooks,
            None,
        );
        let retried = tokio::time::timeout(Duration::from_secs(30), second_scan.notified())
            .await
            .is_ok();
        let retry_elapsed = first_started
            .lock()
            .unwrap()
            .map(|started| started.elapsed());
        shutdown.request();
        tokio::time::timeout(Duration::from_secs(30), worker)
            .await
            .unwrap()
            .unwrap();
        println!(
            "{}",
            serde_json::json!({
                "scenario": "failed-scan-retry", "configured_poll_seconds": 300,
                "retry_ms": retry_elapsed.map(|elapsed| elapsed.as_millis()),
                "scan_calls": scan_calls.load(std::sync::atomic::Ordering::SeqCst),
            })
        );
        assert!(
            retried,
            "the error retry must not wait for the configured poll"
        );
        assert!(scan_calls.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    }
}
