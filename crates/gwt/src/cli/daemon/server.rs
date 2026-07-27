//! Tokio-based Unix-socket IPC server for the runtime daemon (SPEC-2077
//! Phase 1 runtime layer).
//!
//! Foreground entry: caller blocks inside [`serve_blocking`] until the
//! daemon receives `SIGINT` / `SIGTERM`, at which point the listener is
//! dropped, the socket file is removed, and the persisted endpoint file
//! is unlinked. Per-connection workers handle:
//!
//! 1. Read one newline-delimited [`IpcHandshakeRequest`] JSON line.
//! 2. Validate against the in-memory endpoint with
//!    [`validate_handshake`].
//! 3. Write the matching [`IpcHandshakeResponse`] line.
//! 4. While the connection stays open, accept newline-delimited JSON
//!    payloads (today: hook envelopes log + ack, board publish/subscribe
//!    fans out via the broadcast hub, status returns daemon snapshot).
//!
//! Phase H1 (board projection daemon broadcast) is shipped. Hook
//! envelope routing into real GUI-side handlers is still on the
//! per-connection loop's TODO — Phase H2/H3/H4 will graft
//! `handle_runtime_output` / `handle_runtime_status` /
//! `handle_runtime_hook_event` / `handle_launch_complete` /
//! `handle_shell_launch_complete` ownership across the IPC boundary
//! (see SPEC-2077 plan.md Phase H1-H4).

#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use gwt_core::daemon::{
    persist_endpoint, validate_handshake, ClientFrame, DaemonEndpoint, DaemonFrame, DaemonStatus,
    IpcHandshakeRequest, IpcHandshakeResponse, RuntimeScope, DAEMON_PROTOCOL_VERSION,
};
use gwt_github::{client::http::HttpIssueClient, client::ApiError, SpecOpsError};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    runtime::Builder,
    signal::unix::{signal, SignalKind},
    sync::{broadcast::error::RecvError, mpsc, Notify},
};

use super::broadcast::BroadcastHub;

const ACCEPT_BACKOFF_MS: u64 = 50;
const ISSUE_MONITOR_SCAN_TIMEOUT: Duration = Duration::from_secs(60);
const ISSUE_MONITOR_PREFS_TIMEOUT: Duration = Duration::from_millis(250);
const DAEMON_RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);

struct DaemonShutdown {
    requested: AtomicBool,
    notify: Notify,
}

impl DaemonShutdown {
    fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    fn request(&self) {
        self.requested.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    async fn notified(&self) {
        if self.requested.load(Ordering::Acquire) {
            return;
        }
        let notified = self.notify.notified();
        if self.requested.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

pub(super) fn serve_blocking<W: std::io::Write + ?Sized>(
    scope: RuntimeScope,
    endpoint_path: PathBuf,
    writer: &mut W,
) -> Result<i32, SpecOpsError> {
    let socket_path = derive_socket_path(&endpoint_path);
    if let Err(err) = ensure_socket_parent(&socket_path) {
        return Err(config_error(format!(
            "failed to prepare daemon socket directory: {err}"
        )));
    }
    cleanup_stale_socket(&socket_path);

    let auth_token = uuid::Uuid::new_v4().to_string();
    let endpoint = DaemonEndpoint::new(
        scope,
        std::process::id(),
        socket_path.to_string_lossy().to_string(),
        auth_token,
        env!("CARGO_PKG_VERSION").to_string(),
    );

    persist_endpoint(&endpoint_path, &endpoint)
        .map_err(|err| config_error(format!("failed to persist daemon endpoint: {err}")))?;

    // Stream readiness lines to the caller's stdout *before* entering
    // the blocking serve loop. Buffering them in a `&mut String` left
    // supervising scripts unable to detect that the daemon was up
    // until the process eventually exited.
    let _ = writeln!(
        writer,
        "gwtd daemon start: bind={socket}",
        socket = socket_path.display()
    );
    let _ = writeln!(
        writer,
        "gwtd daemon start: pid={pid} version={version}",
        pid = endpoint.pid,
        version = endpoint.daemon_version
    );
    let _ = writer.flush();

    let runtime = Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .map_err(|err| config_error(format!("tokio runtime build failed: {err}")))?;

    let hub = BroadcastHub::new();
    let result = runtime.block_on(run_server(
        endpoint,
        socket_path.clone(),
        endpoint_path.clone(),
        hub,
    ));
    // A non-cooperative spawn_blocking dependency must not keep process
    // shutdown unbounded after the worker's own absolute-deadline cleanup.
    runtime.shutdown_timeout(DAEMON_RUNTIME_SHUTDOWN_TIMEOUT);

    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&endpoint_path);

    result
}

pub async fn run_server(
    endpoint: DaemonEndpoint,
    socket_path: PathBuf,
    endpoint_path: PathBuf,
    hub: BroadcastHub,
) -> Result<i32, SpecOpsError> {
    let shutdown = Arc::new(DaemonShutdown::new());
    spawn_signal_watcher(Arc::clone(&shutdown));
    run_server_with_shutdown_and_worker_config(
        endpoint,
        socket_path,
        endpoint_path,
        hub,
        shutdown,
        crate::IssueMonitorConfig::default(),
        ISSUE_MONITOR_SCAN_TIMEOUT,
    )
    .await
}

async fn run_server_with_shutdown_and_worker_config(
    endpoint: DaemonEndpoint,
    socket_path: PathBuf,
    endpoint_path: PathBuf,
    hub: BroadcastHub,
    shutdown: Arc<DaemonShutdown>,
    monitor_config: crate::IssueMonitorConfig,
    operation_timeout: Duration,
) -> Result<i32, SpecOpsError> {
    let listener = UnixListener::bind(&socket_path).map_err(|err| {
        config_error(format!(
            "failed to bind daemon socket {}: {err}",
            socket_path.display()
        ))
    })?;

    let mut issue_monitor_worker = spawn_issue_monitor_worker_with_config_and_timeout(
        endpoint.scope.clone(),
        hub.clone(),
        Arc::clone(&shutdown),
        monitor_config,
        operation_timeout,
    );

    let endpoint = Arc::new(endpoint);
    let started_at = Instant::now();
    let connections = Arc::new(AtomicUsize::new(0));
    let _endpoint_path = endpoint_path; // retained for symmetry with future watch flows
    loop {
        tokio::select! {
            biased;
            _ = shutdown.notified() => {
                tracing::info!("gwtd daemon: shutdown signal received");
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _addr)) => {
                        let endpoint = Arc::clone(&endpoint);
                        let hub = hub.clone();
                        let connections = Arc::clone(&connections);
                        tokio::spawn(async move {
                            let guard = ConnectionGuard::new(connections);
                            if let Err(err) =
                                handle_connection(stream, endpoint, hub, started_at, &guard).await
                            {
                                tracing::warn!("gwtd daemon: connection error: {err}");
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!("gwtd daemon: accept failed: {err}");
                        tokio::time::sleep(Duration::from_millis(ACCEPT_BACKOFF_MS)).await;
                    }
                }
            }
        }
    }

    // Re-broadcast shutdown after the accept loop exits. This closes the small
    // startup race where Notify::notify_waiters can occur before the worker has
    // registered its waiter.
    shutdown.request();

    // The worker owns authority revocation, durable compensations, and child
    // reaping. Do not let the server return (and drop the Tokio runtime) before
    // that shutdown protocol has had its bounded opportunity to complete.
    let worker_grace = operation_timeout
        .saturating_add(ISSUE_MONITOR_PREFS_TIMEOUT)
        .saturating_add(ISSUE_MONITOR_PREFS_TIMEOUT);
    match tokio::time::timeout(worker_grace, &mut issue_monitor_worker).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "issue monitor worker failed during daemon shutdown");
        }
        Err(_) => {
            tracing::error!("issue monitor worker exceeded daemon shutdown grace");
            issue_monitor_worker.abort();
        }
    }

    Ok(0)
}

/// RAII counter for live IPC connections. The constructor increments
/// the shared counter; `Drop` decrements it. This guarantees the
/// counter stays accurate even on panic or abnormal task abort.
struct ConnectionGuard {
    counter: Arc<AtomicUsize>,
}

impl ConnectionGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self { counter }
    }

    fn snapshot(&self) -> usize {
        self.counter.load(Ordering::SeqCst)
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

fn spawn_signal_watcher(shutdown: Arc<DaemonShutdown>) {
    let term = shutdown;
    tokio::spawn(async move {
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(sig) => sig,
            Err(err) => {
                tracing::warn!("gwtd daemon: failed to install SIGTERM handler: {err}");
                return;
            }
        };
        let mut sigint = match signal(SignalKind::interrupt()) {
            Ok(sig) => sig,
            Err(err) => {
                tracing::warn!("gwtd daemon: failed to install SIGINT handler: {err}");
                return;
            }
        };
        tokio::select! {
            _ = sigterm.recv() => {}
            _ = sigint.recv() => {}
        }
        term.request();
    });
}

#[cfg(test)]
fn spawn_issue_monitor_worker_with_config(
    scope: RuntimeScope,
    hub: BroadcastHub,
    shutdown: Arc<DaemonShutdown>,
    config: crate::IssueMonitorConfig,
) -> tokio::task::JoinHandle<()> {
    spawn_issue_monitor_worker_with_config_and_timeout(
        scope,
        hub,
        shutdown,
        config,
        ISSUE_MONITOR_SCAN_TIMEOUT,
    )
}

fn spawn_issue_monitor_worker_with_config_and_timeout(
    scope: RuntimeScope,
    hub: BroadcastHub,
    shutdown: Arc<DaemonShutdown>,
    config: crate::IssueMonitorConfig,
    operation_timeout: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut control_rx =
            hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL);
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let loaded = load_issue_monitor_state_for_daemon(&prefs_path, config);
        let mut monitor = loaded.monitor;
        // SPEC #3200 (review follow-up): a record persisted mid-review reloads in
        // `Reviewing`, but its review-agent dispatch (not persisted) is gone.
        // Reset such records to `Implementing` so the first scan re-detects the PR
        // and re-issues the review — restoring the pre-persist self-healing. The
        // `now` stamp refreshes last_heartbeat so the reset record is not wrongly
        // reclaimed by stuck detection (which runs before the re-dispatch).
        let resume_now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let resumed = if monitor.pending_effects().is_empty() {
            monitor.resume_inflight_reviews_after_restart(&resume_now)
        } else {
            tracing::info!(
                pending_effects = monitor.pending_effects().len(),
                "issue monitor: deferring review resume until durable effects reconcile"
            );
            Vec::new()
        };
        if !resumed.is_empty() {
            tracing::info!(
                issues = ?resumed,
                "issue monitor: resumed in-flight reviews after restart (Reviewing → Implementing)"
            );
        }
        publish_issue_monitor_payloads(&hub, &mut monitor);
        if loaded.recovery_blocked {
            // The unreadable bytes may describe an Attempting remote mutation.
            // No scan, control, recovery write, or shutdown rewrite is safe
            // until an operator resolves the journal explicitly. Keep the
            // read-only error projection alive for clients that connect after
            // the startup broadcast (BroadcastHub has no replay buffer).
            let mut recovery_status_tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.notified() => break,
                    _ = recovery_status_tick.tick() => {
                        publish_issue_monitor_payloads(&hub, &mut monitor);
                    }
                }
            }
            return;
        }
        let mut interval =
            tokio::time::interval(Duration::from_secs(monitor.config.poll_interval_secs));
        let mut revision = 0_u64;
        let mut scan_requested = false;
        let mut in_flight_scan: Option<InFlightIssueMonitorScan> = None;
        let mut effect_execution_requested = !monitor.pending_effects().is_empty();
        let mut in_flight_effect: Option<InFlightIssueMonitorEffect> = None;

        loop {
            let scan_watchdog_deadline = in_flight_scan
                .as_ref()
                .filter(|scan| !scan.watchdog_fired)
                .map(|scan| scan.deadline);
            let effect_watchdog_deadline = in_flight_effect
                .as_ref()
                .filter(|effect| !effect.watchdog_fired)
                .map(|effect| effect.deadline);
            tokio::select! {
                biased;
                _ = shutdown.notified() => {
                    revoke_issue_monitor_effect_authority_for_shutdown(
                        &prefs_path,
                        &mut monitor,
                    );
                    if let Some(mut scan) = in_flight_scan.take() {
                        scan.handle.abort();
                        let _ = tokio::time::timeout_at(
                            tokio::time::Instant::from_std(scan.deadline),
                            &mut scan.handle,
                        )
                        .await;
                    }
                    if let Some(mut effect) = in_flight_effect.take() {
                        // Abort cancels a queued blocking task. Once started it
                        // cannot be cancelled, so wait only through the absolute
                        // deadline captured before enqueue; the ambient deadline
                        // owns child termination/reaping.
                        effect.handle.abort();
                        let joined = tokio::time::timeout_at(
                            tokio::time::Instant::from_std(effect.deadline),
                            &mut effect.handle,
                        )
                        .await;
                        if let Ok(Ok(completed)) = joined {
                            // Authority was durably revoked first. Committing an
                            // old success can only reconcile/remove the old
                            // Attempting tuple; it cannot launch or deliver, and
                            // its newer Release/Disarm obligation remains.
                            let _ = commit_issue_monitor_effect_result(
                                &prefs_path,
                                &mut monitor,
                                completed,
                            );
                        }
                    }
                    break;
                },
                control = control_rx.recv() => {
                    match control {
                        Ok(DaemonFrame::Event { payload, .. }) => {
                            if let Some(control) = decode_issue_monitor_control(payload) {
                                let should_scan = apply_issue_monitor_control_with_disk_migration(
                                    &prefs_path,
                                    &mut monitor,
                                    control,
                                );
                                let Some(next_revision) = revision.checked_add(1) else {
                                    tracing::error!("issue monitor revision exhausted; stopping worker");
                                    break;
                                };
                                revision = next_revision;
                                publish_issue_monitor_payloads(&hub, &mut monitor);
                                effect_execution_requested =
                                    !monitor.pending_effects().is_empty();
                                if should_scan {
                                    scan_requested = true;
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                (captured_revision, captured_authority_epoch, scan_result) = wait_for_issue_monitor_scan(&mut in_flight_scan) => {
                    in_flight_scan = None;
                    match scan_result {
                        Ok(scanned_monitor) => {
                            if accept_completed_issue_monitor_scan(
                                &prefs_path,
                                &mut monitor,
                                scanned_monitor,
                                captured_revision,
                                revision,
                                captured_authority_epoch,
                            ) {
                                publish_issue_monitor_payloads(&hub, &mut monitor);
                                effect_execution_requested =
                                    !monitor.pending_effects().is_empty();
                            } else {
                                // A direct prefs writer (for example launch-
                                // profile save or a daemon-unavailable GUI
                                // fallback) may have advanced the authority
                                // epoch. The rejected commit rebases that newer
                                // snapshot into `monitor`; reconcile any adopted
                                // journal before the coalesced retry.
                                effect_execution_requested =
                                    !monitor.pending_effects().is_empty();
                                publish_issue_monitor_payloads(&hub, &mut monitor);
                                scan_requested = true;
                            }
                        }
                        Err(error) => {
                            monitor = scan_join_failure_fallback(
                                monitor.clone(),
                                error.to_string(),
                                chrono::Utc::now()
                                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                            );
                            let Some(next_revision) = revision.checked_add(1) else {
                                tracing::error!("issue monitor revision exhausted; stopping worker");
                                break;
                            };
                            revision = next_revision;
                            persist_daemon_issue_monitor_state(&prefs_path, &mut monitor);
                            publish_issue_monitor_payloads(&hub, &mut monitor);
                        }
                    }
                }
                _ = wait_for_issue_monitor_deadline(scan_watchdog_deadline) => {
                    if expire_issue_monitor_scan_at_watchdog(
                        &mut in_flight_scan,
                        &mut monitor,
                        chrono::Utc::now()
                            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    ) {
                        let Some(next_revision) = revision.checked_add(1) else {
                            tracing::error!("issue monitor revision exhausted; stopping worker");
                            break;
                        };
                        revision = next_revision;
                        publish_issue_monitor_payloads(&hub, &mut monitor);
                        // Coalesce one retry, but preserve the in-flight handle:
                        // started spawn_blocking work is not abortable and must
                        // remain the sole lane owner until it really joins.
                        scan_requested = true;
                    }
                }
                effect_result = wait_for_issue_monitor_effect(&mut in_flight_effect) => {
                    in_flight_effect = None;
                    match effect_result {
                        Ok(completed) => {
                            let settled = commit_issue_monitor_effect_result(
                                &prefs_path,
                                &mut monitor,
                                completed,
                            );
                            let Some(next_revision) = revision.checked_add(1) else {
                                tracing::error!("issue monitor revision exhausted; stopping worker");
                                break;
                            };
                            revision = next_revision;
                            publish_issue_monitor_payloads(&hub, &mut monitor);
                            if settled {
                                effect_execution_requested =
                                    !monitor.pending_effects().is_empty();
                                scan_requested = true;
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                error = %error,
                                "issue monitor effect executor task failed"
                            );
                        }
                    }
                }
                _ = wait_for_issue_monitor_deadline(effect_watchdog_deadline) => {
                    if expire_issue_monitor_effect_at_watchdog(
                        &mut in_flight_effect,
                        &mut monitor,
                    ) {
                        let Some(next_revision) = revision.checked_add(1) else {
                            tracing::error!("issue monitor revision exhausted; stopping worker");
                            break;
                        };
                        revision = next_revision;
                        publish_issue_monitor_payloads(&hub, &mut monitor);
                        // As above, the retry remains queued behind the exact
                        // still-running Attempting tuple; never overlap effects.
                        effect_execution_requested = true;
                    }
                }
                _ = interval.tick() => {
                    scan_requested = true;
                    effect_execution_requested = !monitor.pending_effects().is_empty();
                    if in_flight_scan.is_some() {
                        // Re-project the canonical state while the scan is
                        // blocked so current-time stalled status remains
                        // observable without spawning a second scan.
                        publish_issue_monitor_payloads(&hub, &mut monitor);
                    }
                }
            }

            if scan_requested && in_flight_scan.is_none() {
                scan_requested = false;
                let deadline = Instant::now() + operation_timeout;
                in_flight_scan = Some(InFlightIssueMonitorScan {
                    revision,
                    authority_epoch: monitor.effect_authority_epoch(),
                    handle: spawn_issue_monitor_scan_with_deadline(
                        scope.clone(),
                        monitor.clone(),
                        issue_monitor_gui_connected(&hub),
                        deadline,
                    ),
                    deadline,
                    watchdog_fired: false,
                });
            }

            if effect_execution_requested && in_flight_effect.is_none() {
                effect_execution_requested = false;
                let effect = monitor
                    .pending_effects()
                    .iter()
                    .find(|effect| effect.state == crate::IssueMonitorEffectState::Attempting)
                    .cloned()
                    .or_else(|| fence_next_issue_monitor_effect(&prefs_path, &mut monitor));
                if let Some(effect) = effect {
                    let authority_current = effect.authority_epoch
                        == monitor.effect_authority_epoch()
                        && match &effect.payload {
                            crate::IssueMonitorEffectPayload::AcquireClaim { .. } => {
                                monitor.config.enabled
                            }
                            crate::IssueMonitorEffectPayload::ArmAutoMerge { .. } => {
                                monitor.config.enabled && monitor.autonomous_mode()
                            }
                            crate::IssueMonitorEffectPayload::ReleaseClaim { .. }
                            | crate::IssueMonitorEffectPayload::DisarmAutoMerge { .. } => true,
                        };
                    let deadline = Instant::now() + operation_timeout;
                    in_flight_effect = Some(spawn_issue_monitor_effect(
                        scope.clone(),
                        effect,
                        authority_current,
                        deadline,
                    ));
                }
            }
        }
    })
}

struct LoadedDaemonIssueMonitorState {
    monitor: crate::IssueMonitorState,
    recovery_blocked: bool,
}

fn load_issue_monitor_state_for_daemon(
    prefs_path: &Path,
    config: crate::IssueMonitorConfig,
) -> LoadedDaemonIssueMonitorState {
    match crate::load_issue_monitor_prefs(prefs_path) {
        Ok(prefs) => LoadedDaemonIssueMonitorState {
            monitor: crate::IssueMonitorState::with_prefs(config, prefs),
            recovery_blocked: false,
        },
        Err(error) => {
            // Schema/data corruption may contain an Attempting effect whose
            // remote outcome is unknown. Never silently replace it with an
            // empty journal and continue automation. Keep the file untouched,
            // fail closed, and surface a durable-recovery error to operators.
            let mut monitor = crate::IssueMonitorState::with_prefs(
                config,
                crate::IssueMonitorPrefs::recovery_default(),
            );
            monitor.record_scan_error(
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                format!(
                    "Issue Monitor prefs are invalid; automation disabled and recovery journal preserved: {error}"
                ),
            );
            LoadedDaemonIssueMonitorState {
                monitor,
                recovery_blocked: true,
            }
        }
    }
}

fn revoke_issue_monitor_effect_authority_for_shutdown(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
) -> bool {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + ISSUE_MONITOR_PREFS_TIMEOUT,
    );
    let recovery_baseline = monitor.prefs();
    let mut candidate = monitor.clone();
    match crate::mutate_issue_monitor_prefs_recovering(prefs_path, &recovery_baseline, |disk| {
        candidate.rebase_daemon_driver_prefs(disk);
        let advanced = candidate.advance_effect_authority_epoch().is_some();
        if advanced {
            *disk = candidate.prefs();
        }
        advanced
    }) {
        Ok((_, true)) => {
            *monitor = candidate;
            true
        }
        Ok((_, false)) => {
            tracing::error!("issue monitor authority epoch exhausted during shutdown");
            false
        }
        Err(error) => {
            tracing::error!(
                error = %error,
                "issue monitor failed to persist shutdown authority revocation"
            );
            false
        }
    }
}

struct InFlightIssueMonitorScan {
    revision: u64,
    authority_epoch: u64,
    handle: tokio::task::JoinHandle<crate::IssueMonitorState>,
    deadline: Instant,
    watchdog_fired: bool,
}

async fn wait_for_issue_monitor_deadline(deadline: Option<Instant>) {
    let Some(deadline) = deadline else {
        return std::future::pending().await;
    };
    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
}

fn expire_issue_monitor_scan_at_watchdog(
    in_flight: &mut Option<InFlightIssueMonitorScan>,
    monitor: &mut crate::IssueMonitorState,
    now: String,
) -> bool {
    let Some(scan) = in_flight.as_mut() else {
        return false;
    };
    if scan.watchdog_fired {
        return false;
    }
    scan.watchdog_fired = true;
    scan.handle.abort();
    monitor.record_scan_error(now, "issue monitor scan timed out at outer watchdog stage");
    true
}

async fn wait_for_issue_monitor_scan(
    in_flight: &mut Option<InFlightIssueMonitorScan>,
) -> (
    u64,
    u64,
    Result<crate::IssueMonitorState, tokio::task::JoinError>,
) {
    let Some(scan) = in_flight.as_mut() else {
        return std::future::pending().await;
    };
    (
        scan.revision,
        scan.authority_epoch,
        (&mut scan.handle).await,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IssueMonitorControl {
    Enabled(bool),
    /// SPEC #3200 T-046/FR-024: arm/disarm the unattended autonomous mode kill
    /// switch. Disarming stops new autonomous candidates on the next scan.
    AutonomousMode(bool),
    /// SPEC #3200 FR-015: a review agent reported its verdict for a reviewed SHA.
    ReviewVerdict {
        issue_number: u64,
        reviewed_sha: String,
        verdict_raw: String,
    },
    /// SPEC #3200 T-045/FR-025: a monitored autonomous agent showed liveness;
    /// refresh the stuck-detection window for the issue.
    Heartbeat {
        issue_number: u64,
        at: String,
    },
    MaxActiveAgents(usize),
    PriorityOrder(Vec<u64>),
    Launched {
        issue_number: u64,
        window_id: String,
    },
    LaunchFailed {
        issue_number: u64,
        message: String,
    },
    AgentFailed {
        issue_number: Option<u64>,
        window_id: String,
        message: String,
    },
    WindowClosed {
        window_id: String,
    },
}

fn apply_issue_monitor_control(
    monitor: &mut crate::IssueMonitorState,
    control: IssueMonitorControl,
) -> bool {
    if !matches!(
        &control,
        IssueMonitorControl::Enabled(_) | IssueMonitorControl::AutonomousMode(_)
    ) && monitor.advance_effect_authority_epoch().is_none()
    {
        return false;
    }
    match control {
        IssueMonitorControl::Enabled(enabled) => monitor
            .set_enabled_with_effect_revocation(enabled)
            .is_some(),
        IssueMonitorControl::AutonomousMode(enabled) => monitor
            .set_autonomous_mode_with_effect_revocation(enabled)
            .is_some(),
        IssueMonitorControl::ReviewVerdict {
            issue_number,
            reviewed_sha,
            verdict_raw,
        } => {
            // The daemon (trusted) judges the raw verdict; agents cannot self-pass.
            monitor.apply_review_verdict(issue_number, &reviewed_sha, &verdict_raw);
            true
        }
        IssueMonitorControl::Heartbeat { issue_number, at } => {
            monitor.record_autonomous_heartbeat(issue_number, &at);
            false
        }
        IssueMonitorControl::MaxActiveAgents(max_active_agents) => {
            monitor.set_max_active_agents(max_active_agents);
            true
        }
        IssueMonitorControl::PriorityOrder(issue_numbers) => {
            monitor.set_priority_order(issue_numbers);
            true
        }
        IssueMonitorControl::Launched {
            issue_number,
            window_id,
        } => {
            monitor.complete_active_launch(issue_number, window_id);
            true
        }
        IssueMonitorControl::LaunchFailed {
            issue_number,
            message,
        } => {
            monitor.record_launch_failed(issue_number, message);
            true
        }
        IssueMonitorControl::AgentFailed {
            issue_number,
            window_id,
            message,
        } => {
            if let Some(issue_number) = issue_number {
                monitor.record_agent_issue_failed(issue_number, message);
            } else {
                monitor.record_agent_window_failed(&window_id, message);
            }
            true
        }
        IssueMonitorControl::WindowClosed { window_id } => {
            monitor.requeue_window(&window_id);
            true
        }
    }
}

fn apply_issue_monitor_control_with_disk_migration(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    control: IssueMonitorControl,
) -> bool {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + ISSUE_MONITOR_PREFS_TIMEOUT,
    );
    let mut applied = None;
    let recovery_baseline = monitor.prefs();
    let mut candidate = monitor.clone();
    let transaction =
        crate::mutate_issue_monitor_prefs_recovering(prefs_path, &recovery_baseline, |disk| {
            candidate.rebase_daemon_driver_prefs(disk);
            let should_scan = apply_issue_monitor_control(&mut candidate, control.clone());
            applied = Some(should_scan);
            *disk = candidate.prefs();
        });
    match transaction {
        Ok(_) => {
            *monitor = candidate;
            applied.unwrap_or(false)
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "issue monitor control prefs transaction failed; volatile mutation revoked"
            );
            monitor.record_control_commit_error(format!(
                "issue monitor control commit failed at prefs-lock stage: {error}"
            ));
            false
        }
    }
}

fn decode_issue_monitor_control(payload: serde_json::Value) -> Option<IssueMonitorControl> {
    match crate::runtime_daemon_events::decode_runtime_daemon_event(
        crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL,
        payload,
        std::process::id(),
    )? {
        crate::runtime_daemon_events::RuntimeDaemonEvent::IssueMonitor { event } => {
            if event.get("event")?.as_str()? != "control" {
                return None;
            }
            let payload = event.get("payload")?;
            if let Some(enabled) = payload.get("enabled").and_then(serde_json::Value::as_bool) {
                return Some(IssueMonitorControl::Enabled(enabled));
            }
            if let Some(autonomous_mode) = payload
                .get("autonomous_mode")
                .and_then(serde_json::Value::as_bool)
            {
                return Some(IssueMonitorControl::AutonomousMode(autonomous_mode));
            }
            if let Some(heartbeat) = payload.get("heartbeat") {
                let issue_number = heartbeat.get("issue_number")?.as_u64()?;
                let at = heartbeat
                    .get("at")
                    .and_then(serde_json::Value::as_str)?
                    .to_string();
                return Some(IssueMonitorControl::Heartbeat { issue_number, at });
            }
            if let Some(review_verdict) = payload.get("review_verdict") {
                let issue_number = review_verdict.get("issue_number")?.as_u64()?;
                let reviewed_sha = review_verdict
                    .get("reviewed_sha")
                    .and_then(serde_json::Value::as_str)?
                    .to_string();
                let verdict_raw = review_verdict
                    .get("verdict_raw")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                return Some(IssueMonitorControl::ReviewVerdict {
                    issue_number,
                    reviewed_sha,
                    verdict_raw,
                });
            }
            if let Some(max_active_agents) = payload
                .get("max_active_agents")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
            {
                return Some(IssueMonitorControl::MaxActiveAgents(max_active_agents));
            }
            if let Some(launch_failed) = payload.get("launch_failed") {
                let issue_number = launch_failed.get("issue_number")?.as_u64()?;
                let message = launch_failed
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Launch failed")
                    .to_string();
                return Some(IssueMonitorControl::LaunchFailed {
                    issue_number,
                    message,
                });
            }
            if let Some(agent_failed) = payload.get("agent_failed") {
                let issue_number = agent_failed
                    .get("issue_number")
                    .and_then(serde_json::Value::as_u64);
                let window_id = agent_failed
                    .get("window_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let message = agent_failed
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Agent failed")
                    .to_string();
                return Some(IssueMonitorControl::AgentFailed {
                    issue_number,
                    window_id,
                    message,
                });
            }
            if let Some(launched) = payload.get("launched") {
                let issue_number = launched.get("issue_number")?.as_u64()?;
                let window_id = launched
                    .get("window_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                return Some(IssueMonitorControl::Launched {
                    issue_number,
                    window_id,
                });
            }
            if let Some(window_closed) = payload.get("window_closed") {
                let window_id = window_closed.get("window_id")?.as_str()?.to_string();
                return Some(IssueMonitorControl::WindowClosed { window_id });
            }
            let issue_numbers = payload.get("priority_order")?.as_array()?;
            let issue_numbers = issue_numbers
                .iter()
                .map(serde_json::Value::as_u64)
                .collect::<Option<Vec<_>>>()?;
            Some(IssueMonitorControl::PriorityOrder(issue_numbers))
        }
        _ => None,
    }
}

#[cfg(test)]
async fn scan_issue_monitor_once(
    scope: RuntimeScope,
    monitor: crate::IssueMonitorState,
    gui_connected: bool,
) -> crate::IssueMonitorState {
    // Keep a copy of the prior state so a `spawn_blocking` panic preserves it
    // instead of collapsing to a fresh default (see `scan_join_failure_fallback`).
    let preserved = monitor.clone();
    spawn_issue_monitor_scan(scope, monitor, gui_connected)
        .await
        .unwrap_or_else(|error| {
            scan_join_failure_fallback(
                preserved,
                error.to_string(),
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            )
        })
}

#[cfg(test)]
fn spawn_issue_monitor_scan(
    scope: RuntimeScope,
    monitor: crate::IssueMonitorState,
    gui_connected: bool,
) -> tokio::task::JoinHandle<crate::IssueMonitorState> {
    let deadline = Instant::now() + ISSUE_MONITOR_SCAN_TIMEOUT;
    spawn_issue_monitor_scan_with_deadline(scope, monitor, gui_connected, deadline)
}

fn spawn_issue_monitor_scan_with_deadline(
    scope: RuntimeScope,
    monitor: crate::IssueMonitorState,
    gui_connected: bool,
    deadline: Instant,
) -> tokio::task::JoinHandle<crate::IssueMonitorState> {
    tokio::task::spawn_blocking(move || {
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline);
        scan_issue_monitor_once_blocking(scope, monitor, gui_connected)
    })
}

