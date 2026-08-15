//! SPEC-3431 T-087d (FR-033): IPC-layer failure injection for the PM's stop
//! and failover operations.
//!
//! The state-layer matrix (T-086/T-087/T-087c) proves the stop machine and the
//! daemon rebase converge on in-memory structures. This file pins the same
//! contract through the real boundaries those tests skipped: the durable prefs
//! file behind its lock, and the daemon control socket.
//!
//! - a daemon that accepts the connection and then hangs must cost a bounded
//!   wait, never a hang, and must report an outcome-unknown error (no silent
//!   success, no local-fallback authority grab);
//! - a stop written to disk must survive the daemon's own commit path
//!   (`rebase_daemon_driver_prefs` inside `mutate_issue_monitor_prefs`) and
//!   must not be resurrected by the daemon's in-memory view;
//! - an unstoppable target (prefs not writable) must fail closed with a typed
//!   error and leave the running launch untouched — a phantom "stopped" that
//!   never persisted is the one outcome the PM can never recover from;
//! - a failover requeue must stay durable even when the immediate-scan
//!   publish fails, so the next scheduled scan converges on it.

#![cfg(unix)]

use std::os::unix::net::UnixListener;
use std::time::{Duration, Instant};

use gwt::{
    scan_issue_monitor_candidates, IssueMonitorConfig, IssueMonitorIssue, IssueMonitorIssueState,
    IssueMonitorLaunchSessionStrategy, IssueMonitorPrefs, IssueMonitorState,
    IssueMonitorStopOutcome, IssueMonitorStopTarget, MonitorInboxState,
};
use gwt_core::daemon::{persist_endpoint, DaemonEndpoint, RuntimeScope, RuntimeTarget};
use gwt_core::test_support::ScopedEnvVar;
use tempfile::TempDir;

const NOW: &str = "2026-08-09T05:00:00Z";

fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn issue(number: u64) -> IssueMonitorIssue {
    IssueMonitorIssue {
        number,
        title: format!("Issue {number}"),
        labels: vec!["auto-merge".to_string()],
        state: IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
    }
}

/// A monitor with one launched issue bound to `window-1`, as the daemon and
/// the CLI would both observe it.
fn launched_monitor() -> IssueMonitorState {
    let mut monitor = IssueMonitorState::with_prefs(
        IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        },
        IssueMonitorPrefs {
            enabled: true,
            ..IssueMonitorPrefs::default()
        },
    );
    scan_issue_monitor_candidates(&mut monitor, &[issue(42)], NOW);
    monitor.complete_active_launch(42, "tab-1::window-1");
    monitor
}

fn stop_target() -> IssueMonitorStopTarget {
    IssueMonitorStopTarget {
        issue_number: 42,
        claim_id: None,
        delivery_id: None,
        window_id: Some("tab-1::window-1".to_string()),
    }
}

/// T-087d: a daemon that accepts and then never answers the handshake must
/// cost a bounded wait and report the outcome as unknown — the PM's loop can
/// live with a slow "I don't know", but not with a hang or a fabricated ack.
#[test]
fn hung_daemon_control_publish_is_bounded_and_outcome_unknown() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("project tempdir");
    let home = TempDir::new().expect("home tempdir");
    let _home_guard = ScopedEnvVar::set("HOME", home.path());
    let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());

    let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
        .expect("runtime scope");
    let socket_path = project.path().join("hung.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind hung listener");
    // Accept connections but never read or write: the client's per-stage
    // deadline is the only thing standing between the PM and a dead loop.
    let accepting = std::thread::spawn(move || {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept() {
            held.push(stream);
        }
    });

    let endpoint = DaemonEndpoint::new(
        scope.clone(),
        std::process::id(),
        socket_path.to_string_lossy().to_string(),
        "test-token".to_string(),
        "test-daemon".to_string(),
    );
    persist_endpoint(
        &scope.endpoint_path(&gwt_core::paths::gwt_home()),
        &endpoint,
    )
    .expect("persist endpoint");

    let started = Instant::now();
    let error = gwt::daemon_publisher::publish_issue_monitor_control(
        project.path(),
        serde_json::json!({ "scan_now": {} }),
    )
    .expect_err("a silent daemon must not produce an ack");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(5),
        "hung daemon must be bounded by the per-stage deadline, waited {elapsed:?}"
    );
    assert!(
        !error.allows_local_fallback(),
        "a live-but-silent daemon still owns authority; local fallback would fork it: {error}"
    );
    drop(accepting);
}

