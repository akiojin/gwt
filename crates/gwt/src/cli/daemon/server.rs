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
    let listener = UnixListener::bind(&socket_path).map_err(|err| {
        config_error(format!(
            "failed to bind daemon socket {}: {err}",
            socket_path.display()
        ))
    })?;

    let shutdown = Arc::new(Notify::new());
    spawn_signal_watcher(Arc::clone(&shutdown));
    spawn_issue_monitor_worker(endpoint.scope.clone(), hub.clone(), Arc::clone(&shutdown));

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

fn spawn_signal_watcher(shutdown: Arc<Notify>) {
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
        term.notify_waiters();
    });
}

fn spawn_issue_monitor_worker(scope: RuntimeScope, hub: BroadcastHub, shutdown: Arc<Notify>) {
    tokio::spawn(async move {
        let mut control_rx =
            hub.subscribe(crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL);
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&scope.project_root);
        let prefs = crate::load_issue_monitor_prefs(&prefs_path).unwrap_or_default();
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        // SPEC #3200 (review follow-up): a record persisted mid-review reloads in
        // `Reviewing`, but its review-agent dispatch (not persisted) is gone.
        // Reset such records to `Implementing` so the first scan re-detects the PR
        // and re-issues the review — restoring the pre-persist self-healing. The
        // `now` stamp refreshes last_heartbeat so the reset record is not wrongly
        // reclaimed by stuck detection (which runs before the re-dispatch).
        let resume_now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let resumed = monitor.resume_inflight_reviews_after_restart(&resume_now);
        if !resumed.is_empty() {
            tracing::info!(
                issues = ?resumed,
                "issue monitor: resumed in-flight reviews after restart (Reviewing → Implementing)"
            );
        }
        publish_issue_monitor_payloads(&hub, &mut monitor);
        let mut interval =
            tokio::time::interval(Duration::from_secs(monitor.config.poll_interval_secs));

        loop {
            tokio::select! {
                biased;
                _ = shutdown.notified() => break,
                control = control_rx.recv() => {
                    match control {
                        Ok(DaemonFrame::Event { payload, .. }) => {
                            if let Some(control) = decode_issue_monitor_control(payload) {
                                let should_scan = apply_issue_monitor_control(&mut monitor, control);
                                persist_daemon_issue_monitor_state(&prefs_path, &monitor);
                                publish_issue_monitor_payloads(&hub, &mut monitor);
                                if should_scan {
                                    let gui_connected = issue_monitor_gui_connected(&hub);
                                    monitor = scan_and_persist_issue_monitor(
                                        scope.clone(),
                                        monitor,
                                        gui_connected,
                                        &prefs_path,
                                    )
                                    .await;
                                    publish_issue_monitor_payloads(&hub, &mut monitor);
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = interval.tick() => {
                    let gui_connected = issue_monitor_gui_connected(&hub);
                    monitor = scan_and_persist_issue_monitor(
                        scope.clone(),
                        monitor,
                        gui_connected,
                        &prefs_path,
                    )
                    .await;
                    publish_issue_monitor_payloads(&hub, &mut monitor);
                }
            }
        }
    });
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
    match control {
        IssueMonitorControl::Enabled(enabled) => {
            monitor.set_enabled(enabled);
            true
        }
        IssueMonitorControl::AutonomousMode(enabled) => {
            monitor.set_autonomous_mode(enabled);
            true
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

async fn scan_issue_monitor_once(
    scope: RuntimeScope,
    monitor: crate::IssueMonitorState,
    gui_connected: bool,
) -> crate::IssueMonitorState {
    // Keep a copy of the prior state so a `spawn_blocking` panic preserves it
    // instead of collapsing to a fresh default (see `scan_join_failure_fallback`).
    let preserved = monitor.clone();
    tokio::task::spawn_blocking(move || {
        scan_issue_monitor_once_blocking(scope, monitor, gui_connected)
    })
    .await
    .unwrap_or_else(|error| {
        scan_join_failure_fallback(
            preserved,
            error.to_string(),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )
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

/// Scan once, then persist the resulting state. The persist step is the SPEC
/// #3200 review follow-up fix: a periodic (interval-tick) scan runs
/// `reconcile_issue_monitor_merges` + `advance_autonomous_in_flight`, which can
/// `record_merged` / escalate to `NeedsHuman` without any control frame. Without
/// saving prefs after the scan, those transitions were lost on a daemon restart
/// and already-completed work was re-launched. Both worker loop arms route their
/// scan through here so no scan-driven transition can skip persistence.
async fn scan_and_persist_issue_monitor(
    scope: RuntimeScope,
    monitor: crate::IssueMonitorState,
    gui_connected: bool,
    prefs_path: &Path,
) -> crate::IssueMonitorState {
    let monitor = scan_issue_monitor_once(scope, monitor, gui_connected).await;
    persist_daemon_issue_monitor_state(prefs_path, &monitor);
    monitor
}

/// Persist the daemon's issue-monitor state WITHOUT clobbering GUI-owned config.
///
/// The daemon loads prefs once at startup and thereafter only learns config
/// changes it has a control frame for (`enabled` / `max_active_agents` /
/// `priority_order` / `autonomous_mode`). `launch_profile` and
/// `autonomous_tuning` have NO control channel — the GUI process edits them
/// directly on disk — so the daemon's in-memory copy is authoritative-but-stale
/// for those two fields. Writing the whole `monitor.prefs()` would overwrite a
/// newer GUI launch-profile / tuning with the stale startup value, silently
/// reverting the user's Issue Monitor configuration (adversarial review:
/// launch_profile clobber). We therefore reload the on-disk prefs and preserve
/// those two GUI-owned fields, persisting everything else (daemon-owned runtime
/// state + control-synced config) from memory. If the reload fails, we fall back
/// to the in-memory values rather than lose the daemon's runtime state.
fn persist_daemon_issue_monitor_state(prefs_path: &Path, monitor: &crate::IssueMonitorState) {
    let mut prefs = monitor.prefs();
    if let Ok(on_disk) = crate::load_issue_monitor_prefs(prefs_path) {
        prefs.launch_profile = on_disk.launch_profile;
        prefs.autonomous_tuning = on_disk.autonomous_tuning;
    }
    let _ = crate::save_issue_monitor_prefs(prefs_path, &prefs);
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
    let (owner, repo) =
        match crate::issue_monitor_worker::github_remote_owner_and_repo(&scope.project_root) {
            Ok(owner_repo) => owner_repo,
            Err(error) => {
                monitor.record_scan_error(now, error.to_string());
                return monitor;
            }
        };
    let issues = match crate::issue_monitor_worker::load_open_issue_monitor_candidates_for_repo_path(
        &scope.project_root,
        &owner,
        &repo,
    ) {
        Ok(issues) => issues,
        Err(error) => {
            monitor.record_scan_error(now, format!("issue list failed: {error}"));
            return monitor;
        }
    };
    let monitor_owner = format!("{}:{}", whoami::username(), std::process::id());
    crate::scan_issue_monitor_candidates(&mut monitor, &issues, &now);
    crate::issue_monitor_worker::reconcile_issue_monitor_merges(&mut monitor, &scope.project_root);
    // SPEC #3200 T-041/T-044: autonomous pre-launch eligibility gate + stuck-slot
    // recovery. Both are no-ops unless autonomous mode is on (default OFF keeps
    // the SPEC #3165 human-gated flow unchanged).
    crate::issue_monitor_worker::apply_autonomous_eligibility(
        &mut monitor,
        &issues,
        &format!("{owner}/{repo}"),
        &scope.project_root,
        &now,
    );
    monitor.recover_stuck_autonomous(&now);
    // SPEC #3200 kill switch (codex #3217 review): with autonomous mode OFF, any
    // record still Delivering has a live GitHub auto-merge that must be ACTIVELY
    // cancelled — abandoning it locally would let the old PR merge later while
    // nobody is watching. Runs AFTER reconcile so a PR that already merged was
    // recorded as merged instead of pointlessly disarmed. No-op while mode is ON.
    for (issue_number, pr_number) in monitor.take_kill_switch_disarms() {
        let disarmed = gwt_git::pr_status::disable_pr_auto_merge(&scope.project_root, pr_number);
        tracing::info!(
            issue = issue_number,
            pr = pr_number,
            disarmed,
            "kill switch: auto-merge disarm attempted"
        );
        monitor.record_kill_switch_disarm_result(issue_number, pr_number, disarmed);
    }
    // SPEC #3200 Option A: advance in-flight autonomous issues through the loop
    // (PR detect → review → gate → merge → watch). No-op unless autonomous mode
    // is on; default OFF keeps the SPEC #3165 flow unchanged.
    crate::issue_monitor_worker::advance_autonomous_in_flight(
        &mut monitor,
        &issues,
        &format!("{owner}/{repo}"),
        &scope.project_root,
        daemon_run_secret(),
        &now,
    );
    if monitor.config.enabled && gui_connected {
        let active_cap = if monitor.has_launch_profile() {
            monitor.config.max_active.max(1)
        } else {
            0
        };
        if monitor.active_count() < active_cap {
            match HttpIssueClient::from_gh_auth(&owner, &repo) {
                Ok(client) => {
                    monitor.claim_next_launch_requests_with_active_cap(
                        &client,
                        &monitor_owner,
                        &now,
                        active_cap,
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "issue monitor GitHub claim authentication unavailable"
                    );
                    monitor.record_launch_auth_required(now);
                }
            }
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
    use std::{path::Path, time::Duration};

    use gwt_core::daemon::{
        ClientFrame, DaemonEndpoint, HookEnvelope, IpcHandshakeRequest, RuntimeScope,
        RuntimeTarget, DAEMON_PROTOCOL_VERSION,
    };
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::UnixStream,
    };

    use super::{
        apply_issue_monitor_control, build_handshake_response, decode_issue_monitor_control,
        run_server, BroadcastHub, IssueMonitorControl,
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
        monitor.next_launch_request().expect("launch request");
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
        monitor.next_launch_request().expect("launch request");
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
        monitor.next_launch_request().expect("launch request");
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
        monitor.next_launch_request().expect("launch request");
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
        monitor.next_launch_request().expect("launch request");
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

        super::persist_daemon_issue_monitor_state(&prefs_path, &monitor);

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