/// Fallback state for a `scan_issue_monitor_once` `JoinError` (the scan task
/// panicked). It preserves the prior in-memory state — the `enabled` flag,
/// `merged_issues`, autonomous records — and only records the scan error.
///
/// Returning a fresh `IssueMonitorState::new(default)` here (the previous
/// behavior) would let `scan_and_persist_issue_monitor` overwrite good prefs on
/// disk with empty/default state on a transient scan panic, losing merge
/// completion and re-launching finished work — and would also reset the GUI's
/// view (codex P2 review, #3209).
fn scan_join_failure_fallback(
    mut preserved: crate::IssueMonitorState,
    error: String,
    now: String,
) -> crate::IssueMonitorState {
    preserved.record_scan_error(now, format!("issue monitor worker join failed: {error}"));
    preserved
}

/// Test-only compatibility seam that scans once and persists the resulting
/// state. Production worker scans are supervised by the revisioned single-flight
/// driver, which persists only a result whose captured revision is still current.
#[cfg(test)]
async fn scan_and_persist_issue_monitor(
    scope: RuntimeScope,
    monitor: crate::IssueMonitorState,
    gui_connected: bool,
    prefs_path: &Path,
) -> crate::IssueMonitorState {
    let mut monitor = scan_issue_monitor_once(scope, monitor, gui_connected).await;
    persist_daemon_issue_monitor_state(prefs_path, &mut monitor);
    monitor
}

fn persist_daemon_issue_monitor_state(prefs_path: &Path, monitor: &mut crate::IssueMonitorState) {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + ISSUE_MONITOR_PREFS_TIMEOUT,
    );
    let recovery_baseline = monitor.prefs();
    if let Err(error) =
        crate::mutate_issue_monitor_prefs_recovering(prefs_path, &recovery_baseline, |disk| {
            monitor.rebase_daemon_driver_prefs(disk);
            *disk = monitor.prefs();
        })
    {
        tracing::warn!(
            error = %error,
            "issue monitor daemon prefs transaction failed"
        );
    }
}

/// Commit one side-effect-free scan proposal only while its captured authority
/// is still current both in memory and on disk. Proposed effects are restored
/// after the normal daemon rebase because the disk snapshot intentionally owns
/// the previously committed journal, not additions created by this scan clone.
fn commit_issue_monitor_scan_if_current(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    mut scanned: crate::IssueMonitorState,
    captured_authority_epoch: u64,
) -> bool {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + ISSUE_MONITOR_PREFS_TIMEOUT,
    );
    if monitor.effect_authority_epoch() != captured_authority_epoch {
        return false;
    }
    let proposed_effects = scanned.pending_effects().to_vec();
    let recovery_baseline = monitor.prefs();
    let transaction =
        crate::mutate_issue_monitor_prefs_recovering(prefs_path, &recovery_baseline, |disk| {
            if disk.effect_authority_epoch != captured_authority_epoch {
                return false;
            }
            scanned.rebase_daemon_driver_prefs(disk);
            for effect in proposed_effects.iter().cloned() {
                if !scanned
                    .pending_effects()
                    .iter()
                    .any(|pending| pending.effect_id == effect.effect_id)
                {
                    let _ = scanned.prepare_effect(effect);
                }
            }
            *disk = scanned.prefs();
            true
        });
    match transaction {
        Ok((_, true)) => {
            *monitor = scanned;
            true
        }
        Ok((committed, false)) => {
            // The scan was correctly rejected, but the daemon must not keep
            // retrying from the obsolete generation forever. Adopt the latest
            // committed authority/config/journal while discarding every
            // proposal produced by the stale scan.
            monitor.rebase_daemon_driver_prefs(&committed);
            false
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "issue monitor scan proposal commit failed"
            );
            false
        }
    }
}

fn accept_completed_issue_monitor_scan(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    scanned: crate::IssueMonitorState,
    captured_revision: u64,
    current_revision: u64,
    captured_authority_epoch: u64,
) -> bool {
    if captured_revision != current_revision {
        return false;
    }
    commit_issue_monitor_scan_if_current(prefs_path, monitor, scanned, captured_authority_epoch)
}

/// Persist the Prepared -> Attempting execution fence before handing an effect
/// to the remote executor. A caller that receives `Some` therefore has a
/// durable `(effect_id, authority_epoch, attempt)` receipt to reconcile after a
/// crash or outcome-ambiguous command.
fn fence_next_issue_monitor_effect(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
) -> Option<crate::PendingIssueMonitorEffect> {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + ISSUE_MONITOR_PREFS_TIMEOUT,
    );
    let recovery_baseline = monitor.prefs();
    let mut candidate = monitor.clone();
    let mut fenced = None;
    let transaction =
        crate::mutate_issue_monitor_prefs_recovering(prefs_path, &recovery_baseline, |disk| {
            candidate.rebase_daemon_driver_prefs(disk);
            let effect = candidate
                .pending_effects()
                .iter()
                .find(|effect| effect.state == crate::IssueMonitorEffectState::Prepared)
                .cloned();
            let Some(effect) = effect else {
                return;
            };
            let key = effect.attempt_key();
            if candidate.mark_pending_effect_attempting(&key) {
                fenced = candidate
                    .pending_effects()
                    .iter()
                    .find(|pending| pending.attempt_key() == key)
                    .cloned();
                *disk = candidate.prefs();
            }
        });
    match transaction {
        Ok(_) => {
            *monitor = candidate;
            fenced
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "issue monitor effect fence commit failed"
            );
            None
        }
    }
}

#[derive(Debug)]
enum IssueMonitorEffectOutcome {
    Claim(
        gwt_github::client::OwnerMutationResult<gwt_github::issue_auto_claim::ClaimAcquireOutcome>,
    ),
    RevokedClaim(
        gwt_github::client::OwnerMutationResult<gwt_github::issue_auto_claim::ClaimReleaseOutcome>,
    ),
    Release(
        gwt_github::client::OwnerMutationResult<gwt_github::issue_auto_claim::ClaimReleaseOutcome>,
    ),
    AutoMerge(gwt_git::pr_status::AutoMergeMutationOutcome),
}

#[derive(Debug)]
struct CompletedIssueMonitorEffect {
    effect: crate::PendingIssueMonitorEffect,
    outcome: IssueMonitorEffectOutcome,
    completed_at: String,
}

struct InFlightIssueMonitorEffect {
    handle: tokio::task::JoinHandle<CompletedIssueMonitorEffect>,
    deadline: Instant,
    watchdog_fired: bool,
}

fn expire_issue_monitor_effect_at_watchdog(
    in_flight: &mut Option<InFlightIssueMonitorEffect>,
    monitor: &mut crate::IssueMonitorState,
) -> bool {
    let Some(effect) = in_flight.as_mut() else {
        return false;
    };
    if effect.watchdog_fired {
        return false;
    }
    effect.watchdog_fired = true;
    effect.handle.abort();
    // The exact Attempting tuple stays durable. Its remote result is unknown;
    // the next executor pass must begin with fresh readback before any retry.
    monitor.record_control_commit_error(
        "issue monitor effect executor timed out at outer watchdog stage; remote outcome unknown",
    );
    true
}

async fn wait_for_issue_monitor_effect(
    in_flight: &mut Option<InFlightIssueMonitorEffect>,
) -> Result<CompletedIssueMonitorEffect, tokio::task::JoinError> {
    let Some(effect) = in_flight.as_mut() else {
        return std::future::pending().await;
    };
    (&mut effect.handle).await
}

fn spawn_issue_monitor_effect(
    scope: RuntimeScope,
    effect: crate::PendingIssueMonitorEffect,
    authority_current: bool,
    deadline: Instant,
) -> InFlightIssueMonitorEffect {
    let handle = tokio::task::spawn_blocking(move || {
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline);
        let execution_now = chrono::Utc::now();
        let outcome = execute_issue_monitor_effect(
            &scope,
            &effect,
            authority_current,
            &execution_now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
        CompletedIssueMonitorEffect {
            effect,
            outcome,
            completed_at: execution_now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }
    });
    InFlightIssueMonitorEffect {
        handle,
        deadline,
        watchdog_fired: false,
    }
}

