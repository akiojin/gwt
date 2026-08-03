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
    collections::VecDeque,
    fs, io,
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

use super::broadcast::{
    BroadcastHub, IssueMonitorControlCompletion, IssueMonitorControlQueueError,
    IssueMonitorControlRequest,
};

const ACCEPT_BACKOFF_MS: u64 = 50;
const ISSUE_MONITOR_SCAN_TIMEOUT: Duration = Duration::from_secs(60);
const ISSUE_MONITOR_PREFS_TIMEOUT: Duration = Duration::from_millis(250);
const ISSUE_MONITOR_AUTHORITY_RETRY_DELAY: Duration = Duration::from_millis(50);
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
    // Establish the control-lane state from the durable snapshot before the
    // server can accept a publisher connection. Starting publishers wait on
    // the watch state until this load either installs the sole Ready receiver
    // or publishes the stable RecoveryBlocked terminal state.
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
    let loaded = load_issue_monitor_state_for_daemon(&prefs_path, config);
    let control_rx = if loaded.recovery_blocked {
        hub.mark_issue_monitor_control_recovery_blocked();
        None
    } else {
        hub.take_issue_monitor_control_receiver()
    };
    refresh_issue_monitor_agent_status(&hub, &loaded.monitor);
    tokio::spawn(async move {
        let LoadedDaemonIssueMonitorState {
            mut monitor,
            recovery_blocked,
            authority_fence,
            authority_lease,
        } = loaded;
        // SPEC #3200 (review follow-up): a record persisted mid-review reloads in
        // `Reviewing`, but its review-agent dispatch (not persisted) is gone.
        // Reset such records to `Implementing` so the first scan re-detects the PR
        // and re-issues the review — restoring the pre-persist self-healing. The
        // `now` stamp refreshes last_heartbeat so the reset record is not wrongly
        // reclaimed by stuck detection (which runs before the re-dispatch).
        let mut deferred_restart_reviews = monitor
            .prefs()
            .autonomous_records
            .into_iter()
            .filter(|record| {
                record.phase == crate::AutonomousPhase::Reviewing && record.review_passed.is_none()
            })
            .collect::<Vec<_>>();
        if recovery_blocked {
            publish_issue_monitor_read_only_payloads(&hub, &monitor);
            // The unreadable bytes may describe an Attempting remote mutation.
            // No scan, control, recovery write, or shutdown rewrite is safe
            // until an operator resolves the journal explicitly. Keep the
            // read-only error projection alive for clients that connect after
            // the startup broadcast (BroadcastHub has no replay buffer).
            let _control_lane_guard = IssueMonitorControlLaneGuard::new(hub.clone());
            let mut recovery_status_tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.notified() => break,
                    _ = recovery_status_tick.tick() => {
                        publish_issue_monitor_read_only_payloads(&hub, &monitor);
                    }
                }
            }
            hub.close_issue_monitor_controls();
            return;
        }
        if monitor.pending_effects().is_empty() {
            resume_deferred_restart_reviews(
                &prefs_path,
                &mut monitor,
                &mut deferred_restart_reviews,
            );
        } else {
            tracing::info!(
                pending_effects = monitor.pending_effects().len(),
                issues = ?deferred_restart_reviews
                    .iter()
                    .map(|record| record.issue_number)
                    .collect::<Vec<_>>(),
                "issue monitor: deferring review resume until durable effects reconcile"
            );
        }
        publish_issue_monitor_payloads(&hub, &mut monitor);
        let Some(mut control_rx) = control_rx else {
            tracing::error!("issue monitor control receiver already claimed; stopping worker");
            hub.close_issue_monitor_controls();
            return;
        };
        let mut interval =
            tokio::time::interval(Duration::from_secs(monitor.config.poll_interval_secs));
        let mut revision = 0_u64;
        let mut scan_requested = false;
        let mut in_flight_scan: Option<InFlightIssueMonitorScan> = None;
        let mut effect_execution_requested = !monitor.pending_effects().is_empty();
        let mut in_flight_effect: Option<InFlightIssueMonitorEffect> = None;
        let mut effect_permit = IssueMonitorEffectPermit::new();
        let mut pending_authority_controls: Option<PendingIssueMonitorAuthorityControls> = None;
        let mut deferred_grant_result: Option<CompletedIssueMonitorEffect> = None;
        let mut control_open = true;
        // Declared after all receipt-owning worker state so unwinding changes
        // the public state to Closed before queued/pending completions drop.
        let mut control_lane_guard = IssueMonitorControlLaneGuard::new_with_authority(
            hub.clone(),
            effect_permit.lane_open(),
            prefs_path.clone(),
            authority_fence.expect("ready worker owns a durable authority fence"),
            authority_lease.expect("ready worker owns a lifetime authority lease"),
        );

        loop {
            let scan_watchdog_deadline = in_flight_scan
                .as_ref()
                .filter(|scan| !scan.watchdog_fired)
                .map(|scan| scan.deadline);
            let effect_watchdog_deadline = in_flight_effect
                .as_ref()
                .filter(|effect| !effect.watchdog_fired)
                .map(|effect| effect.deadline);
            let authority_retry_deadline = pending_authority_controls
                .as_ref()
                .and_then(PendingIssueMonitorAuthorityControls::retry_at);
            tokio::select! {
                biased;
                _ = shutdown.notified() => {
                    close_issue_monitor_control_lane(
                        &hub,
                        &mut control_rx,
                        &mut pending_authority_controls,
                        IssueMonitorControlQueueError::Closed,
                    );
                    effect_permit.close_lane();
                    let authority_settlement = settle_issue_monitor_effect_authority_for_shutdown(
                        &prefs_path,
                        Some(&mut monitor),
                        control_lane_guard
                            .authority_fence()
                            .expect("ready worker authority fence"),
                    );
                    let authority_revoked = authority_settlement.authority_revoked();
                    if authority_revoked {
                        if let Some(completed) = deferred_grant_result.take() {
                            let _ = commit_issue_monitor_effect_result(
                                &prefs_path,
                                &mut monitor,
                                completed,
                            );
                        }
                    }
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
                        if authority_revoked {
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
                    }
                    if authority_settlement.cleanup_safely_fenced() {
                        control_lane_guard.disarm_authority_cleanup();
                    }
                    break;
                },
                control = control_rx.recv(), if control_open && pending_authority_controls.is_none() => {
                    match control {
                        Some(request) => {
                            let (frame, completion) = request.into_parts();
                            let DaemonFrame::Event { payload, .. } = frame else {
                                completion.reject(IssueMonitorControlQueueError::Rejected);
                                continue;
                            };
                            if let Some(control) = decode_issue_monitor_control(payload) {
                                let Some(next_revision) = revision.checked_add(1) else {
                                    tracing::error!("issue monitor revision exhausted; stopping worker");
                                    completion.reject(IssueMonitorControlQueueError::Closed);
                                    close_issue_monitor_control_lane(
                                        &hub,
                                        &mut control_rx,
                                        &mut pending_authority_controls,
                                        IssueMonitorControlQueueError::Closed,
                                    );
                                    break;
                                };
                                revision = next_revision;
                                let should_scan = apply_or_queue_issue_monitor_control(
                                    &hub,
                                    &prefs_path,
                                    &mut monitor,
                                    control,
                                    &mut effect_permit,
                                    &mut pending_authority_controls,
                                    Some(completion),
                                );
                                if pending_authority_controls
                                    .as_ref()
                                    .is_some_and(PendingIssueMonitorAuthorityControls::is_terminal)
                                {
                                    close_issue_monitor_control_lane(
                                        &hub,
                                        &mut control_rx,
                                        &mut pending_authority_controls,
                                        IssueMonitorControlQueueError::Rejected,
                                    );
                                    control_open = false;
                                }
                                publish_issue_monitor_payloads(&hub, &mut monitor);
                                effect_execution_requested =
                                    !monitor.pending_effects().is_empty();
                                // Preserve committed scan intent while the
                                // authority barrier is active. The launch gate
                                // below suppresses only the start, not the
                                // request itself, so it runs after the barrier
                                // drains.
                                scan_requested |= should_scan;
                            } else {
                                completion.reject(IssueMonitorControlQueueError::Rejected);
                            }
                        }
                        None => {
                            close_issue_monitor_control_lane(
                                &hub,
                                &mut control_rx,
                                &mut pending_authority_controls,
                                IssueMonitorControlQueueError::Closed,
                            );
                            control_open = false;
                        }
                    }
                }
                _ = wait_for_issue_monitor_deadline(authority_retry_deadline) => {
                    let Some(accepted) = pending_authority_controls
                        .as_ref()
                        .and_then(PendingIssueMonitorAuthorityControls::front_accepted)
                        .cloned()
                    else {
                        continue;
                    };
                    let Some(next_revision) = revision.checked_add(1) else {
                        tracing::error!(
                            "issue monitor revision exhausted before control retry; stopping worker"
                        );
                        close_issue_monitor_control_lane(
                            &hub,
                            &mut control_rx,
                            &mut pending_authority_controls,
                            IssueMonitorControlQueueError::Closed,
                        );
                        break;
                    };
                    revision = next_revision;
                    match try_apply_accepted_issue_monitor_control_with_disk_migration(
                        &prefs_path,
                        &mut monitor,
                        accepted,
                    ) {
                        IssueMonitorControlCommit::Committed {
                            should_scan,
                            authority_changed,
                        } => {
                            scan_requested |= should_scan;
                            let (drained, completion, authorizing) = pending_authority_controls
                                .as_mut()
                                .expect("authority barrier exists during retry")
                                .committed_front();
                            if authorizing {
                                reconcile_deferred_grant_after_authority_commit(
                                    &prefs_path,
                                    &mut monitor,
                                    &mut deferred_grant_result,
                                    authority_changed,
                                    drained,
                                );
                            }
                            if let Some(completion) = completion {
                                commit_issue_monitor_control_completion(
                                    &hub,
                                    &monitor,
                                    completion,
                                );
                            }
                            if drained {
                                pending_authority_controls = None;
                                if authorizing {
                                    effect_permit.reopen();
                                }
                            }
                            effect_execution_requested = !monitor.pending_effects().is_empty();
                        }
                        IssueMonitorControlCommit::RetryableFailure => {
                            pending_authority_controls
                                .as_mut()
                                .expect("authority barrier exists during retry")
                                .retry_failed();
                        }
                        IssueMonitorControlCommit::TerminalFailure => {
                            pending_authority_controls
                                .as_mut()
                                .expect("authority barrier exists during retry")
                                .terminal_failure();
                            close_issue_monitor_control_lane(
                                &hub,
                                &mut control_rx,
                                &mut pending_authority_controls,
                                IssueMonitorControlQueueError::Rejected,
                            );
                            control_open = false;
                        }
                    }
                    publish_issue_monitor_payloads(&hub, &mut monitor);
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
                (captured_revision, captured_authority_epoch, captured_deadline, scan_result) = wait_for_issue_monitor_scan(&mut in_flight_scan) => {
                    in_flight_scan = None;
                    match scan_result {
                        Ok(Ok(scanned_monitor)) => {
                            if accept_completed_issue_monitor_scan(
                                &prefs_path,
                                &mut monitor,
                                scanned_monitor,
                                captured_revision,
                                revision,
                                captured_authority_epoch,
                                captured_deadline,
                            ) {
                                publish_issue_monitor_payloads(&hub, &mut monitor);
                                effect_execution_requested =
                                    !monitor.pending_effects().is_empty();
                            } else if captured_revision == revision
                                && Instant::now() >= captured_deadline
                            {
                                monitor.record_scan_error(
                                    chrono::Utc::now()
                                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                                    "issue monitor scan completed at or after the outer watchdog boundary",
                                );
                                let Some(next_revision) = revision.checked_add(1) else {
                                    tracing::error!("issue monitor revision exhausted; stopping worker");
                                    break;
                                };
                                revision = next_revision;
                                persist_daemon_issue_monitor_state(&prefs_path, &mut monitor);
                                publish_issue_monitor_payloads(&hub, &mut monitor);
                                scan_requested = true;
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
                                tokio::task::yield_now().await;
                            }
                        }
                        Ok(Err(failure)) => {
                            monitor = scan_failure_fallback(
                                monitor.clone(),
                                failure,
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
                effect_result = wait_for_issue_monitor_effect(&mut in_flight_effect) => {
                    in_flight_effect = None;
                    match effect_result {
                        Ok(completed) => {
                            if pending_authority_controls
                                .as_ref()
                                .is_some_and(PendingIssueMonitorAuthorityControls::front_is_authorizing)
                                && !issue_monitor_effect_is_safety(&completed.effect)
                            {
                                deferred_grant_result = Some(completed);
                                effect_execution_requested =
                                    !monitor.pending_effects().is_empty();
                            } else {
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
                    if !pending_authority_controls
                        .as_ref()
                        .is_some_and(PendingIssueMonitorAuthorityControls::front_is_authorizing)
                    {
                        scan_requested = true;
                    }
                    effect_execution_requested = !monitor.pending_effects().is_empty();
                    if in_flight_scan.is_some() {
                        // Re-project the canonical state while the scan is
                        // blocked so current-time stalled status remains
                        // observable without spawning a second scan.
                        publish_issue_monitor_payloads(&hub, &mut monitor);
                    }
                }
            }

            resume_deferred_restart_reviews(
                &prefs_path,
                &mut monitor,
                &mut deferred_restart_reviews,
            );

            if scan_requested
                && in_flight_scan.is_none()
                && !pending_authority_controls
                    .as_ref()
                    .is_some_and(PendingIssueMonitorAuthorityControls::front_is_authorizing)
            {
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
                let current_permit = effect_permit.capture();
                let effect = monitor
                    .pending_effects()
                    .iter()
                    .find(|effect| {
                        effect.state == crate::IssueMonitorEffectState::Attempting
                            && issue_monitor_effect_permitted(effect, &current_permit)
                    })
                    .cloned()
                    .or_else(|| {
                        fence_next_issue_monitor_effect_with_permit(
                            &prefs_path,
                            &mut monitor,
                            &current_permit,
                        )
                    });
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
                        current_permit,
                        deadline,
                    ));
                }
            }
        }
        close_issue_monitor_control_lane(
            &hub,
            &mut control_rx,
            &mut pending_authority_controls,
            IssueMonitorControlQueueError::Closed,
        );
    })
}

struct IssueMonitorControlLaneGuard {
    hub: BroadcastHub,
    grant_lane_open: Option<Arc<AtomicBool>>,
    prefs_path: Option<PathBuf>,
    authority_fence: Option<crate::IssueMonitorAuthorityFence>,
    _authority_lease: Option<crate::IssueMonitorAuthorityLease>,
    authority_cleanup_armed: bool,
}

impl IssueMonitorControlLaneGuard {
    fn new(hub: BroadcastHub) -> Self {
        Self {
            hub,
            grant_lane_open: None,
            prefs_path: None,
            authority_fence: None,
            _authority_lease: None,
            authority_cleanup_armed: false,
        }
    }

    fn new_with_authority(
        hub: BroadcastHub,
        grant_lane_open: Arc<AtomicBool>,
        prefs_path: PathBuf,
        authority_fence: crate::IssueMonitorAuthorityFence,
        authority_lease: crate::IssueMonitorAuthorityLease,
    ) -> Self {
        Self {
            hub,
            grant_lane_open: Some(grant_lane_open),
            prefs_path: Some(prefs_path),
            authority_fence: Some(authority_fence),
            _authority_lease: Some(authority_lease),
            authority_cleanup_armed: true,
        }
    }

    #[cfg(test)]
    fn new_with_authority_without_lease(
        hub: BroadcastHub,
        grant_lane_open: Arc<AtomicBool>,
        prefs_path: PathBuf,
        authority_fence: crate::IssueMonitorAuthorityFence,
    ) -> Self {
        Self {
            hub,
            grant_lane_open: Some(grant_lane_open),
            prefs_path: Some(prefs_path),
            authority_fence: Some(authority_fence),
            _authority_lease: None,
            authority_cleanup_armed: true,
        }
    }

    fn disarm_authority_cleanup(&mut self) {
        self.authority_cleanup_armed = false;
    }

    fn authority_fence(&self) -> Option<&crate::IssueMonitorAuthorityFence> {
        self.authority_fence.as_ref()
    }
}

impl Drop for IssueMonitorControlLaneGuard {
    fn drop(&mut self) {
        // Covers panic, cancellation, and every future early-return path. Deny
        // the stable lane gate before detached spawn_blocking work can observe
        // its generation permit, then synchronously persist the revocation.
        if let Some(grant_lane_open) = &self.grant_lane_open {
            grant_lane_open.store(false, Ordering::Release);
        }
        self.hub.close_issue_monitor_controls();
        if !self.authority_cleanup_armed {
            return;
        }
        let Some(prefs_path) = &self.prefs_path else {
            return;
        };
        let Some(authority_fence) = &self.authority_fence else {
            return;
        };
        let _ =
            settle_issue_monitor_effect_authority_for_shutdown(prefs_path, None, authority_fence);
    }
}

struct LoadedDaemonIssueMonitorState {
    monitor: crate::IssueMonitorState,
    recovery_blocked: bool,
    authority_fence: Option<crate::IssueMonitorAuthorityFence>,
    authority_lease: Option<crate::IssueMonitorAuthorityLease>,
}

fn load_issue_monitor_state_for_daemon(
    prefs_path: &Path,
    config: crate::IssueMonitorConfig,
) -> LoadedDaemonIssueMonitorState {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + ISSUE_MONITOR_PREFS_TIMEOUT,
    );
    let authority_fence = crate::IssueMonitorAuthorityFence::current_process();
    match crate::establish_issue_monitor_authority_fence(
        prefs_path,
        &authority_fence,
        crate::process::is_process_alive,
    ) {
        Ok((prefs, authority_lease)) => LoadedDaemonIssueMonitorState {
            monitor: crate::IssueMonitorState::with_prefs(config, prefs),
            recovery_blocked: false,
            authority_fence: Some(authority_fence),
            authority_lease: Some(authority_lease),
        },
        Err(error) => {
            // Invalid prefs, an ambiguous fence, or a live overlapping daemon
            // may retain remote-effect authority. Never publish Ready until the
            // stable prefs lock has established this process's durable fence.
            let prefs = crate::load_issue_monitor_prefs(prefs_path)
                .unwrap_or_else(|_| crate::IssueMonitorPrefs::recovery_default());
            let mut monitor = crate::IssueMonitorState::with_prefs(config, prefs);
            monitor.record_scan_error(
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                format!(
                    "Issue Monitor authority recovery is blocked; automation disabled and durable state preserved: {error}"
                ),
            );
            LoadedDaemonIssueMonitorState {
                monitor,
                recovery_blocked: true,
                authority_fence: None,
                authority_lease: None,
            }
        }
    }
}

fn issue_monitor_shutdown_revoke_marker_path(prefs_path: &Path) -> PathBuf {
    crate::issue_monitor_authority_fence_path(prefs_path)
}

#[cfg(test)]
fn persist_issue_monitor_shutdown_revoke_marker(prefs_path: &Path) -> io::Result<()> {
    crate::persist_legacy_issue_monitor_shutdown_revoke_fence(prefs_path)
}

/// Revoke shutdown authority while the daemon's lifetime fence still records
/// its exact identity. The fence is cleared only after the epoch revocation
/// commits; any unsettled clear keeps Drop armed so it can retry, and a
/// retained fence prevents a replacement daemon from publishing Ready without
/// first revoking the abandoned authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueMonitorShutdownAuthoritySettlement {
    Revoked,
    DeferredToDurableMarker,
    Unsettled,
}