/// T-087d: the CLI's durable stop must survive the daemon's own commit path —
/// the real `mutate_issue_monitor_prefs` transaction rebasing the daemon's
/// stale in-memory view — and must not be resurrected by it.
#[test]
fn disk_stop_survives_daemon_commit_and_is_not_resurrected() {
    let _env_lock = env_test_lock();
    let temp = TempDir::new().expect("tempdir");
    let prefs_path = temp.path().join("issue-monitor.json");

    // The daemon's in-memory view: launch still running.
    let mut daemon = launched_monitor();
    gwt::save_issue_monitor_prefs(&prefs_path, &daemon.prefs()).expect("seed prefs");

    // The CLI stop, exactly as run_monitor_stop performs it: a durable
    // transaction on the prefs file.
    let (_, outcome) = gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let mut monitor =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        let outcome = monitor.stop_only(&stop_target(), "provider stuck", NOW);
        if !matches!(outcome, IssueMonitorStopOutcome::Mismatch(_)) {
            *prefs = monitor.prefs();
        }
        Ok(outcome)
    })
    .expect("stop transaction");
    assert!(
        matches!(outcome, IssueMonitorStopOutcome::Stopped { .. }),
        "the live launch must be stoppable: {outcome:?}"
    );

    // The daemon commits its (stale) view through the real transaction path.
    let (committed, _) = gwt::mutate_issue_monitor_prefs(&prefs_path, |disk| {
        daemon.rebase_daemon_driver_prefs(disk);
        *disk = daemon.prefs();
        true
    })
    .expect("daemon commit");

    assert!(
        !daemon.active_issue_numbers().contains(&42),
        "the rebased daemon must adopt the stop instead of resurrecting the launch"
    );
    assert!(
        !committed.priority_order.contains(&42) || daemon.queued_issue_numbers().is_empty(),
        "a stopped launch must not be requeued by the daemon commit"
    );
    let disk_after = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    let daemon_after = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), disk_after);
    assert!(
        !daemon_after.active_issue_numbers().contains(&42),
        "the durable prefs must keep the stop after the daemon commit"
    );

    // A fresh scan over the same candidate must not relaunch the stopped
    // issue behind the PM's back.
    let mut rescanned = daemon.clone();
    scan_issue_monitor_candidates(&mut rescanned, &[issue(42)], "2026-08-09T05:10:00Z");
    assert!(
        !rescanned.queued_issue_numbers().contains(&42),
        "a stop-held issue must stay held across the next scan"
    );
}

/// T-087d: when the durable layer refuses the write (read-only project
/// state), the stop must fail closed with a typed error — and the running
/// launch must remain observably running, because a phantom stop the daemon
/// never saw is unrecoverable for the PM.
#[test]
fn unwritable_prefs_fails_the_stop_closed_without_phantom_state() {
    let _env_lock = env_test_lock();
    let temp = TempDir::new().expect("tempdir");
    let state_dir = temp.path().join("state");
    std::fs::create_dir_all(&state_dir).expect("state dir");
    let prefs_path = state_dir.join("issue-monitor.json");

    let monitor = launched_monitor();
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed prefs");

    // Injection: the directory becomes read-only, so the transaction's
    // scratch-and-rename cannot land.
    let mut perms = std::fs::metadata(&state_dir)
        .expect("metadata")
        .permissions();
    let restore = perms.clone();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o555);
    std::fs::set_permissions(&state_dir, perms).expect("set read-only");

    let result = gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let mut inner = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        let outcome = inner.stop_only(&stop_target(), "provider stuck", NOW);
        if !matches!(outcome, IssueMonitorStopOutcome::Mismatch(_)) {
            *prefs = inner.prefs();
        }
        Ok(outcome)
    });
    std::fs::set_permissions(&state_dir, restore).expect("restore permissions");

    assert!(
        result.is_err(),
        "an unpersistable stop must surface a typed error, not a phantom success"
    );
    // The durable contract is the launch accounting (the inbox is rebuilt on
    // scan, not persisted — T-087b): the launch must still be observably
    // running, and the next scan must treat it as running rather than
    // double-queueing or resurrecting it from a half-torn state.
    let reloaded = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    let mut observed = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), reloaded);
    assert!(
        observed.active_issue_numbers().contains(&42),
        "a failed stop must leave the launch observably running"
    );
    scan_issue_monitor_candidates(&mut observed, &[issue(42)], "2026-08-09T05:20:00Z");
    assert!(
        observed.active_issue_numbers().contains(&42),
        "the running launch must survive the next scan untouched"
    );
    assert!(
        !observed.queued_issue_numbers().contains(&42),
        "a still-running launch must not be double-queued after the failed stop"
    );
    assert_eq!(
        observed.inbox_item(42).map(|item| item.state),
        Some(MonitorInboxState::Launched),
        "the rescanned inbox must show the launch as still launched"
    );
}