fn execute_issue_monitor_effect(
    scope: &RuntimeScope,
    effect: &crate::PendingIssueMonitorEffect,
    authority_current: bool,
    execution_now: &str,
) -> IssueMonitorEffectOutcome {
    use gwt_github::client::OwnerMutationError;
    use gwt_github::issue_auto_claim::{
        acquire_claim_mutation, release_claim_mutation, ClaimComment, ClaimStatus,
    };
    use gwt_github::IssueNumber;

    match &effect.payload {
        crate::IssueMonitorEffectPayload::AcquireClaim {
            issue_number,
            claim_id,
            owner,
            heartbeat_at,
            expires_at,
            launched_work_id,
        } => {
            let client = issue_monitor_http_client(scope);
            if !authority_current {
                return IssueMonitorEffectOutcome::RevokedClaim(match client {
                    Ok(client) => {
                        release_claim_mutation(&client, IssueNumber(*issue_number), claim_id)
                    }
                    Err(error) => Err(OwnerMutationError::PreSubmit(
                        gwt_github::ApiError::Network(error),
                    )),
                });
            }
            let ttl_secs = claim_ttl_secs(heartbeat_at, expires_at);
            let expires_at = (chrono::DateTime::parse_from_rfc3339(execution_now)
                .map(|now| now + chrono::Duration::seconds(ttl_secs as i64)))
            .map(|expires| expires.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_else(|_| expires_at.clone());
            IssueMonitorEffectOutcome::Claim(match client {
                Ok(client) => acquire_claim_mutation(
                    &client,
                    IssueNumber(*issue_number),
                    ClaimComment {
                        comment_id: None,
                        claim_id: claim_id.clone(),
                        owner: owner.clone(),
                        issue_number: *issue_number,
                        status: ClaimStatus::Active,
                        heartbeat_at: execution_now.to_string(),
                        expires_at,
                        launched_work_id: launched_work_id.clone(),
                    },
                    execution_now,
                ),
                Err(error) => Err(OwnerMutationError::PreSubmit(
                    gwt_github::ApiError::Network(error),
                )),
            })
        }
        crate::IssueMonitorEffectPayload::ReleaseClaim {
            issue_number,
            claim_id,
        } => IssueMonitorEffectOutcome::Release(match issue_monitor_http_client(scope) {
            Ok(client) => release_claim_mutation(&client, IssueNumber(*issue_number), claim_id),
            Err(error) => Err(OwnerMutationError::PreSubmit(
                gwt_github::ApiError::Network(error),
            )),
        }),
        crate::IssueMonitorEffectPayload::ArmAutoMerge {
            pr_number,
            reviewed_sha,
            ..
        } => {
            let Some(remote) = gwt_git::pr_status::fetch_pr_auto_merge_remote_state(
                &scope.project_root,
                *pr_number,
            ) else {
                return IssueMonitorEffectOutcome::AutoMerge(
                    gwt_git::pr_status::AutoMergeMutationOutcome::RemoteOutcomeUnknown(
                        "auto-merge readback unavailable".to_string(),
                    ),
                );
            };
            if authority_current {
                IssueMonitorEffectOutcome::AutoMerge(gwt_git::pr_status::arm_pr_auto_merge(
                    &scope.project_root,
                    *pr_number,
                    reviewed_sha,
                    &remote,
                ))
            } else {
                let outcome = match remote {
                    gwt_git::pr_status::PrAutoMergeRemoteState::Open {
                        auto_merge_requested: true,
                        ..
                    } => gwt_git::pr_status::AutoMergeMutationOutcome::AlreadyTargetState,
                    _ => gwt_git::pr_status::AutoMergeMutationOutcome::PreSubmit(
                        "authority revoked before auto-merge submission".to_string(),
                    ),
                };
                IssueMonitorEffectOutcome::AutoMerge(outcome)
            }
        }
        crate::IssueMonitorEffectPayload::DisarmAutoMerge { pr_number, .. } => {
            let Some(remote) = gwt_git::pr_status::fetch_pr_auto_merge_remote_state(
                &scope.project_root,
                *pr_number,
            ) else {
                return IssueMonitorEffectOutcome::AutoMerge(
                    gwt_git::pr_status::AutoMergeMutationOutcome::RemoteOutcomeUnknown(
                        "auto-merge readback unavailable".to_string(),
                    ),
                );
            };
            IssueMonitorEffectOutcome::AutoMerge(gwt_git::pr_status::disarm_pr_auto_merge(
                &scope.project_root,
                *pr_number,
                &remote,
            ))
        }
    }
}

fn claim_ttl_secs(heartbeat_at: &str, expires_at: &str) -> u64 {
    let heartbeat = chrono::DateTime::parse_from_rfc3339(heartbeat_at);
    let expires = chrono::DateTime::parse_from_rfc3339(expires_at);
    heartbeat
        .ok()
        .zip(expires.ok())
        .and_then(|(heartbeat, expires)| (expires - heartbeat).num_seconds().try_into().ok())
        .filter(|ttl| *ttl > 0)
        .unwrap_or(crate::IssueMonitorConfig::default().claim_ttl_secs)
}

fn issue_monitor_http_client(scope: &RuntimeScope) -> Result<HttpIssueClient, String> {
    #[cfg(test)]
    if let Some(marker) = std::env::var_os("GWT_TEST_ISSUE_MONITOR_HTTP_CLIENT_MARKER") {
        let _ = fs::write(marker, b"attempted");
    }
    let (owner, repo) =
        crate::issue_monitor_worker::github_remote_owner_and_repo(&scope.project_root)
            .map_err(|error| error.to_string())?;
    HttpIssueClient::from_gh_auth(&owner, &repo).map_err(|error| error.to_string())
}

/// Commit an executor result only against the exact Attempting tuple still on
/// disk. Returns true when the attempt reached a terminal local transition, so
/// the driver may immediately service a queued safety compensation.
fn commit_issue_monitor_effect_result(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    completed: CompletedIssueMonitorEffect,
) -> bool {
    use gwt_git::pr_status::AutoMergeMutationOutcome;
    use gwt_github::client::OwnerMutationError;
    use gwt_github::issue_auto_claim::ClaimAcquireOutcome;

    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + ISSUE_MONITOR_PREFS_TIMEOUT,
    );

    let key = completed.effect.attempt_key();
    let recovery_baseline = monitor.prefs();
    let mut candidate = monitor.clone();
    let mut settled = false;
    let transaction = crate::mutate_issue_monitor_prefs_recovering(
        prefs_path,
        &recovery_baseline,
        |disk| {
            candidate.rebase_daemon_driver_prefs(disk);
            let exact_attempting = candidate.pending_effects().iter().any(|effect| {
                effect.state == crate::IssueMonitorEffectState::Attempting
                    && effect.attempt_key() == key
            });
            if !exact_attempting {
                return;
            }
            let current_authority = candidate.effect_authority_epoch() == key.authority_epoch;
            match (&completed.effect.payload, completed.outcome) {
                (
                    crate::IssueMonitorEffectPayload::AcquireClaim { issue_number, .. },
                    IssueMonitorEffectOutcome::Claim(Ok(ClaimAcquireOutcome::Acquired(claim))),
                ) => {
                    let _ = candidate.complete_pending_effect(&key);
                    if current_authority && candidate.config.enabled {
                        let _ = candidate.apply_confirmed_claim(
                            *issue_number,
                            claim.claim_id,
                            &completed.completed_at,
                        );
                    }
                    settled = true;
                }
                (
                    crate::IssueMonitorEffectPayload::AcquireClaim { issue_number, .. },
                    IssueMonitorEffectOutcome::Claim(Ok(ClaimAcquireOutcome::Blocked(claim))),
                ) => {
                    let _ = candidate.complete_pending_effect(&key);
                    if current_authority {
                        if let Some(issue) = candidate
                            .inbox_item(*issue_number)
                            .map(|item| item.issue.clone())
                        {
                            candidate.record_blocked_by_claim(issue, claim.owner, claim.expires_at);
                        }
                    }
                    settled = true;
                }
                (
                    crate::IssueMonitorEffectPayload::AcquireClaim { issue_number, .. },
                    IssueMonitorEffectOutcome::Claim(Ok(ClaimAcquireOutcome::Lost {
                        winning_claim,
                        ..
                    })),
                ) => {
                    let _ = candidate.complete_pending_effect(&key);
                    if current_authority {
                        if let Some(issue) = candidate
                            .inbox_item(*issue_number)
                            .map(|item| item.issue.clone())
                        {
                            candidate.record_blocked_by_claim(
                                issue,
                                winning_claim.owner,
                                winning_claim.expires_at,
                            );
                        }
                    }
                    settled = true;
                }
                (
                    crate::IssueMonitorEffectPayload::AcquireClaim { .. },
                    IssueMonitorEffectOutcome::RevokedClaim(Ok(_)),
                ) => {
                    let _ = candidate.complete_pending_effect(&key);
                    settled = true;
                }
                (
                    crate::IssueMonitorEffectPayload::ReleaseClaim { .. },
                    IssueMonitorEffectOutcome::Release(Ok(_)),
                ) => {
                    let _ = candidate.complete_pending_effect(&key);
                    settled = true;
                }
                (
                    crate::IssueMonitorEffectPayload::ArmAutoMerge { issue_number, .. },
                    IssueMonitorEffectOutcome::AutoMerge(outcome),
                ) if outcome.is_success() => {
                    let _ = candidate.complete_pending_effect(&key);
                    if current_authority && candidate.config.enabled && candidate.autonomous_mode()
                    {
                        candidate.begin_delivering(*issue_number);
                        candidate.record_auto_merge_armed(*issue_number);
                    }
                    settled = true;
                }
                (
                    crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                        issue_number,
                        pr_number,
                        ..
                    },
                    IssueMonitorEffectOutcome::AutoMerge(outcome),
                ) if outcome.is_success() => {
                    let _ = candidate.complete_pending_effect(&key);
                    candidate.record_kill_switch_disarm_result(*issue_number, *pr_number, true);
                    settled = true;
                }
                (
                    crate::IssueMonitorEffectPayload::DisarmAutoMerge { issue_number, .. },
                    IssueMonitorEffectOutcome::AutoMerge(
                        AutoMergeMutationOutcome::AuthorityMismatch(reason),
                    ),
                ) => {
                    let _ = candidate.complete_pending_effect(&key);
                    candidate.escalate_to_needs_human(
                        *issue_number,
                        format!("kill-switch disarm authority failure: {reason}"),
                    );
                    settled = true;
                }
                (
                    crate::IssueMonitorEffectPayload::ArmAutoMerge {
                        issue_number,
                        pr_number,
                        ..
                    },
                    IssueMonitorEffectOutcome::AutoMerge(AutoMergeMutationOutcome::HeadChanged {
                        expected,
                        actual,
                    }),
                ) => {
                    let _ = candidate.complete_pending_effect(&key);
                    let arm_effect_id = completed.effect.effect_id.clone();
                    let compensation_exists = candidate.pending_effects().iter().any(|pending| {
                        matches!(
                            &pending.payload,
                            crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                                compensates_effect_id,
                                ..
                            } if compensates_effect_id == &arm_effect_id
                        )
                    });
                    if !compensation_exists {
                        let epoch = candidate.effect_authority_epoch();
                        let _ =
                            candidate.prepare_effect(crate::PendingIssueMonitorEffect::prepared(
                                format!("disarm:{arm_effect_id}:{epoch}"),
                                epoch,
                                crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                                    issue_number: *issue_number,
                                    pr_number: *pr_number,
                                    compensates_effect_id: arm_effect_id,
                                },
                            ));
                    }
                    if current_authority {
                        candidate.escalate_to_needs_human(
                            *issue_number,
                            format!(
                                "auto-merge authority rejected: reviewed HEAD {expected}, current HEAD {actual}"
                            ),
                        );
                    }
                    settled = true;
                }
                (
                    crate::IssueMonitorEffectPayload::ArmAutoMerge { issue_number, .. },
                    IssueMonitorEffectOutcome::AutoMerge(
                        AutoMergeMutationOutcome::AuthorityMismatch(reason),
                    ),
                ) => {
                    let _ = candidate.complete_pending_effect(&key);
                    if current_authority {
                        candidate.escalate_to_needs_human(*issue_number, reason);
                    }
                    settled = true;
                }
                (
                    crate::IssueMonitorEffectPayload::ReleaseClaim { .. }
                    | crate::IssueMonitorEffectPayload::DisarmAutoMerge { .. },
                    IssueMonitorEffectOutcome::AutoMerge(AutoMergeMutationOutcome::PreSubmit(_))
                    | IssueMonitorEffectOutcome::RevokedClaim(Err(OwnerMutationError::PreSubmit(_)))
                    | IssueMonitorEffectOutcome::Release(Err(OwnerMutationError::PreSubmit(_))),
                ) => {
                    // Safety effects are monotonic obligations, not grants.
                    // A later control may advance the epoch but cannot revoke
                    // release/disarm; definite pre-submit failure must retry.
                    let _ = candidate.retry_pending_effect(&key);
                }
                (
                    _,
                    IssueMonitorEffectOutcome::AutoMerge(AutoMergeMutationOutcome::PreSubmit(_))
                    | IssueMonitorEffectOutcome::Claim(Err(OwnerMutationError::PreSubmit(_))),
                ) => {
                    if current_authority {
                        let _ = candidate.retry_pending_effect(&key);
                    } else {
                        let _ = candidate.complete_pending_effect(&key);
                        settled = true;
                    }
                }
                (
                    crate::IssueMonitorEffectPayload::AcquireClaim { .. },
                    IssueMonitorEffectOutcome::RevokedClaim(Err(OwnerMutationError::PreSubmit(_))),
                ) => {
                    // Authority advancement already appended an independent
                    // durable ReleaseClaim obligation for this stable claim id.
                    let _ = candidate.complete_pending_effect(&key);
                    settled = true;
                }
                (
                    _,
                    IssueMonitorEffectOutcome::Claim(Err(
                        OwnerMutationError::RemoteOutcomeUnknown(_),
                    ))
                    | IssueMonitorEffectOutcome::RevokedClaim(Err(
                        OwnerMutationError::RemoteOutcomeUnknown(_),
                    ))
                    | IssueMonitorEffectOutcome::Release(Err(
                        OwnerMutationError::RemoteOutcomeUnknown(_),
                    )),
                ) => {
                    // Keep Attempting. Stable claim_id readback reconciles this
                    // attempt before any subsequent mutation or launch.
                }
                (
                    _,
                    IssueMonitorEffectOutcome::AutoMerge(
                        AutoMergeMutationOutcome::RemoteOutcomeUnknown(_),
                    ),
                ) => {
                    // Keep Attempting. A later executor pass starts with fresh
                    // readback and reconciles before another target mutation.
                }
                (_, IssueMonitorEffectOutcome::AutoMerge(AutoMergeMutationOutcome::Confirmed))
                | (
                    _,
                    IssueMonitorEffectOutcome::AutoMerge(
                        AutoMergeMutationOutcome::AlreadyTargetState,
                    ),
                ) => unreachable!("success outcomes handled by payload-specific arms"),
                _ => {
                    tracing::error!(
                        effect = %completed.effect.effect_id,
                        "issue monitor effect result/payload mismatch"
                    );
                }
            }
            if settled && candidate.pending_effects().is_empty() {
                let resumed =
                    candidate.resume_inflight_reviews_after_restart(&completed.completed_at);
                if !resumed.is_empty() {
                    tracing::info!(
                        issues = ?resumed,
                        "issue monitor: resumed reviews after durable effects reconciled"
                    );
                }
            }
            *disk = candidate.prefs();
        },
    );
    match transaction {
        Ok(_) => {
            *monitor = candidate;
            settled
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "issue monitor effect result commit failed"
            );
            false
        }
    }
}

/// SPEC #3200 Option A: a per-process secret the daemon uses to sign autonomous
/// merge-authorization audit tokens. Agents never see it, so they cannot forge a
/// daemon authorization. Stable for the daemon's lifetime.
fn daemon_run_secret() -> &'static [u8] {
    static SECRET: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    SECRET
        .get_or_init(|| uuid::Uuid::new_v4().as_bytes().to_vec())
        .as_slice()
}

fn scan_issue_monitor_once_blocking(
    scope: RuntimeScope,
    mut monitor: crate::IssueMonitorState,
    gui_connected: bool,
) -> crate::IssueMonitorState {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // Issue #3222: refresh the GUI-owned prefs fields (launch profile / tuning)
    // from disk. They have no control frame — the GUI writes them straight to
    // the prefs file — so without this the daemon keeps its stale startup view
    // (`has_launch_profile()==false` ⇒ active cap 0) and can never act as the
    // launch driver, leaving refills to the GUI's racy re-entrant scans.
    if let Ok(disk) = crate::load_issue_monitor_prefs(
        &crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root),
    ) {
        monitor.rebase_daemon_driver_prefs(&disk);
    }
    // #3223 follow-up (codex P2): expire claimed-but-never-acked launches past
    // claim_ttl_secs so a crashed launch cannot hold a slot forever.
    monitor.expire_stale_unbound_launches(&now);
    let (owner, repo) =
        match crate::issue_monitor_worker::github_remote_owner_and_repo(&scope.project_root) {
            Ok(owner_repo) => owner_repo,
            Err(error) => {
                monitor.record_scan_error(now, error.to_string());
                return monitor;
            }
        };
    let loaded = match crate::issue_monitor_worker::load_open_issue_monitor_candidates_for_repo_path_with_provenance(
        &scope.project_root,
        &owner,
        &repo,
    ) {
        Ok(loaded) => loaded,
        Err(error) => {
            monitor.record_scan_error(now, format!("issue list failed: {error}"));
            return monitor;
        }
    };
    let monitor_owner = format!("{}:{}", whoami::username(), std::process::id());
    crate::issue_monitor_worker::scan_loaded_issue_monitor_candidates(
        &mut monitor,
        &loaded,
        &scope.project_root,
        &now,
    );
    crate::issue_monitor_worker::reconcile_issue_monitor_merges(&mut monitor, &scope.project_root);
    // SPEC #3200 T-041/T-044: autonomous pre-launch eligibility gate + stuck-slot
    // recovery. Both are no-ops unless autonomous mode is on (default OFF keeps
    // the SPEC #3165 human-gated flow unchanged).
    if loaded.authorizes_remote_effects() {
        crate::issue_monitor_worker::apply_autonomous_eligibility(
            &mut monitor,
            &loaded.issues,
            &format!("{owner}/{repo}"),
            &scope.project_root,
            &now,
        );
    }
    monitor.recover_stuck_autonomous(&now);
    // SPEC #3200 Phase 7: a scan only proposes kill-switch disarms. Executing
    // `gh pr merge --disable-auto` here would let a stale cloned scan mutate
    // GitHub even after the canonical driver rejected its result. The durable
    // executor owns submission after this proposal commits and receives a
    // separate Attempting fence.
    for (issue_number, pr_number) in monitor.kill_switch_disarm_targets() {
        let epoch = monitor.effect_authority_epoch();
        monitor.prepare_effect(crate::PendingIssueMonitorEffect {
            effect_id: format!("disarm:kill-switch:{issue_number}:{pr_number}:{epoch}"),
            authority_epoch: epoch,
            attempt: 0,
            state: crate::IssueMonitorEffectState::Prepared,
            payload: crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                issue_number,
                pr_number,
                compensates_effect_id: format!("legacy-delivery:{issue_number}:{pr_number}"),
            },
        });
    }
    // SPEC #3200 Option A: advance in-flight autonomous issues through the loop
    // (PR detect → review → gate → merge → watch). No-op unless autonomous mode
    // is on; default OFF keeps the SPEC #3165 flow unchanged.
    if loaded.authorizes_remote_effects() {
        crate::issue_monitor_worker::advance_autonomous_in_flight(
            &mut monitor,
            &loaded.issues,
            &format!("{owner}/{repo}"),
            &scope.project_root,
            daemon_run_secret(),
            &now,
        );
    }
    if loaded.authorizes_remote_effects() && monitor.config.enabled && gui_connected {
        let active_cap = if monitor.has_launch_profile() {
            monitor.config.max_active.max(1)
        } else {
            0
        };
        if monitor.active_count() < active_cap {
            monitor.prepare_claim_effects_with_probe(
                &monitor_owner,
                &now,
                active_cap,
                |issue_number| {
                    crate::issue_monitor_worker::issue_completed_by_merged_pr(
                        &owner,
                        &repo,
                        issue_number,
                    )
                },
            );
        }
    }
    monitor
}

fn publish_issue_monitor_payloads(hub: &BroadcastHub, monitor: &mut crate::IssueMonitorState) {
    let gui_connected = issue_monitor_gui_connected(hub);
    for payload in
        crate::issue_monitor_worker::issue_monitor_daemon_payloads(monitor, gui_connected)
    {
        let payload = crate::runtime_daemon_events::issue_monitor_payload(
            &payload.event,
            payload.payload,
            std::process::id(),
        );
        let _ = hub.publish(
            crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL,
            DaemonFrame::Event {
                channel: crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL.to_string(),
                payload,
            },
        );
    }
}

fn issue_monitor_gui_connected(hub: &BroadcastHub) -> bool {
    hub.receiver_count(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL) > 0
}