impl IssueMonitorShutdownAuthoritySettlement {
    fn authority_revoked(self) -> bool {
        self == Self::Revoked
    }

    fn cleanup_safely_fenced(self) -> bool {
        self != Self::Unsettled
    }
}

fn settle_issue_monitor_effect_authority_for_shutdown(
    prefs_path: &Path,
    monitor: Option<&mut crate::IssueMonitorState>,
    authority_fence: &crate::IssueMonitorAuthorityFence,
) -> IssueMonitorShutdownAuthoritySettlement {
    let fence_result = match crate::load_issue_monitor_authority_fence(prefs_path) {
        Ok(crate::IssueMonitorAuthorityFenceState::Active(existing))
            if existing == *authority_fence =>
        {
            Ok(())
        }
        Ok(crate::IssueMonitorAuthorityFenceState::Missing) => {
            crate::persist_issue_monitor_authority_fence(prefs_path, authority_fence)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Issue Monitor authority fence identity changed during shutdown",
        )),
        Err(error) => Err(error),
    };
    if let Err(error) = &fence_result {
        tracing::error!(
            error = %error,
            path = %issue_monitor_shutdown_revoke_marker_path(prefs_path).display(),
            "issue monitor lifetime authority fence is not durably owned during shutdown"
        );
    }

    let authority_revoked = match monitor {
        Some(monitor) => revoke_issue_monitor_effect_authority_for_shutdown(prefs_path, monitor),
        None => match crate::load_issue_monitor_prefs(prefs_path) {
            Ok(prefs) => {
                let mut monitor = crate::IssueMonitorState::with_prefs(
                    crate::IssueMonitorConfig::default(),
                    prefs,
                );
                revoke_issue_monitor_effect_authority_for_shutdown(prefs_path, &mut monitor)
            }
            Err(error) => {
                tracing::error!(
                    error = %error,
                    path = %prefs_path.display(),
                    "issue monitor abnormal-exit authority snapshot is unreadable"
                );
                false
            }
        },
    };

    if authority_revoked && fence_result.is_ok() {
        match crate::clear_issue_monitor_authority_fence(prefs_path, authority_fence) {
            Ok(()) => IssueMonitorShutdownAuthoritySettlement::Revoked,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    path = %issue_monitor_shutdown_revoke_marker_path(prefs_path).display(),
                    "issue monitor authority is revoked but lifetime fence clear is unsettled"
                );
                IssueMonitorShutdownAuthoritySettlement::Unsettled
            }
        }
    } else if !authority_revoked && fence_result.is_ok() {
        IssueMonitorShutdownAuthoritySettlement::DeferredToDurableMarker
    } else {
        tracing::error!(
            path = %prefs_path.display(),
            "fatal: issue monitor lifetime fence and authority revocation are both unsettled"
        );
        IssueMonitorShutdownAuthoritySettlement::Unsettled
    }
}