/// T-087d: a failover's requeue is durable prefs state; losing the
/// immediate-scan publish (hung or absent daemon) must degrade to the next
/// scheduled scan, never lose the requeue itself.
#[test]
fn failover_requeue_is_durable_even_when_scan_publish_fails() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("project tempdir");
    let home = TempDir::new().expect("home tempdir");
    let _home_guard = ScopedEnvVar::set("HOME", home.path());
    let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
    let prefs_path = project.path().join("issue-monitor.json");

    let monitor = launched_monitor();
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed prefs");

    let (_, outcome) = gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let mut inner = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        let outcome = inner.failover_restart(&stop_target(), "rate limited", NOW);
        if !matches!(outcome, gwt::IssueMonitorFailoverOutcome::Mismatch(_)) {
            *prefs = inner.prefs();
        }
        Ok(outcome)
    })
    .expect("failover transaction");
    assert!(
        matches!(outcome, gwt::IssueMonitorFailoverOutcome::Restarting { .. }),
        "the live launch must be failover-restartable: {outcome:?}"
    );

    // No daemon is registered in this HOME: the immediate-scan publish fails.
    let publish = gwt::daemon_publisher::publish_issue_monitor_control(
        project.path(),
        serde_json::json!({ "scan_now": {} }),
    );
    assert!(publish.is_err(), "no daemon means no immediate scan");

    // The requeue must not depend on that publish: the durable prefs already
    // carry the issue at the priority head for the next scheduled scan.
    let reloaded = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(
        reloaded.priority_order.first().copied(),
        Some(42),
        "failover must keep the issue at the priority head durably"
    );
    let observed = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), reloaded);
    assert!(
        !observed.active_issue_numbers().contains(&42),
        "the stopped launch must not be resurrected by the reload"
    );
}

/// SPEC #3165 FR-100/FR-101 / T-224: the disk transaction is the real
/// failover durability boundary. A subsequent daemon process must project the
/// requeued issue into a FreshRequired delivery and keep it fresh on replay,
/// without charging failover to the autonomous retry/NeedsHuman budget.
#[test]
fn failover_fresh_strategy_survives_disk_reload_until_delivery_ack() {
    let _env_lock = env_test_lock();
    let temp = TempDir::new().expect("tempdir");
    let prefs_path = temp.path().join("issue-monitor.json");

    let monitor = launched_monitor();
    let attempts_before = monitor.attempt_count(42);
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed prefs");

    gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let mut inner = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        let outcome = inner.failover_restart(&stop_target(), "rate limited", NOW);
        assert!(matches!(
            outcome,
            gwt::IssueMonitorFailoverOutcome::Restarting { .. }
        ));
        assert_eq!(inner.attempt_count(42), attempts_before);
        assert_eq!(
            inner
                .autonomous_record(42)
                .and_then(|record| record.retry_not_before.as_ref()),
            None
        );
        assert_ne!(
            inner.autonomous_record(42).map(|record| record.phase),
            Some(gwt::AutonomousPhase::NeedsHuman)
        );
        *prefs = inner.prefs();
        Ok(())
    })
    .expect("persist failover");

    let reloaded = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload queued failover");
    let mut next_daemon = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), reloaded);
    scan_issue_monitor_candidates(&mut next_daemon, &[issue(42)], "2026-08-09T05:00:01Z");
    assert!(next_daemon.apply_confirmed_claim(
        42,
        "claim-42-retry",
        "host/session",
        "effect-42-retry",
        "2026-08-09T05:00:02Z",
    ));
    gwt::save_issue_monitor_prefs(&prefs_path, &next_daemon.prefs())
        .expect("persist fresh delivery");

    let delivery_prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload fresh delivery");
    assert_eq!(
        delivery_prefs.pending_launch_deliveries[0].launch_session_strategy,
        IssueMonitorLaunchSessionStrategy::FreshRequired
    );
    let mut replayed = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), delivery_prefs);
    let first = replayed.take_pending_launch_requests();
    let second = replayed.take_pending_launch_requests();
    assert_eq!(first, second, "delivery remains replayable until its ACK");
    assert_eq!(
        first[0].launch_session_strategy,
        IssueMonitorLaunchSessionStrategy::FreshRequired,
        "disk reload and replay must not downgrade failover to ResumeIfSafe"
    );
}