async fn handle_connection(
    stream: UnixStream,
    endpoint: Arc<DaemonEndpoint>,
    hub: BroadcastHub,
    started_at: Instant,
    connection_guard: &ConnectionGuard,
) -> Result<(), String> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let request = read_handshake(&mut reader).await?;
    let response = build_handshake_response(&endpoint, &request);
    write_json_line(&mut write_half, &response).await?;

    let validation = validate_handshake(&endpoint, &request, &response);
    if validation.is_err() {
        return Ok(()); // we already told the client; drop the connection.
    }

    // After handshake, all writes flow through `out_tx` so the reader loop
    // and any broadcast forwarders can send concurrently without sharing
    // `write_half` directly.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<DaemonFrame>();
    // Cancellation primitive fired when the reader exits (peer
    // closed, EOF, or read error). Each per-channel forwarder spawns
    // with a clone of `out_tx`, so without this signal the forwarders
    // would stay parked on `rx.recv()` forever, keeping the writer
    // task alive (out_rx still has senders) and leaking both the
    // connection task and its `ConnectionGuard` — the connection
    // counter in `DaemonStatus` would be permanently inflated.
    //
    // We use a `(AtomicBool, Notify)` pair instead of `Notify` alone
    // because `notify_waiters` is fire-and-forget: a forwarder that
    // is between `rx.recv()` and `out_tx.send()` when the cancel
    // fires would miss the notification and re-enter `select!` on a
    // fresh `notified()` future that never resolves. The atomic flag
    // is checked at the top of each iteration to close that race.
    let forwarder_cancel = Arc::new(AtomicBool::new(false));
    let forwarder_notify = Arc::new(Notify::new());
    let writer = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            if let Err(err) = write_json_line(&mut write_half, &frame).await {
                tracing::warn!(target: "gwtd::daemon", error = %err, "writer task failed");
                break;
            }
        }
    });

    let mut line = String::new();
    loop {
        line.clear();
        let n = match reader.read_line(&mut line).await {
            Ok(n) => n,
            Err(err) => {
                tracing::warn!(target: "gwtd::daemon", error = %err, "read frame failed");
                break;
            }
        };
        if n == 0 {
            break; // peer closed
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<ClientFrame>(trimmed) {
            Ok(ClientFrame::Hook(envelope)) => {
                // Hook envelope routing into real GUI-side handlers is
                // gated on Phase H3 (handle_runtime_hook_event daemon
                // migration). Until then we ack so the client side knows
                // the daemon received the frame, and the existing
                // synchronous `gwt hook ...` dispatch path remains the
                // outward-facing fallback.
                tracing::debug!(
                    target: "gwtd::daemon",
                    hook = %envelope.hook_name,
                    "received hook envelope"
                );
                if out_tx.send(DaemonFrame::Ack).is_err() {
                    break;
                }
            }
            Ok(ClientFrame::Subscribe { channels }) => {
                for channel in channels {
                    let mut rx = hub.subscribe(&channel);
                    let out_tx = out_tx.clone();
                    let channel_for_log = channel.clone();
                    let cancel = Arc::clone(&forwarder_cancel);
                    let notify = Arc::clone(&forwarder_notify);
                    tokio::spawn(async move {
                        loop {
                            // Atomic flag check protects against the
                            // race where `notify_waiters` fires while
                            // we're in the match arm below; a fresh
                            // `notified()` future created the next
                            // iteration would otherwise miss the
                            // notification and park forever.
                            if cancel.load(Ordering::SeqCst) {
                                break;
                            }
                            tokio::select! {
                                biased;
                                _ = notify.notified() => break,
                                result = rx.recv() => {
                                    match result {
                                        Ok(frame) => {
                                            if out_tx.send(frame).is_err() {
                                                break;
                                            }
                                        }
                                        // `Lagged` is the broadcast
                                        // channel's "you're behind by
                                        // N frames" signal: capacity
                                        // is `DEFAULT_CHANNEL_CAPACITY`
                                        // (64) and a slow subscriber
                                        // can drop frames if a publish
                                        // burst overruns the
                                        // forwarder's drain. The
                                        // subscription itself is still
                                        // healthy — keep reading the
                                        // newer frames so the slow
                                        // client recovers instead of
                                        // silently losing the channel
                                        // forever.
                                        Err(RecvError::Lagged(skipped)) => {
                                            tracing::warn!(
                                                target: "gwtd::daemon",
                                                channel = %channel_for_log,
                                                skipped,
                                                "broadcast receiver lagged; resuming with newer frames"
                                            );
                                        }
                                        Err(RecvError::Closed) => {
                                            tracing::debug!(
                                                target: "gwtd::daemon",
                                                channel = %channel_for_log,
                                                "broadcast receiver closed"
                                            );
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    });
                }
                if out_tx.send(DaemonFrame::Ack).is_err() {
                    break;
                }
            }
            Ok(ClientFrame::Status) => {
                let snapshot = DaemonStatus {
                    protocol_version: endpoint.protocol_version,
                    daemon_version: endpoint.daemon_version.clone(),
                    uptime_seconds: started_at.elapsed().as_secs(),
                    broadcast_channels: hub.channel_count(),
                    connections: connection_guard.snapshot(),
                };
                if out_tx.send(DaemonFrame::Status(snapshot)).is_err() {
                    break;
                }
            }
            Ok(ClientFrame::Publish { channel, payload }) => {
                if channel == crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL {
                    // Issue Monitor control is a command, not a lossy
                    // notification. Publish first so the response reflects
                    // actual fan-out: during daemon startup a zero-subscriber
                    // Ack would make the GUI skip its safe local fallback and
                    // silently discard the user's control.
                    let (response, queued) = fan_out_issue_monitor_control(&hub, &channel, payload);
                    if out_tx.send(response).is_err() {
                        break;
                    }
                    tracing::debug!(
                        target: "gwtd::daemon",
                        %channel,
                        queued,
                        "issue monitor control frame fanned out"
                    );
                    continue;
                }
                // Enqueue the Ack into our `out_tx` *before* the
                // broadcast fan-out so a client that is both
                // subscribed and publishing on the same connection
                // never observes its own broadcast Event arrive
                // before the Ack for the Publish that triggered it.
                // Without this ordering the spawned per-channel
                // forwarder task can race the Publish reader and
                // push `DaemonFrame::Event` into `out_tx` first,
                // desynchronizing any caller doing a simple
                // `send_frame(Publish) -> read_frame::<Ack>` flow.
                if out_tx.send(DaemonFrame::Ack).is_err() {
                    break;
                }
                let queued = hub.publish(
                    &channel,
                    DaemonFrame::Event {
                        channel: channel.clone(),
                        payload,
                    },
                );
                tracing::debug!(
                    target: "gwtd::daemon",
                    %channel,
                    queued,
                    "publish frame fanned out"
                );
            }
            Err(err) => {
                tracing::warn!(target: "gwtd::daemon", frame = %trimmed, error = %err, "rejected unrecognized frame");
                if out_tx
                    .send(DaemonFrame::Error {
                        message: format!("frame parse failed: {err}"),
                    })
                    .is_err()
                {
                    break;
                }
            }
        }
    }

    // Reader exited (peer closed, EOF, or read error). Wake every
    // active forwarder so they drop their `out_tx` clones; once all
    // senders are dropped the writer task's `out_rx.recv()` returns
    // `None` and the task ends, allowing this connection task (and
    // its `ConnectionGuard`) to be released.
    forwarder_cancel.store(true, Ordering::SeqCst);
    forwarder_notify.notify_waiters();
    drop(out_tx);
    let _ = writer.await;
    Ok(())
}

fn fan_out_issue_monitor_control(
    hub: &BroadcastHub,
    channel: &str,
    payload: serde_json::Value,
) -> (DaemonFrame, usize) {
    let queued = hub.publish(
        channel,
        DaemonFrame::Event {
            channel: channel.to_string(),
            payload,
        },
    );
    let response = if queued == 0 {
        DaemonFrame::Error {
            message: "issue monitor control worker is not ready".to_string(),
        }
    } else {
        DaemonFrame::Ack
    };
    (response, queued)
}

async fn read_handshake(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
) -> Result<IpcHandshakeRequest, String> {
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .await
        .map_err(|err| format!("handshake read failed: {err}"))?;
    if n == 0 {
        return Err("client closed before handshake".to_string());
    }
    serde_json::from_str(line.trim_end()).map_err(|err| format!("handshake parse failed: {err}"))
}

fn build_handshake_response(
    endpoint: &DaemonEndpoint,
    request: &IpcHandshakeRequest,
) -> IpcHandshakeResponse {
    let mut response = IpcHandshakeResponse {
        protocol_version: DAEMON_PROTOCOL_VERSION,
        daemon_version: endpoint.daemon_version.clone(),
        accepted: true,
        rejection_reason: None,
    };
    if request.protocol_version != endpoint.protocol_version {
        response.accepted = false;
        response.rejection_reason = Some("protocol version mismatch".to_string());
        return response;
    }
    if request.auth_token != endpoint.auth_token {
        response.accepted = false;
        response.rejection_reason = Some("auth token mismatch".to_string());
        return response;
    }
    if request.scope != endpoint.scope {
        response.accepted = false;
        response.rejection_reason = Some("scope mismatch".to_string());
        return response;
    }
    response
}

async fn write_json_line<T, W>(writer: &mut W, value: &T) -> Result<(), String>
where
    T: serde::Serialize,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut payload =
        serde_json::to_vec(value).map_err(|err| format!("serialize failed: {err}"))?;
    payload.push(b'\n');
    writer
        .write_all(&payload)
        .await
        .map_err(|err| format!("write failed: {err}"))?;
    Ok(())
}

fn derive_socket_path(endpoint_path: &Path) -> PathBuf {
    endpoint_path.with_extension("sock")
}

fn ensure_socket_parent(socket_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn cleanup_stale_socket(socket_path: &Path) {
    if socket_path.exists() {
        let _ = fs::remove_file(socket_path);
    }
}

fn config_error(message: impl Into<String>) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(message.into()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        path::Path,
        sync::{mpsc, Arc},
        thread,
        time::{Duration, Instant},
    };

    use fs2::FileExt;
    use gwt_core::daemon::{
        ClientFrame, DaemonEndpoint, DaemonFrame, HookEnvelope, IpcHandshakeRequest, RuntimeScope,
        RuntimeTarget, DAEMON_PROTOCOL_VERSION,
    };
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::UnixStream,
    };

    use super::{
        apply_issue_monitor_control, build_handshake_response, decode_issue_monitor_control,
        run_server, run_server_with_shutdown_and_worker_config,
        spawn_issue_monitor_worker_with_config, spawn_issue_monitor_worker_with_config_and_timeout,
        BroadcastHub, DaemonShutdown, IssueMonitorControl,
    };

    fn sample_endpoint(scope: RuntimeScope, socket_path: &Path, token: &str) -> DaemonEndpoint {
        DaemonEndpoint::new(
            scope,
            std::process::id(),
            socket_path.to_string_lossy().to_string(),
            token.to_string(),
            "test-daemon".to_string(),
        )
    }

    fn sample_scope(temp: &TempDir) -> RuntimeScope {
        RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            temp.path().to_path_buf(),
            RuntimeTarget::Host,
        )
        .expect("scope")
    }

    fn init_git_repo(path: &Path) {
        let status = gwt_core::process::hidden_command("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .expect("git init");
        assert!(status.success(), "git init must succeed");
    }

    fn git_remote_add_origin(path: &Path, remote_url: &str) {
        let status = gwt_core::process::hidden_command("git")
            .args(["remote", "add", "origin", remote_url])
            .current_dir(path)
            .status()
            .expect("git remote add origin");
        assert!(status.success(), "git remote add origin must succeed");
    }

    fn commit_initial_branch(path: &Path) {
        let output = gwt_core::process::hidden_command("git")
            .args([
                "-c",
                "user.name=gwt test",
                "-c",
                "user.email=gwt-test@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ])
            .current_dir(path)
            .output()
            .expect("git commit");
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_fake_gh_issue_list(temp_root: &Path) -> std::path::PathBuf {
        let fake_gh = temp_root.join("gh");
        fs::write(
            &fake_gh,
            r#"#!/bin/sh
case "$*" in
  *"--method POST"*|*"--method PATCH"*|*"-X POST"*|*"-X PATCH"*|*"pr merge"*)
    if [ -n "$GWT_FAKE_GH_MUTATION_MARKER" ]; then
      : > "$GWT_FAKE_GH_MUTATION_MARKER"
    fi
    ;;
esac
if [ "$GWT_FAKE_GH_MODE" = "fail" ]; then
  printf '%s\n' 'gh refresh failed' >&2
  exit 1
fi
if [ "$GWT_FAKE_GH_MODE" = "block" ]; then
  printf '%s\n' 'started' >> "$GWT_FAKE_GH_STARTED"
  if [ -n "$GWT_FAKE_GH_PID" ]; then
    printf '%s\n' "$$" > "$GWT_FAKE_GH_PID"
  fi
  if mkdir "$GWT_FAKE_GH_ACTIVE"; then
    owns_active=1
  else
    : > "$GWT_FAKE_GH_OVERLAP"
    owns_active=0
  fi
  while [ ! -f "$GWT_FAKE_GH_RELEASE" ]; do
    sleep 0.05
  done
  if [ "$owns_active" = "1" ]; then
    rmdir "$GWT_FAKE_GH_ACTIVE"
  fi
fi
printf '%s\n' '[{"number":43,"title":"Live issue","body":"Live body","labels":[{"name":"bug"}],"state":"OPEN","url":"https://example.test/issues/43"}]'
exit 0
"#,
        )
        .expect("write fake gh");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755)).expect("chmod fake gh");
        fake_gh
    }

    fn prepend_fake_gh_to_path(fake_gh: &Path) -> ScopedEnvVar {
        let mut paths = vec![fake_gh.parent().expect("fake gh parent").to_path_buf()];
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        ScopedEnvVar::set("PATH", std::env::join_paths(paths).expect("join PATH"))
    }

    async fn wait_for_path(path: &Path, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, async {
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok()
    }

    fn process_exists(pid: u32) -> bool {
        let request = gwt_core::process::ProcessPlanRequest::new("kill")
            .args(["-0", pid.to_string().as_str()]);
        gwt_core::process::resolved_command(request)
            .expect("resolve kill")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    async fn recv_issue_monitor_status_matching(
        receiver: &mut tokio::sync::broadcast::Receiver<DaemonFrame>,
        timeout: Duration,
        predicate: impl Fn(&crate::IssueMonitorStatusView) -> bool,
    ) -> Option<crate::IssueMonitorStatusView> {
        tokio::time::timeout(timeout, async {
            loop {
                let frame = receiver.recv().await.ok()?;
                let DaemonFrame::Event { channel, payload } = frame else {
                    continue;
                };
                if channel != crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL
                    || payload.get("event").and_then(serde_json::Value::as_str) != Some("status")
                {
                    continue;
                }
                let status = serde_json::from_value(payload.get("payload")?.clone()).ok()?;
                if predicate(&status) {
                    return Some(status);
                }
            }
        })
        .await
        .ok()
        .flatten()
    }

    fn legacy_git_failure(project_root: &Path) -> String {
        format!(
            "Current branch is unavailable: Git error: Not a git repository: {}",
            project_root.display()
        )
    }

    fn legacy_failed_prefs(project_root: &Path) -> crate::IssueMonitorPrefs {
        crate::IssueMonitorPrefs {
            enabled: false,
            legacy_git_launch_failure_migration_version: 0,
            failed_issues: vec![crate::IssueMonitorFailedIssue {
                issue_number: 43,
                message: legacy_git_failure(project_root),
                window_id: None,
            }],
            ..crate::IssueMonitorPrefs::default()
        }
    }

    fn write_issue_monitor_prefs_without_lock(path: &Path, prefs: &crate::IssueMonitorPrefs) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create prefs parent");
        }
        fs::write(
            path,
            serde_json::to_vec_pretty(prefs).expect("serialize prefs"),
        )
        .expect("write prefs without lock");
    }

    fn issue_monitor_prefs_lock_for_test(path: &Path) -> std::fs::File {
        let lock_path = path.with_extension("lock");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .expect("open issue monitor prefs lock");
        lock.lock_exclusive()
            .expect("hold issue monitor prefs lock");
        lock
    }

    fn sample_issue_monitor_profile() -> crate::IssueMonitorLaunchProfile {
        crate::IssueMonitorLaunchProfile {
            agent_id: "claude".to_string(),
            model: Some("sonnet".to_string()),
            reasoning: None,
            version: None,
            session_mode: Default::default(),
            skip_permissions: false,
            codex_fast_mode: false,
            runtime_target: Default::default(),
            docker_service: None,
            docker_lifecycle_intent: Default::default(),
            windows_shell: None,
        }
    }

    fn sample_issue_monitor_issue(issue_number: u64) -> crate::IssueMonitorIssue {
        crate::IssueMonitorIssue {
            number: issue_number,
            title: format!("Issue {issue_number}"),
            labels: Vec::new(),
            state: crate::IssueMonitorIssueState::Open,
            body: None,
            url: None,
        }
    }

    #[test]
    fn build_handshake_response_rejects_protocol_version_mismatch() {
        let temp = TempDir::new().unwrap();
        let scope = sample_scope(&temp);
        let endpoint = sample_endpoint(scope.clone(), &temp.path().join("daemon.sock"), "tok");
        let request = IpcHandshakeRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION + 99,
            auth_token: "tok".to_string(),
            scope,
        };
        let response = super::build_handshake_response(&endpoint, &request);
        assert!(!response.accepted);
        assert_eq!(
            response.rejection_reason.as_deref(),
            Some("protocol version mismatch")
        );
    }

    #[test]
    fn build_handshake_response_rejects_auth_token_mismatch() {
        let temp = TempDir::new().unwrap();
        let scope = sample_scope(&temp);
        let endpoint = sample_endpoint(scope.clone(), &temp.path().join("daemon.sock"), "tok");
        let request = IpcHandshakeRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            auth_token: "wrong".to_string(),
            scope,
        };
        let response = build_handshake_response(&endpoint, &request);
        assert!(!response.accepted);
        assert_eq!(
            response.rejection_reason.as_deref(),
            Some("auth token mismatch")
        );
    }

    #[test]
    fn issue_monitor_control_ack_requires_actual_worker_fanout() {
        let hub = BroadcastHub::new();
        let channel = crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL;
        let payload = serde_json::json!({"enabled": true});

        let (response, queued) =
            super::fan_out_issue_monitor_control(&hub, channel, payload.clone());
        assert_eq!(queued, 0);
        assert!(matches!(
            response,
            DaemonFrame::Error { message }
                if message == "issue monitor control worker is not ready"
        ));

        let mut worker_rx = hub.subscribe(channel);
        let (response, queued) =
            super::fan_out_issue_monitor_control(&hub, channel, payload.clone());
        assert_eq!(queued, 1);
        assert_eq!(response, DaemonFrame::Ack);
        assert!(matches!(
            worker_rx.try_recv(),
            Ok(DaemonFrame::Event {
                channel: received_channel,
                payload: received_payload,
            }) if received_channel == channel && received_payload == payload
        ));
    }

    #[test]
    fn build_handshake_response_accepts_matching_request() {
        let temp = TempDir::new().unwrap();
        let scope = sample_scope(&temp);
        let endpoint = sample_endpoint(scope.clone(), &temp.path().join("daemon.sock"), "tok");
        let request = IpcHandshakeRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            auth_token: "tok".to_string(),
            scope,
        };
        let response = build_handshake_response(&endpoint, &request);
        assert!(response.accepted);
        assert!(response.rejection_reason.is_none());
    }

    #[test]
    fn issue_monitor_autonomous_mode_control_toggles_kill_switch() {
        // SPEC #3200 T-046/FR-024: the autonomous_mode control arms/disarms the
        // kill switch, observable in the status view, and requests a rescan.
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            ..crate::IssueMonitorConfig::default()
        });
        assert!(!monitor.autonomous_mode());

        let arm =
            decode_issue_monitor_control(crate::runtime_daemon_events::issue_monitor_payload(
                "control",
                serde_json::json!({ "autonomous_mode": true }),
                std::process::id() + 1,
            ))
            .expect("arm control decodes");
        assert!(
            apply_issue_monitor_control(&mut monitor, arm),
            "rescan requested"
        );
        assert!(monitor.autonomous_mode(), "kill switch armed");
        assert!(monitor.status_view().autonomous_mode);

        let disarm =
            decode_issue_monitor_control(crate::runtime_daemon_events::issue_monitor_payload(
                "control",
                serde_json::json!({ "autonomous_mode": false }),
                std::process::id() + 1,
            ))
            .expect("disarm control decodes");
        apply_issue_monitor_control(&mut monitor, disarm);
        assert!(!monitor.autonomous_mode(), "kill switch disarmed");
    }

    #[test]
    fn issue_monitor_review_verdict_control_records_daemon_judged_outcome() {
        // SPEC #3200 FR-015: a review agent's raw verdict is decoded and judged
        // by the daemon (SHA-bound), setting review_passed on the record.
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        monitor.capture_acceptance_snapshot(
            42,
            crate::issue_monitor_gate::classify_acceptance_criteria(
                "## Acceptance Criteria\n- [ ] AC-1: x\n",
            )
            .snapshot(),
        );
        monitor.begin_review(42, 99, "abc123");

        let verdict = r#"{"schema":"gwt-autonomous-review/v1","overall":"pass","criteria":[{"id":"AC-1","verdict":"pass"}]}"#;
        let payload = crate::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({
                "review_verdict": {
                    "issue_number": 42,
                    "reviewed_sha": "abc123",
                    "verdict_raw": verdict,
                }
            }),
            std::process::id() + 1,
        );
        let control = decode_issue_monitor_control(payload).expect("review verdict decodes");
        apply_issue_monitor_control(&mut monitor, control);

        assert_eq!(
            monitor.autonomous_record(42).and_then(|r| r.review_passed),
            Some(true),
            "daemon judged the verdict pass",
        );
    }

    #[test]
    fn issue_monitor_heartbeat_control_refreshes_liveness() {
        // SPEC #3200 T-045: a heartbeat control refreshes the stuck-detection
        // window for the issue.
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        monitor.set_autonomous_phase(42, crate::AutonomousPhase::Implementing);
        let payload = crate::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({
                "heartbeat": { "issue_number": 42, "at": "2026-06-29T00:05:00Z" }
            }),
            std::process::id() + 1,
        );
        let control = decode_issue_monitor_control(payload).expect("heartbeat decodes");
        apply_issue_monitor_control(&mut monitor, control);
        assert_eq!(
            monitor
                .autonomous_record(42)
                .and_then(|r| r.last_heartbeat.clone())
                .as_deref(),
            Some("2026-06-29T00:05:00Z"),
        );
    }

    #[test]
    fn issue_monitor_launch_failed_control_marks_active_item_failed() {
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            ..crate::IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_claimed(
            crate::IssueMonitorIssue {
                number: 42,
                title: "Issue 42".to_string(),
                labels: Vec::new(),
                state: crate::IssueMonitorIssueState::Open,
                body: None,
                url: None,
            },
            "claim-a",
        );
        monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .expect("launch request");
        let payload = crate::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({
                "launch_failed": {
                    "issue_number": 42,
                    "message": "binary missing",
                }
            }),
            std::process::id() + 1,
        );
        let control = decode_issue_monitor_control(payload).expect("control");

        let should_scan = apply_issue_monitor_control(&mut monitor, control);

        assert!(should_scan);
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(
            monitor.inbox_item(42).expect("inbox item").state,
            crate::MonitorInboxState::LaunchFailed
        );
    }

    #[test]
    fn issue_monitor_launched_control_marks_active_item_launched() {
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            ..crate::IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_claimed(
            crate::IssueMonitorIssue {
                number: 42,
                title: "Issue 42".to_string(),
                labels: Vec::new(),
                state: crate::IssueMonitorIssueState::Open,
                body: None,
                url: None,
            },
            "claim-a",
        );
        monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .expect("launch request");
        let payload = crate::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({
                "launched": {
                    "issue_number": 42,
                    "window_id": "tab-1::agent-1",
                }
            }),
            std::process::id() + 1,
        );
        let control = decode_issue_monitor_control(payload).expect("control");

        let should_scan = apply_issue_monitor_control(&mut monitor, control);

        assert!(should_scan);
        assert_eq!(monitor.status_view().state, "active");
        assert_eq!(monitor.active_count(), 1);
        let item = monitor.inbox_item(42).expect("inbox item");
        assert_eq!(item.state, crate::MonitorInboxState::Launched);
        assert_eq!(item.launched_window_id.as_deref(), Some("tab-1::agent-1"));
    }

    #[test]
    fn issue_monitor_agent_failed_control_marks_launched_item_failed() {
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            ..crate::IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_claimed(
            crate::IssueMonitorIssue {
                number: 42,
                title: "Issue 42".to_string(),
                labels: Vec::new(),
                state: crate::IssueMonitorIssueState::Open,
                body: None,
                url: None,
            },
            "claim-a",
        );
        monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .expect("launch request");
        monitor.complete_active_launch(42, "tab-1::agent-1");
        let payload = crate::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({
                "agent_failed": {
                    "window_id": "tab-1::agent-1",
                    "message": "Stop-block hit an error",
                }
            }),
            std::process::id() + 1,
        );
        let control = decode_issue_monitor_control(payload).expect("control");

        let should_scan = apply_issue_monitor_control(&mut monitor, control);

        assert!(should_scan);
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(monitor.status_view().state, "error");
        assert_eq!(
            monitor.status_view().last_error.as_deref(),
            Some("issue #42: Stop-block hit an error")
        );
        let item = monitor.inbox_item(42).expect("inbox item");
        assert_eq!(item.state, crate::MonitorInboxState::AgentFailed);
        assert_eq!(item.launched_window_id, None);
        assert_eq!(
            item.error_message.as_deref(),
            Some("Stop-block hit an error")
        );
    }

    #[test]
    fn issue_monitor_launch_failed_control_routes_inflight_autonomous_issue_through_retry() {
        // SPEC #3200 (review follow-up): when the independent review agent fails
        // to spawn, the daemon receives a `launch_failed` control. For an
        // in-flight autonomous issue this must route through the autonomous
        // retry machinery (attempt counted, re-queued) instead of marking the
        // inbox `LaunchFailed` and stranding the record in `Reviewing` forever.
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            ..crate::IssueMonitorConfig::default()
        });
        monitor.set_autonomous_mode(true);
        monitor.set_gui_connected(true);
        monitor.record_claimed(
            crate::IssueMonitorIssue {
                number: 42,
                title: "Issue 42".to_string(),
                labels: Vec::new(),
                state: crate::IssueMonitorIssueState::Open,
                body: None,
                url: None,
            },
            "claim-a",
        );
        monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .expect("launch request");
        monitor.complete_active_launch(42, "tab-1::agent-1");
        monitor.set_autonomous_phase(42, crate::AutonomousPhase::Implementing);
        monitor.begin_review(42, 99, "abc123"); // Implementing → Reviewing
        assert!(monitor.is_autonomous_in_flight(42));

        let payload = crate::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({
                "launch_failed": {
                    "issue_number": 42,
                    "message": "Independent review could not start",
                }
            }),
            std::process::id() + 1,
        );
        let control = decode_issue_monitor_control(payload).expect("control");

        let should_scan = apply_issue_monitor_control(&mut monitor, control);

        assert!(should_scan);
        assert_eq!(
            monitor.autonomous_record(42).map(|r| r.phase),
            Some(crate::AutonomousPhase::Idle),
            "routed back to Idle for retry, not stranded in Reviewing"
        );
        assert_eq!(
            monitor.attempt_count(42),
            1,
            "the failed attempt is counted"
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(crate::MonitorInboxState::Queued),
            "re-queued for automatic relaunch"
        );
    }

    #[test]
    fn issue_monitor_agent_failed_control_uses_issue_number_hint_when_window_is_unmapped() {
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            ..crate::IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_claimed(
            crate::IssueMonitorIssue {
                number: 42,
                title: "Issue 42".to_string(),
                labels: Vec::new(),
                state: crate::IssueMonitorIssueState::Open,
                body: None,
                url: None,
            },
            "claim-a",
        );
        monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .expect("launch request");
        let payload = crate::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({
                "agent_failed": {
                    "issue_number": 42,
                    "window_id": "unmapped-agent-window",
                    "message": "Stop-block hit an error",
                }
            }),
            std::process::id() + 1,
        );
        let control = decode_issue_monitor_control(payload).expect("control");

        let should_scan = apply_issue_monitor_control(&mut monitor, control);

        assert!(should_scan);
        assert_eq!(monitor.active_count(), 0);
        let item = monitor.inbox_item(42).expect("inbox item");
        assert_eq!(item.state, crate::MonitorInboxState::AgentFailed);
        assert_eq!(
            item.error_message.as_deref(),
            Some("Stop-block hit an error")
        );
    }

    #[test]
    fn issue_monitor_runtime_controls_request_immediate_scan_when_launch_order_changes() {
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            max_active: 1,
            ..crate::IssueMonitorConfig::default()
        });

        let should_scan =
            apply_issue_monitor_control(&mut monitor, IssueMonitorControl::MaxActiveAgents(5));
        assert!(should_scan);
        assert_eq!(monitor.status_view().max_active_agents, 5);

        let should_scan = apply_issue_monitor_control(
            &mut monitor,
            IssueMonitorControl::PriorityOrder(vec![43, 42]),
        );
        assert!(should_scan);
    }

    #[test]
    fn issue_monitor_control_adopts_newer_disk_migration_before_same_failure_mutation() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default())
            .expect("seed GUI migration result");
        let failure = legacy_git_failure(temp.path());
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            legacy_failed_prefs(temp.path()),
        );

        let should_scan = super::apply_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut monitor,
            IssueMonitorControl::LaunchFailed {
                issue_number: 43,
                message: failure.clone(),
            },
        );

        assert!(should_scan);
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert_eq!(
            persisted.legacy_git_launch_failure_migration_version,
            crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
        );
        assert_eq!(persisted.failed_issues.len(), 1);
        assert_eq!(
            persisted.failed_issues[0].message, failure,
            "the equal marker must preserve the new post-migration failure"
        );
    }

    #[test]
    fn issue_monitor_scan_reports_missing_origin_instead_of_generic_unavailable() {
        let temp = TempDir::new().expect("tempdir");
        init_git_repo(temp.path());
        let scope = sample_scope(&temp);
        let monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());

        let monitor = super::scan_issue_monitor_once_blocking(scope, monitor, false);

        let error = monitor
            .status_view()
            .last_error
            .expect("origin resolution error");
        assert!(
            error.starts_with("Git origin remote is not configured"),
            "unexpected error: {error}"
        );
        assert_ne!(error, "GitHub origin remote is unavailable");
    }

    #[test]
    fn issue_monitor_scan_reports_non_github_origin_instead_of_generic_unavailable() {
        let temp = TempDir::new().expect("tempdir");
        init_git_repo(temp.path());
        git_remote_add_origin(temp.path(), "https://example.com/owner/repo.git");
        let scope = sample_scope(&temp);
        let monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());

        let monitor = super::scan_issue_monitor_once_blocking(scope, monitor, false);

        let error = monitor
            .status_view()
            .last_error
            .expect("origin resolution error");
        assert_eq!(
            error,
            "Git origin remote is not a GitHub URL: https://example.com/owner/repo.git"
        );
    }

    #[test]
    fn daemon_live_scan_migrates_and_atomically_persists_legacy_failure() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create gwt home");
        let _home = ScopedGwtHome::set(&home);
        let fake_gh = write_fake_gh_issue_list(temp.path());
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        commit_initial_branch(&repo);
        git_remote_add_origin(&repo, "https://github.com/example/repo.git");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo.clone(),
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let legacy = legacy_failed_prefs(&scope.project_root);
        crate::save_issue_monitor_prefs(&prefs_path, &legacy).expect("seed legacy prefs");
        let monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), legacy);

        let mut monitor = super::scan_issue_monitor_once_blocking(scope, monitor, false);
        super::persist_daemon_issue_monitor_state(&prefs_path, &mut monitor);

        assert_eq!(
            monitor.prefs().legacy_git_launch_failure_migration_version,
            crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
        );
        assert_eq!(
            monitor.inbox_item(43).map(|item| item.state),
            Some(crate::MonitorInboxState::Queued)
        );
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert_eq!(
            persisted.legacy_git_launch_failure_migration_version,
            crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
        );
        assert!(persisted.failed_issues.is_empty());
        let scratch_prefix = format!(
            ".{}.tmp-",
            prefs_path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("prefs filename")
        );
        assert!(
            fs::read_dir(prefs_path.parent().expect("prefs parent"))
                .expect("read prefs parent")
                .flatten()
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&scratch_prefix)),
            "the atomic save must leave no scratch file"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // global fake-gh env must stay isolated for the full worker run
    async fn issue_monitor_worker_applies_control_during_scan_without_rewinding_mutation() {
        // SPEC #3200 T-127/T-128 (FR-040/FR-041): a blocking external scan must
        // not stall the driver's control plane, and its stale result must not
        // overwrite a control mutation that committed while the scan ran.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create gwt home");
        let _home = ScopedGwtHome::set(&home);
        let fake_gh = write_fake_gh_issue_list(temp.path());
        let scan_started_path = temp.path().join("scan-started");
        let release_scan_path = temp.path().join("release-scan");
        let active_scan_path = temp.path().join("active-scan");
        let overlap_scan_path = temp.path().join("overlap-scan");
        let mutation_marker_path = temp.path().join("remote-mutation");
        let http_client_marker_path = temp.path().join("claim-http-client");
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "block");
        let _started = ScopedEnvVar::set("GWT_FAKE_GH_STARTED", &scan_started_path);
        let _release = ScopedEnvVar::set("GWT_FAKE_GH_RELEASE", &release_scan_path);
        let _active = ScopedEnvVar::set("GWT_FAKE_GH_ACTIVE", &active_scan_path);
        let _overlap = ScopedEnvVar::set("GWT_FAKE_GH_OVERLAP", &overlap_scan_path);
        let _mutation = ScopedEnvVar::set("GWT_FAKE_GH_MUTATION_MARKER", &mutation_marker_path);
        let _http_client = ScopedEnvVar::set(
            "GWT_TEST_ISSUE_MONITOR_HTTP_CLIENT_MARKER",
            &http_client_marker_path,
        );

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        commit_initial_branch(&repo);
        git_remote_add_origin(&repo, "https://github.com/example/repo.git");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let original_heartbeat = "2099-01-01T00:00:00Z";
        let control_heartbeat = "2099-01-02T00:00:00Z";
        let launch_config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("work/issue-43")
            .build();
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                launch_profile: Some(crate::IssueMonitorLaunchProfile::from(&launch_config)),
                autonomous_records: vec![crate::AutonomousIssueRecord {
                    issue_number: 43,
                    phase: crate::AutonomousPhase::Implementing,
                    active_launch_id: None,
                    attempts: 1,
                    acceptance_snapshot: None,
                    retry_not_before: None,
                    last_heartbeat: Some(original_heartbeat.to_string()),
                    pr_number: None,
                    reviewed_sha: None,
                    review_passed: None,
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed issue monitor prefs");

        let hub = BroadcastHub::new();
        let mut status_rx = hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL);
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = spawn_issue_monitor_worker_with_config(
            scope,
            hub.clone(),
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 1,
                ..crate::IssueMonitorConfig::default()
            },
        );

        let scan_started = wait_for_path(&scan_started_path, Duration::from_secs(2)).await;
        let source_pid = std::process::id().wrapping_add(1);
        let heartbeat_queued = hub.publish(
            crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL,
            DaemonFrame::Event {
                channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
                payload: crate::runtime_daemon_events::issue_monitor_payload(
                    "control",
                    serde_json::json!({
                        "heartbeat": {
                            "issue_number": 43,
                            "at": control_heartbeat,
                        }
                    }),
                    source_pid,
                ),
            },
        );
        let max_active_queued = hub.publish(
            crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL,
            DaemonFrame::Event {
                channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
                payload: crate::runtime_daemon_events::issue_monitor_payload(
                    "control",
                    serde_json::json!({"max_active_agents": 7}),
                    source_pid,
                ),
            },
        );

        let responsive_status = recv_issue_monitor_status_matching(
            &mut status_rx,
            Duration::from_millis(500),
            |status| status.max_active_agents == 7,
        )
        .await;
        let tick_status = recv_issue_monitor_status_matching(
            &mut status_rx,
            Duration::from_millis(1_500),
            |status| status.max_active_agents == 7,
        )
        .await;
        let scans_started_while_blocked = fs::read_to_string(&scan_started_path)
            .unwrap_or_default()
            .lines()
            .count();
        let scan_overlap_while_blocked = overlap_scan_path.exists();
        let disabled_queued = hub.publish(
            crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL,
            DaemonFrame::Event {
                channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
                payload: crate::runtime_daemon_events::issue_monitor_payload(
                    "control",
                    serde_json::json!({"enabled": false}),
                    source_pid,
                ),
            },
        );
        let disabled_status = recv_issue_monitor_status_matching(
            &mut status_rx,
            Duration::from_millis(500),
            |status| !status.enabled,
        )
        .await;

        // Always release the fake process before asserting RED so a failed test
        // cannot strand a blocking child or the Tokio blocking pool.
        fs::write(&release_scan_path, b"release").expect("release fake gh scan");
        let settled_status =
            recv_issue_monitor_status_matching(&mut status_rx, Duration::from_secs(3), |status| {
                status.max_active_agents == 7 && status.last_scan_at.is_some()
            })
            .await;
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        let persisted_heartbeat = persisted
            .autonomous_records
            .iter()
            .find(|record| record.issue_number == 43)
            .and_then(|record| record.last_heartbeat.as_deref())
            .map(str::to_string);
        shutdown.request();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .expect("worker shutdown is bounded")
            .expect("worker exits cleanly");

        assert!(scan_started, "fake gh scan must be in flight");
        assert_eq!(heartbeat_queued, 1, "worker must subscribe to controls");
        assert_eq!(max_active_queued, 1, "worker must subscribe to controls");
        assert_eq!(disabled_queued, 1, "OFF control must reach the worker");
        assert!(
            responsive_status.is_some(),
            "control status must publish before the blocked fake gh scan is released"
        );
        assert!(
            tick_status.is_some(),
            "an in-flight tick must re-publish current canonical status"
        );
        assert!(
            disabled_status.is_some(),
            "authority revocation must publish before the stale scan is released"
        );
        assert_eq!(
            scans_started_while_blocked, 1,
            "in-flight ticks must coalesce without spawning a second scan"
        );
        assert!(
            !scan_overlap_while_blocked,
            "should-scan controls and ticks must not overlap fake gh scans"
        );
        assert!(settled_status.is_some(), "released scan must settle");
        assert!(
            !mutation_marker_path.exists(),
            "a stale scan must cause zero external claim/arm mutations after OFF"
        );
        assert!(
            !http_client_marker_path.exists(),
            "a proposal-capable stale scan must never reach the claim HTTP adapter after OFF"
        );
        assert!(persisted.pending_effects.is_empty());
        assert!(!persisted.enabled);
        assert_eq!(
            persisted_heartbeat.as_deref(),
            Some(control_heartbeat),
            "the stale scan result must not rewind the in-flight control mutation"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn production_server_shutdown_waits_for_authority_revocation_and_reaps_child() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create gwt home");
        let _home = ScopedGwtHome::set(&home);
        let fake_gh = write_fake_gh_issue_list(temp.path());
        let started_path = temp.path().join("scan-started");
        let release_path = temp.path().join("never-release");
        let active_path = temp.path().join("active-scan");
        let overlap_path = temp.path().join("overlap-scan");
        let pid_path = temp.path().join("gh.pid");
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "block");
        let _started = ScopedEnvVar::set("GWT_FAKE_GH_STARTED", &started_path);
        let _release = ScopedEnvVar::set("GWT_FAKE_GH_RELEASE", &release_path);
        let _active = ScopedEnvVar::set("GWT_FAKE_GH_ACTIVE", &active_path);
        let _overlap = ScopedEnvVar::set("GWT_FAKE_GH_OVERLAP", &overlap_path);
        let _pid = ScopedEnvVar::set("GWT_FAKE_GH_PID", &pid_path);

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        commit_initial_branch(&repo);
        git_remote_add_origin(&repo, "https://github.com/example/repo.git");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                effect_authority_epoch: 7,
                pending_effects: vec![crate::PendingIssueMonitorEffect {
                    effect_id: "arm:42:99:abc:7".to_string(),
                    authority_epoch: 7,
                    attempt: 1,
                    state: crate::IssueMonitorEffectState::Attempting,
                    payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                        issue_number: 42,
                        pr_number: 99,
                        reviewed_sha: "abc".to_string(),
                    },
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let shutdown = Arc::new(DaemonShutdown::new());
        let socket_path = temp.path().join("shutdown-daemon.sock");
        let endpoint_path = temp.path().join("shutdown-endpoint.json");
        let endpoint = sample_endpoint(scope, &socket_path, "shutdown-secret");
        let server = tokio::spawn(run_server_with_shutdown_and_worker_config(
            endpoint,
            socket_path,
            endpoint_path,
            BroadcastHub::new(),
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 1,
                ..crate::IssueMonitorConfig::default()
            },
            Duration::from_secs(1),
        ));

        assert!(wait_for_path(&started_path, Duration::from_secs(2)).await);
        assert!(wait_for_path(&pid_path, Duration::from_secs(1)).await);
        let pid = fs::read_to_string(&pid_path)
            .expect("read fake gh pid")
            .trim()
            .parse::<u32>()
            .expect("parse fake gh pid");
        assert!(process_exists(pid), "fake gh must be alive before shutdown");

        let shutdown_started = Instant::now();
        shutdown.request();
        let exit_code = tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("server shutdown is bounded")
            .expect("server task exits cleanly")
            .expect("server exits successfully");
        assert_eq!(exit_code, 0);

        let reaped = tokio::time::timeout(Duration::from_secs(1), async {
            while process_exists(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok();
        assert!(reaped, "deadline owner must terminate and reap fake gh");
        assert!(
            shutdown_started.elapsed() < Duration::from_secs(3),
            "shutdown exceeded its absolute operation deadline"
        );
        let persisted =
            crate::load_issue_monitor_prefs(&prefs_path).expect("reload shutdown prefs");
        assert_eq!(persisted.effect_authority_epoch, 8);
        assert!(
            persisted.pending_effects.iter().any(|effect| matches!(
                &effect.payload,
                crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                    issue_number: 42,
                    pr_number: 99,
                    compensates_effect_id,
                } if compensates_effect_id == "arm:42:99:abc:7"
            )),
            "shutdown must durably retain auto-merge compensation"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // serializes process-wide fake-gh environment
    async fn worker_off_during_blocked_arm_serializes_compensating_disarm() {
        // SPEC #3200 Phase 7 Scenarios 30-31: OFF commits while an already
        // started arm command is blocked. The old command may still succeed,
        // but its exact result cannot enter delivery; one durable disarm must
        // run afterward on the same single-flight effect lane.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create gwt home");
        let _home = ScopedGwtHome::set(&home);
        let fake_gh = temp.path().join("gh");
        fs::write(
            &fake_gh,
            r#"#!/bin/sh
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  printf '%s\n' '[]'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  if [ -f "$GWT_FAKE_ARM_DONE" ]; then
    printf '%s\n' '{"state":"OPEN","headRefOid":"abc","autoMergeRequest":{"enabledAt":"2026-07-27T00:00:00Z"},"mergeCommit":null}'
  else
    printf '%s\n' '{"state":"OPEN","headRefOid":"abc","autoMergeRequest":null,"mergeCommit":null}'
  fi
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ] && [ "$4" = "--auto" ]; then
  printf '%s\n' 'arm' >> "$GWT_FAKE_EFFECT_CALLS"
  : > "$GWT_FAKE_ARM_STARTED"
  while [ ! -f "$GWT_FAKE_ARM_RELEASE" ]; do
    sleep 0.02
  done
  : > "$GWT_FAKE_ARM_DONE"
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ] && [ "$4" = "--disable-auto" ]; then
  printf '%s\n' 'disarm' >> "$GWT_FAKE_EFFECT_CALLS"
  : > "$GWT_FAKE_DISARM_DONE"
  exit 0
fi
exit 1
"#,
        )
        .expect("write fake gh");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755)).expect("chmod fake gh");
        let arm_started = temp.path().join("arm-started");
        let arm_release = temp.path().join("arm-release");
        let arm_done = temp.path().join("arm-done");
        let disarm_done = temp.path().join("disarm-done");
        let effect_calls = temp.path().join("effect-calls");
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _arm_started = ScopedEnvVar::set("GWT_FAKE_ARM_STARTED", &arm_started);
        let _arm_release = ScopedEnvVar::set("GWT_FAKE_ARM_RELEASE", &arm_release);
        let _arm_done = ScopedEnvVar::set("GWT_FAKE_ARM_DONE", &arm_done);
        let _disarm_done = ScopedEnvVar::set("GWT_FAKE_DISARM_DONE", &disarm_done);
        let _effect_calls = ScopedEnvVar::set("GWT_FAKE_EFFECT_CALLS", &effect_calls);

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        commit_initial_branch(&repo);
        git_remote_add_origin(&repo, "https://github.com/example/repo.git");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let arm = crate::PendingIssueMonitorEffect {
            effect_id: "arm:42:99:abc:7".to_string(),
            authority_epoch: 7,
            attempt: 1,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                reviewed_sha: "abc".to_string(),
            },
        };
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                effect_authority_epoch: 7,
                pending_effects: vec![arm],
                autonomous_records: vec![crate::AutonomousIssueRecord {
                    issue_number: 42,
                    phase: crate::AutonomousPhase::Reviewing,
                    active_launch_id: None,
                    attempts: 1,
                    acceptance_snapshot: None,
                    retry_not_before: None,
                    last_heartbeat: Some("2026-07-27T00:00:00Z".to_string()),
                    pr_number: Some(99),
                    reviewed_sha: Some("abc".to_string()),
                    review_passed: Some(true),
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed attempting arm");

        let hub = BroadcastHub::new();
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = spawn_issue_monitor_worker_with_config_and_timeout(
            scope,
            hub.clone(),
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 60,
                ..crate::IssueMonitorConfig::default()
            },
            Duration::from_secs(3),
        );
        assert!(
            wait_for_path(&arm_started, Duration::from_secs(2)).await,
            "worker must start the durable Attempting arm"
        );

        let source_pid = std::process::id().wrapping_add(1);
        assert_eq!(
            hub.publish(
                crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL,
                DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload: crate::runtime_daemon_events::issue_monitor_payload(
                        "control",
                        serde_json::json!({"autonomous_mode": false}),
                        source_pid,
                    ),
                },
            ),
            1
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load OFF prefs");
                if prefs.effect_authority_epoch == 8
                    && prefs.pending_effects.iter().any(|effect| {
                        matches!(
                            &effect.payload,
                            crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                                compensates_effect_id,
                                ..
                            } if compensates_effect_id == "arm:42:99:abc:7"
                        )
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("OFF must durably queue compensation before arm returns");

        fs::write(&arm_release, b"release").expect("release arm command");
        assert!(
            wait_for_path(&disarm_done, Duration::from_secs(2)).await,
            "compensating disarm must run after the stale arm result"
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let prefs =
                    crate::load_issue_monitor_prefs(&prefs_path).expect("load settled prefs");
                if prefs.pending_effects.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("disarm result must settle exact journal tuple");
        let settled = crate::load_issue_monitor_prefs(&prefs_path).expect("reload settled prefs");
        let calls = fs::read_to_string(&effect_calls).expect("read effect calls");
        assert_eq!(calls.lines().collect::<Vec<_>>(), vec!["arm", "disarm"]);
        assert!(!settled.autonomous_mode);
        assert_eq!(settled.effect_authority_epoch, 8);
        assert_eq!(
            settled
                .autonomous_records
                .iter()
                .find(|record| record.issue_number == 42)
                .map(|record| record.phase),
            Some(crate::AutonomousPhase::NeedsHuman)
        );

        shutdown.request();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .expect("worker shutdown is bounded")
            .expect("worker exits cleanly");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // holds the deliberate cross-process lock across worker await
    async fn worker_never_mutates_remote_when_attempt_fence_cannot_commit() {
        // T-135: only a durably fenced Attempting tuple grants execution.
        // Keeping the prefs lock busy forces the fence transaction to fail;
        // the real worker must exit without ever invoking `gh pr merge`.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create gwt home");
        let _home = ScopedGwtHome::set(&home);
        let fake_gh = temp.path().join("gh");
        fs::write(
            &fake_gh,
            r#"#!/bin/sh
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  printf '%s\n' '[]'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  printf '%s\n' '{"state":"OPEN","headRefOid":"abc","autoMergeRequest":null,"mergeCommit":null}'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
  : > "$GWT_FAKE_GH_MUTATION_MARKER"
  exit 0
fi
exit 1
"#,
        )
        .expect("write armable fake gh");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755)).expect("chmod fake gh");
        let mutation_marker = temp.path().join("unexpected-remote-mutation");
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _mutation = ScopedEnvVar::set("GWT_FAKE_GH_MUTATION_MARKER", &mutation_marker);

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        commit_initial_branch(&repo);
        git_remote_add_origin(&repo, "https://github.com/example/repo.git");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let prepared = crate::PendingIssueMonitorEffect::prepared(
            "arm:42:99:abc:7",
            7,
            crate::IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                reviewed_sha: "abc".to_string(),
            },
        );
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                effect_authority_epoch: 7,
                pending_effects: vec![prepared.clone()],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed Prepared arm");
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let hub = BroadcastHub::new();
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = spawn_issue_monitor_worker_with_config_and_timeout(
            scope,
            hub,
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 60,
                ..crate::IssueMonitorConfig::default()
            },
            Duration::from_secs(1),
        );

        tokio::time::sleep(Duration::from_millis(350)).await;
        shutdown.request();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .expect("worker shutdown is bounded despite the lock")
            .expect("worker exits cleanly");
        FileExt::unlock(&lock).expect("release prefs lock");

        assert!(
            !mutation_marker.exists(),
            "failed fence must cause zero remote mutations"
        );
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert_eq!(persisted.effect_authority_epoch, 7);
        assert_eq!(persisted.pending_effects, vec![prepared]);
    }

    #[tokio::test]
    async fn daemon_shutdown_request_is_sticky_before_worker_wait_registration() {
        let shutdown = DaemonShutdown::new();
        shutdown.request();

        tokio::time::timeout(Duration::from_millis(50), shutdown.notified())
            .await
            .expect("a pre-registered shutdown request must remain observable");
    }

    #[tokio::test]
    async fn outer_scan_watchdog_keeps_started_blocking_task_as_single_flight_owner() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let initial = crate::IssueMonitorPrefs {
            effect_authority_epoch: 11,
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed prefs");
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut late = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            initial.clone(),
        );
        late.set_max_active_agents(9);
        let handle = tokio::task::spawn_blocking(move || {
            started_tx.send(()).expect("signal started scan");
            release_rx.recv().expect("release scan");
            late
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("scan entered blocking pool");
        let deadline = Instant::now() + Duration::from_millis(25);
        let mut in_flight = Some(super::InFlightIssueMonitorScan {
            revision: 7,
            authority_epoch: 11,
            handle,
            deadline,
            watchdog_fired: false,
        });
        let mut canonical =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), initial);

        super::wait_for_issue_monitor_deadline(Some(deadline)).await;
        assert!(super::expire_issue_monitor_scan_at_watchdog(
            &mut in_flight,
            &mut canonical,
            "2026-07-27T00:00:00Z".to_string(),
        ));

        let retained = in_flight.as_ref().expect("single-flight handle retained");
        assert!(retained.watchdog_fired);
        assert!(!retained.handle.is_finished());
        assert!(canonical
            .status_view()
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("outer watchdog stage")));

        release_tx.send(()).expect("release scan");
        let (captured_revision, captured_epoch, result) = tokio::time::timeout(
            Duration::from_secs(1),
            super::wait_for_issue_monitor_scan(&mut in_flight),
        )
        .await
        .expect("started scan eventually joins");
        assert_eq!(captured_revision, 7);
        assert_eq!(captured_epoch, 11);
        let late_result = result.expect("late scan result remains inspectable");
        assert_eq!(late_result.status_view().max_active_agents, 9);
        let revision_after_watchdog = 8;
        assert!(!super::accept_completed_issue_monitor_scan(
            &prefs_path,
            &mut canonical,
            late_result,
            captured_revision,
            revision_after_watchdog,
            captured_epoch,
        ));
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("late result not persisted")
                .max_active_agents,
            1
        );
        assert!(canonical
            .status_view()
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("outer watchdog stage")));

        let mut coalesced_retry = canonical.clone();
        crate::scan_issue_monitor_candidates(
            &mut coalesced_retry,
            &[sample_issue_monitor_issue(42)],
            "2026-07-27T00:00:01Z",
        );
        assert!(super::accept_completed_issue_monitor_scan(
            &prefs_path,
            &mut canonical,
            coalesced_retry,
            revision_after_watchdog,
            revision_after_watchdog,
            11,
        ));
        assert_eq!(
            canonical.status_view().last_scan_at.as_deref(),
            Some("2026-07-27T00:00:01Z")
        );
        assert_eq!(canonical.status_view().last_error, None);
    }

    #[tokio::test]
    async fn outer_effect_watchdog_retains_exact_attempt_until_started_executor_joins() {
        let effect = crate::PendingIssueMonitorEffect {
            effect_id: "arm:42:99:abc:7".to_string(),
            authority_epoch: 7,
            attempt: 1,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                reviewed_sha: "abc".to_string(),
            },
        };
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                effect_authority_epoch: 7,
                pending_effects: vec![effect.clone()],
                ..crate::IssueMonitorPrefs::default()
            },
        );
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let completed_effect = effect.clone();
        let handle = tokio::task::spawn_blocking(move || {
            started_tx.send(()).expect("signal started effect");
            release_rx.recv().expect("release effect");
            super::CompletedIssueMonitorEffect {
                effect: completed_effect,
                outcome: super::IssueMonitorEffectOutcome::AutoMerge(
                    gwt_git::pr_status::AutoMergeMutationOutcome::PreSubmit(
                        "not submitted".to_string(),
                    ),
                ),
                completed_at: "2026-07-27T00:00:00Z".to_string(),
            }
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("effect entered blocking pool");
        let deadline = Instant::now() + Duration::from_millis(25);
        let mut in_flight = Some(super::InFlightIssueMonitorEffect {
            handle,
            deadline,
            watchdog_fired: false,
        });

        super::wait_for_issue_monitor_deadline(Some(deadline)).await;
        assert!(super::expire_issue_monitor_effect_at_watchdog(
            &mut in_flight,
            &mut monitor,
        ));

        let retained = in_flight.as_ref().expect("effect lane remains occupied");
        assert!(retained.watchdog_fired);
        assert!(!retained.handle.is_finished());
        assert_eq!(monitor.pending_effects(), std::slice::from_ref(&effect));
        assert!(monitor
            .status_view()
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("remote outcome unknown")));

        release_tx.send(()).expect("release effect");
        let completed = tokio::time::timeout(
            Duration::from_secs(1),
            super::wait_for_issue_monitor_effect(&mut in_flight),
        )
        .await
        .expect("started effect eventually joins")
        .expect("effect task joins cleanly");
        assert_eq!(completed.effect.effect_id, effect.effect_id);
    }

    #[test]
    fn queued_effect_is_cancelled_before_blocking_pool_start() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .expect("runtime");
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");

        runtime.block_on(async move {
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                started_tx.send(()).expect("signal blocker");
                release_rx.recv().expect("release blocker");
            });
            started_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("blocking pool occupied");
            let effect = crate::PendingIssueMonitorEffect::prepared(
                "release:claim:42:7:8",
                8,
                crate::IssueMonitorEffectPayload::ReleaseClaim {
                    issue_number: 42,
                    claim_id: "stable-claim-42".to_string(),
                },
            );
            let mut queued = super::spawn_issue_monitor_effect(
                scope,
                effect,
                true,
                Instant::now() + Duration::from_secs(1),
            );

            queued.handle.abort();
            release_tx.send(()).expect("release blocker");
            blocker.await.expect("blocker exits");
            let error = (&mut queued.handle)
                .await
                .expect_err("queued remote effect must never start after abort");

            assert!(error.is_cancelled());
        });
    }

    #[test]
    fn stale_scan_cannot_commit_a_prepared_arm_after_authority_is_revoked() {
        // SPEC #3200 Phase 7 T-134/FR-041/FR-044: a scan may only propose an
        // external mutation. If OFF commits while that scan is in flight, the
        // old-epoch proposal must never enter the durable execution journal.
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let initial = crate::IssueMonitorPrefs {
            autonomous_mode: true,
            effect_authority_epoch: 7,
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed prefs");
        let mut canonical =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), initial);
        let mut stale_scan = canonical.clone();
        stale_scan.prepare_effect(crate::PendingIssueMonitorEffect {
            effect_id: "arm:42:99:abc123:7".to_string(),
            authority_epoch: 7,
            attempt: 0,
            state: crate::IssueMonitorEffectState::Prepared,
            payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                reviewed_sha: "abc123".to_string(),
            },
        });

        assert!(super::apply_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut canonical,
            IssueMonitorControl::AutonomousMode(false),
        ));
        assert!(!super::commit_issue_monitor_scan_if_current(
            &prefs_path,
            &mut canonical,
            stale_scan,
            7,
        ));

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert_eq!(persisted.effect_authority_epoch, 8);
        assert!(persisted.pending_effects.is_empty());
        assert!(!persisted.autonomous_mode);
        assert_eq!(canonical.effect_authority_epoch(), 8);
        assert!(!canonical.autonomous_mode());
    }

    #[test]
    fn scan_commit_adopts_newer_disk_authority_before_retrying() {
        // A launch-profile save is intentionally a direct prefs transaction.
        // The daemon must absorb its newer authority generation after rejecting
        // the stale scan; otherwise every retry captures the same old epoch and
        // is rejected forever.
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let initial = crate::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 7,
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed prefs");
        let mut canonical = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            initial.clone(),
        );
        let stale_scan = canonical.clone();

        let profile = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("work/issue-3349")
            .build();
        crate::mutate_issue_monitor_prefs(&prefs_path, |disk| {
            disk.advance_effect_authority_epoch()
                .expect("advance direct-writer authority");
            disk.launch_profile = Some(crate::IssueMonitorLaunchProfile::from(&profile));
        })
        .expect("save launch profile externally");

        assert!(!super::commit_issue_monitor_scan_if_current(
            &prefs_path,
            &mut canonical,
            stale_scan,
            7,
        ));
        assert_eq!(canonical.effect_authority_epoch(), 8);
        assert!(canonical.has_launch_profile());

        let retry = canonical.clone();
        assert!(super::commit_issue_monitor_scan_if_current(
            &prefs_path,
            &mut canonical,
            retry,
            8,
        ));
        assert_eq!(canonical.effect_authority_epoch(), 8);
        assert!(canonical.has_launch_profile());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn malformed_known_attempting_effect_blocks_real_worker_without_overwriting_journal() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create gwt home");
        let _home = ScopedGwtHome::set(&home);
        let fake_gh = write_fake_gh_issue_list(temp.path());
        let scan_started = temp.path().join("unexpected-scan");
        let release_scan = temp.path().join("never-release");
        let active_scan = temp.path().join("unexpected-active");
        let overlap_scan = temp.path().join("unexpected-overlap");
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "block");
        let _started = ScopedEnvVar::set("GWT_FAKE_GH_STARTED", &scan_started);
        let _release = ScopedEnvVar::set("GWT_FAKE_GH_RELEASE", &release_scan);
        let _active = ScopedEnvVar::set("GWT_FAKE_GH_ACTIVE", &active_scan);
        let _overlap = ScopedEnvVar::set("GWT_FAKE_GH_OVERLAP", &overlap_scan);

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        commit_initial_branch(&repo);
        git_remote_add_origin(&repo, "https://github.com/example/repo.git");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        fs::create_dir_all(prefs_path.parent().expect("prefs parent"))
            .expect("create prefs parent");
        let malformed = r#"{
            "enabled": true,
            "max_active_agents": 1,
            "priority_order": [],
            "autonomous_mode": true,
            "effect_authority_epoch": 7,
            "pending_effects": [{
                "effect_id": "arm:42:99:abc:7",
                "authority_epoch": 7,
                "attempt": 1,
                "state": "attempting",
                "payload": {
                    "kind": "arm_auto_merge",
                    "issue_number": 42,
                    "pr_number": 99
                }
            }]
        }"#;
        fs::write(&prefs_path, malformed).expect("seed malformed known journal");

        let hub = BroadcastHub::new();
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = spawn_issue_monitor_worker_with_config_and_timeout(
            scope,
            hub.clone(),
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 1,
                ..crate::IssueMonitorConfig::default()
            },
            Duration::from_millis(100),
        );

        // Subscribe after the startup publish to prove the recovery error is
        // re-projected for operators that connect later.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut status_rx = hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL);
        let status =
            recv_issue_monitor_status_matching(&mut status_rx, Duration::from_secs(2), |status| {
                status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("recovery journal preserved"))
            })
            .await
            .expect("recovery-blocked status");
        assert!(!status.enabled);
        assert!(!status.autonomous_mode);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !scan_started.exists(),
            "recovery-blocked worker must not enter its immediate first scan"
        );
        assert_eq!(
            fs::read_to_string(&prefs_path).expect("journal file remains"),
            malformed,
            "real worker must not replace an ambiguous journal with defaults"
        );
        shutdown.request();
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .expect("recovery-blocked worker shutdown is bounded")
            .expect("worker exits cleanly");
    }

    #[test]
    fn effect_executor_fence_is_durable_before_the_attempt_is_returned() {
        // SPEC #3200 Phase 7 T-135/FR-044: Prepared -> Attempting is a separate
        // commit receipt. Returning work before this write would let a crash
        // submit a remote mutation with no durable tuple to reconcile.
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let effect = crate::PendingIssueMonitorEffect {
            effect_id: "arm:42:99:abc123:4".to_string(),
            authority_epoch: 4,
            attempt: 0,
            state: crate::IssueMonitorEffectState::Prepared,
            payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                reviewed_sha: "abc123".to_string(),
            },
        };
        let prefs = crate::IssueMonitorPrefs {
            autonomous_mode: true,
            effect_authority_epoch: 4,
            pending_effects: vec![effect.clone()],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);

        let fenced = super::fence_next_issue_monitor_effect(&prefs_path, &mut monitor)
            .expect("Prepared effect receives a durable attempt fence");

        assert_eq!(fenced.effect_id, effect.effect_id);
        assert_eq!(fenced.state, crate::IssueMonitorEffectState::Attempting);
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert_eq!(persisted.pending_effects, vec![fenced]);
    }

    #[test]
    fn stale_arm_result_cannot_publish_delivery_and_keeps_new_epoch_disarm() {
        // Phase 7 T-136: OFF may race an already-started arm. Its old tuple is
        // reconciled and removed, but cannot authorize Delivering; the newer
        // durable disarm remains next in the serialized executor.
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let arm = crate::PendingIssueMonitorEffect {
            effect_id: "arm:42:99:abc:7".to_string(),
            authority_epoch: 7,
            attempt: 0,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                reviewed_sha: "abc".to_string(),
            },
        };
        let disarm = crate::PendingIssueMonitorEffect::prepared(
            "disarm:arm:42:99:abc:7:8",
            8,
            crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                compensates_effect_id: arm.effect_id.clone(),
            },
        );
        let prefs = crate::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: false,
            effect_authority_epoch: 8,
            pending_effects: vec![arm.clone(), disarm.clone()],
            autonomous_records: vec![crate::AutonomousIssueRecord {
                issue_number: 42,
                phase: crate::AutonomousPhase::Reviewing,
                active_launch_id: None,
                attempts: 1,
                acceptance_snapshot: None,
                retry_not_before: None,
                last_heartbeat: None,
                pr_number: Some(99),
                reviewed_sha: Some("abc".to_string()),
                review_passed: Some(true),
            }],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);

        assert!(super::commit_issue_monitor_effect_result(
            &prefs_path,
            &mut monitor,
            super::CompletedIssueMonitorEffect {
                effect: arm,
                outcome: super::IssueMonitorEffectOutcome::AutoMerge(
                    gwt_git::pr_status::AutoMergeMutationOutcome::AlreadyTargetState,
                ),
                completed_at: "2026-07-27T00:00:00Z".to_string(),
            },
        ));

        assert_eq!(
            monitor.autonomous_record(42).map(|record| record.phase),
            Some(crate::AutonomousPhase::Reviewing)
        );
        assert_eq!(monitor.pending_effects(), std::slice::from_ref(&disarm));
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert_eq!(persisted.pending_effects, vec![disarm]);
    }

    #[test]
    #[allow(clippy::await_holding_lock)]
    fn replayed_arm_with_advanced_head_compensates_before_settling() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let fake_gh = temp.path().join("gh");
        let arm_marker = temp.path().join("arm-called");
        let disarm_marker = temp.path().join("disarm-called");
        fs::write(
            &fake_gh,
            r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  printf '%s\n' '{"state":"OPEN","headRefOid":"sha-b","autoMergeRequest":{"enabledAt":"2026-07-27T00:00:00Z"},"mergeCommit":null}'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ] && [ "$4" = "--disable-auto" ]; then
  : > "$GWT_TEST_DISARM_MARKER"
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
  : > "$GWT_TEST_ARM_MARKER"
  exit 0