fn revoke_issue_monitor_effect_authority_for_shutdown(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
) -> bool {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + ISSUE_MONITOR_PREFS_TIMEOUT,
    );
    let mut candidate = monitor.clone();
    match crate::mutate_issue_monitor_prefs(prefs_path, |disk| {
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
    handle: tokio::task::JoinHandle<
        Result<crate::IssueMonitorState, crate::issue_monitor_worker::IssueMonitorScanFailure>,
    >,
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
    Instant,
    Result<
        Result<crate::IssueMonitorState, crate::issue_monitor_worker::IssueMonitorScanFailure>,
        tokio::task::JoinError,
    >,
) {
    let Some(scan) = in_flight.as_mut() else {
        return std::future::pending().await;
    };
    (
        scan.revision,
        scan.authority_epoch,
        scan.deadline,
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
    ConfigSet {
        enabled: Option<bool>,
        autonomous_mode: Option<bool>,
        max_active_agents: Option<usize>,
    },
    ClaimLaunchDelivery {
        issue_number: u64,
        delivery_id: String,
        materializer_id: String,
        materializer_pid: u32,
        materializer_window_id: String,
    },
    LaunchDeliveryMaterialized {
        issue_number: u64,
        delivery_id: String,
        materializer_id: String,
        materializer_window_id: String,
    },
    LaunchDeliveryWorkspaceDurable {
        issue_number: u64,
        delivery_id: String,
        materializer_id: String,
        materializer_window_id: String,
    },
    Launched {
        issue_number: u64,
        window_id: String,
        delivery_id: Option<String>,
    },
    LaunchFailed {
        issue_number: u64,
        message: String,
        delivery_id: Option<String>,
        materializer_id: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct AcceptedIssueMonitorControl {
    control_id: String,
    control: IssueMonitorControl,
}

impl AcceptedIssueMonitorControl {
    fn new(control: IssueMonitorControl) -> Self {
        Self {
            control_id: uuid::Uuid::new_v4().to_string(),
            control,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueMonitorControlCommit {
    Committed {
        should_scan: bool,
        authority_changed: bool,
    },
    RetryableFailure,
    TerminalFailure,
}

struct PendingIssueMonitorAuthorityControl {
    accepted: AcceptedIssueMonitorControl,
    completion: Option<IssueMonitorControlCompletion>,
    authorizing: bool,
}

struct PendingIssueMonitorAuthorityControls {
    controls: VecDeque<PendingIssueMonitorAuthorityControl>,
    retry_at: Option<Instant>,
}

impl PendingIssueMonitorAuthorityControls {
    #[cfg(test)]
    fn after_failure(control: IssueMonitorControl) -> Self {
        Self::after_accepted_failure(AcceptedIssueMonitorControl::new(control), None)
    }

    fn after_accepted_failure(
        accepted: AcceptedIssueMonitorControl,
        completion: Option<IssueMonitorControlCompletion>,
    ) -> Self {
        Self {
            controls: VecDeque::from([PendingIssueMonitorAuthorityControl {
                authorizing: issue_monitor_control_is_authorizing(&accepted.control),
                accepted,
                completion,
            }]),
            retry_at: Some(Instant::now() + ISSUE_MONITOR_AUTHORITY_RETRY_DELAY),
        }
    }

    fn terminal(
        accepted: AcceptedIssueMonitorControl,
        completion: Option<IssueMonitorControlCompletion>,
    ) -> Self {
        Self {
            controls: VecDeque::from([PendingIssueMonitorAuthorityControl {
                authorizing: issue_monitor_control_is_authorizing(&accepted.control),
                accepted,
                completion,
            }]),
            retry_at: None,
        }
    }

    #[cfg(test)]
    fn push(&mut self, control: IssueMonitorControl) {
        self.push_accepted_with_completion(AcceptedIssueMonitorControl::new(control), None);
    }

    fn push_accepted_with_completion(
        &mut self,
        accepted: AcceptedIssueMonitorControl,
        completion: Option<IssueMonitorControlCompletion>,
    ) {
        // Preserve the exact accepted order and every receipt. Production
        // stops receiving while this barrier exists, so this in-memory queue
        // remains one item and the bounded transport queue owns backpressure.
        self.controls
            .push_back(PendingIssueMonitorAuthorityControl {
                authorizing: issue_monitor_control_is_authorizing(&accepted.control),
                accepted,
                completion,
            });
    }

    #[cfg(test)]
    fn front(&self) -> Option<&IssueMonitorControl> {
        self.controls.front().map(|entry| &entry.accepted.control)
    }

    fn front_accepted(&self) -> Option<&AcceptedIssueMonitorControl> {
        self.controls.front().map(|entry| &entry.accepted)
    }

    fn front_is_authorizing(&self) -> bool {
        self.controls.front().is_some_and(|entry| entry.authorizing)
    }

    fn retry_at(&self) -> Option<Instant> {
        self.retry_at
    }

    fn committed_front(&mut self) -> (bool, Option<IssueMonitorControlCompletion>, bool) {
        let entry = self.controls.pop_front();
        let authorizing = entry.as_ref().is_some_and(|entry| entry.authorizing);
        let completion = entry.and_then(|entry| entry.completion);
        if self.controls.is_empty() {
            self.retry_at = None;
            (true, completion, authorizing)
        } else {
            self.retry_at = Some(Instant::now());
            (false, completion, authorizing)
        }
    }

    fn retry_failed(&mut self) {
        self.retry_at = Some(Instant::now() + ISSUE_MONITOR_AUTHORITY_RETRY_DELAY);
    }

    fn terminal_failure(&mut self) {
        self.retry_at = None;
    }

    fn is_terminal(&self) -> bool {
        self.retry_at.is_none() && !self.controls.is_empty()
    }

    fn reject_all(&mut self, error: IssueMonitorControlQueueError) {
        for entry in self.controls.drain(..) {
            if let Some(completion) = entry.completion {
                completion.reject(error);
            }
        }
        self.retry_at = None;
    }
}

fn close_issue_monitor_control_lane(
    hub: &BroadcastHub,
    control_rx: &mut mpsc::Receiver<IssueMonitorControlRequest>,
    pending_authority_controls: &mut Option<PendingIssueMonitorAuthorityControls>,
    error: IssueMonitorControlQueueError,
) {
    // State first wakes Starting publishers. Closing the receiver then makes
    // every concurrent send fail, after which both worker-owned and buffered
    // receipts can be deterministically rejected.
    hub.close_issue_monitor_controls();
    control_rx.close();
    if let Some(mut pending) = pending_authority_controls.take() {
        pending.reject_all(error);
    }
    while let Ok(request) = control_rx.try_recv() {
        let (_, completion) = request.into_parts();
        completion.reject(error);
    }
}

fn issue_monitor_control_is_authorizing(control: &IssueMonitorControl) -> bool {
    matches!(
        control,
        IssueMonitorControl::Enabled(_) | IssueMonitorControl::AutonomousMode(_)
    ) || matches!(
        control,
        IssueMonitorControl::ConfigSet {
            enabled,
            autonomous_mode,
            ..
        } if enabled.is_some() || autonomous_mode.is_some()
    )
}

fn apply_or_queue_issue_monitor_control(
    hub: &BroadcastHub,
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    control: IssueMonitorControl,
    effect_permit: &mut IssueMonitorEffectPermit,
    pending_authority_controls: &mut Option<PendingIssueMonitorAuthorityControls>,
    completion: Option<IssueMonitorControlCompletion>,
) -> bool {
    let accepted = AcceptedIssueMonitorControl::new(control);
    let authorizing = issue_monitor_control_is_authorizing(&accepted.control);
    if authorizing {
        // The canonical predicate is not known until the prefs transaction
        // acquires its lock and rebases. Deny first even when the local
        // projection appears to contain the requested value.
        effect_permit.deny();
        if let Some(pending) = pending_authority_controls.as_mut() {
            pending.push_accepted_with_completion(accepted, completion);
            return false;
        }
    }

    match try_apply_accepted_issue_monitor_control_with_disk_migration(
        prefs_path,
        monitor,
        accepted.clone(),
    ) {
        IssueMonitorControlCommit::Committed { should_scan, .. } => {
            if authorizing {
                effect_permit.reopen();
            }
            if let Some(completion) = completion {
                commit_issue_monitor_control_completion(hub, monitor, completion);
            }
            should_scan
        }
        IssueMonitorControlCommit::RetryableFailure => {
            *pending_authority_controls = Some(
                PendingIssueMonitorAuthorityControls::after_accepted_failure(accepted, completion),
            );
            false
        }
        IssueMonitorControlCommit::TerminalFailure => {
            *pending_authority_controls = Some(PendingIssueMonitorAuthorityControls::terminal(
                accepted, completion,
            ));
            false
        }
    }
}

fn reconcile_deferred_grant_after_authority_commit(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    deferred_grant_result: &mut Option<CompletedIssueMonitorEffect>,
    authority_changed: bool,
    queue_drained: bool,
) {
    if !authority_changed && !queue_drained {
        return;
    }
    if let Some(completed) = deferred_grant_result.take() {
        let _ = commit_issue_monitor_effect_result(prefs_path, monitor, completed);
    }
}

fn try_apply_issue_monitor_control(
    monitor: &mut crate::IssueMonitorState,
    control: IssueMonitorControl,
) -> Option<bool> {
    match control {
        IssueMonitorControl::Enabled(enabled) => monitor
            .set_enabled_with_effect_revocation(enabled)
            .map(|_| true),
        IssueMonitorControl::AutonomousMode(enabled) => monitor
            .set_autonomous_mode_with_effect_revocation(enabled)
            .map(|_| true),
        IssueMonitorControl::ConfigSet {
            enabled,
            autonomous_mode,
            max_active_agents,
        } => {
            if enabled == Some(true)
                || autonomous_mode == Some(true)
                || max_active_agents == Some(0)
                || (enabled.is_none() && autonomous_mode.is_none() && max_active_agents.is_none())
            {
                return None;
            }
            let mut candidate = monitor.clone();
            if let Some(enabled) = enabled {
                candidate.set_enabled_with_effect_revocation(enabled)?;
            }
            if let Some(autonomous_mode) = autonomous_mode {
                candidate.set_autonomous_mode_with_effect_revocation(autonomous_mode)?;
            }
            if let Some(max_active_agents) = max_active_agents {
                candidate.set_max_active_agents(max_active_agents);
            }
            *monitor = candidate;
            Some(true)
        }
        control => Some(apply_routine_issue_monitor_control(monitor, control)),
    }
}

#[cfg(test)]
fn apply_issue_monitor_control(
    monitor: &mut crate::IssueMonitorState,
    control: IssueMonitorControl,
) -> bool {
    try_apply_issue_monitor_control(monitor, control).unwrap_or(false)
}

fn apply_routine_issue_monitor_control(
    monitor: &mut crate::IssueMonitorState,
    control: IssueMonitorControl,
) -> bool {
    match control {
        IssueMonitorControl::Enabled(_)
        | IssueMonitorControl::AutonomousMode(_)
        | IssueMonitorControl::ConfigSet { .. } => false,
        IssueMonitorControl::ClaimLaunchDelivery {
            issue_number,
            delivery_id,
            materializer_id,
            materializer_pid,
            materializer_window_id,
        } => {
            let _ = monitor.claim_launch_delivery(
                issue_number,
                &delivery_id,
                &materializer_id,
                materializer_pid,
                &materializer_window_id,
                crate::process::is_host_process_alive,
            );
            false
        }
        IssueMonitorControl::LaunchDeliveryMaterialized {
            issue_number,
            delivery_id,
            materializer_id,
            materializer_window_id,
        } => {
            let _ = monitor.mark_launch_delivery_materialized(
                issue_number,
                &delivery_id,
                &materializer_id,
                &materializer_window_id,
            );
            false
        }
        IssueMonitorControl::LaunchDeliveryWorkspaceDurable {
            issue_number,
            delivery_id,
            materializer_id,
            materializer_window_id,
        } => {
            let _ = monitor.mark_launch_delivery_workspace_durable(
                issue_number,
                &delivery_id,
                &materializer_id,
                &materializer_window_id,
            );
            false
        }
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
            delivery_id,
        } => {
            monitor.complete_active_launch_delivery(issue_number, window_id, delivery_id.as_deref())
        }
        IssueMonitorControl::LaunchFailed {
            issue_number,
            message,
            delivery_id,
            materializer_id,
        } => monitor.record_launch_failed_delivery(
            issue_number,
            message,
            delivery_id.as_deref(),
            materializer_id.as_deref(),
        ),
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

#[cfg(test)]
fn apply_issue_monitor_control_with_disk_migration(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    control: IssueMonitorControl,
) -> bool {
    match try_apply_issue_monitor_control_with_disk_migration(prefs_path, monitor, control) {
        IssueMonitorControlCommit::Committed { should_scan, .. } => should_scan,
        IssueMonitorControlCommit::RetryableFailure
        | IssueMonitorControlCommit::TerminalFailure => false,
    }
}

#[cfg(test)]
fn try_apply_issue_monitor_control_with_disk_migration(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    control: IssueMonitorControl,
) -> IssueMonitorControlCommit {
    try_apply_accepted_issue_monitor_control_with_disk_migration(
        prefs_path,
        monitor,
        AcceptedIssueMonitorControl::new(control),
    )
}

fn try_apply_accepted_issue_monitor_control_with_disk_migration(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    accepted: AcceptedIssueMonitorControl,
) -> IssueMonitorControlCommit {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + ISSUE_MONITOR_PREFS_TIMEOUT,
    );
    let mut applied = None;
    let mut authority_changed = false;
    let monitor_has_exact_receipt = monitor
        .last_control_receipt()
        .is_some_and(|receipt| receipt.control_id == accepted.control_id);
    let mut receipt_convergence_failed = false;
    let recovery_baseline = monitor.prefs();
    let mut candidate = monitor.clone();
    let transaction =
        crate::mutate_issue_monitor_prefs_recovering(prefs_path, &recovery_baseline, |disk| {
            candidate.rebase_daemon_driver_prefs(disk);
            if let Some(receipt) = disk
                .last_control_receipt
                .as_ref()
                .filter(|receipt| receipt.control_id == accepted.control_id)
                .cloned()
            {
                if !monitor_has_exact_receipt {
                    // The rename may have become visible while the prior
                    // parent-directory sync (and the lock-free confirmation
                    // reread) failed. Converge the stale volatile projection by
                    // applying the accepted control exactly once in memory,
                    // then require its complete prefs snapshot to equal the
                    // durable receipt snapshot before ACKing.
                    let mut converged = monitor.clone();
                    converged.rebase_daemon_driver_prefs(disk);
                    let authority_epoch_before = converged.effect_authority_epoch();
                    let converged_result =
                        try_apply_issue_monitor_control(&mut converged, accepted.control.clone());
                    let converged_authority_changed =
                        converged.effect_authority_epoch() != authority_epoch_before;
                    converged.set_last_control_receipt(receipt.clone());
                    if converged_result != Some(receipt.should_scan)
                        || converged_authority_changed != receipt.authority_changed
                        || converged.prefs() != *disk
                    {
                        receipt_convergence_failed = true;
                        return;
                    }
                    candidate = converged;
                }
                applied = Some(Some(receipt.should_scan));
                authority_changed = receipt.authority_changed;
                return;
            }
            let authority_epoch_before = candidate.effect_authority_epoch();
            applied = Some(try_apply_issue_monitor_control(
                &mut candidate,
                accepted.control.clone(),
            ));
            authority_changed = candidate.effect_authority_epoch() != authority_epoch_before;
            if applied.is_some_and(|result| result.is_some()) {
                candidate.set_last_control_receipt(crate::IssueMonitorControlReceipt {
                    control_id: accepted.control_id.clone(),
                    should_scan: applied.flatten().unwrap_or(false),
                    authority_changed,
                });
                *disk = candidate.prefs();
            }
        });
    match transaction {
        Ok(_) => {
            if receipt_convergence_failed {
                monitor.record_control_commit_error(
                    "issue monitor control receipt matched durable state but volatile convergence was not exact; ACK withheld",
                );
                return IssueMonitorControlCommit::RetryableFailure;
            }
            let Some(should_scan) = applied.flatten() else {
                *monitor = candidate;
                monitor.record_control_commit_error(
                    "issue monitor control rejected: effect authority epoch exhausted; automation remains denied",
                );
                return IssueMonitorControlCommit::TerminalFailure;
            };
            *monitor = candidate;
            IssueMonitorControlCommit::Committed {
                should_scan,
                authority_changed,
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "issue monitor control prefs transaction failed; volatile mutation revoked"
            );
            // A parent-directory sync can fail after the atomic rename became
            // visible. Adopt the applied candidate only when a lock-free reread
            // proves that the canonical bytes are the exact full snapshot we
            // attempted and contain this admission's receipt. The receipt stays
            // unACKed behind the pending barrier; this convergence only keeps
            // the same-process retry from replaying against stale volatile state.
            let candidate_prefs = candidate.prefs();
            let renamed_snapshot_is_exact = candidate_prefs
                .last_control_receipt
                .as_ref()
                .is_some_and(|receipt| receipt.control_id == accepted.control_id)
                && crate::load_issue_monitor_prefs(prefs_path)
                    .is_ok_and(|visible| visible == candidate_prefs);
            if renamed_snapshot_is_exact {
                *monitor = candidate;
            }
            monitor.record_control_commit_error(format!(
                "issue monitor control commit failed at prefs-lock stage: {error}"
            ));
            IssueMonitorControlCommit::RetryableFailure
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
            if let Some(config) = payload
                .get("config_set")
                .and_then(serde_json::Value::as_object)
            {
                let enabled = match config.get("enabled") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(value) => Some(value.as_bool()?),
                };
                let autonomous_mode = match config.get("autonomous_mode") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(value) => Some(value.as_bool()?),
                };
                let max_active_agents = match config.get("max_active_agents") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(value) => Some(usize::try_from(value.as_u64()?).ok()?),
                };
                if enabled == Some(true)
                    || autonomous_mode == Some(true)
                    || max_active_agents == Some(0)
                    || (enabled.is_none()
                        && autonomous_mode.is_none()
                        && max_active_agents.is_none())
                {
                    return None;
                }
                return Some(IssueMonitorControl::ConfigSet {
                    enabled,
                    autonomous_mode,
                    max_active_agents,
                });
            }
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
            if let Some(claim) = payload.get("claim_launch_delivery") {
                let issue_number = claim.get("issue_number")?.as_u64()?;
                let delivery_id = claim.get("delivery_id")?.as_str()?.to_string();
                let materializer_id = claim.get("materializer_id")?.as_str()?.to_string();
                let materializer_pid =
                    u32::try_from(claim.get("materializer_pid")?.as_u64()?).ok()?;
                let materializer_window_id =
                    claim.get("materializer_window_id")?.as_str()?.to_string();
                return Some(IssueMonitorControl::ClaimLaunchDelivery {
                    issue_number,
                    delivery_id,
                    materializer_id,
                    materializer_pid,
                    materializer_window_id,
                });
            }
            if let Some(materialized) = payload.get("launch_delivery_materialized") {
                let issue_number = materialized.get("issue_number")?.as_u64()?;
                let delivery_id = materialized.get("delivery_id")?.as_str()?.to_string();
                let materializer_id = materialized.get("materializer_id")?.as_str()?.to_string();
                let materializer_window_id = materialized
                    .get("materializer_window_id")?
                    .as_str()?
                    .to_string();
                return Some(IssueMonitorControl::LaunchDeliveryMaterialized {
                    issue_number,
                    delivery_id,
                    materializer_id,
                    materializer_window_id,
                });
            }
            if let Some(durable) = payload.get("launch_delivery_workspace_durable") {
                let issue_number = durable.get("issue_number")?.as_u64()?;
                let delivery_id = durable.get("delivery_id")?.as_str()?.to_string();
                let materializer_id = durable.get("materializer_id")?.as_str()?.to_string();
                let materializer_window_id =
                    durable.get("materializer_window_id")?.as_str()?.to_string();
                return Some(IssueMonitorControl::LaunchDeliveryWorkspaceDurable {
                    issue_number,
                    delivery_id,
                    materializer_id,
                    materializer_window_id,
                });
            }
            if let Some(launch_failed) = payload.get("launch_failed") {
                let issue_number = launch_failed.get("issue_number")?.as_u64()?;
                let message = launch_failed
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Launch failed")
                    .to_string();
                let delivery_id = launch_failed
                    .get("delivery_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let materializer_id = launch_failed
                    .get("materializer_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                return Some(IssueMonitorControl::LaunchFailed {
                    issue_number,
                    message,
                    delivery_id,
                    materializer_id,
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
                let delivery_id = launched
                    .get("delivery_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                return Some(IssueMonitorControl::Launched {
                    issue_number,
                    window_id,
                    delivery_id,
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
    match spawn_issue_monitor_scan(scope, monitor, gui_connected).await {
        Ok(Ok(scanned)) => scanned,
        Ok(Err(failure)) => scan_failure_fallback(
            preserved,
            failure,
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        ),
        Err(error) => scan_join_failure_fallback(
            preserved,
            error.to_string(),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        ),
    }
}

#[cfg(test)]
fn spawn_issue_monitor_scan(
    scope: RuntimeScope,
    monitor: crate::IssueMonitorState,
    gui_connected: bool,
) -> tokio::task::JoinHandle<
    Result<crate::IssueMonitorState, crate::issue_monitor_worker::IssueMonitorScanFailure>,
> {
    let deadline = Instant::now() + ISSUE_MONITOR_SCAN_TIMEOUT;
    spawn_issue_monitor_scan_with_deadline(scope, monitor, gui_connected, deadline)
}

fn spawn_issue_monitor_scan_with_deadline(
    scope: RuntimeScope,
    monitor: crate::IssueMonitorState,
    gui_connected: bool,
    deadline: Instant,
) -> tokio::task::JoinHandle<
    Result<crate::IssueMonitorState, crate::issue_monitor_worker::IssueMonitorScanFailure>,
> {
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

fn scan_failure_fallback(
    mut preserved: crate::IssueMonitorState,
    failure: crate::issue_monitor_worker::IssueMonitorScanFailure,
    now: String,
) -> crate::IssueMonitorState {
    preserved.record_scan_error(now, failure.to_string());
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

fn persist_daemon_issue_monitor_state(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
) -> bool {
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + ISSUE_MONITOR_PREFS_TIMEOUT,
    );
    let recovery_baseline = monitor.prefs();
    match crate::mutate_issue_monitor_prefs_recovering(prefs_path, &recovery_baseline, |disk| {
        monitor.rebase_daemon_driver_prefs(disk);
        *disk = monitor.prefs();
    }) {
        Ok((_persisted, ())) => true,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "issue monitor daemon prefs transaction failed"
            );
            false
        }
    }
}

fn resume_deferred_restart_reviews(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    deferred_restart_reviews: &mut Vec<crate::AutonomousIssueRecord>,
) {
    if deferred_restart_reviews.is_empty() || !monitor.pending_effects().is_empty() {
        return;
    }
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut candidate = monitor.clone();
    let resumed =
        candidate.resume_inflight_reviews_after_restart_for(deferred_restart_reviews, &now);
    if resumed.is_empty() {
        deferred_restart_reviews.clear();
        return;
    }
    if !persist_daemon_issue_monitor_state(prefs_path, &mut candidate) {
        return;
    }
    *monitor = candidate;
    deferred_restart_reviews.clear();
    tracing::info!(
        issues = ?resumed,
        "issue monitor: resumed startup-deferred reviews after durable effects reconciled"
    );
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
    captured_deadline: Instant,
) -> bool {
    // Keep the scan's absolute deadline ambient through the synchronous prefs
    // transaction. The commit helper installs its own prefs timeout, but the
    // scoped deadline contract takes the minimum, so lock contention cannot
    // let a result that was ready just before the watchdog boundary commit
    // after that boundary.
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(captured_deadline);
    if Instant::now() >= captured_deadline || captured_revision != current_revision {
        return false;
    }
    commit_issue_monitor_scan_if_current(prefs_path, monitor, scanned, captured_authority_epoch)
}

/// Persist the Prepared -> Attempting execution fence before handing an effect
/// to the remote executor. A caller that receives `Some` therefore has a
/// durable `(effect_id, authority_epoch, attempt)` receipt to reconcile after a
/// crash or outcome-ambiguous command.
#[cfg(test)]
fn fence_next_issue_monitor_effect(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
) -> Option<crate::PendingIssueMonitorEffect> {
    fence_next_issue_monitor_effect_with_permit(
        prefs_path,
        monitor,
        &IssueMonitorEffectPermitToken::always_open(),
    )
}

fn fence_next_issue_monitor_effect_with_permit(
    prefs_path: &Path,
    monitor: &mut crate::IssueMonitorState,
    permit: &IssueMonitorEffectPermitToken,
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
                .find(|effect| {
                    effect.state == crate::IssueMonitorEffectState::Prepared
                        && issue_monitor_effect_permitted(effect, permit)
                })
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
    VolatileDenied,
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

#[derive(Debug)]
struct IssueMonitorEffectPermit {
    current_generation: Arc<AtomicBool>,
    lane_open: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
struct IssueMonitorEffectPermitToken {
    generation_open: Arc<AtomicBool>,
    lane_open: Arc<AtomicBool>,
}

impl IssueMonitorEffectPermitToken {
    #[cfg(test)]
    fn always_open() -> Self {
        Self {
            generation_open: Arc::new(AtomicBool::new(true)),
            lane_open: Arc::new(AtomicBool::new(true)),
        }
    }

    fn load(&self, ordering: Ordering) -> bool {
        self.generation_open.load(ordering) && self.lane_open.load(ordering)
    }
}

impl IssueMonitorEffectPermit {
    fn new() -> Self {
        Self {
            current_generation: Arc::new(AtomicBool::new(true)),
            lane_open: Arc::new(AtomicBool::new(true)),
        }
    }

    fn capture(&self) -> IssueMonitorEffectPermitToken {
        IssueMonitorEffectPermitToken {
            generation_open: Arc::clone(&self.current_generation),
            lane_open: Arc::clone(&self.lane_open),
        }
    }

    fn deny(&self) {
        self.current_generation.store(false, Ordering::Release);
    }

    fn reopen(&mut self) {
        self.current_generation = Arc::new(AtomicBool::new(true));
    }

    fn lane_open(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.lane_open)
    }

    fn close_lane(&self) {
        self.lane_open.store(false, Ordering::Release);
        self.deny();
    }
}

fn issue_monitor_effect_is_safety(effect: &crate::PendingIssueMonitorEffect) -> bool {
    matches!(
        effect.payload,
        crate::IssueMonitorEffectPayload::ReleaseClaim { .. }
            | crate::IssueMonitorEffectPayload::DisarmAutoMerge { .. }
    )
}

fn issue_monitor_effect_permitted(
    effect: &crate::PendingIssueMonitorEffect,
    permit: &IssueMonitorEffectPermitToken,
) -> bool {
    issue_monitor_effect_is_safety(effect) || permit.load(Ordering::Acquire)
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
    permit: IssueMonitorEffectPermitToken,
    deadline: Instant,
) -> InFlightIssueMonitorEffect {
    let handle = tokio::task::spawn_blocking(move || {
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline);
        #[cfg(test)]
        if let (Some(started), Some(release)) = (
            std::env::var_os("GWT_TEST_EFFECT_BEFORE_PERMIT_STARTED"),
            std::env::var_os("GWT_TEST_EFFECT_BEFORE_PERMIT_RELEASE"),
        ) {
            let _ = fs::write(started, b"started");
            while !Path::new(&release).exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let execution_now = chrono::Utc::now();
        let outcome = if issue_monitor_effect_permitted(&effect, &permit) {
            execute_issue_monitor_effect(
                &scope,
                &effect,
                authority_current,
                &execution_now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            )
        } else {
            IssueMonitorEffectOutcome::VolatileDenied
        };
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
                        release_claim_mutation(&client, IssueNumber(*issue_number), claim_id, owner)
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
            owner,
        } => IssueMonitorEffectOutcome::Release(match issue_monitor_http_client(scope) {
            Ok(client) => {
                release_claim_mutation(&client, IssueNumber(*issue_number), claim_id, owner)
            }
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
                (_, IssueMonitorEffectOutcome::VolatileDenied) => {
                    if !current_authority {
                        let _ = candidate.complete_pending_effect(&key);
                        settled = true;
                    }
                }
                (
                    crate::IssueMonitorEffectPayload::AcquireClaim {
                        issue_number,
                        owner,
                        ..
                    },
                    IssueMonitorEffectOutcome::Claim(Ok(ClaimAcquireOutcome::Acquired(claim))),
                ) => {
                    let _ = candidate.complete_pending_effect(&key);
                    if current_authority && candidate.config.enabled {
                        let _ = candidate.apply_confirmed_claim(
                            *issue_number,
                            claim.claim_id,
                            owner,
                            &completed.effect.effect_id,
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
                _ => {
                    tracing::error!(
                        effect = %completed.effect.effect_id,
                        "issue monitor effect result/payload mismatch"
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
) -> Result<crate::IssueMonitorState, crate::issue_monitor_worker::IssueMonitorScanFailure> {
    use crate::issue_monitor_worker::{IssueMonitorScanFailure, IssueMonitorScanStage};

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
    let (owner, repo) = crate::issue_monitor_worker::run_scan_stage(
        IssueMonitorScanStage::RemoteResolution,
        || crate::issue_monitor_worker::github_remote_owner_and_repo(&scope.project_root),
    )?;
    let loaded = crate::issue_monitor_worker::run_scan_stage(
        IssueMonitorScanStage::CandidateLoad,
        || {
            crate::issue_monitor_worker::load_open_issue_monitor_candidates_for_repo_path_with_provenance(
                &scope.project_root,
                &owner,
                &repo,
            )
        },
    )?;
    if let Some(error) = &loaded.live_error {
        return Err(IssueMonitorScanFailure::new(
            IssueMonitorScanStage::CandidateLoad,
            format!("live issue list failed; cache proposal discarded: {error}"),
        ));
    }
    let monitor_owner = format!("{}:{}", whoami::username(), std::process::id());
    crate::issue_monitor_worker::scan_loaded_issue_monitor_candidates(
        &mut monitor,
        &loaded,
        &scope.project_root,
        &now,
    );
    crate::issue_monitor_worker::run_scan_stage(
        IssueMonitorScanStage::MergeReconciliation,
        || {
            crate::issue_monitor_worker::reconcile_issue_monitor_merges(
                &mut monitor,
                &scope.project_root,
            )
        },
    )?;
    // SPEC #3200 T-041/T-044: autonomous pre-launch eligibility gate + stuck-slot
    // recovery. Both are no-ops unless autonomous mode is on (default OFF keeps
    // the SPEC #3165 human-gated flow unchanged).
    if loaded.authorizes_remote_effects() {
        crate::issue_monitor_worker::try_apply_autonomous_eligibility(
            &mut monitor,
            &loaded.issues,
            &format!("{owner}/{repo}"),
            &scope.project_root,
            &now,
        )?;
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
        crate::issue_monitor_worker::try_advance_autonomous_in_flight(
            &mut monitor,
            &loaded.issues,
            &format!("{owner}/{repo}"),
            &scope.project_root,
            daemon_run_secret(),
            &now,
        )?;
    }
    if loaded.authorizes_remote_effects() && monitor.config.enabled && gui_connected {
        let active_cap = if monitor.has_launch_profile() {
            monitor.config.max_active.max(1)
        } else {
            0
        };
        if monitor.active_count() < active_cap {
            monitor.try_prepare_claim_effects_with_probe(
                &monitor_owner,
                &now,
                active_cap,
                |issue_number| {
                    crate::issue_monitor_worker::try_issue_completed_by_merged_pr(
                        &owner,
                        &repo,
                        issue_number,
                    )
                },
            )?;
        }
    }
    crate::issue_monitor_worker::ensure_scan_deadline(IssueMonitorScanStage::ProposalReturn)?;
    Ok(monitor)
}

fn publish_issue_monitor_payloads(hub: &BroadcastHub, monitor: &mut crate::IssueMonitorState) {
    refresh_issue_monitor_agent_status(hub, monitor);
    let gui_connected = issue_monitor_gui_connected(hub);
    publish_issue_monitor_daemon_payloads(
        hub,
        crate::issue_monitor_worker::issue_monitor_daemon_payloads(monitor, gui_connected),
    );
}

fn publish_issue_monitor_read_only_payloads(
    hub: &BroadcastHub,
    monitor: &crate::IssueMonitorState,
) {
    refresh_issue_monitor_agent_status(hub, monitor);
    publish_issue_monitor_daemon_payloads(
        hub,
        crate::issue_monitor_worker::issue_monitor_read_only_daemon_payloads(monitor),
    );
}

fn refresh_issue_monitor_agent_status(hub: &BroadcastHub, monitor: &crate::IssueMonitorState) {
    hub.set_issue_monitor_status(
        serde_json::to_value(monitor.agent_status())
            .expect("Issue Monitor agent status serializes"),
    );
}

fn commit_issue_monitor_control_completion(
    hub: &BroadcastHub,
    monitor: &crate::IssueMonitorState,
    completion: IssueMonitorControlCompletion,
) {
    refresh_issue_monitor_agent_status(hub, monitor);
    completion.commit();
}

fn publish_issue_monitor_daemon_payloads(
    hub: &BroadcastHub,
    payloads: Vec<crate::issue_monitor_worker::IssueMonitorDaemonPayload>,
) {
    for payload in payloads {
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
                    issue_monitor: hub.issue_monitor_status(),
                };
                if out_tx.send(DaemonFrame::Status(snapshot)).is_err() {
                    break;
                }
            }
            Ok(ClientFrame::Publish { channel, payload }) => {
                if channel == crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL {
                    // Issue Monitor control is a command, not a lossy
                    // notification. Await the bounded worker queue first so an
                    // Ack means the exact frame was accepted. During daemon
                    // startup an unready-worker Ack would make the GUI skip its
                    // safe local fallback and silently discard the control.
                    let (response, queued) =
                        enqueue_issue_monitor_control(&hub, &channel, payload).await;
                    if out_tx.send(response).is_err() {
                        break;
                    }
                    tracing::debug!(
                        target: "gwtd::daemon",
                        %channel,
                        queued,
                        "issue monitor control frame accepted by worker queue"
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

async fn enqueue_issue_monitor_control(
    hub: &BroadcastHub,
    channel: &str,
    payload: serde_json::Value,
) -> (DaemonFrame, usize) {
    let result = hub
        .publish_issue_monitor_control(DaemonFrame::Event {
            channel: channel.to_string(),
            payload,
        })
        .await;
    let (response, queued) = match result {
        Ok(()) => (DaemonFrame::Ack, true),
        Err(error) => (
            DaemonFrame::Error {
                message: error.message().to_string(),
            },
            false,
        ),
    };
    (response, usize::from(queued))
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
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
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
            r###"#!/bin/sh
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
if [ "$GWT_FAKE_GH_MODE" = "merge_fail" ] && [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '%s\n' 'gh merged query failed' >&2
  exit 1
fi
if [ "$GWT_FAKE_GH_MODE" = "merge_success" ] && [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  if [ "$(git rev-parse --is-bare-repository 2>/dev/null)" != "true" ]; then
    printf '%s\n' 'gh merged query ran outside child bare repository' >&2
    exit 1
  fi
  printf '%s\n' '[{"headRefName":"work/issue-43","state":"MERGED"}]'
  exit 0
fi
if [ "$GWT_FAKE_GH_MODE" = "branch_protection_fail" ]; then
  case "$*" in
    *"/branches/"*"/protection"*)
      printf '%s\n' 'operation deadline expired during branch protection' >&2
      exit 1
      ;;
  esac
fi
if [ "$GWT_FAKE_GH_MODE" = "claim_probe_fail" ]; then
  case "$*" in
    *"timelineItems"*)
      printf '%s\n' 'operation deadline expired during linked PR readback' >&2
      exit 1
      ;;
  esac
fi
if [ "$GWT_FAKE_GH_MODE" = "branch_protection_fail" ]; then
  printf '%s\n' '[{"number":43,"title":"Live issue","body":"## Acceptance Criteria\n- [ ] AC-1: verified by tests","labels":[{"name":"auto-improve"},{"name":"auto-merge"}],"state":"OPEN","url":"https://example.test/issues/43"}]'
  exit 0
fi
if [ "$GWT_FAKE_GH_MODE" = "claim_probe_fail" ]; then
  printf '%s\n' '[{"number":43,"title":"Live issue","body":"Live body","labels":[{"name":"auto-improve"}],"state":"OPEN","url":"https://example.test/issues/43"}]'
  exit 0
fi
printf '%s\n' '[{"number":43,"title":"Live issue","body":"Live body","labels":[{"name":"bug"}],"state":"OPEN","url":"https://example.test/issues/43"}]'
exit 0
"###,
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

    async fn commit_issue_monitor_control_with_ack_for_test(
        prefs_path: &Path,
        monitor: &mut crate::IssueMonitorState,
        payload: serde_json::Value,
    ) -> bool {
        let hub = BroadcastHub::new();
        let mut receiver = hub
            .take_issue_monitor_control_receiver()
            .expect("claim daemon control receiver");
        super::refresh_issue_monitor_agent_status(&hub, monitor);
        let publisher = tokio::spawn({
            let hub = hub.clone();
            async move {
                hub.publish_issue_monitor_control(DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload,
                })
                .await
            }
        });
        let request = receiver.recv().await.expect("receive admitted control");
        let (frame, completion) = request.into_parts();
        let DaemonFrame::Event { payload, .. } = frame else {
            panic!("control queue must preserve the event frame");
        };
        let control = decode_issue_monitor_control(payload).expect("decode admitted control");
        let mut effect_permit = super::IssueMonitorEffectPermit::new();
        let mut pending = None;

        let should_scan = super::apply_or_queue_issue_monitor_control(
            &hub,
            prefs_path,
            monitor,
            control,
            &mut effect_permit,
            &mut pending,
            Some(completion),
        );

        assert!(
            pending.is_none(),
            "routine control commits without a barrier"
        );
        assert_eq!(
            publisher.await.expect("publisher task joins"),
            Ok(()),
            "successful receipt is the daemon ACK boundary after durable commit"
        );
        should_scan
    }

    #[tokio::test]
    async fn issue_monitor_control_ack_follows_agent_projection_update() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let initial = crate::IssueMonitorPrefs {
            max_active_agents: 1,
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), initial);
        let hub = BroadcastHub::new();
        let mut receiver = hub
            .take_issue_monitor_control_receiver()
            .expect("claim daemon control receiver");
        hub.set_issue_monitor_status(
            serde_json::to_value(monitor.agent_status()).expect("serialize initial status"),
        );
        let publisher = tokio::spawn({
            let hub = hub.clone();
            async move {
                hub.publish_issue_monitor_control(DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload: crate::runtime_daemon_events::issue_monitor_payload(
                        "control",
                        serde_json::json!({
                            "config_set": {
                                "max_active_agents": 3,
                            }
                        }),
                        std::process::id().wrapping_add(1),
                    ),
                })
                .await
            }
        });
        let request = receiver.recv().await.expect("receive admitted control");
        let (frame, completion) = request.into_parts();
        let DaemonFrame::Event { payload, .. } = frame else {
            panic!("control queue must preserve the event frame");
        };
        let control = decode_issue_monitor_control(payload).expect("decode config control");
        let mut effect_permit = super::IssueMonitorEffectPermit::new();
        let mut pending = None;

        assert!(super::apply_or_queue_issue_monitor_control(
            &hub,
            &prefs_path,
            &mut monitor,
            control,
            &mut effect_permit,
            &mut pending,
            Some(completion),
        ));
        assert!(pending.is_none(), "config control commits immediately");
        assert_eq!(
            publisher.await.expect("publisher task joins"),
            Ok(()),
            "receipt reaches ACK after the durable transaction"
        );
        let projected: crate::IssueMonitorAgentStatus = serde_json::from_value(
            hub.issue_monitor_status()
                .expect("ready daemon exposes agent projection"),
        )
        .expect("deserialize agent projection");

        assert_eq!(projected, monitor.agent_status());
        assert_eq!(projected.max_active, 3);
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

    async fn assert_ambiguous_autonomous_failure_receipt_replays_once(
        failure_payload: serde_json::Value,
    ) {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig {
                enabled: true,
                ..crate::IssueMonitorConfig::default()
            },
            crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        monitor.record_candidate(sample_issue_monitor_issue(42));
        monitor.complete_active_launch(42, "tab-1::agent-42");
        monitor.set_autonomous_phase(42, crate::AutonomousPhase::Implementing);
        monitor.begin_review(42, 99, "abc123");
        crate::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed prefs");

        let hub = BroadcastHub::new();
        let mut receiver = hub
            .take_issue_monitor_control_receiver()
            .expect("claim daemon control receiver");
        let publisher = tokio::spawn({
            let hub = hub.clone();
            async move {
                hub.publish_issue_monitor_control(DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload: crate::runtime_daemon_events::issue_monitor_payload(
                        "control",
                        failure_payload,
                        std::process::id().wrapping_add(1),
                    ),
                })
                .await
            }
        });
        let request = receiver.recv().await.expect("receive admitted control");
        let (frame, completion) = request.into_parts();
        let DaemonFrame::Event { payload, .. } = frame else {
            panic!("control queue must preserve the event frame");
        };
        let control = decode_issue_monitor_control(payload).expect("decode admitted control");
        let fail_once = prefs_path.with_extension("parent-sync-fail-once");
        fs::write(&fail_once, b"fail once").expect("seed parent sync failure trigger");
        let _failure = ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            &prefs_path,
        );
        let mut effect_permit = super::IssueMonitorEffectPermit::new();
        let mut pending = None;

        assert!(!super::apply_or_queue_issue_monitor_control(
            &hub,
            &prefs_path,
            &mut monitor,
            control,
            &mut effect_permit,
            &mut pending,
            Some(completion),
        ));
        assert!(
            !publisher.is_finished(),
            "rename without confirmed parent sync must not ACK the receipt"
        );

        let visible = crate::load_issue_monitor_prefs(&prefs_path)
            .expect("renamed snapshot is visible despite the sync error");
        let first = visible
            .autonomous_records
            .iter()
            .find(|record| record.issue_number == 42)
            .expect("first failure outcome is visible");
        assert_eq!(first.attempts, 1);
        assert_eq!(first.phase, crate::AutonomousPhase::Idle);
        assert!(first.retry_not_before.is_some());
        assert!(
            visible
                .failed_issues
                .iter()
                .all(|failed| failed.issue_number != 42),
            "the first failure used autonomous retry, not a plain terminal failure"
        );
        let live_record = monitor
            .autonomous_record(42)
            .expect("visible rename converges the live monitor before retry");
        assert_eq!(live_record.attempts, 1);
        assert_eq!(live_record.phase, crate::AutonomousPhase::Idle);
        assert!(live_record.retry_not_before.is_some());
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(crate::MonitorInboxState::Queued),
            "the same-process retry starts from the exact visible control outcome"
        );

        let pending_controls = pending
            .as_mut()
            .expect("receipt waits behind retry barrier");
        let retry = super::try_apply_accepted_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut monitor,
            pending_controls
                .front_accepted()
                .expect("pending exact accepted control")
                .clone(),
        );
        assert!(matches!(
            retry,
            super::IssueMonitorControlCommit::Committed { .. }
        ));
        let (drained, completion, _) = pending_controls.committed_front();
        assert!(drained);
        super::commit_issue_monitor_control_completion(
            &hub,
            &monitor,
            completion.expect("receipt completion retained"),
        );
        assert_eq!(
            publisher.await.expect("publisher task joins"),
            Ok(()),
            "the exact receipt ACKs only after the retry commits"
        );
        let projected: crate::IssueMonitorAgentStatus = serde_json::from_value(
            hub.issue_monitor_status()
                .expect("retry ACK retains the live agent projection"),
        )
        .expect("deserialize retry projection");
        assert_eq!(
            projected,
            monitor.agent_status(),
            "retry ACK follows the reconciled agent projection"
        );

        let final_record = monitor
            .autonomous_record(42)
            .expect("retry outcome remains recorded");
        assert_eq!(final_record.attempts, 1, "receipt applies only once");
        assert_eq!(final_record.phase, crate::AutonomousPhase::Idle);
        assert!(final_record.retry_not_before.is_some());
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(crate::MonitorInboxState::Queued),
            "replay preserves the autonomous retry inbox outcome"
        );
        let durable = crate::load_issue_monitor_prefs(&prefs_path).expect("reload final prefs");
        let durable_record = durable
            .autonomous_records
            .iter()
            .find(|record| record.issue_number == 42)
            .expect("durable retry outcome");
        assert_eq!(durable_record.attempts, 1);
        assert_eq!(durable_record.phase, crate::AutonomousPhase::Idle);
        assert!(durable_record.retry_not_before.is_some());
        assert!(
            durable
                .failed_issues
                .iter()
                .all(|failed| failed.issue_number != 42),
            "replay must not replace autonomous retry with plain terminal failure"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn ambiguous_launch_failed_receipt_replays_autonomous_retry_once() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_ambiguous_autonomous_failure_receipt_replays_once(serde_json::json!({
            "launch_failed": {
                "issue_number": 42,
                "message": "independent review could not start",
            }
        }))
        .await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn ambiguous_agent_failed_receipt_replays_autonomous_retry_once() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_ambiguous_autonomous_failure_receipt_replays_once(serde_json::json!({
            "agent_failed": {
                "issue_number": 42,
                "window_id": "tab-1::agent-42",
                "message": "agent exited before review",
            }
        }))
        .await;
    }

    #[test]
    fn distinct_pre_materialization_failure_admission_is_not_deduped_by_last_receipt() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig {
                enabled: true,
                ..crate::IssueMonitorConfig::default()
            },
            crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        monitor.record_candidate(sample_issue_monitor_issue(42));
        monitor.complete_active_launch(42, "tab-1::agent-42");
        monitor.set_autonomous_phase(42, crate::AutonomousPhase::Implementing);
        monitor.begin_review(42, 99, "abc123");
        crate::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed prefs");

        assert!(matches!(
            super::try_apply_issue_monitor_control_with_disk_migration(
                &prefs_path,
                &mut monitor,
                IssueMonitorControl::LaunchFailed {
                    issue_number: 42,
                    message: "review launch failed".to_string(),
                    delivery_id: None,
                    materializer_id: None,
                },
            ),
            super::IssueMonitorControlCommit::Committed { .. }
        ));
        let first_receipt = monitor
            .last_control_receipt()
            .expect("first admission receipt")
            .control_id
            .clone();
        assert_eq!(
            monitor.autonomous_record(42).map(|record| record.phase),
            Some(crate::AutonomousPhase::Idle)
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(crate::MonitorInboxState::Queued)
        );

        // A manual Launch Now can fail before the daemon observes any
        // materializing/launched marker. Its separate admission ID, rather than
        // lifecycle-state heuristics, distinguishes it from the first control's
        // durability retry.
        assert!(matches!(
            super::try_apply_issue_monitor_control_with_disk_migration(
                &prefs_path,
                &mut monitor,
                IssueMonitorControl::LaunchFailed {
                    issue_number: 42,
                    message: "fresh manual launch failed before materialization".to_string(),
                    delivery_id: None,
                    materializer_id: None,
                },
            ),
            super::IssueMonitorControlCommit::Committed { .. }
        ));
        let second_receipt = monitor
            .last_control_receipt()
            .expect("second admission receipt");
        assert_ne!(second_receipt.control_id, first_receipt);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(crate::MonitorInboxState::LaunchFailed)
        );
        assert!(crate::load_issue_monitor_prefs(&prefs_path)
            .expect("reload distinct admission")
            .failed_issues
            .iter()
            .any(|failure| failure.issue_number == 42));
    }

    #[test]
    fn durable_receipt_converges_stale_volatile_state_before_commit_result() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig {
                enabled: true,
                ..crate::IssueMonitorConfig::default()
            },
            crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        monitor.record_candidate(sample_issue_monitor_issue(42));
        monitor.complete_active_launch(42, "tab-1::agent-42");
        monitor.set_autonomous_phase(42, crate::AutonomousPhase::Implementing);
        monitor.begin_review(42, 99, "abc123");
        crate::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed prefs");
        let stale_before_control = monitor.clone();
        let accepted = super::AcceptedIssueMonitorControl::new(IssueMonitorControl::LaunchFailed {
            issue_number: 42,
            message: "review launch failed".to_string(),
            delivery_id: None,
            materializer_id: None,
        });
        let fail_once = prefs_path.with_extension("parent-sync-fail-once");
        fs::write(&fail_once, b"fail once").expect("seed parent sync failure trigger");
        let _failure = ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            &prefs_path,
        );

        assert_eq!(
            super::try_apply_accepted_issue_monitor_control_with_disk_migration(
                &prefs_path,
                &mut monitor,
                accepted.clone(),
            ),
            super::IssueMonitorControlCommit::RetryableFailure
        );
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("visible receipt snapshot")
                .last_control_receipt
                .as_ref()
                .map(|receipt| receipt.control_id.as_str()),
            Some(accepted.control_id.as_str())
        );

        // Model the confirmation reread being unavailable: the pending retry
        // still has the exact admission ID, but the volatile projection is the
        // pre-control snapshot. The receipt branch must converge it before
        // returning Committed.
        monitor = stale_before_control;
        assert!(matches!(
            super::try_apply_accepted_issue_monitor_control_with_disk_migration(
                &prefs_path,
                &mut monitor,
                accepted,
            ),
            super::IssueMonitorControlCommit::Committed { .. }
        ));
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(
            monitor.autonomous_record(42).map(|record| record.phase),
            Some(crate::AutonomousPhase::Idle)
        );
        assert_eq!(monitor.attempt_count(42), 1);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(crate::MonitorInboxState::Queued)
        );
        assert_eq!(
            monitor.prefs(),
            crate::load_issue_monitor_prefs(&prefs_path).expect("reload converged receipt")
        );
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

    #[tokio::test]
    async fn issue_monitor_control_ack_requires_actual_worker_acceptance() {
        let channel = crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL;
        let payload = serde_json::json!({"enabled": true});

        let recovery_hub = BroadcastHub::new();
        recovery_hub.mark_issue_monitor_control_recovery_blocked();
        let (response, queued) =
            super::enqueue_issue_monitor_control(&recovery_hub, channel, payload.clone()).await;
        assert_eq!(queued, 0);
        assert!(matches!(
            response,
            DaemonFrame::Error { message }
                if message == crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_RECOVERY_BLOCKED_ERROR
        ));

        let hub = BroadcastHub::new();
        let mut worker_rx = hub
            .take_issue_monitor_control_receiver()
            .expect("worker receiver");
        let accepted = tokio::spawn({
            let hub = hub.clone();
            let payload = payload.clone();
            async move { super::enqueue_issue_monitor_control(&hub, channel, payload).await }
        });
        let request = worker_rx.recv().await.expect("worker request");
        assert!(
            !accepted.is_finished(),
            "enqueue without a transaction receipt must not ACK"
        );
        assert!(matches!(
            request.frame(),
            DaemonFrame::Event {
                channel: received_channel,
                payload: received_payload,
            } if received_channel == channel && received_payload == &payload
        ));
        request.commit();
        let (response, queued) = accepted.await.expect("publisher joins");
        assert_eq!(queued, 1);
        assert_eq!(response, DaemonFrame::Ack);

        worker_rx.close();
        hub.close_issue_monitor_controls();
        let (response, queued) = super::enqueue_issue_monitor_control(&hub, channel, payload).await;
        assert_eq!(queued, 0);
        assert!(matches!(
            response,
            DaemonFrame::Error { message }
                if message == crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CLOSED_ERROR
        ));
    }

    #[tokio::test]
    async fn issue_monitor_control_worker_panic_closes_state_and_receipt() {
        let hub = BroadcastHub::new();
        let mut worker_rx = hub
            .take_issue_monitor_control_receiver()
            .expect("worker receiver");
        let worker = tokio::spawn({
            let hub = hub.clone();
            async move {
                let request = worker_rx.recv().await.expect("accepted request");
                let _guard = super::IssueMonitorControlLaneGuard::new(hub);
                let _request_owns_uncommitted_receipt = request;
                panic!("simulated issue monitor worker panic");
            }
        });
        let publisher = tokio::spawn({
            let hub = hub.clone();
            async move {
                hub.publish_issue_monitor_control(DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload: serde_json::json!({"enabled": false}),
                })
                .await
            }
        });

        assert!(
            worker.await.is_err(),
            "worker panic is observed by its owner"
        );
        assert_eq!(
            publisher.await.expect("publisher joins"),
            Err(super::IssueMonitorControlQueueError::Closed)
        );
        assert_eq!(
            hub.publish_issue_monitor_control(DaemonFrame::Ack).await,
            Err(super::IssueMonitorControlQueueError::Closed),
            "watch state remains Closed after unwind"
        );
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
    fn routine_controls_invalidate_scan_without_revoking_effects() {
        let attempting = crate::PendingIssueMonitorEffect {
            effect_id: "claim:42:stable".to_string(),
            authority_epoch: 7,
            attempt: 2,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "stable-claim-42".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-27T00:00:00Z".to_string(),
                expires_at: "2026-07-27T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        };
        let base = crate::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            effect_authority_epoch: 7,
            pending_effects: vec![attempting],
            autonomous_records: vec![crate::AutonomousIssueRecord {
                issue_number: 42,
                phase: crate::AutonomousPhase::Implementing,
                active_launch_id: None,
                attempts: 1,
                acceptance_snapshot: None,
                retry_not_before: None,
                last_heartbeat: None,
                pr_number: None,
                reviewed_sha: None,
                review_passed: None,
            }],
            ..crate::IssueMonitorPrefs::default()
        };
        let cases = [
            (
                "heartbeat",
                IssueMonitorControl::Heartbeat {
                    issue_number: 42,
                    at: "2026-07-27T00:05:00Z".to_string(),
                },
                false,
            ),
            ("max active", IssueMonitorControl::MaxActiveAgents(7), true),
            (
                "priority",
                IssueMonitorControl::PriorityOrder(vec![42]),
                true,
            ),
        ];

        for (name, control, expected_scan) in cases {
            let mut monitor = crate::IssueMonitorState::with_prefs(
                crate::IssueMonitorConfig::default(),
                base.clone(),
            );
            assert_eq!(
                super::apply_issue_monitor_control(&mut monitor, control),
                expected_scan,
                "{name} scan request"
            );
            assert_eq!(
                monitor.effect_authority_epoch(),
                base.effect_authority_epoch,
                "{name} must not revoke remote-effect authority"
            );
            assert_eq!(
                monitor.pending_effects(),
                base.pending_effects.as_slice(),
                "{name} must preserve the exact Prepared/Attempting journal"
            );
        }

        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(&prefs_path, &base).expect("seed prefs");
        let mut canonical = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            base.clone(),
        );
        let stale_scan = canonical.clone();
        assert!(!super::accept_completed_issue_monitor_scan(
            &prefs_path,
            &mut canonical,
            stale_scan,
            4,
            5,
            base.effect_authority_epoch,
            Instant::now() + Duration::from_secs(1),
        ));
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("stale revision stays uncommitted")
                .pending_effects,
            base.pending_effects
        );
    }

    #[tokio::test]
    async fn review_verdict_ack_preserves_exact_effect_authority_epoch_and_journal() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let attempting = crate::PendingIssueMonitorEffect {
            effect_id: "claim:77:sc34-review".to_string(),
            authority_epoch: 7,
            attempt: 3,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 77,
                claim_id: "stable-claim-77".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-28T00:00:00Z".to_string(),
                expires_at: "2026-07-28T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-77".to_string()),
            },
        };
        let initial = crate::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            effect_authority_epoch: 7,
            pending_effects: vec![attempting.clone()],
            autonomous_records: vec![crate::AutonomousIssueRecord {
                issue_number: 42,
                phase: crate::AutonomousPhase::Reviewing,
                active_launch_id: Some("review-42".to_string()),
                attempts: 1,
                acceptance_snapshot: None,
                retry_not_before: None,
                last_heartbeat: Some("2026-07-28T00:00:00Z".to_string()),
                pr_number: Some(99),
                reviewed_sha: Some("abc123".to_string()),
                review_passed: None,
            }],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed durable journal");
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            initial.clone(),
        );
        let verdict = r#"{"schema":"gwt-autonomous-review/v1","overall":"pass","criteria":[]}"#;
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

        assert!(
            commit_issue_monitor_control_with_ack_for_test(&prefs_path, &mut monitor, payload)
                .await
        );

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload ACKed prefs");
        assert_eq!(persisted.effect_authority_epoch, 7);
        assert_eq!(persisted.pending_effects, vec![attempting]);
        assert_eq!(
            persisted
                .autonomous_records
                .iter()
                .find(|record| record.issue_number == 42)
                .and_then(|record| record.review_passed),
            Some(true)
        );
    }

    #[tokio::test]
    async fn launched_ack_preserves_exact_effect_authority_epoch_and_journal() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let attempting = crate::PendingIssueMonitorEffect {
            effect_id: "claim:77:sc34-launched".to_string(),
            authority_epoch: 11,
            attempt: 4,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 77,
                claim_id: "stable-claim-77".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-28T00:00:00Z".to_string(),
                expires_at: "2026-07-28T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-77".to_string()),
            },
        };
        let initial = crate::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 11,
            pending_effects: vec![attempting.clone()],
            launching_issues: vec![crate::IssueMonitorLaunchingIssue {
                issue_number: 42,
                claimed_at: Some("2026-07-28T00:00:00Z".to_string()),
            }],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed durable journal");
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            initial.clone(),
        );
        let payload = crate::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({
                "launched": {
                    "issue_number": 42,
                    "window_id": "tab-1::agent-42",
                }
            }),
            std::process::id() + 1,
        );

        assert!(
            commit_issue_monitor_control_with_ack_for_test(&prefs_path, &mut monitor, payload)
                .await
        );

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload ACKed prefs");
        assert_eq!(persisted.effect_authority_epoch, 11);
        assert_eq!(persisted.pending_effects, vec![attempting]);
        assert_eq!(
            persisted.launched_issues,
            vec![crate::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "tab-1::agent-42".to_string(),
            }]
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
    fn issue_monitor_config_set_decodes_and_commits_atomically() {
        let payload = crate::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({
                "config_set": {
                    "enabled": false,
                    "autonomous_mode": false,
                    "max_active_agents": 4,
                }
            }),
            std::process::id() + 1,
        );
        let control = decode_issue_monitor_control(payload).expect("config control");
        assert_eq!(
            control,
            IssueMonitorControl::ConfigSet {
                enabled: Some(false),
                autonomous_mode: Some(false),
                max_active_agents: Some(4),
            }
        );

        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let initial = crate::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            effect_authority_epoch: 7,
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), initial);

        assert!(super::apply_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut monitor,
            control,
        ));
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        assert!(!persisted.enabled);
        assert!(!persisted.autonomous_mode);
        assert_eq!(persisted.max_active_agents, 4);
        assert_eq!(persisted.effect_authority_epoch, 9);
    }

    #[test]
    fn issue_monitor_config_set_decoder_rejects_enabling_controls() {
        for config_set in [
            serde_json::json!({"enabled": true}),
            serde_json::json!({"autonomous_mode": true}),
            serde_json::json!({"max_active_agents": 0}),
            serde_json::json!({}),
        ] {
            let payload = crate::runtime_daemon_events::issue_monitor_payload(
                "control",
                serde_json::json!({"config_set": config_set}),
                std::process::id() + 1,
            );
            assert!(decode_issue_monitor_control(payload).is_none());
        }
    }

    #[test]
    fn issue_monitor_config_set_epoch_overflow_is_atomic() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let initial = crate::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            max_active_agents: 1,
            effect_authority_epoch: u64::MAX,
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed prefs");
        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            initial.clone(),
        );

        assert!(!super::apply_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut monitor,
            IssueMonitorControl::ConfigSet {
                enabled: Some(false),
                autonomous_mode: Some(false),
                max_active_agents: Some(4),
            },
        ));
        assert_eq!(std::fs::read(&prefs_path).expect("prefs bytes"), before);
        assert_eq!(monitor.prefs(), initial);
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
                delivery_id: None,
                materializer_id: None,
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

        let failure = super::scan_issue_monitor_once_blocking(scope, monitor, false)
            .expect_err("missing origin is a typed scan failure");

        assert_eq!(
            failure.stage,
            crate::issue_monitor_worker::IssueMonitorScanStage::RemoteResolution
        );
        let error = failure.detail;
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

        let failure = super::scan_issue_monitor_once_blocking(scope, monitor, false)
            .expect_err("non-GitHub origin is a typed scan failure");

        assert_eq!(
            failure.stage,
            crate::issue_monitor_worker::IssueMonitorScanStage::RemoteResolution
        );
        let error = failure.detail;
        assert_eq!(
            error,
            "Git origin remote is not a GitHub URL: https://example.com/owner/repo.git"
        );
    }

    #[test]
    fn daemon_scan_records_merge_reconciliation_error_and_preserves_active_slot() {
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
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "merge_fail");

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
        let monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                    issue_number: 43,
                    window_id: "window-43".to_string(),
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        );

        let preserved = monitor.clone();
        let failure = super::scan_issue_monitor_once_blocking(scope, monitor, false)
            .expect_err("merge reconciliation failure stays typed");
        assert_eq!(
            failure.stage,
            crate::issue_monitor_worker::IssueMonitorScanStage::MergeReconciliation
        );
        let error = failure.to_string();
        assert!(error.contains("merge-reconciliation"), "{error}");
        assert!(error.contains("gh merged query failed"), "{error}");
        let monitor =
            super::scan_failure_fallback(preserved, failure, "2026-07-28T00:00:00Z".to_string());
        let status = monitor.status_view();
        assert_eq!(
            status.active_count, 1,
            "query failure keeps the active slot"
        );
        assert!(monitor
            .prefs()
            .launched_issues
            .iter()
            .any(|launched| launched.issue_number == 43));
        assert!(!monitor.prefs().merged_issues.contains(&43));
    }

    #[test]
    fn daemon_scan_preserves_branch_protection_transport_failure_stage() {
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
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "branch_protection_fail");

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        commit_initial_branch(&repo);
        git_remote_add_origin(&repo, "https://github.com/example/repo.git");
        let symbolic_ref = gwt_core::process::hidden_command("git")
            .args([
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ])
            .current_dir(&repo)
            .status()
            .expect("set origin HEAD");
        assert!(symbolic_ref.success());
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let preserved = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        crate::save_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root),
            &preserved.prefs(),
        )
        .expect("seed issue monitor prefs");

        let failure = super::scan_issue_monitor_once_blocking(scope, preserved.clone(), false)
            .expect_err("branch-protection transport failure stays typed");

        assert_eq!(
            failure.stage,
            crate::issue_monitor_worker::IssueMonitorScanStage::BranchProtection
        );
        assert!(failure.detail.contains("deadline"), "{failure}");
        assert!(preserved.pending_effects().is_empty());
        assert!(preserved.prefs().autonomous_records.is_empty());
    }

    #[test]
    fn daemon_scan_preserves_claim_completion_readback_failure_stage() {
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
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "claim_probe_fail");

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
        let mut preserved = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 1,
                launch_profile: Some(sample_issue_monitor_profile()),
                ..crate::IssueMonitorPrefs::default()
            },
        );
        preserved.set_gui_connected(true);
        crate::save_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root),
            &preserved.prefs(),
        )
        .expect("seed issue monitor prefs");

        let failure = super::scan_issue_monitor_once_blocking(scope, preserved.clone(), true)
            .expect_err("claim completion readback failure stays typed");

        assert_eq!(
            failure.stage,
            crate::issue_monitor_worker::IssueMonitorScanStage::ClaimCompletionReadback
        );
        assert!(failure.detail.contains("linked PR readback"), "{failure}");
        assert!(preserved.pending_effects().is_empty());
        assert!(preserved.prefs().launching_issues.is_empty());
    }

    #[test]
    fn daemon_scan_reconciles_merged_issue_from_workspace_home_child_bare_repo() {
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
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "merge_success");

        let workspace_home = temp.path().join("workspace");
        let bare_repo = workspace_home.join("repo.git");
        fs::create_dir_all(&workspace_home).expect("create workspace home");
        let init = gwt_core::process::hidden_command("git")
            .args(["init", "--bare", "-q"])
            .arg(&bare_repo)
            .output()
            .expect("git init --bare");
        assert!(
            init.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        git_remote_add_origin(&bare_repo, "https://github.com/example/repo.git");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            workspace_home,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                    issue_number: 43,
                    window_id: "window-43".to_string(),
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        );

        let monitor = super::scan_issue_monitor_once_blocking(scope, monitor, false)
            .expect("merged scan succeeds");

        assert_eq!(monitor.status_view().active_count, 0);
        assert_eq!(
            monitor.inbox_item(43).map(|item| item.state),
            Some(crate::MonitorInboxState::Merged),
            "a positive merged-branch signal from the child bare repo frees the slot"
        );
        assert!(monitor.prefs().merged_issues.contains(&43));
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

        let mut monitor = super::scan_issue_monitor_once_blocking(scope, monitor, false)
            .expect("live scan succeeds");
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

    /// Wait until `path` accumulates at least `expected` newline-terminated
    /// markers, so a test can observe repeated fake-gh invocations rather than
    /// only the first one.
    async fn wait_for_marker_count(path: &Path, expected: usize, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, async {
            loop {
                if fs::read_to_string(path).unwrap_or_default().lines().count() >= expected {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .is_ok()
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // global fake-gh env must stay isolated for the full worker run
    async fn hung_scan_is_cut_at_its_deadline_and_the_driver_scans_again_on_the_next_tick() {
        // Issue #3349: the driver froze forever because a single `gh` call
        // inside the scan never returned. The scan now carries an absolute
        // deadline that kills the hung child, the driver records the expiry as
        // a scan error, and the next tick starts a fresh scan instead of
        // staying silently stalled.
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create gwt home");
        let _home = ScopedGwtHome::set(&home);
        let fake_gh = write_fake_gh_issue_list(temp.path());
        let scan_started_path = temp.path().join("scan-started");
        // Deliberately never created: every fake gh invocation hangs until its
        // deadline terminates the process tree.
        let release_scan_path = temp.path().join("never-released");
        let active_scan_path = temp.path().join("active-scan");
        let overlap_scan_path = temp.path().join("overlap-scan");
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "block");
        let _started = ScopedEnvVar::set("GWT_FAKE_GH_STARTED", &scan_started_path);
        let _release = ScopedEnvVar::set("GWT_FAKE_GH_RELEASE", &release_scan_path);
        let _active = ScopedEnvVar::set("GWT_FAKE_GH_ACTIVE", &active_scan_path);
        let _overlap = ScopedEnvVar::set("GWT_FAKE_GH_OVERLAP", &overlap_scan_path);

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
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed issue monitor prefs");

        let hub = BroadcastHub::new();
        let mut status_rx = hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL);
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = spawn_issue_monitor_worker_with_config_and_timeout(
            scope,
            hub.clone(),
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 1,
                ..crate::IssueMonitorConfig::default()
            },
            Duration::from_millis(1_500),
        );

        let first_scan_started = wait_for_path(&scan_started_path, Duration::from_secs(5)).await;
        let expired_status =
            recv_issue_monitor_status_matching(&mut status_rx, Duration::from_secs(8), |status| {
                status.last_error.as_deref().is_some_and(|error| {
                    error.contains("deadline")
                        || error.contains("timed out")
                        || error.contains("watchdog")
                })
            })
            .await;
        let driver_recovered =
            wait_for_marker_count(&scan_started_path, 2, Duration::from_secs(10)).await;

        shutdown.request();
        tokio::time::timeout(Duration::from_secs(10), worker)
            .await
            .expect("worker shutdown is bounded")
            .expect("worker exits cleanly");

        assert!(
            first_scan_started,
            "the hanging fake gh must be reached by the first scan"
        );
        assert!(
            expired_status.is_some(),
            "the expired scan must surface as an operator-visible scan error"
        );
        assert!(
            driver_recovered,
            "the driver must start a fresh scan after the hung one is cut at its deadline"
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
        let heartbeat_queued = hub
            .publish_issue_monitor_control(DaemonFrame::Event {
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
            })
            .await
            .is_ok();
        let max_active_queued = hub
            .publish_issue_monitor_control(DaemonFrame::Event {
                channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
                payload: crate::runtime_daemon_events::issue_monitor_payload(
                    "control",
                    serde_json::json!({"max_active_agents": 7}),
                    source_pid,
                ),
            })
            .await
            .is_ok();

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
        let disabled_queued = hub
            .publish_issue_monitor_control(DaemonFrame::Event {
                channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
                payload: crate::runtime_daemon_events::issue_monitor_payload(
                    "control",
                    serde_json::json!({"enabled": false}),
                    source_pid,
                ),
            })
            .await
            .is_ok();
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
        assert!(heartbeat_queued, "worker must receive controls");
        assert!(max_active_queued, "worker must receive controls");
        assert!(disabled_queued, "OFF control must reach the worker");
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
    async fn valid_startup_off_commits_before_any_stale_prepared_grant_starts() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create gwt home");
        let _home = ScopedGwtHome::set(&home);
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_git_repo(&repo);
        git_remote_add_origin(&repo, "https://github.com/example/repo.git");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let grant = crate::PendingIssueMonitorEffect::prepared(
            "claim:42:startup",
            7,
            crate::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "startup-claim-42".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-28T00:00:00Z".to_string(),
                expires_at: "2026-07-28T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        );
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                effect_authority_epoch: 7,
                pending_effects: vec![grant],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed stale startup grant");
        let client_marker = temp.path().join("claim-http-client-started");
        let _client_marker =
            ScopedEnvVar::set("GWT_TEST_ISSUE_MONITOR_HTTP_CLIENT_MARKER", &client_marker);
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
            Duration::from_secs(1),
        );

        hub.publish_issue_monitor_control(DaemonFrame::Event {
            channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
            payload: crate::runtime_daemon_events::issue_monitor_payload(
                "control",
                serde_json::json!({"enabled": false}),
                std::process::id().wrapping_add(1),
            ),
        })
        .await
        .expect("OFF ACK follows durable commit");

        let committed = crate::load_issue_monitor_prefs(&prefs_path).expect("reload OFF prefs");
        assert!(!committed.enabled);
        assert_eq!(committed.effect_authority_epoch, 8);
        assert!(committed.pending_effects.is_empty());
        assert!(
            !client_marker.exists(),
            "stale startup snapshot must not begin a grant before OFF commits"
        );
        shutdown.request();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .expect("worker shutdown is bounded")
            .expect("worker exits cleanly");
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
        hub.publish_issue_monitor_control(DaemonFrame::Event {
            channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
            payload: crate::runtime_daemon_events::issue_monitor_payload(
                "control",
                serde_json::json!({"autonomous_mode": false}),
                source_pid,
            ),
        })
        .await
        .expect("worker accepts autonomous OFF");
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
        assert_eq!(persisted.pending_effects, vec![prepared.clone()]);
        let shutdown_marker = super::issue_monitor_shutdown_revoke_marker_path(&prefs_path);
        assert!(
            !shutdown_marker.exists(),
            "a worker that never established its fence must not create one outside the prefs lock"
        );

        let replacement = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );
        assert!(
            !replacement.recovery_blocked,
            "replacement establishes the first lifetime fence after the lock is released"
        );
        let replayed = crate::load_issue_monitor_prefs(&prefs_path).expect("reload replayed prefs");
        assert_eq!(replayed.effect_authority_epoch, 7);
        assert_eq!(
            replayed.pending_effects,
            vec![prepared],
            "a recovery-blocked non-owner cannot revoke a predecessor's durable obligation"
        );
        assert!(matches!(
            crate::load_issue_monitor_authority_fence(&prefs_path)
                .expect("load replacement authority fence"),
            crate::IssueMonitorAuthorityFenceState::Active(_)
        ));
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
    async fn daemon_shutdown_request_between_sticky_check_and_await_is_observed() {
        let shutdown = DaemonShutdown::new();
        assert!(!shutdown.requested.load(Ordering::Acquire));
        let notified = shutdown.notify.notified();
        assert!(!shutdown.requested.load(Ordering::Acquire));
        // Tokio snapshots the notify_waiters generation when Notified is
        // created, so a broadcast before its first poll remains observable.
        shutdown.request();

        let observed = tokio::time::timeout(Duration::from_millis(50), notified).await;

        observed.expect(
            "Notified must observe notify_waiters after its generation snapshot and before polling",
        );
    }

    #[test]
    fn startup_marker_epoch_exhaustion_is_recovery_blocked_and_retained() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                effect_authority_epoch: u64::MAX,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed exhausted authority");
        let before = fs::read(&prefs_path).expect("read seeded prefs");
        super::persist_issue_monitor_shutdown_revoke_marker(&prefs_path)
            .expect("persist shutdown marker");
        let marker = super::issue_monitor_shutdown_revoke_marker_path(&prefs_path);

        let loaded = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );

        assert!(loaded.recovery_blocked);
        assert!(marker.exists());
        assert_eq!(fs::read(&prefs_path).expect("reload prefs"), before);
        assert!(loaded
            .monitor
            .status_view()
            .last_error
            .is_some_and(|error| error.contains("authority recovery is blocked")));
    }

    #[test]
    fn startup_without_marker_persists_lifetime_authority_fence_before_ready() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");

        let loaded = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );

        assert!(!loaded.recovery_blocked);
        let marker = super::issue_monitor_shutdown_revoke_marker_path(&prefs_path);
        let fence: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker).expect("read active fence"))
                .expect("active fence JSON");
        assert_eq!(
            fence.get("pid").and_then(serde_json::Value::as_u64),
            Some(u64::from(std::process::id()))
        );
        assert!(fence
            .get("instance_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| !id.is_empty()));
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload prefs")
                .effect_authority_epoch,
            7,
            "a clean startup fence does not fabricate crash recovery"
        );
    }

    #[test]
    fn startup_lifetime_lease_blocks_overlap_and_recovers_after_owner_drop() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");

        let first = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );
        assert!(!first.recovery_blocked);

        let overlap = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );
        assert!(overlap.recovery_blocked);
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("overlap preserves authority epoch")
                .effect_authority_epoch,
            7,
        );

        drop(first);
        let replacement = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );
        assert!(!replacement.recovery_blocked);
        assert_eq!(
            replacement.monitor.effect_authority_epoch(),
            8,
            "a free lifetime lock proves the retained current fence is stale",
        );
    }

    #[tokio::test]
    async fn recovery_blocked_worker_never_publishes_or_drains_launch_delivery() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let mut seeded = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            ..crate::IssueMonitorConfig::default()
        });
        seeded.record_candidate(sample_issue_monitor_issue(42));
        assert!(seeded.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));
        crate::save_issue_monitor_prefs(&prefs_path, &seeded.prefs()).expect("seed delivery");
        let authority_owner = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );
        assert!(!authority_owner.recovery_blocked);

        let hub = BroadcastHub::new();
        let mut events = hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL);
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = super::spawn_issue_monitor_worker_with_config(
            scope,
            hub,
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig::default(),
        );

        let deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        let mut published_events = Vec::new();
        while let Ok(Ok(DaemonFrame::Event { payload, .. })) =
            tokio::time::timeout_at(deadline, events.recv()).await
        {
            if let Some(event) = payload.get("event").and_then(serde_json::Value::as_str) {
                published_events.push(event.to_string());
            }
        }

        shutdown.request();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .expect("worker shutdown is bounded")
            .expect("worker exits cleanly");
        assert!(
            published_events
                .iter()
                .all(|event| event == "status" || event == "inbox"),
            "recovery-blocked worker published delivery events: {published_events:?}",
        );
        assert!(published_events.iter().any(|event| event == "status"));
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload delivery")
                .pending_launch_deliveries
                .len(),
            1,
            "read-only recovery projection must not drain the durable outbox",
        );
        drop(authority_owner);
    }

    #[test]
    fn malformed_lifetime_authority_fence_blocks_ready_and_is_retained() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let marker = super::issue_monitor_shutdown_revoke_marker_path(&prefs_path);
        let malformed = b"{\"version\":1,";
        fs::write(&marker, malformed).expect("seed malformed fence");

        let loaded = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );

        assert!(loaded.recovery_blocked);
        assert_eq!(fs::read(&marker).expect("reload fence"), malformed);
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload prefs")
                .effect_authority_epoch,
            7
        );
    }

    #[test]
    fn live_lifetime_authority_fence_blocks_overlapping_ready() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let marker = super::issue_monitor_shutdown_revoke_marker_path(&prefs_path);
        let live = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "pid": std::process::id(),
            "instance_id": "already-live-daemon"
        }))
        .expect("serialize live fence");
        fs::write(&marker, &live).expect("seed live fence");

        let loaded = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );

        assert!(loaded.recovery_blocked);
        assert_eq!(fs::read(&marker).expect("reload fence"), live);
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload prefs")
                .effect_authority_epoch,
            7
        );
    }

    #[test]
    fn dead_lifetime_authority_fence_revokes_then_is_replaced_before_ready() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let marker = super::issue_monitor_shutdown_revoke_marker_path(&prefs_path);
        fs::write(
            &marker,
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "pid": i32::MAX as u32,
                "instance_id": "dead-daemon"
            }))
            .expect("serialize dead fence"),
        )
        .expect("seed dead fence");

        let loaded = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );

        assert!(!loaded.recovery_blocked);
        assert_eq!(loaded.monitor.effect_authority_epoch(), 8);
        let current: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker).expect("read current fence"))
                .expect("parse current fence");
        assert_eq!(
            current.get("pid").and_then(serde_json::Value::as_u64),
            Some(u64::from(std::process::id()))
        );
        assert_ne!(
            current
                .get("instance_id")
                .and_then(serde_json::Value::as_str),
            Some("dead-daemon")
        );
    }

    #[test]
    fn startup_marker_lock_timeout_is_recovery_blocked_until_retry() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed authority");
        let before = fs::read(&prefs_path).expect("read seeded prefs");
        super::persist_issue_monitor_shutdown_revoke_marker(&prefs_path)
            .expect("persist shutdown marker");
        let marker = super::issue_monitor_shutdown_revoke_marker_path(&prefs_path);
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);

        let started = Instant::now();
        let blocked = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );

        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(blocked.recovery_blocked);
        assert!(marker.exists());
        assert_eq!(fs::read(&prefs_path).expect("reload locked prefs"), before);
        FileExt::unlock(&lock).expect("release prefs lock");

        let retried = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );
        assert!(!retried.recovery_blocked);
        assert_eq!(retried.monitor.effect_authority_epoch(), 8);
        assert!(matches!(
            crate::load_issue_monitor_authority_fence(&prefs_path)
                .expect("load replacement authority fence"),
            crate::IssueMonitorAuthorityFenceState::Active(_)
        ));
    }

    #[test]
    fn startup_marker_revokes_attempting_grant_and_persists_compensation() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
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
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed attempting authority");
        super::persist_issue_monitor_shutdown_revoke_marker(&prefs_path)
            .expect("persist shutdown marker");

        let loaded = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );

        assert!(!loaded.recovery_blocked);
        let replayed = crate::load_issue_monitor_prefs(&prefs_path).expect("reload replayed prefs");
        assert_eq!(replayed.effect_authority_epoch, 8);
        assert!(replayed.pending_effects.iter().any(|effect| matches!(
            &effect.payload,
            crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                compensates_effect_id,
                ..
            } if compensates_effect_id == "arm:42:99:abc:7"
        )));
        assert!(replayed.pending_effects.iter().all(|effect| {
            !matches!(
                effect.payload,
                crate::IssueMonitorEffectPayload::ArmAutoMerge { .. }
            ) || effect.authority_epoch < replayed.effect_authority_epoch
        }));
        assert!(matches!(
            crate::load_issue_monitor_authority_fence(&prefs_path)
                .expect("load replacement authority fence"),
            crate::IssueMonitorAuthorityFenceState::Active(_)
        ));
    }

    #[test]
    fn abnormal_lane_drop_leaves_marker_until_replacement_revokes_authority() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let authority_fence = crate::IssueMonitorAuthorityFence::current_process();
        crate::persist_issue_monitor_authority_fence(&prefs_path, &authority_fence)
            .expect("persist active fence");
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let lane_open = Arc::new(std::sync::atomic::AtomicBool::new(true));
        {
            let _guard = super::IssueMonitorControlLaneGuard::new_with_authority_without_lease(
                BroadcastHub::new(),
                Arc::clone(&lane_open),
                prefs_path.clone(),
                authority_fence,
            );
        }

        assert!(!lane_open.load(Ordering::Acquire));
        let marker = super::issue_monitor_shutdown_revoke_marker_path(&prefs_path);
        assert!(marker.exists());
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload locked prefs")
                .effect_authority_epoch,
            7
        );
        FileExt::unlock(&lock).expect("release prefs lock");

        let mut exited_fence = match crate::load_issue_monitor_authority_fence(&prefs_path)
            .expect("load exited daemon fence")
        {
            crate::IssueMonitorAuthorityFenceState::Active(fence) => fence,
            state => panic!("expected retained active fence, got {state:?}"),
        };
        exited_fence.pid = i32::MAX as u32;
        crate::persist_issue_monitor_authority_fence(&prefs_path, &exited_fence)
            .expect("model prior daemon process exit");

        let replacement = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );
        assert!(!replacement.recovery_blocked);
        assert_eq!(replacement.monitor.effect_authority_epoch(), 8);
        assert!(matches!(
            crate::load_issue_monitor_authority_fence(&prefs_path)
                .expect("load replacement authority fence"),
            crate::IssueMonitorAuthorityFenceState::Active(_)
        ));
    }

    #[test]
    fn shutdown_marker_and_revoke_double_failure_is_typed_unsettled() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        let marker = super::issue_monitor_shutdown_revoke_marker_path(&prefs_path);
        fs::create_dir(&marker).expect("make marker replacement fail");
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let authority_fence = crate::IssueMonitorAuthorityFence::current_process();

        let settlement = super::settle_issue_monitor_effect_authority_for_shutdown(
            &prefs_path,
            Some(&mut monitor),
            &authority_fence,
        );
        FileExt::unlock(&lock).expect("release prefs lock");

        assert_eq!(
            settlement,
            super::IssueMonitorShutdownAuthoritySettlement::Unsettled
        );
        assert!(!settlement.cleanup_safely_fenced());
        assert!(marker.is_dir());
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload prefs")
                .effect_authority_epoch,
            7
        );

        let replacement = super::load_issue_monitor_state_for_daemon(
            &prefs_path,
            crate::IssueMonitorConfig::default(),
        );
        assert!(replacement.recovery_blocked);
        assert!(marker.is_dir(), "ambiguous fence evidence is retained");
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload prefs after blocked replacement")
                .effect_authority_epoch,
            7,
            "replacement grants no authority after persistent double failure"
        );
    }

    #[test]
    fn shutdown_fence_unlink_parent_sync_failure_keeps_drop_retry_armed() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let authority_fence = crate::IssueMonitorAuthorityFence::current_process();
        crate::persist_issue_monitor_authority_fence(&prefs_path, &authority_fence)
            .expect("persist lifetime authority fence");
        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        let lane_open = Arc::new(AtomicBool::new(true));
        let mut guard = super::IssueMonitorControlLaneGuard::new_with_authority_without_lease(
            BroadcastHub::new(),
            Arc::clone(&lane_open),
            prefs_path.clone(),
            authority_fence.clone(),
        );
        let fail_once = temp.path().join("fail-fence-parent-sync-once");
        fs::write(&fail_once, b"fail once").expect("seed failure trigger");
        let _failure = ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_FENCE_PARENT_SYNC_ONCE",
            &fail_once,
        );

        let settlement = super::settle_issue_monitor_effect_authority_for_shutdown(
            &prefs_path,
            Some(&mut monitor),
            &authority_fence,
        );
        if settlement.cleanup_safely_fenced() {
            guard.disarm_authority_cleanup();
        }

        assert_eq!(
            settlement,
            super::IssueMonitorShutdownAuthoritySettlement::Unsettled
        );
        assert!(!settlement.cleanup_safely_fenced());
        assert!(matches!(
            crate::load_issue_monitor_authority_fence(&prefs_path).expect("load unlinked fence"),
            crate::IssueMonitorAuthorityFenceState::Missing
        ));
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload first revocation")
                .effect_authority_epoch,
            8
        );

        drop(guard);

        assert!(!lane_open.load(Ordering::Acquire));
        assert!(matches!(
            crate::load_issue_monitor_authority_fence(&prefs_path)
                .expect("load fence after Drop retry"),
            crate::IssueMonitorAuthorityFenceState::Missing
        ));
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload Drop revocation")
                .effect_authority_epoch,
            9,
            "Drop retries the unsettled clear under a newly persisted fence"
        );
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
            Ok(late)
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
        let (captured_revision, captured_epoch, captured_deadline, result) = tokio::time::timeout(
            Duration::from_secs(1),
            super::wait_for_issue_monitor_scan(&mut in_flight),
        )
        .await
        .expect("started scan eventually joins");
        assert_eq!(captured_revision, 7);
        assert_eq!(captured_epoch, 11);
        let late_result = result
            .expect("late scan task joins")
            .expect("late scan result remains inspectable");
        assert_eq!(late_result.status_view().max_active_agents, 9);
        let revision_after_watchdog = 8;
        assert!(!super::accept_completed_issue_monitor_scan(
            &prefs_path,
            &mut canonical,
            late_result,
            captured_revision,
            revision_after_watchdog,
            captured_epoch,
            captured_deadline,
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
            Instant::now() + Duration::from_secs(1),
        ));
        assert_eq!(
            canonical.status_view().last_scan_at.as_deref(),
            Some("2026-07-27T00:00:01Z")
        );
        assert_eq!(canonical.status_view().last_error, None);
    }

    #[test]
    fn completed_scan_at_deadline_is_rejected_without_persisting_proposals() {
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
        let mut scanned = canonical.clone();
        assert!(
            scanned.prepare_effect(crate::PendingIssueMonitorEffect::prepared(
                "claim:42:late",
                7,
                crate::IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: 42,
                    claim_id: "claim-late".to_string(),
                    owner: "host/session".to_string(),
                    heartbeat_at: "2026-07-28T00:00:00Z".to_string(),
                    expires_at: "2026-07-28T00:30:00Z".to_string(),
                    launched_work_id: Some("work/issue-42".to_string()),
                },
            ))
        );

        assert!(!super::accept_completed_issue_monitor_scan(
            &prefs_path,
            &mut canonical,
            scanned,
            9,
            9,
            7,
            Instant::now(),
        ));
        assert!(crate::load_issue_monitor_prefs(&prefs_path)
            .expect("reload prefs")
            .pending_effects
            .is_empty());
        assert!(canonical.pending_effects().is_empty());
    }

    #[test]
    fn completed_scan_cannot_wait_past_deadline_for_prefs_lock_then_commit() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let initial = crate::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 7,
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed prefs");
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let mut canonical = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            initial.clone(),
        );
        let mut scanned = canonical.clone();
        assert!(
            scanned.prepare_effect(crate::PendingIssueMonitorEffect::prepared(
                "claim:42:lock-boundary",
                7,
                crate::IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: 42,
                    claim_id: "claim-lock-boundary".to_string(),
                    owner: "host/session".to_string(),
                    heartbeat_at: "2026-07-28T00:00:00Z".to_string(),
                    expires_at: "2026-07-28T00:30:00Z".to_string(),
                    launched_work_id: Some("work/issue-42".to_string()),
                },
            ))
        );
        let deadline = Instant::now() + Duration::from_millis(30);
        let thread_prefs_path = prefs_path.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let join = std::thread::spawn(move || {
            started_tx.send(()).expect("signal accept start");
            let accepted = super::accept_completed_issue_monitor_scan(
                &thread_prefs_path,
                &mut canonical,
                scanned,
                9,
                9,
                7,
                deadline,
            );
            (accepted, canonical)
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("accept thread started");
        std::thread::sleep(Duration::from_millis(80));
        FileExt::unlock(&lock).expect("release prefs lock after scan deadline");
        let (accepted, canonical) = join.join().expect("accept thread joins");

        assert!(!accepted);
        assert!(canonical.pending_effects().is_empty());
        assert!(crate::load_issue_monitor_prefs(&prefs_path)
            .expect("reload prefs")
            .pending_effects
            .is_empty());
    }

    #[test]
    fn typed_scan_failure_preserves_the_canonical_state_and_journal() {
        let effect = crate::PendingIssueMonitorEffect::prepared(
            "release:claim:42",
            7,
            crate::IssueMonitorEffectPayload::ReleaseClaim {
                issue_number: 42,
                claim_id: "claim-42".to_string(),
                owner: "host/session".to_string(),
            },
        );
        let preserved = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                enabled: true,
                effect_authority_epoch: 7,
                pending_effects: vec![effect.clone()],
                ..crate::IssueMonitorPrefs::default()
            },
        );
        let failure = crate::issue_monitor_worker::IssueMonitorScanFailure::new(
            crate::issue_monitor_worker::IssueMonitorScanStage::BranchProtection,
            "operation deadline expired",
        );

        let failed =
            super::scan_failure_fallback(preserved, failure, "2026-07-28T00:00:00Z".to_string());

        assert_eq!(failed.effect_authority_epoch(), 7);
        assert_eq!(failed.pending_effects(), &[effect]);
        assert!(failed
            .status_view()
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("branch-protection")));
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
                    owner: "host/session".to_string(),
                },
            );
            let mut queued = super::spawn_issue_monitor_effect(
                scope,
                effect,
                true,
                super::IssueMonitorEffectPermitToken::always_open(),
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
    #[allow(clippy::await_holding_lock)]
    fn worker_abort_denies_detached_grant_and_durably_revokes_authority() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            // The worker schedules its immediate first scan before the
            // prepared effect. Keep a separate blocking slot so this test
            // deterministically reaches the started-effect abort boundary;
            // the queued-effect case is covered by the adjacent test.
            .max_blocking_threads(2)
            .enable_all()
            .build()
            .expect("runtime");
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create home");
        let _home = ScopedGwtHome::set(&home);
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
        let grant = crate::PendingIssueMonitorEffect::prepared(
            "claim:42:abort",
            7,
            crate::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "abort-claim-42".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-28T00:00:00Z".to_string(),
                expires_at: "2026-07-28T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        );
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                effect_authority_epoch: 7,
                pending_effects: vec![grant],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prepared grant");
        let client_marker = temp.path().join("claim-http-client-started");
        let effect_started = temp.path().join("effect-before-permit-started");
        let effect_release = temp.path().join("effect-before-permit-release");
        let _client_marker =
            ScopedEnvVar::set("GWT_TEST_ISSUE_MONITOR_HTTP_CLIENT_MARKER", &client_marker);
        let _effect_started =
            ScopedEnvVar::set("GWT_TEST_EFFECT_BEFORE_PERMIT_STARTED", &effect_started);
        let _effect_release =
            ScopedEnvVar::set("GWT_TEST_EFFECT_BEFORE_PERMIT_RELEASE", &effect_release);

        runtime.block_on(async {
            let hub = BroadcastHub::new();
            let shutdown = Arc::new(DaemonShutdown::new());
            let worker = spawn_issue_monitor_worker_with_config_and_timeout(
                scope,
                hub,
                shutdown,
                crate::IssueMonitorConfig {
                    poll_interval_secs: 60,
                    ..crate::IssueMonitorConfig::default()
                },
                Duration::from_secs(2),
            );
            assert!(
                wait_for_path(&effect_started, Duration::from_secs(1)).await,
                "grant executor starts and pauses before its final permit check"
            );

            worker.abort();
            assert!(worker
                .await
                .expect_err("worker is cancelled")
                .is_cancelled());
            fs::write(&effect_release, b"release").expect("release started effect");
            tokio::time::sleep(Duration::from_millis(100)).await;
        });

        assert!(
            !client_marker.exists(),
            "abnormal worker exit must deny a detached grant before remote mutation"
        );
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload revoked prefs");
        assert_eq!(persisted.effect_authority_epoch, 8);
        assert!(persisted.pending_effects.iter().any(|effect| matches!(
            &effect.payload,
            crate::IssueMonitorEffectPayload::ReleaseClaim {
                issue_number: 42,
                claim_id,
                owner,
            } if claim_id == "abort-claim-42" && owner == "host/session"
        )));
    }

    #[test]
    #[allow(clippy::await_holding_lock)]
    fn off_lock_timeout_denies_grant_already_queued_in_blocking_pool() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        git_remote_add_origin(&repo, "https://github.com/example/repo.git");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            repo,
            RuntimeTarget::Host,
        )
        .expect("scope");
        let client_marker = temp.path().join("claim-http-client-started");
        let _client_marker =
            ScopedEnvVar::set("GWT_TEST_ISSUE_MONITOR_HTTP_CLIENT_MARKER", &client_marker);
        let prefs_path = temp.path().join("issue-monitor.json");
        let prefs = crate::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 7,
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        let prefs_lock = issue_monitor_prefs_lock_for_test(&prefs_path);

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

            let permit = super::IssueMonitorEffectPermit::new();
            let effect = crate::PendingIssueMonitorEffect {
                effect_id: "claim:42:queued".to_string(),
                authority_epoch: 7,
                attempt: 1,
                state: crate::IssueMonitorEffectState::Attempting,
                payload: crate::IssueMonitorEffectPayload::AcquireClaim {
                    issue_number: 42,
                    claim_id: "queued-claim-42".to_string(),
                    owner: "host/session".to_string(),
                    heartbeat_at: "2026-07-27T00:00:00Z".to_string(),
                    expires_at: "2026-07-27T00:30:00Z".to_string(),
                    launched_work_id: Some("work/issue-42".to_string()),
                },
            };
            let mut queued = super::spawn_issue_monitor_effect(
                scope,
                effect,
                true,
                permit.capture(),
                Instant::now() + Duration::from_secs(2),
            );

            permit.deny();
            assert!(!super::apply_issue_monitor_control_with_disk_migration(
                &prefs_path,
                &mut monitor,
                IssueMonitorControl::Enabled(false),
            ));
            release_tx.send(()).expect("release blocker");
            blocker.await.expect("blocker exits");
            let completed = (&mut queued.handle).await.expect("queued effect joins");
            assert!(matches!(
                completed.outcome,
                super::IssueMonitorEffectOutcome::VolatileDenied
            ));
        });

        FileExt::unlock(&prefs_lock).expect("release prefs lock");
        assert!(
            !client_marker.exists(),
            "closure-side permit check must run before the claim HTTP adapter"
        );
    }

    #[test]
    fn volatile_deny_runs_safety_effects_but_not_grants() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let grant = crate::PendingIssueMonitorEffect::prepared(
            "claim:42:grant",
            7,
            crate::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "claim-42".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-27T00:00:00Z".to_string(),
                expires_at: "2026-07-27T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        );
        let safety = crate::PendingIssueMonitorEffect::prepared(
            "release:claim:41",
            7,
            crate::IssueMonitorEffectPayload::ReleaseClaim {
                issue_number: 41,
                claim_id: "claim-41".to_string(),
                owner: "host/session".to_string(),
            },
        );
        let prefs = crate::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 7,
            pending_effects: vec![grant.clone(), safety.clone()],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        let permit = super::IssueMonitorEffectPermit::new();
        permit.deny();

        let fenced = super::fence_next_issue_monitor_effect_with_permit(
            &prefs_path,
            &mut monitor,
            &permit.capture(),
        )
        .expect("safety effect remains executable while grants are denied");

        assert_eq!(fenced.effect_id, safety.effect_id);
        assert_eq!(fenced.state, crate::IssueMonitorEffectState::Attempting);
        assert_eq!(monitor.pending_effects()[0], grant);
        assert_eq!(
            monitor.pending_effects()[0].state,
            crate::IssueMonitorEffectState::Prepared
        );
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
        let remote_mutation = temp.path().join("unexpected-remote-mutation");
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "block");
        let _started = ScopedEnvVar::set("GWT_FAKE_GH_STARTED", &scan_started);
        let _release = ScopedEnvVar::set("GWT_FAKE_GH_RELEASE", &release_scan);
        let _active = ScopedEnvVar::set("GWT_FAKE_GH_ACTIVE", &active_scan);
        let _overlap = ScopedEnvVar::set("GWT_FAKE_GH_OVERLAP", &overlap_scan);
        let _mutation = ScopedEnvVar::set("GWT_FAKE_GH_MUTATION_MARKER", &remote_mutation);

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
        super::persist_issue_monitor_shutdown_revoke_marker(&prefs_path)
            .expect("persist shutdown marker");
        let shutdown_marker = super::issue_monitor_shutdown_revoke_marker_path(&prefs_path);

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
        assert_eq!(
            hub.publish_issue_monitor_control(DaemonFrame::Event {
                channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
                payload: serde_json::json!({"enabled": false}),
            })
            .await,
            Err(super::IssueMonitorControlQueueError::RecoveryBlocked),
            "corrupt startup exposes a stable recovery-blocked control state"
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
                    .is_some_and(|error| error.contains("authority recovery is blocked"))
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
        assert!(
            !remote_mutation.exists(),
            "recovery-blocked startup grants zero remote-effect authority"
        );
        assert_eq!(
            fs::read_to_string(&prefs_path).expect("journal file remains"),
            malformed,
            "real worker must not replace an ambiguous journal with defaults"
        );
        assert!(
            shutdown_marker.exists(),
            "corrupt prefs must retain the independent shutdown marker"
        );
        shutdown.request();
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .expect("recovery-blocked worker shutdown is bounded")
            .expect("worker exits cleanly");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // global test environment stays isolated for the worker lifetime
    async fn worker_shutdown_after_ready_preserves_new_corrupt_bytes() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create home");
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
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default())
            .expect("seed valid prefs");
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
        let corrupt = b"{\"enabled\":true";
        fs::write(&prefs_path, corrupt).expect("corrupt prefs after Ready");
        shutdown.request();
        worker.await.expect("worker exits");

        assert_eq!(fs::read(&prefs_path).expect("read corrupt prefs"), corrupt);
        let quarantine_count = fs::read_dir(prefs_path.parent().expect("prefs parent"))
            .expect("read prefs dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("issue-monitor.json.corrupt-")
            })
            .count();
        assert_eq!(quarantine_count, 0);
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
    fn routine_effect_settlement_does_not_rewind_a_live_review() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let release = crate::PendingIssueMonitorEffect {
            effect_id: "release:77:claim-77:7".to_string(),
            authority_epoch: 7,
            attempt: 1,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::ReleaseClaim {
                issue_number: 77,
                claim_id: "claim-77".to_string(),
                owner: "host/session".to_string(),
            },
        };
        let prefs = crate::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            effect_authority_epoch: 7,
            pending_effects: vec![release.clone()],
            autonomous_records: vec![crate::AutonomousIssueRecord {
                issue_number: 42,
                phase: crate::AutonomousPhase::Reviewing,
                active_launch_id: None,
                attempts: 1,
                acceptance_snapshot: None,
                retry_not_before: None,
                last_heartbeat: Some("2026-07-28T00:00:00Z".to_string()),
                pr_number: Some(99),
                reviewed_sha: Some("abc123".to_string()),
                review_passed: None,
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
                effect: release,
                outcome: super::IssueMonitorEffectOutcome::Release(Ok(
                    gwt_github::issue_auto_claim::ClaimReleaseOutcome::AlreadyReleased(None),
                )),
                completed_at: "2026-07-28T00:00:01Z".to_string(),
            },
        ));

        let record = monitor.autonomous_record(42).expect("live review record");
        assert_eq!(record.phase, crate::AutonomousPhase::Reviewing);
        assert_eq!(
            record.last_heartbeat.as_deref(),
            Some("2026-07-28T00:00:00Z"),
            "an unrelated runtime effect must not masquerade as restart recovery",
        );
    }

    #[test]
    fn deferred_restart_resume_consumes_only_the_startup_review_set_once() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let prefs = crate::IssueMonitorPrefs {
            autonomous_mode: true,
            autonomous_records: vec![
                crate::AutonomousIssueRecord {
                    issue_number: 7,
                    phase: crate::AutonomousPhase::Reviewing,
                    active_launch_id: None,
                    attempts: 1,
                    acceptance_snapshot: None,
                    retry_not_before: None,
                    last_heartbeat: None,
                    pr_number: Some(70),
                    reviewed_sha: Some("sha-7".to_string()),
                    review_passed: None,
                },
                crate::AutonomousIssueRecord {
                    issue_number: 8,
                    phase: crate::AutonomousPhase::Reviewing,
                    active_launch_id: None,
                    attempts: 1,
                    acceptance_snapshot: None,
                    retry_not_before: None,
                    last_heartbeat: Some("2026-07-28T00:00:00Z".to_string()),
                    pr_number: Some(80),
                    reviewed_sha: Some("sha-8".to_string()),
                    review_passed: None,
                },
            ],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        let mut deferred = vec![monitor
            .autonomous_record(7)
            .expect("startup review record")
            .clone()];

        super::resume_deferred_restart_reviews(&prefs_path, &mut monitor, &mut deferred);

        assert!(deferred.is_empty(), "startup recovery set is one-shot");
        assert_eq!(
            monitor.autonomous_record(7).map(|record| record.phase),
            Some(crate::AutonomousPhase::Implementing),
        );
        assert_eq!(
            monitor.autonomous_record(8).map(|record| record.phase),
            Some(crate::AutonomousPhase::Reviewing),
        );
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert_eq!(
            persisted
                .autonomous_records
                .iter()
                .find(|record| record.issue_number == 7)
                .map(|record| record.phase),
            Some(crate::AutonomousPhase::Implementing),
        );
        assert_eq!(
            persisted
                .autonomous_records
                .iter()
                .find(|record| record.issue_number == 8)
                .map(|record| record.phase),
            Some(crate::AutonomousPhase::Reviewing),
        );
    }

    #[test]
    fn deferred_restart_resume_retries_after_persistence_failure() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let prefs = crate::IssueMonitorPrefs {
            autonomous_mode: true,
            autonomous_records: vec![crate::AutonomousIssueRecord {
                issue_number: 7,
                phase: crate::AutonomousPhase::Reviewing,
                active_launch_id: None,
                attempts: 1,
                acceptance_snapshot: None,
                retry_not_before: None,
                last_heartbeat: None,
                pr_number: Some(70),
                reviewed_sha: Some("sha-7".to_string()),
                review_passed: None,
            }],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        let mut deferred = vec![monitor
            .autonomous_record(7)
            .expect("startup review record")
            .clone()];
        let fail_once = prefs_path.with_extension("parent-sync-fail-once");
        fs::write(&fail_once, b"fail once").expect("seed parent sync failure trigger");
        let failure = ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            &prefs_path,
        );

        super::resume_deferred_restart_reviews(&prefs_path, &mut monitor, &mut deferred);

        assert_eq!(
            deferred.len(),
            1,
            "failed durability confirmation keeps startup recovery retryable",
        );
        assert_eq!(
            monitor.autonomous_record(7).map(|record| record.phase),
            Some(crate::AutonomousPhase::Reviewing),
            "in-memory state is not committed before durable persistence succeeds",
        );

        drop(failure);
        super::resume_deferred_restart_reviews(&prefs_path, &mut monitor, &mut deferred);

        assert!(
            deferred.is_empty(),
            "successful retry consumes recovery once"
        );
        assert_eq!(
            monitor.autonomous_record(7).map(|record| record.phase),
            Some(crate::AutonomousPhase::Implementing),
        );
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert_eq!(
            persisted
                .autonomous_records
                .iter()
                .find(|record| record.issue_number == 7)
                .map(|record| record.phase),
            Some(crate::AutonomousPhase::Implementing),
        );
    }

    #[test]
    fn auto_merge_success_mismatch_is_fail_closed_without_panicking() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let effect = crate::PendingIssueMonitorEffect {
            effect_id: "claim:42:mismatch:7".to_string(),
            authority_epoch: 7,
            attempt: 1,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "claim-42".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-28T00:00:00Z".to_string(),
                expires_at: "2026-07-28T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        };
        let prefs = crate::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: 7,
            pending_effects: vec![effect.clone()],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);

        let settled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::commit_issue_monitor_effect_result(
                &prefs_path,
                &mut monitor,
                super::CompletedIssueMonitorEffect {
                    effect: effect.clone(),
                    outcome: super::IssueMonitorEffectOutcome::AutoMerge(
                        gwt_git::pr_status::AutoMergeMutationOutcome::Confirmed,
                    ),
                    completed_at: "2026-07-28T00:00:01Z".to_string(),
                },
            )
        }))
        .expect("a mismatched executor outcome must not panic the daemon worker");

        assert!(!settled, "a mismatched result cannot settle the receipt");
        assert_eq!(monitor.pending_effects(), std::slice::from_ref(&effect));
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert_eq!(persisted.pending_effects, vec![effect]);
    }

    #[test]
    fn acquired_claim_commit_persists_launching_and_outbox_in_one_transaction() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            ..crate::IssueMonitorConfig::default()
        });
        monitor.record_candidate(sample_issue_monitor_issue(42));
        let effect = crate::PendingIssueMonitorEffect::prepared(
            "claim-effect-42",
            0,
            crate::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "claim-42".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-28T00:00:00Z".to_string(),
                expires_at: "2026-07-28T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        );
        let key = monitor
            .prepare_pending_effect(effect.effect_id.clone(), effect.payload.clone())
            .expect("prepare claim effect");
        assert!(monitor.mark_pending_effect_attempting(&key));
        let attempting = monitor.pending_effects()[0].clone();
        crate::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed prefs");

        assert!(super::commit_issue_monitor_effect_result(
            &prefs_path,
            &mut monitor,
            super::CompletedIssueMonitorEffect {
                effect: attempting,
                outcome: super::IssueMonitorEffectOutcome::Claim(Ok(
                    gwt_github::issue_auto_claim::ClaimAcquireOutcome::Acquired(
                        gwt_github::issue_auto_claim::ClaimComment {
                            comment_id: Some(gwt_github::CommentId(99)),
                            claim_id: "claim-42".to_string(),
                            owner: "host/session".to_string(),
                            issue_number: 42,
                            status: gwt_github::issue_auto_claim::ClaimStatus::Active,
                            heartbeat_at: "2026-07-28T00:00:00Z".to_string(),
                            expires_at: "2026-07-28T00:30:00Z".to_string(),
                            launched_work_id: Some("work/issue-42".to_string()),
                        },
                    ),
                )),
                completed_at: "2026-07-28T00:00:01Z".to_string(),
            },
        ));

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert!(persisted.pending_effects.is_empty());
        assert_eq!(persisted.launching_issues.len(), 1);
        assert_eq!(persisted.pending_launch_deliveries.len(), 1);
        assert_eq!(
            persisted.pending_launch_deliveries[0].delivery_id,
            "launch:claim-effect-42"
        );
        assert_eq!(
            persisted.pending_launch_deliveries[0].claim_owner,
            "host/session"
        );
    }

    #[test]
    fn launch_delivery_claim_control_keeps_one_live_materializer() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let mut monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            ..crate::IssueMonitorConfig::default()
        });
        monitor.record_candidate(sample_issue_monitor_issue(42));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));
        crate::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed prefs");
        let live_pid = std::process::id();

        assert!(matches!(
            super::try_apply_issue_monitor_control_with_disk_migration(
                &prefs_path,
                &mut monitor,
                super::IssueMonitorControl::ClaimLaunchDelivery {
                    issue_number: 42,
                    delivery_id: "launch:effect-42".to_string(),
                    materializer_id: "gui-a".to_string(),
                    materializer_pid: live_pid,
                    materializer_window_id: "tab-a::agent-1".to_string(),
                },
            ),
            super::IssueMonitorControlCommit::Committed { .. }
        ));
        assert!(matches!(
            super::try_apply_issue_monitor_control_with_disk_migration(
                &prefs_path,
                &mut monitor,
                super::IssueMonitorControl::ClaimLaunchDelivery {
                    issue_number: 42,
                    delivery_id: "launch:effect-42".to_string(),
                    materializer_id: "gui-b".to_string(),
                    materializer_pid: live_pid,
                    materializer_window_id: "tab-b::agent-1".to_string(),
                },
            ),
            super::IssueMonitorControlCommit::Committed { .. }
        ));

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        let delivery = &persisted.pending_launch_deliveries[0];
        assert_eq!(delivery.materializer_id.as_deref(), Some("gui-a"));
        assert_eq!(
            delivery.materializer_window_id.as_deref(),
            Some("tab-a::agent-1")
        );

        let _ = super::try_apply_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut monitor,
            super::IssueMonitorControl::LaunchFailed {
                issue_number: 42,
                message: "non-owner launch failure".to_string(),
                delivery_id: Some("launch:effect-42".to_string()),
                materializer_id: Some("gui-b".to_string()),
            },
        );
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload rejected failure")
                .pending_launch_deliveries
                .len(),
            1,
        );

        let _ = super::try_apply_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut monitor,
            super::IssueMonitorControl::Launched {
                issue_number: 42,
                window_id: "tab-a::agent-1".to_string(),
                delivery_id: Some("launch:effect-42".to_string()),
            },
        );
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload premature ACK")
                .pending_launch_deliveries
                .len(),
            1,
        );

        for control in [
            super::IssueMonitorControl::LaunchDeliveryMaterialized {
                issue_number: 42,
                delivery_id: "launch:effect-42".to_string(),
                materializer_id: "gui-a".to_string(),
                materializer_window_id: "tab-a::agent-1".to_string(),
            },
            super::IssueMonitorControl::LaunchDeliveryWorkspaceDurable {
                issue_number: 42,
                delivery_id: "launch:effect-42".to_string(),
                materializer_id: "gui-a".to_string(),
                materializer_window_id: "tab-a::agent-1".to_string(),
            },
            super::IssueMonitorControl::Launched {
                issue_number: 42,
                window_id: "tab-a::agent-1".to_string(),
                delivery_id: Some("launch:effect-42".to_string()),
            },
        ] {
            let _ = super::try_apply_issue_monitor_control_with_disk_migration(
                &prefs_path,
                &mut monitor,
                control,
            );
        }
        assert!(crate::load_issue_monitor_prefs(&prefs_path)
            .expect("reload exact ACK")
            .pending_launch_deliveries
            .is_empty());
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

        let preserved = monitor.clone();
        let failure = super::scan_issue_monitor_once_blocking(scope, monitor, false)
            .expect_err("remote failure stays typed");
        let mut monitor =
            super::scan_failure_fallback(preserved, failure, "2026-07-28T00:00:00Z".to_string());
        super::persist_daemon_issue_monitor_state(&prefs_path, &mut monitor);

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
    fn daemon_persist_does_not_restore_a_launch_merged_by_the_scan() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor-prefs.json");
        let disk = crate::IssueMonitorPrefs {
            enabled: true,
            launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                issue_number: 42,
                window_id: "tab-1::agent-1".to_string(),
            }],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &disk).expect("seed launched prefs");
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig {
                enabled: true,
                ..crate::IssueMonitorConfig::default()
            },
            disk,
        );
        assert_eq!(monitor.status_view().active_count, 1);

        monitor.record_merged(42);
        assert_eq!(monitor.status_view().active_count, 0);

        super::persist_daemon_issue_monitor_state(&prefs_path, &mut monitor);

        assert_eq!(
            monitor.status_view().active_count,
            0,
            "the persist rebase must not restore the stale disk launch"
        );
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert!(persisted.merged_issues.contains(&42));
        assert!(
            persisted
                .launched_issues
                .iter()
                .all(|launched| launched.issue_number != 42),
            "the merged issue must not persist as both launched and merged"
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

        let preserved = monitor.clone();
        let failure = super::scan_issue_monitor_once_blocking(scope, monitor, false)
            .expect_err("remote failure stays typed");
        let mut monitor =
            super::scan_failure_fallback(preserved, failure, "2026-07-28T00:00:00Z".to_string());
        super::persist_daemon_issue_monitor_state(&prefs_path, &mut monitor);
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
                    delivery_id: None,
                    materializer_id: None,
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
    fn stale_local_off_failure_denies_and_enters_retry_barrier() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed canonical ON");
        let mut stale_local = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                enabled: false,
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let mut permit = super::IssueMonitorEffectPermit::new();
        let captured_grant = permit.capture();
        let mut pending = None;
        let hub = BroadcastHub::new();

        let should_scan = super::apply_or_queue_issue_monitor_control(
            &hub,
            &prefs_path,
            &mut stale_local,
            IssueMonitorControl::Enabled(false),
            &mut permit,
            &mut pending,
            None,
        );

        assert!(!should_scan);
        assert!(
            !captured_grant.load(Ordering::Acquire),
            "authorizing controls deny before canonical state is readable"
        );
        let pending = pending.as_ref().expect("lock failure installs barrier");
        assert_eq!(pending.front(), Some(&IssueMonitorControl::Enabled(false)));
        assert!(pending.retry_at().is_some());
        assert!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("canonical prefs remain readable")
                .enabled
        );

        FileExt::unlock(&lock).expect("release prefs lock");
        let retry = super::try_apply_accepted_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut stale_local,
            pending.front_accepted().expect("pending OFF").clone(),
        );
        assert!(matches!(
            retry,
            super::IssueMonitorControlCommit::Committed { .. }
        ));
        let committed = crate::load_issue_monitor_prefs(&prefs_path).expect("reload durable OFF");
        assert!(!committed.enabled);
        assert_eq!(committed.effect_authority_epoch, 8);
    }

    #[test]
    fn control_commit_reports_authority_change_from_canonical_rebase() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed canonical ON");
        let mut stale_local = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                enabled: false,
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        );

        let commit = super::try_apply_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut stale_local,
            IssueMonitorControl::Enabled(false),
        );

        assert!(matches!(
            commit,
            super::IssueMonitorControlCommit::Committed {
                should_scan: true,
                authority_changed: true,
            }
        ));
        assert!(!stale_local.config.enabled);
        assert_eq!(stale_local.effect_authority_epoch(), 8);
    }

    #[test]
    fn changed_off_commit_reconciles_deferred_grant_before_queued_on_overflow() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let old_grant = crate::PendingIssueMonitorEffect {
            effect_id: "claim:42:before-overflow".to_string(),
            authority_epoch: u64::MAX - 1,
            attempt: 1,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::AcquireClaim {
                issue_number: 42,
                claim_id: "stable-claim-42".to_string(),
                owner: "host/session".to_string(),
                heartbeat_at: "2026-07-27T00:00:00Z".to_string(),
                expires_at: "2026-07-27T00:30:00Z".to_string(),
                launched_work_id: Some("work/issue-42".to_string()),
            },
        };
        let initial = crate::IssueMonitorPrefs {
            enabled: true,
            effect_authority_epoch: u64::MAX - 1,
            pending_effects: vec![old_grant.clone()],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &initial).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), initial);
        let mut pending = super::PendingIssueMonitorAuthorityControls::after_failure(
            IssueMonitorControl::Enabled(false),
        );
        pending.push(IssueMonitorControl::Enabled(true));
        let mut deferred = Some(super::CompletedIssueMonitorEffect {
            effect: old_grant.clone(),
            outcome: super::IssueMonitorEffectOutcome::VolatileDenied,
            completed_at: "2026-07-27T00:00:00Z".to_string(),
        });

        let off = super::try_apply_accepted_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut monitor,
            pending.front_accepted().expect("pending OFF").clone(),
        );
        let super::IssueMonitorControlCommit::Committed {
            authority_changed, ..
        } = off
        else {
            panic!("OFF commits at the last available epoch");
        };
        let (drained, completion, authorizing) = pending.committed_front();
        assert!(completion.is_none());
        assert!(authorizing);
        assert!(!drained, "queued ON remains behind the OFF barrier");
        super::reconcile_deferred_grant_after_authority_commit(
            &prefs_path,
            &mut monitor,
            &mut deferred,
            authority_changed,
            drained,
        );

        assert!(deferred.is_none(), "changed OFF reconciles immediately");
        let on = super::try_apply_accepted_issue_monitor_control_with_disk_migration(
            &prefs_path,
            &mut monitor,
            pending.front_accepted().expect("queued ON").clone(),
        );
        assert_eq!(on, super::IssueMonitorControlCommit::TerminalFailure);
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert!(!persisted.enabled);
        assert_eq!(persisted.effect_authority_epoch, u64::MAX);
        assert!(
            persisted
                .pending_effects
                .iter()
                .all(|effect| effect.attempt_key() != old_grant.attempt_key()),
            "later ON overflow cannot retain the exact stale grant result forever"
        );
    }

    #[test]
    fn intermediate_noop_retains_deferred_grant_until_controls_drain() {
        let temp = TempDir::new().expect("tempdir");
        let prefs_path = temp.path().join("issue-monitor.json");
        let arm = crate::PendingIssueMonitorEffect {
            effect_id: "arm:42:99:no-op-drain".to_string(),
            authority_epoch: 7,
            attempt: 1,
            state: crate::IssueMonitorEffectState::Attempting,
            payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                issue_number: 42,
                pr_number: 99,
                reviewed_sha: "abc123".to_string(),
            },
        };
        let prefs = crate::IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            effect_authority_epoch: 7,
            pending_effects: vec![arm.clone()],
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).expect("seed prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        let mut deferred = Some(super::CompletedIssueMonitorEffect {
            effect: arm,
            outcome: super::IssueMonitorEffectOutcome::AutoMerge(
                gwt_git::pr_status::AutoMergeMutationOutcome::PreSubmit(
                    "not submitted".to_string(),
                ),
            ),
            completed_at: "2026-07-27T00:00:00Z".to_string(),
        });

        super::reconcile_deferred_grant_after_authority_commit(
            &prefs_path,
            &mut monitor,
            &mut deferred,
            false,
            false,
        );
        assert!(deferred.is_some(), "a leading no-op is not a revoke fence");
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload retained attempt")
                .pending_effects[0]
                .state,
            crate::IssueMonitorEffectState::Attempting
        );

        super::reconcile_deferred_grant_after_authority_commit(
            &prefs_path,
            &mut monitor,
            &mut deferred,
            false,
            true,
        );
        assert!(deferred.is_none(), "all-no-op drain reconciles the result");
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload reconciled attempt")
                .pending_effects[0]
                .state,
            crate::IssueMonitorEffectState::Prepared
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn off_lock_timeout_retries_and_commits_revocation_after_unlock() {
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
  : > "$GWT_FAKE_EFFECT_STARTED"
  while [ ! -f "$GWT_FAKE_EFFECT_RELEASE" ]; do
    sleep 0.02
  done
  if [ -f "$GWT_FAKE_ARM_DONE" ]; then
    printf '%s\n' '{"state":"OPEN","headRefOid":"abc","autoMergeRequest":{"enabledAt":"2026-07-27T00:00:00Z"},"mergeCommit":null}'
  else
    printf '%s\n' '{"state":"OPEN","headRefOid":"abc","autoMergeRequest":null,"mergeCommit":null}'
  fi
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ] && [ "$4" = "--auto" ]; then
  : > "$GWT_FAKE_ARM_DONE"
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ] && [ "$4" = "--disable-auto" ]; then
  : > "$GWT_FAKE_DISARM_DONE"
  exit 0