fi
exit 1
"#,
        )
        .expect("write fake gh");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755)).expect("chmod fake gh");
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _arm_marker = ScopedEnvVar::set("GWT_TEST_ARM_MARKER", &arm_marker);
        let _disarm_marker = ScopedEnvVar::set("GWT_TEST_DISARM_MARKER", &disarm_marker);

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let arm = crate::PendingIssueMonitorEffect {
            effect_id: "arm:42:99:sha-a:7".to_string(),
            authority_epoch: 7,
            attempt: 1,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                reviewed_sha: "sha-a".to_string(),
            },
        };
        let prefs = crate::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            effect_authority_epoch: 7,
            pending_effects: vec![arm.clone()],
            autonomous_records: vec![crate::AutonomousIssueRecord {
                issue_number: 42,
                phase: crate::AutonomousPhase::Reviewing,
                active_launch_id: None,
                attempts: 1,
                acceptance_snapshot: None,
                retry_not_before: None,
                last_heartbeat: None,
                pr_number: Some(99),
                reviewed_sha: Some("sha-a".to_string()),
                review_passed: Some(true),
            }],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed replay journal");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);

        let arm_outcome =
            super::execute_issue_monitor_effect(&scope, &arm, true, "2026-07-27T00:00:00Z");
        assert!(matches!(
            arm_outcome,
            super::IssueMonitorEffectOutcome::AutoMerge(
                gwt_git::pr_status::AutoMergeMutationOutcome::HeadChanged { .. }
            )
        ));
        assert!(!arm_marker.exists(), "replay must not resend arm");
        assert!(super::commit_issue_monitor_effect_result(
            &prefs_path,
            &mut monitor,
            super::CompletedIssueMonitorEffect {
                effect: arm.clone(),
                outcome: arm_outcome,
                completed_at: "2026-07-27T00:00:00Z".to_string(),
            },
        ));

        let disarm = super::fence_next_issue_monitor_effect(&prefs_path, &mut monitor)
            .expect("HEAD change prepares durable disarm");
        assert!(matches!(
            &disarm.payload,
            crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                compensates_effect_id,
                ..
            } if compensates_effect_id == &arm.effect_id
        ));
        let disarm_outcome =
            super::execute_issue_monitor_effect(&scope, &disarm, true, "2026-07-27T00:00:01Z");
        assert!(
            disarm_marker.exists(),
            "compensation must disable auto-merge"
        );
        assert!(super::commit_issue_monitor_effect_result(
            &prefs_path,
            &mut monitor,
            super::CompletedIssueMonitorEffect {
                effect: disarm,
                outcome: disarm_outcome,
                completed_at: "2026-07-27T00:00:01Z".to_string(),
            },
        ));

        assert!(monitor.pending_effects().is_empty());
        assert_eq!(
            monitor.autonomous_record(42).map(|record| record.phase),
            Some(crate::AutonomousPhase::NeedsHuman)
        );
    }

    #[test]
    fn stale_safety_effect_pre_submit_failure_remains_retryable() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let disarm = crate::PendingIssueMonitorEffect {
            effect_id: "disarm:arm:42:99:abc:7:8".to_string(),
            authority_epoch: 8,
            attempt: 0,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                compensates_effect_id: "arm:42:99:abc:7".to_string(),
            },
        };
        let prefs = crate::IssueMonitorPrefs {
            effect_authority_epoch: 9,
            pending_effects: vec![disarm.clone()],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);

        assert!(!super::commit_issue_monitor_effect_result(
            &prefs_path,
            &mut monitor,
            super::CompletedIssueMonitorEffect {
                effect: disarm,
                outcome: super::IssueMonitorEffectOutcome::AutoMerge(
                    gwt_git::pr_status::AutoMergeMutationOutcome::PreSubmit(
                        "gh was not started".to_string(),
                    ),
                ),
                completed_at: "2026-07-27T00:00:00Z".to_string(),
            },
        ));

        let pending = monitor.pending_effects();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].state, crate::IssueMonitorEffectState::Prepared);
        assert_eq!(pending[0].attempt, 1);
        assert_eq!(pending[0].authority_epoch, 8);
    }

    #[test]
    fn merged_before_disarm_is_needs_human_not_false_kill_switch_success() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let disarm = crate::PendingIssueMonitorEffect {
            effect_id: "disarm:arm:42:99:abc:7:8".to_string(),
            authority_epoch: 8,
            attempt: 0,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                compensates_effect_id: "arm:42:99:abc:7".to_string(),
            },
        };
        let prefs = crate::IssueMonitorPrefs {
            enabled: false,
            autonomous_mode: false,
            effect_authority_epoch: 8,
            pending_effects: vec![disarm.clone()],
            autonomous_records: vec![crate::AutonomousIssueRecord {
                issue_number: 42,
                phase: crate::AutonomousPhase::Delivering,
                active_launch_id: None,
                attempts: 1,
                acceptance_snapshot: None,
                retry_not_before: None,
                last_heartbeat: None,
                pr_number: Some(99),
                reviewed_sha: Some("abc".to_string()),
                review_passed: Some(true),
            }],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);

        assert!(super::commit_issue_monitor_effect_result(
            &prefs_path,
            &mut monitor,
            super::CompletedIssueMonitorEffect {
                effect: disarm,
                outcome: super::IssueMonitorEffectOutcome::AutoMerge(
                    gwt_git::pr_status::AutoMergeMutationOutcome::AuthorityMismatch(
                        "pull request merged before kill-switch disarm was confirmed".to_string(),
                    ),
                ),
                completed_at: "2026-07-27T00:00:00Z".to_string(),
            },
        ));

        assert!(monitor.pending_effects().is_empty());
        assert_eq!(
            monitor.autonomous_record(42).map(|record| record.phase),
            Some(crate::AutonomousPhase::NeedsHuman)
        );
        let error = monitor
            .status_view()
            .last_error
            .expect("merged-before-disarm safety failure is visible");
        assert!(error.contains("kill-switch disarm authority failure"));
        assert!(!error.contains("auto-merge disarmed"));
        assert!(
            monitor
                .take_autonomous_notices()
                .iter()
                .all(|notice| !notice.message.contains("will retry next scan")),
            "a terminal merged-before-disarm outcome must not promise a retry"
        );
    }

    #[test]
    fn claim_remote_outcome_unknown_stays_attempting_until_revocation_adds_release() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let claim = crate::PendingIssueMonitorEffect {
            effect_id: "claim:42:7".to_string(),
            authority_epoch: 7,
            attempt: 0,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "stable-claim-42".to_string(),
                owner: "host:1".to_string(),
                heartbeat_at: "2026-07-27T00:00:00Z".to_string(),
                expires_at: "2026-07-27T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        };
        let prefs = crate::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 7,
            pending_effects: vec![claim.clone()],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);

        assert!(!super::commit_issue_monitor_effect_result(
            &prefs_path,
            &mut monitor,
            super::CompletedIssueMonitorEffect {
                effect: claim,
                outcome: super::IssueMonitorEffectOutcome::Claim(Err(
                    gwt_github::client::OwnerMutationError::RemoteOutcomeUnknown(
                        gwt_github::ApiError::Timeout {
                            operation: "create claim comment".to_string(),
                        },
                    ),
                )),
                completed_at: "2026-07-27T00:00:01Z".to_string(),
            },
        ));
        assert_eq!(monitor.pending_effects().len(), 1);
        assert_eq!(
            monitor.pending_effects()[0].state,
            crate::IssueMonitorEffectState::Attempting
        );

        assert_eq!(monitor.set_enabled_with_effect_revocation(false), Some(8));
        assert!(monitor.pending_effects().iter().any(|effect| matches!(
            &effect.payload,
            crate::IssueMonitorEffectPayload::ReleaseClaim { claim_id, .. }
                if claim_id == "stable-claim-42"
        )));
    }

    #[test]
    fn current_arm_result_enters_delivering_only_after_exact_result_commit() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let arm = crate::PendingIssueMonitorEffect {
            effect_id: "arm:42:99:abc:7".to_string(),
            authority_epoch: 7,
            attempt: 2,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                reviewed_sha: "abc".to_string(),
            },
        };
        let prefs = crate::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            effect_authority_epoch: 7,
            pending_effects: vec![arm.clone()],
            autonomous_records: vec![crate::AutonomousIssueRecord {
                issue_number: 42,
                phase: crate::AutonomousPhase::Reviewing,
                active_launch_id: None,
                attempts: 1,
                acceptance_snapshot: None,
                retry_not_before: None,
                last_heartbeat: None,
                pr_number: Some(99),
                reviewed_sha: Some("abc".to_string()),
                review_passed: Some(true),
            }],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);

        assert!(super::commit_issue_monitor_effect_result(
            &prefs_path,
            &mut monitor,
            super::CompletedIssueMonitorEffect {
                effect: arm,
                outcome: super::IssueMonitorEffectOutcome::AutoMerge(
                    gwt_git::pr_status::AutoMergeMutationOutcome::Confirmed,
                ),
                completed_at: "2026-07-27T00:00:00Z".to_string(),
            },
        ));

        assert_eq!(
            monitor.autonomous_record(42).map(|record| record.phase),
            Some(crate::AutonomousPhase::Delivering)
        );
        assert!(monitor.pending_effects().is_empty());
    }

    #[test]
    fn daemon_scan_adopts_newer_disk_migration_before_remote_error() {
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create gwt home");
        let _home = ScopedGwtHome::set(&home);
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo.clone(),
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default())
            .expect("seed newer disk migration");
        let monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            legacy_failed_prefs(&scope.project_root),
        );

        let monitor = super::scan_issue_monitor_once_blocking(scope, monitor, false);

        assert_eq!(
            monitor.prefs().legacy_git_launch_failure_migration_version,
            crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
        );
        assert!(
            monitor.prefs().failed_issues.is_empty(),
            "newer cleanup is adopted even when the later remote scan errors"
        );
    }

    #[tokio::test]
    async fn scan_and_persist_issue_monitor_writes_scan_transitions_to_prefs() {
        // SPEC #3200 (review follow-up): a periodic (interval-tick) scan can
        // complete a merge / escalate without any control frame. The worker must
        // persist prefs after every scan so a daemon restart never loses that
        // completion and re-launches already-finished work. This asserts the
        // scan→persist seam actually writes the merged state to the prefs file.
        let temp = TempDir::new().expect("tempdir");
        init_git_repo(temp.path());
        let scope = sample_scope(&temp);
        let prefs_path = temp.path().join("issue-monitor-prefs.json");

        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        monitor.record_merged(42); // a scan-driven transition that must survive restart
        assert!(!prefs_path.exists(), "prefs not written before the scan");

        let _monitor =
            super::scan_and_persist_issue_monitor(scope, monitor, false, &prefs_path).await;

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("prefs written");
        assert!(
            persisted.merged_issues.contains(&42),
            "scan-driven merge completion is persisted, so a restart will not re-launch it"
        );
    }

    #[test]
    fn scan_join_failure_fallback_preserves_prior_state_so_persist_is_safe() {
        // codex P2 (#3209): a scan-task panic (`JoinError`) must NOT collapse to a
        // fresh `IssueMonitorState::new(default)`. `scan_and_persist` saves the
        // returned state, so a fresh default would overwrite good prefs with
        // `enabled=false` / empty merged_issues / empty autonomous records on a
        // transient panic — losing completion and re-launching finished work. The
        // fallback preserves the prior state and only records the scan error.
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            ..crate::IssueMonitorConfig::default()
        });
        monitor.record_merged(42);

        let out = super::scan_join_failure_fallback(
            monitor,
            "task panicked".to_string(),
            "2026-06-30T00:00:00Z".to_string(),
        );

        assert!(
            out.config.enabled,
            "enabled flag preserved across a scan panic"
        );
        assert!(
            out.prefs().merged_issues.contains(&42),
            "merge completion preserved (not wiped to an empty default)"
        );
        let error = out
            .status_view()
            .last_error
            .expect("the scan error is recorded");
        assert!(
            error.contains("join failed"),
            "records the join failure: {error}"
        );
    }

    #[test]
    fn persist_daemon_state_preserves_gui_owned_launch_profile_and_tuning() {
        // adversarial review (launch_profile clobber): launch_profile and
        // autonomous_tuning have no daemon control channel, so the daemon's
        // stale-since-startup in-memory copy must NOT overwrite the GUI's newer
        // on-disk values. Only daemon-owned runtime state (merged_issues,
        // autonomous_records, ...) is persisted from memory.
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");

        // The GUI wrote a launch_profile + custom tuning straight to disk.
        let on_disk = crate::IssueMonitorPrefs {
            launch_profile: Some(crate::IssueMonitorLaunchProfile {
                agent_id: "claude".to_string(),
                model: None,
                reasoning: None,
                version: None,
                session_mode: Default::default(),
                skip_permissions: false,
                codex_fast_mode: false,
                runtime_target: Default::default(),
                docker_service: None,
                docker_lifecycle_intent: Default::default(),
                windows_shell: None,
            }),
            autonomous_tuning: crate::issue_monitor::AutonomousTuning {
                max_attempts: 9,
                ..crate::issue_monitor::AutonomousTuning::default()
            },
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &on_disk).expect("seed disk");

        // The daemon's in-memory monitor has NO launch_profile (stale startup)
        // but has a daemon-owned merge completion to persist.
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        monitor.record_merged(42);
        assert!(
            monitor.prefs().launch_profile.is_none(),
            "daemon has no profile"
        );

        super::persist_daemon_issue_monitor_state(&prefs_path, &mut monitor);

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload");
        assert!(
            persisted.launch_profile.is_some(),
            "GUI launch_profile preserved (not clobbered by the daemon's stale None)"
        );
        assert_eq!(
            persisted.autonomous_tuning.max_attempts, 9,
            "GUI autonomous_tuning preserved"
        );
        assert!(
            persisted.merged_issues.contains(&42),
            "daemon-owned merge completion is still persisted from memory"
        );
    }

    #[test]
    fn daemon_persist_keeps_local_record_and_unions_latest_disk_owned_state() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let record = |issue_number, phase, attempts| crate::AutonomousIssueRecord {
            issue_number,
            phase,
            active_launch_id: None,
            attempts,
            acceptance_snapshot: None,
            retry_not_before: None,
            last_heartbeat: None,
            pr_number: None,
            reviewed_sha: None,
            review_passed: None,
        };
        let disk_same_key = record(42, crate::AutonomousPhase::Implementing, 1);
        let local_same_key = record(42, crate::AutonomousPhase::Reviewing, 2);
        let disk_only = record(99, crate::AutonomousPhase::Implementing, 3);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 4,
                priority_order: vec![99, 42],
                merged_issues: vec![88],
                autonomous_mode: true,
                autonomous_tuning: crate::issue_monitor::AutonomousTuning {
                    max_attempts: 9,
                    ..crate::issue_monitor::AutonomousTuning::default()
                },
                autonomous_records: vec![disk_same_key, disk_only.clone()],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed latest disk state");
        let mut daemon = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                enabled: false,
                max_active_agents: 1,
                priority_order: vec![42],
                merged_issues: vec![77],
                autonomous_mode: false,
                autonomous_records: vec![local_same_key.clone()],
                ..crate::IssueMonitorPrefs::default()
            },
        );

        super::persist_daemon_issue_monitor_state(&prefs_path, &mut daemon);

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload daemon state");
        for prefs in [&persisted, &daemon.prefs()] {
            assert!(prefs.enabled, "latest disk enabled flag wins");
            assert_eq!(prefs.max_active_agents, 4);
            assert_eq!(prefs.priority_order, vec![99, 42]);
            assert!(prefs.autonomous_mode);
            assert_eq!(prefs.autonomous_tuning.max_attempts, 9);
            assert_eq!(prefs.merged_issues, vec![77, 88], "merged state is unioned");
            assert_eq!(
                prefs.autonomous_records,
                vec![local_same_key.clone(), disk_only.clone()],
                "daemon keeps its same-key scan result and absorbs disk-only records"
            );
        }
    }

    #[test]
    fn daemon_persist_waits_for_sibling_lock_and_rebases_committed_state() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let unrelated_failure = crate::IssueMonitorFailedIssue {
            issue_number: 99,
            message: "unrelated failure".to_string(),
            window_id: None,
        };
        let mut stale_prefs = legacy_failed_prefs(temp.path());
        stale_prefs.enabled = true;
        stale_prefs.max_active_agents = 3;
        stale_prefs.priority_order = vec![99, 43];
        stale_prefs.autonomous_mode = true;
        stale_prefs.failed_issues.push(unrelated_failure.clone());
        crate::save_issue_monitor_prefs(&prefs_path, &stale_prefs)
            .expect("seed stale daemon prefs");
        let mut stale_monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), stale_prefs);
        crate::scan_issue_monitor_candidates(
            &mut stale_monitor,
            &[
                sample_issue_monitor_issue(43),
                sample_issue_monitor_issue(99),
            ],
            "2026-07-21T00:00:00Z",
        );

        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let writer_path = prefs_path.clone();
        let writer = thread::spawn(move || {
            started_tx.send(()).expect("signal writer start");
            super::persist_daemon_issue_monitor_state(&writer_path, &mut stale_monitor);
            done_tx
                .send(stale_monitor)
                .expect("return committed monitor");
        });
        started_rx.recv().expect("writer started");

        assert!(
            done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "the real daemon transaction writer must wait for the sibling lock"
        );

        let disk_profile = sample_issue_monitor_profile();
        let newer_disk = crate::IssueMonitorPrefs {
            legacy_git_launch_failure_migration_version:
                crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION,
            launch_profile: Some(disk_profile.clone()),
            launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                issue_number: 43,
                window_id: "tab-1::agent-43".to_string(),
            }],
            failed_issues: vec![unrelated_failure.clone()],
            autonomous_tuning: crate::issue_monitor::AutonomousTuning {
                max_attempts: 9,
                ..crate::issue_monitor::AutonomousTuning::default()
            },
            ..crate::IssueMonitorPrefs::default()
        };
        write_issue_monitor_prefs_without_lock(&prefs_path, &newer_disk);
        FileExt::unlock(&lock).expect("release issue monitor prefs lock");

        let committed_monitor = done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("daemon writer completes after unlock");
        writer.join().expect("daemon writer thread");
        let committed =
            crate::load_issue_monitor_prefs(&prefs_path).expect("reload committed prefs");

        for prefs in [&committed, &committed_monitor.prefs()] {
            assert_eq!(
                prefs.legacy_git_launch_failure_migration_version,
                crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
            );
            assert!(
                !prefs
                    .failed_issues
                    .iter()
                    .any(|failed| failed.issue_number == 43),
                "the stale legacy failure stays removed"
            );
            assert_eq!(
                prefs
                    .failed_issues
                    .iter()
                    .find(|failed| failed.issue_number == 99),
                Some(&unrelated_failure)
            );
            assert_eq!(prefs.launch_profile.as_ref(), Some(&disk_profile));
            assert_eq!(prefs.autonomous_tuning.max_attempts, 9);
            assert!(!prefs.enabled, "latest committed disk config wins");
            assert_eq!(prefs.max_active_agents, 1);
            assert!(prefs.priority_order.is_empty());
            assert!(!prefs.autonomous_mode);
            assert_eq!(
                prefs.launched_issues,
                vec![crate::IssueMonitorLaunchedIssue {
                    issue_number: 43,
                    window_id: "tab-1::agent-43".to_string(),
                }]
            );
        }
        assert_eq!(
            committed_monitor.launched_window_issue("tab-1::agent-43"),
            Some(43)
        );
        assert!(committed_monitor.inbox_item(43).is_none());
        assert_eq!(
            committed_monitor
                .inbox_item(99)
                .and_then(|item| item.error_message.as_deref()),
            Some("unrelated failure")
        );
        assert_eq!(
            committed_monitor.status_view().last_error.as_deref(),
            Some("issue #99: unrelated failure")
        );
    }

    #[test]
    fn daemon_scan_merges_disk_launch_before_newer_migration_adoption() {
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create gwt home");
        let _home = ScopedGwtHome::set(&home);
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let competing_failure = crate::IssueMonitorFailedIssue {
            issue_number: 43,
            message: "stale competing failure".to_string(),
            window_id: None,
        };
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                    issue_number: 43,
                    window_id: "tab-1::agent-43".to_string(),
                }],
                failed_issues: vec![competing_failure],
                autonomous_records: vec![crate::AutonomousIssueRecord {
                    issue_number: 43,
                    phase: crate::AutonomousPhase::NeedsHuman,
                    active_launch_id: None,
                    attempts: 6,
                    acceptance_snapshot: None,
                    retry_not_before: None,
                    last_heartbeat: None,
                    pr_number: None,
                    reviewed_sha: None,
                    review_passed: None,
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed newer disk state");
        let monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                legacy_git_launch_failure_migration_version: 0,
                ..crate::IssueMonitorPrefs::default()
            },
        );

        let monitor = super::scan_issue_monitor_once_blocking(scope, monitor, false);
        let prefs = monitor.prefs();

        assert_eq!(
            prefs.launched_issues,
            vec![crate::IssueMonitorLaunchedIssue {
                issue_number: 43,
                window_id: "tab-1::agent-43".to_string(),
            }],
            "the disk-only real launch is merged before migration adoption"
        );
        assert!(
            prefs
                .failed_issues
                .iter()
                .all(|failed| failed.issue_number != 43),
            "migration adoption cannot create a launched+failed split-brain row"
        );
        assert!(
            prefs
                .autonomous_records
                .iter()
                .all(|record| record.issue_number != 43),
            "a rejected failure cannot smuggle its NeedsHuman companion past a real launch"
        );
    }

    #[test]
    fn daemon_control_waits_for_migration_commit_then_keeps_fresh_same_failure() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let stale_prefs = legacy_failed_prefs(temp.path());
        crate::save_issue_monitor_prefs(&prefs_path, &stale_prefs).expect("seed legacy prefs");
        let stale_monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), stale_prefs);
        let fresh_failure = legacy_git_failure(temp.path());

        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let writer_path = prefs_path.clone();
        let writer_failure = fresh_failure.clone();
        let writer = thread::spawn(move || {
            let mut monitor = stale_monitor;
            started_tx.send(()).expect("signal control start");
            let should_scan = super::apply_issue_monitor_control_with_disk_migration(
                &writer_path,
                &mut monitor,
                IssueMonitorControl::LaunchFailed {
                    issue_number: 43,
                    message: writer_failure,
                },
            );
            done_tx
                .send((should_scan, monitor))
                .expect("return committed control state");
        });
        started_rx.recv().expect("control writer started");

        assert!(
            done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "migration adoption, control mutation, and save must share the sibling lock"
        );

        write_issue_monitor_prefs_without_lock(&prefs_path, &crate::IssueMonitorPrefs::default());
        FileExt::unlock(&lock).expect("release issue monitor prefs lock");

        let (should_scan, committed_monitor) = done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("control transaction completes after unlock");
        writer.join().expect("control writer thread");
        assert!(should_scan);
        let committed =
            crate::load_issue_monitor_prefs(&prefs_path).expect("reload committed prefs");
        for prefs in [&committed, &committed_monitor.prefs()] {
            assert_eq!(
                prefs.legacy_git_launch_failure_migration_version,
                crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
            );
            assert_eq!(prefs.failed_issues.len(), 1);
            assert_eq!(prefs.failed_issues[0].message, fresh_failure);
        }
        assert_eq!(
            committed_monitor.status_view().last_error.as_deref(),
            Some(format!("issue #43: {fresh_failure}").as_str())
        );
    }

    #[test]
    fn daemon_control_is_bounded_and_revokes_volatile_state_when_prefs_lock_is_stuck() {
        // Phase 7 T-138/FR-047: a sibling that never releases the prefs lock
        // must not hang the daemon control plane indefinitely. No commit
        // receipt means the in-memory control is revoked as well.
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default())
            .expect("seed prefs");
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        let before = monitor.prefs();
        let started = Instant::now();

        let should_scan = super::apply_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut monitor,
            IssueMonitorControl::Enabled(true),
        );

        assert!(!should_scan);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(monitor.prefs(), before, "uncommitted control is revoked");
        let last_error = monitor
            .status_view()
            .last_error
            .expect("control commit failure is operator-visible");
        assert!(last_error.contains("control commit failed"));
        assert!(last_error.contains("prefs-lock stage"));
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs"),
            before
        );
        FileExt::unlock(&lock).expect("release test lock");
    }

    #[test]
    fn persist_daemon_state_adopts_newer_disk_migration_without_restoring_old_failure() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default())
            .expect("seed newer disk migration");
        let mut stale = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            legacy_failed_prefs(temp.path()),
        );

        super::persist_daemon_issue_monitor_state(&prefs_path, &mut stale);

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert_eq!(
            persisted.legacy_git_launch_failure_migration_version,
            crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
        );
        assert!(
            persisted.failed_issues.is_empty(),
            "a stale daemon cannot restore the failure removed by the GUI"
        );
    }

    #[test]
    fn daemon_persist_adopts_newer_marker_without_dropping_local_needs_human_failure() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default())
            .expect("seed newer disk migration");
        let mut daemon = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            legacy_failed_prefs(temp.path()),
        );
        daemon.escalate_to_needs_human(100, "local terminal failure");

        super::persist_daemon_issue_monitor_state(&prefs_path, &mut daemon);

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert!(
            persisted
                .failed_issues
                .iter()
                .all(|failed| failed.issue_number != 43),
            "the newer marker still removes the legacy failure"
        );
        assert_eq!(
            persisted
                .failed_issues
                .iter()
                .find(|failed| failed.issue_number == 100)
                .map(|failed| failed.message.as_str()),
            Some("local terminal failure")
        );
        assert!(persisted.autonomous_records.iter().any(|record| {
            record.issue_number == 100 && record.phase == crate::AutonomousPhase::NeedsHuman
        }));

        let mut restored =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), persisted);
        crate::scan_issue_monitor_candidates(
            &mut restored,
            &[sample_issue_monitor_issue(100)],
            "2026-07-21T00:01:00Z",
        );
        assert_eq!(
            restored.inbox_item(100).map(|item| item.state),
            Some(crate::MonitorInboxState::NeedsHuman)
        );
    }

    #[test]
    fn daemon_persist_adopts_newer_marker_without_dropping_local_failed_window() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default())
            .expect("seed newer disk migration");
        let mut local = legacy_failed_prefs(temp.path());
        local.failed_issues.push(crate::IssueMonitorFailedIssue {
            issue_number: 100,
            message: "local windowed failure".to_string(),
            window_id: Some("window-100".to_string()),
        });
        let mut daemon =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), local);

        super::persist_daemon_issue_monitor_state(&prefs_path, &mut daemon);

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert!(
            persisted
                .failed_issues
                .iter()
                .all(|failed| failed.issue_number != 43),
            "the newer marker still removes the legacy failure"
        );
        assert!(persisted.failed_issues.iter().any(|failed| {
            failed.issue_number == 100
                && failed.message == "local windowed failure"
                && failed.window_id.as_deref() == Some("window-100")
        }));
    }

    #[test]
    fn daemon_persist_recovers_malformed_prefs_from_in_memory_state() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        fs::write(&prefs_path, b"{").expect("seed malformed prefs");
        let mut daemon = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                enabled: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        daemon.record_merged(42);

        super::persist_daemon_issue_monitor_state(&prefs_path, &mut daemon);

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("recovered prefs");
        assert!(persisted.enabled);
        assert_eq!(persisted.merged_issues, vec![42]);
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
        assert_eq!(quarantines.len(), 1, "malformed source is quarantined once");
        assert_eq!(
            fs::read(&quarantines[0]).expect("read quarantine"),
            b"{",
            "quarantine preserves the malformed source bytes"
        );
    }

    #[test]
    fn daemon_control_recovers_malformed_prefs_before_persisting_mutation() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        fs::write(&prefs_path, b"{").expect("seed malformed prefs");
        let mut daemon = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs::recovery_default(),
        );

        assert!(super::apply_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut daemon,
            IssueMonitorControl::Enabled(true),
        ));

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("recovered prefs");
        assert!(persisted.enabled);
        assert_eq!(
            persisted.legacy_git_launch_failure_migration_version, 0,
            "daemon startup recovery must wait for a successful live scan"
        );
        let quarantine_count = fs::read_dir(temp.path())
            .expect("read prefs directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("issue-monitor.json.corrupt-")
            })
            .count();
        assert_eq!(quarantine_count, 1, "malformed source is quarantined once");
    }

    #[test]
    fn daemon_newer_failure_adoption_keeps_needs_human_and_clears_restored_launch() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let failure = crate::IssueMonitorFailedIssue {
            issue_number: 100,
            message: "human review required".to_string(),
            window_id: None,
        };
        let needs_human = crate::AutonomousIssueRecord {
            issue_number: 100,
            phase: crate::AutonomousPhase::NeedsHuman,
            active_launch_id: None,
            attempts: 6,
            acceptance_snapshot: None,
            retry_not_before: None,
            last_heartbeat: Some("2026-07-21T00:00:00Z".to_string()),
            pr_number: None,
            reviewed_sha: None,
            review_passed: None,
        };
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                failed_issues: vec![failure.clone()],
                autonomous_records: vec![needs_human.clone()],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed newer NeedsHuman migration");
        let mut daemon = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                enabled: true,
                legacy_git_launch_failure_migration_version: 0,
                launching_issues: vec![crate::IssueMonitorLaunchingIssue {
                    issue_number: 100,
                    claimed_at: None,
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        );
        assert_eq!(
            daemon.active_count(),
            1,
            "legacy launch restores an active slot"
        );
        assert!(
            daemon.take_pending_launch_requests().is_empty(),
            "the restored launch deliberately has no pending request"
        );

        super::persist_daemon_issue_monitor_state(&prefs_path, &mut daemon);

        assert_eq!(
            daemon.active_count(),
            0,
            "adopted failure releases the active slot"
        );
        assert!(daemon.take_pending_launch_requests().is_empty());
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload adopted state");
        assert!(persisted.launching_issues.is_empty());
        assert_eq!(persisted.failed_issues, vec![failure]);
        assert_eq!(persisted.autonomous_records, vec![needs_human]);

        let mut restored =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), persisted);
        crate::scan_issue_monitor_candidates(
            &mut restored,
            &[sample_issue_monitor_issue(100)],
            "2026-07-21T00:01:00Z",
        );
        assert_eq!(
            restored.inbox_item(100).map(|item| item.state),
            Some(crate::MonitorInboxState::NeedsHuman),
            "save/load reconstructs the terminal NeedsHuman row"
        );
    }

    #[test]
    fn persist_daemon_state_keeps_equal_marker_new_failure_and_never_decreases_marker() {
        let temp = TempDir::new().expect("tempdir");
        let equal_path = temp.path().join("equal.json");
        crate::save_issue_monitor_prefs(&equal_path, &crate::IssueMonitorPrefs::default())
            .expect("seed equal marker");
        let new_failure = legacy_git_failure(temp.path());
        let mut equal_monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                failed_issues: vec![crate::IssueMonitorFailedIssue {
                    issue_number: 43,
                    message: new_failure.clone(),
                    window_id: None,
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        );

        super::persist_daemon_issue_monitor_state(&equal_path, &mut equal_monitor);

        let equal = crate::load_issue_monitor_prefs(&equal_path).expect("reload equal prefs");
        assert_eq!(equal.failed_issues.len(), 1);
        assert_eq!(equal.failed_issues[0].message, new_failure);

        let future_path = temp.path().join("future.json");
        crate::save_issue_monitor_prefs(&future_path, &crate::IssueMonitorPrefs::default())
            .expect("seed older disk marker");
        let mut future_monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                legacy_git_launch_failure_migration_version:
                    crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION + 1,
                ..crate::IssueMonitorPrefs::default()
            },
        );

        super::persist_daemon_issue_monitor_state(&future_path, &mut future_monitor);

        let future = crate::load_issue_monitor_prefs(&future_path).expect("reload future prefs");
        assert_eq!(
            future.legacy_git_launch_failure_migration_version,
            crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION + 1,
            "an older disk snapshot cannot decrease the daemon marker"
        );
    }

    #[test]
    fn daemon_persist_keeps_equal_marker_disk_only_fresh_failures() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let fresh_failure = legacy_git_failure(temp.path());
        let disk = crate::IssueMonitorPrefs {
            failed_issues: vec![
                crate::IssueMonitorFailedIssue {
                    issue_number: 43,
                    message: fresh_failure,
                    window_id: None,
                },
                crate::IssueMonitorFailedIssue {
                    issue_number: 99,
                    message: "unrelated failure".to_string(),
                    window_id: Some("tab-1::agent-99".to_string()),
                },
            ],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &disk)
            .expect("seed equal-marker disk failures");
        let mut stale = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs::default(),
        );

        super::persist_daemon_issue_monitor_state(&prefs_path, &mut stale);

        let persisted =
            crate::load_issue_monitor_prefs(&prefs_path).expect("reload committed prefs");
        for prefs in [&persisted, &stale.prefs()] {
            assert_eq!(
                prefs.legacy_git_launch_failure_migration_version,
                crate::issue_monitor::LEGACY_GIT_LAUNCH_FAILURE_MIGRATION_VERSION
            );
            assert_eq!(prefs.failed_issues, disk.failed_issues);
        }
    }

    #[test]
    fn daemon_control_rebase_applies_explicit_lifecycle_mutation_last() {
        let temp = TempDir::new().expect("tempdir");

        let unrelated_path = temp.path().join("unrelated.json");
        let unrelated_failure = crate::IssueMonitorFailedIssue {
            issue_number: 99,
            message: "unrelated failure".to_string(),
            window_id: Some("tab-1::agent-99".to_string()),
        };
        crate::save_issue_monitor_prefs(
            &unrelated_path,
            &crate::IssueMonitorPrefs {
                failed_issues: vec![unrelated_failure.clone()],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed unrelated disk failure");
        let mut launched = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        super::apply_issue_monitor_control_with_disk_migration(
            &unrelated_path,
            &mut launched,
            IssueMonitorControl::Launched {
                issue_number: 43,
                window_id: "tab-1::agent-43".to_string(),
            },
        );
        let launched_prefs =
            crate::load_issue_monitor_prefs(&unrelated_path).expect("reload launched prefs");
        assert_eq!(launched_prefs.failed_issues, vec![unrelated_failure]);
        assert_eq!(
            launched_prefs.launched_issues,
            vec![crate::IssueMonitorLaunchedIssue {
                issue_number: 43,
                window_id: "tab-1::agent-43".to_string(),
            }]
        );
        assert_eq!(launched.prefs(), launched_prefs);

        let same_issue_path = temp.path().join("same-issue.json");
        crate::save_issue_monitor_prefs(
            &same_issue_path,
            &crate::IssueMonitorPrefs {
                failed_issues: vec![crate::IssueMonitorFailedIssue {
                    issue_number: 43,
                    message: "disk failure".to_string(),
                    window_id: None,
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed same-Issue disk failure");
        let mut same_issue = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        super::apply_issue_monitor_control_with_disk_migration(
            &same_issue_path,
            &mut same_issue,
            IssueMonitorControl::Launched {
                issue_number: 43,
                window_id: "tab-1::agent-43".to_string(),
            },
        );
        let same_issue_prefs =
            crate::load_issue_monitor_prefs(&same_issue_path).expect("reload same-Issue prefs");
        assert!(same_issue_prefs.failed_issues.is_empty());
        assert_eq!(same_issue_prefs.launched_issues.len(), 1);
        assert_eq!(same_issue.prefs(), same_issue_prefs);

        let failure_wins_path = temp.path().join("failure-wins.json");
        crate::save_issue_monitor_prefs(
            &failure_wins_path,
            &crate::IssueMonitorPrefs {
                launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                    issue_number: 43,
                    window_id: "tab-1::agent-43".to_string(),
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed disk launch");
        let mut failed = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        super::apply_issue_monitor_control_with_disk_migration(
            &failure_wins_path,
            &mut failed,
            IssueMonitorControl::LaunchFailed {
                issue_number: 43,
                message: "fresh failure".to_string(),
            },
        );
        let failed_prefs =
            crate::load_issue_monitor_prefs(&failure_wins_path).expect("reload failed prefs");
        assert!(failed_prefs.launched_issues.is_empty());
        assert_eq!(failed_prefs.failed_issues.len(), 1);
        assert_eq!(failed_prefs.failed_issues[0].message, "fresh failure");
        assert_eq!(failed.prefs(), failed_prefs);
    }

    #[tokio::test]
    async fn run_server_accepts_handshake_and_acknowledges_frames() {
        let temp = TempDir::new().expect("tempdir");
        let scope = sample_scope(&temp);
        let socket_path = temp.path().join("daemon.sock");
        let endpoint_path = temp.path().join("endpoint.json");
        let endpoint = sample_endpoint(scope.clone(), &socket_path, "secret");

        // Pre-create the socket file by binding inside run_server. We need
        // run_server to bind, then a client connects, exchanges handshake,
        // and sends one frame.
        let server_socket = socket_path.clone();
        let server_endpoint_path = endpoint_path.clone();
        let server_hub = BroadcastHub::new();
        let server_handle = tokio::spawn(async move {
            run_server(endpoint, server_socket, server_endpoint_path, server_hub).await
        });

        // wait until the socket is bound
        let mut attempts = 0;
        while !socket_path.exists() && attempts < 50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            attempts += 1;
        }
        assert!(socket_path.exists(), "socket bound");

        let stream = UnixStream::connect(&socket_path).await.expect("connect");
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);

        let request = IpcHandshakeRequest {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            auth_token: "secret".to_string(),
            scope,
        };
        let payload = serde_json::to_vec(&request).expect("serialize");
        write_half.write_all(&payload).await.expect("write request");
        write_half.write_all(b"\n").await.expect("write newline");

        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .await
            .expect("read response");
        assert!(response_line.contains("\"accepted\":true"));

        // Send a typed `ClientFrame::Hook` and expect a `DaemonFrame::Ack`.
        let request_scope = sample_scope(&temp);
        let frame = ClientFrame::Hook(HookEnvelope {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            scope: request_scope,
            hook_name: "runtime-state".to_string(),
            session_id: None,
            cwd: temp.path().to_path_buf(),
            payload: serde_json::json!({}),
        });
        let mut frame_bytes = serde_json::to_vec(&frame).expect("serialize frame");
        frame_bytes.push(b'\n');
        write_half
            .write_all(&frame_bytes)
            .await
            .expect("write frame");
        let mut ack = String::new();
        reader.read_line(&mut ack).await.expect("read ack");
        assert!(
            ack.contains("\"type\":\"ack\""),
            "expected typed ack frame, got: {ack}"
        );

        // Send a malformed line and expect a typed Error frame back.
        write_half
            .write_all(b"not-json\n")
            .await
            .expect("write malformed frame");
        let mut error_line = String::new();
        reader
            .read_line(&mut error_line)
            .await
            .expect("read error frame");
        assert!(
            error_line.contains("\"type\":\"error\""),
            "expected typed error frame, got: {error_line}"
        );

        // Closing the client should let the per-connection task finish.
        drop(write_half);
        drop(reader);

        // Cancel the server (simulating SIGINT) by aborting.
        server_handle.abort();
        let _ = server_handle.await;
    }
}