fi
exit 1
"#,
        )
        .expect("write fake gh");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755)).expect("chmod fake gh");
        let effect_started = temp.path().join("effect-started");
        let effect_release = temp.path().join("effect-release");
        let arm_done = temp.path().join("arm-done");
        let disarm_done = temp.path().join("disarm-done");
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _started = ScopedEnvVar::set("GWT_FAKE_EFFECT_STARTED", &effect_started);
        let _release = ScopedEnvVar::set("GWT_FAKE_EFFECT_RELEASE", &effect_release);
        let _arm_done = ScopedEnvVar::set("GWT_FAKE_ARM_DONE", &arm_done);
        let _disarm_done = ScopedEnvVar::set("GWT_FAKE_DISARM_DONE", &disarm_done);

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
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed attempting arm");

        let hub = BroadcastHub::new();
        let mut status_rx = hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL);
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
        assert!(wait_for_path(&effect_started, Duration::from_secs(2)).await);
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let source_pid = std::process::id().wrapping_add(1);
        let off_receipt = tokio::spawn({
            let hub = hub.clone();
            async move {
                hub.publish_issue_monitor_control(DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload: crate::runtime_daemon_events::issue_monitor_payload(
                        "control",
                        serde_json::json!({"autonomous_mode": false}),
                        source_pid,
                    ),
                })
                .await
            }
        });
        let failed_status =
            recv_issue_monitor_status_matching(&mut status_rx, Duration::from_secs(1), |status| {
                status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("control commit failed"))
            })
            .await;
        tokio::time::sleep(Duration::from_millis(650)).await;
        FileExt::unlock(&lock).expect("release prefs lock");
        tokio::time::timeout(Duration::from_secs(2), off_receipt)
            .await
            .expect("OFF receipt resolves after retry")
            .expect("OFF publisher joins")
            .expect("durable OFF commit is acknowledged");

        let committed = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
                let compensation_count = prefs
                    .pending_effects
                    .iter()
                    .filter(|effect| {
                        matches!(
                            &effect.payload,
                            crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                                compensates_effect_id,
                                ..
                            } if compensates_effect_id == "arm:42:99:abc:7"
                        )
                    })
                    .count();
                if !prefs.autonomous_mode
                    && prefs.effect_authority_epoch == 8
                    && compensation_count == 1
                {
                    break prefs;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .ok();
        let stable = crate::load_issue_monitor_prefs(&prefs_path).expect("reload stable prefs");
        fs::write(&effect_release, b"release").expect("release in-flight effect");
        shutdown.request();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .expect("worker shutdown is bounded")
            .expect("worker exits cleanly");

        assert!(
            failed_status.is_some(),
            "lock failure remains operator-visible"
        );
        assert!(
            committed.is_some(),
            "short internal retry commits OFF after unlock"
        );
        assert_eq!(
            stable.effect_authority_epoch, 8,
            "retry advances epoch once"
        );
        assert_eq!(
            stable
                .pending_effects
                .iter()
                .filter(|effect| matches!(
                    &effect.payload,
                    crate::IssueMonitorEffectPayload::DisarmAutoMerge {
                        compensates_effect_id,
                        ..
                    } if compensates_effect_id == "arm:42:99:abc:7"
                ))
                .count(),
            1,
            "retry commits one compensation"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn pending_off_is_barrier_to_reenable_without_permit_aba() {
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
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "pass");
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
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let hub = BroadcastHub::new();
        let mut status_rx = hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL);
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = spawn_issue_monitor_worker_with_config_and_timeout(
            scope,
            hub.clone(),
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 60,
                ..crate::IssueMonitorConfig::default()
            },
            Duration::from_secs(2),
        );
        let _ =
            recv_issue_monitor_status_matching(&mut status_rx, Duration::from_secs(1), |status| {
                status.enabled
            })
            .await;
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let source_pid = std::process::id().wrapping_add(1);
        let mut publishers = Vec::new();
        for enabled in [false, true, false, true] {
            publishers.push(tokio::spawn({
                let hub = hub.clone();
                async move {
                    hub.publish_issue_monitor_control(DaemonFrame::Event {
                        channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                            .to_string(),
                        payload: crate::runtime_daemon_events::issue_monitor_payload(
                            "control",
                            serde_json::json!({"enabled": enabled}),
                            source_pid,
                        ),
                    })
                    .await
                }
            }));
            tokio::task::yield_now().await;
        }
        let failed =
            recv_issue_monitor_status_matching(&mut status_rx, Duration::from_secs(1), |status| {
                status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("control commit failed"))
            })
            .await;
        assert!(failed.is_some(), "OFF lock timeout becomes visible");
        assert!(
            publishers.iter().all(|publisher| !publisher.is_finished()),
            "no ordered receipt resolves before the first durable commit"
        );
        FileExt::unlock(&lock).expect("release prefs lock");
        for publisher in publishers {
            tokio::time::timeout(Duration::from_secs(2), publisher)
                .await
                .expect("ordered control receipt resolves")
                .expect("ordered control publisher joins")
                .expect("ordered durable control commits");
        }

        let ordered = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
                if prefs.enabled && prefs.effect_authority_epoch == 11 {
                    break prefs;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .ok();
        shutdown.request();
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .expect("worker shutdown is bounded")
            .expect("worker exits cleanly");

        let mut permit = super::IssueMonitorEffectPermit::new();
        let old = permit.capture();
        permit.deny();
        permit.reopen();
        assert!(
            !old.load(Ordering::Acquire),
            "old queued executor stays denied"
        );
        assert!(
            permit.capture().load(Ordering::Acquire),
            "reopen installs a distinct current permit"
        );
        assert!(
            ordered.is_some(),
            "all four authority controls commit in strict FIFO order"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn shutdown_rejects_pending_and_queued_control_receipts_without_ack() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                effect_authority_epoch: 7,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let hub = BroadcastHub::new();
        let mut status_rx = hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL);
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = spawn_issue_monitor_worker_with_config_and_timeout(
            scope,
            hub.clone(),
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 60,
                ..crate::IssueMonitorConfig::default()
            },
            Duration::from_secs(1),
        );
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let source_pid = std::process::id().wrapping_add(1);
        let pending = tokio::spawn({
            let hub = hub.clone();
            async move {
                hub.publish_issue_monitor_control(DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload: crate::runtime_daemon_events::issue_monitor_payload(
                        "control",
                        serde_json::json!({"enabled": false}),
                        source_pid,
                    ),
                })
                .await
            }
        });
        let queued = tokio::spawn({
            let hub = hub.clone();
            async move {
                hub.publish_issue_monitor_control(DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload: crate::runtime_daemon_events::issue_monitor_payload(
                        "control",
                        serde_json::json!({"max_active_agents": 4}),
                        source_pid,
                    ),
                })
                .await
            }
        });

        let failure =
            recv_issue_monitor_status_matching(&mut status_rx, Duration::from_secs(1), |status| {
                status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("control commit failed"))
            })
            .await;
        assert!(failure.is_some(), "OFF reaches retry barrier");
        assert!(!pending.is_finished(), "retryable OFF has no ACK yet");
        assert!(!queued.is_finished(), "FIFO successor has no ACK yet");

        shutdown.request();
        let pending_result = tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .expect("pending receipt resolves")
            .expect("pending publisher joins");
        let queued_result = tokio::time::timeout(Duration::from_secs(2), queued)
            .await
            .expect("queued receipt resolves")
            .expect("queued publisher joins");
        FileExt::unlock(&lock).expect("release prefs lock");
        tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .expect("worker shutdown is bounded")
            .expect("worker exits cleanly");

        assert!(pending_result.is_err());
        assert!(queued_result.is_err());
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        assert!(persisted.enabled, "uncommitted OFF is never acknowledged");
        assert_eq!(persisted.max_active_agents, 1);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn routine_control_lock_retry_keeps_receipt_and_fifo_until_commit() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create home");
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
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                max_active_agents: 1,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let hub = BroadcastHub::new();
        let mut status_rx = hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL);
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = spawn_issue_monitor_worker_with_config_and_timeout(
            scope,
            hub.clone(),
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 60,
                ..crate::IssueMonitorConfig::default()
            },
            Duration::from_secs(1),
        );
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let publish = |hub: BroadcastHub, max_active_agents| {
            tokio::spawn(async move {
                hub.publish_issue_monitor_control(DaemonFrame::Event {
                    channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                        .to_string(),
                    payload: crate::runtime_daemon_events::issue_monitor_payload(
                        "control",
                        serde_json::json!({"max_active_agents": max_active_agents}),
                        std::process::id().wrapping_add(1),
                    ),
                })
                .await
            })
        };
        let first = publish(hub.clone(), 4);
        tokio::task::yield_now().await;
        let second = publish(hub.clone(), 7);
        let failure =
            recv_issue_monitor_status_matching(&mut status_rx, Duration::from_secs(1), |status| {
                status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("control commit failed"))
            })
            .await;
        assert!(failure.is_some());
        assert!(
            !first.is_finished(),
            "retryable routine receipt stays pending"
        );
        assert!(!second.is_finished(), "FIFO successor stays pending");
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("load unchanged prefs")
                .max_active_agents,
            1
        );

        FileExt::unlock(&lock).expect("release prefs lock");
        first
            .await
            .expect("first publisher joins")
            .expect("first routine commit ACK");
        second
            .await
            .expect("second publisher joins")
            .expect("second routine commit ACK");
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("reload committed prefs")
                .max_active_agents,
            7,
            "routine controls commit in accepted FIFO order"
        );
        shutdown.request();
        worker.await.expect("worker exits");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn routine_retry_fences_a_scan_captured_while_the_control_is_pending() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("create home");
        let _home = ScopedGwtHome::set(&home);
        let fake_gh = write_fake_gh_issue_list(temp.path());
        let scan_started_path = temp.path().join("scan-started");
        let release_scan_path = temp.path().join("release-scan");
        let active_scan_path = temp.path().join("active-scan");
        let overlap_scan_path = temp.path().join("overlap-scan");
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "block");
        let _started = ScopedEnvVar::set("GWT_FAKE_GH_STARTED", &scan_started_path);
        let _release = ScopedEnvVar::set("GWT_FAKE_GH_RELEASE", &release_scan_path);
        let _active = ScopedEnvVar::set("GWT_FAKE_GH_ACTIVE", &active_scan_path);
        let _overlap = ScopedEnvVar::set("GWT_FAKE_GH_OVERLAP", &overlap_scan_path);
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
        let old_heartbeat = "2099-01-01T00:00:00Z";
        let committed_heartbeat = "2099-01-02T00:00:00Z";
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_records: vec![crate::AutonomousIssueRecord {
                    issue_number: 43,
                    phase: crate::AutonomousPhase::Implementing,
                    active_launch_id: None,
                    attempts: 1,
                    acceptance_snapshot: None,
                    retry_not_before: None,
                    last_heartbeat: Some(old_heartbeat.to_string()),
                    pr_number: None,
                    reviewed_sha: None,
                    review_passed: None,
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let hub = BroadcastHub::new();
        let mut status_rx = hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CHANNEL);
        let shutdown = Arc::new(DaemonShutdown::new());
        let worker = spawn_issue_monitor_worker_with_config_and_timeout(
            scope,
            hub.clone(),
            Arc::clone(&shutdown),
            crate::IssueMonitorConfig {
                poll_interval_secs: 60,
                ..crate::IssueMonitorConfig::default()
            },
            Duration::from_secs(2),
        );
        let lock = issue_monitor_prefs_lock_for_test(&prefs_path);
        let mut receipt = hub
            .enqueue_issue_monitor_control(DaemonFrame::Event {
                channel: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
                payload: crate::runtime_daemon_events::issue_monitor_payload(
                    "control",
                    serde_json::json!({
                        "heartbeat": {
                            "issue_number": 43,
                            "at": committed_heartbeat,
                        }
                    }),
                    std::process::id().wrapping_add(1),
                ),
            })
            .await
            .expect("control is admitted before the worker loop runs");
        let failure =
            recv_issue_monitor_status_matching(&mut status_rx, Duration::from_secs(1), |status| {
                status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("control commit failed"))
            })
            .await;
        assert!(failure.is_some(), "initial heartbeat commit reaches retry");
        assert!(
            wait_for_path(&scan_started_path, Duration::from_secs(2)).await,
            "routine pending state keeps scan progress alive"
        );
        assert!(
            receipt.try_recv().is_err(),
            "heartbeat has no ACK before its durable commit"
        );

        FileExt::unlock(&lock).expect("release prefs lock");
        tokio::time::timeout(Duration::from_secs(2), receipt)
            .await
            .expect("heartbeat receipt resolves after retry")
            .expect("heartbeat receipt sender remains live")
            .expect("heartbeat retry commits");
        fs::write(&release_scan_path, b"release").expect("release captured scan");
        let settled =
            recv_issue_monitor_status_matching(&mut status_rx, Duration::from_secs(3), |status| {
                status.last_scan_at.is_some()
            })
            .await;
        assert!(settled.is_some(), "released scan settles or retries");
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
        let heartbeat = persisted
            .autonomous_records
            .iter()
            .find(|record| record.issue_number == 43)
            .and_then(|record| record.last_heartbeat.as_deref());
        shutdown.request();
        tokio::time::timeout(Duration::from_secs(3), worker)
            .await
            .expect("worker shutdown is bounded")
            .expect("worker exits");

        assert_eq!(heartbeat, Some(committed_heartbeat));
        assert!(
            !overlap_scan_path.exists(),
            "captured scan remains single-flight during the retry"
        );
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
                delivery_id: None,
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
                delivery_id: None,
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
                delivery_id: None,
                materializer_id: None,
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
