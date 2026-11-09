//! PTY close proof and Session finalization independent of the GUI.

use crate::agent_capability::{AgentCapabilityIssuer, ManualExecutionHandoffReservation};
use gwt_terminal::PtyHandle;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub struct CloseHandoffReservation {
    issuer: AgentCapabilityIssuer,
    reservation: ManualExecutionHandoffReservation,
    state: CloseHandoffState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseHandoffState {
    Pending,
    Committed,
    Settled,
}

impl CloseHandoffReservation {
    pub fn new(
        issuer: AgentCapabilityIssuer,
        reservation: ManualExecutionHandoffReservation,
    ) -> Self {
        Self {
            issuer,
            reservation,
            state: CloseHandoffState::Pending,
        }
    }

    pub fn commit_and_hold(&mut self) -> bool {
        // Child exit is the irreversible point. Even if the registry reports
        // that the reservation vanished, Drop must never restore its bearer.
        self.state = CloseHandoffState::Committed;
        self.issuer
            .commit_manual_execution_handoff(&self.reservation)
    }

    pub fn release_committed(&mut self) -> bool {
        if self.state != CloseHandoffState::Committed {
            return false;
        }
        let released = self
            .issuer
            .release_manual_execution_handoff(&self.reservation);
        if released {
            self.state = CloseHandoffState::Settled;
        }
        released
    }

    pub fn is_committed(&self) -> bool {
        self.state == CloseHandoffState::Committed
    }

    pub fn rollback(&mut self) -> bool {
        let rolled_back = self
            .issuer
            .rollback_manual_execution_handoff(&self.reservation);
        if rolled_back {
            self.state = CloseHandoffState::Settled;
        }
        rolled_back
    }
}

impl Drop for CloseHandoffReservation {
    fn drop(&mut self) {
        match self.state {
            CloseHandoffState::Pending => {
                if !self
                    .issuer
                    .rollback_manual_execution_handoff(&self.reservation)
                {
                    tracing::warn!(
                        target: "gwt.pane.teardown",
                        "dropped pane-close handoff reservation could not be rolled back"
                    );
                }
            }
            CloseHandoffState::Committed => {
                if !self
                    .issuer
                    .release_manual_execution_handoff(&self.reservation)
                {
                    tracing::warn!(
                        target: "gwt.pane.teardown",
                        "dropped committed pane-close handoff reservation could not be released"
                    );
                }
            }
            CloseHandoffState::Settled => {}
        }
    }
}

/// Stop and reap one captured PTY; never targets a successor resolved by name.
pub fn kill_and_reap(pty: &PtyHandle, window_id: &str) -> std::io::Result<bool> {
    // The leader may already be reaped while descendants still hold the PTY.
    // Always terminate the captured group/Job before declaring teardown done.
    tracing::info!(
        target: "gwt.pane.teardown",
        window_id = %window_id,
        stage = "pty_kill",
        outcome = "starting",
        "starting detached PTY teardown stage"
    );
    let kill_started = Instant::now();
    let kill_result = pty.terminate_and_wait_for_tree(Duration::from_secs(2));
    let tree_exited = matches!(&kill_result, Ok(true));
    let kill_elapsed_ms = u64::try_from(kill_started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        target: "gwt.pane.teardown",
        window_id = %window_id,
        stage = "pty_kill",
        elapsed_ms = kill_elapsed_ms,
        ok = tree_exited,
        outcome = if tree_exited { "completed" } else { "failed" },
        "detached PTY teardown stage completed"
    );
    if !kill_result.map_err(|error| std::io::Error::other(error.to_string()))? {
        return Ok(false);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if pty
            .try_wait()
            .map_err(|error| std::io::Error::other(error.to_string()))?
            .is_some()
        {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            #[cfg(target_os = "macos")]
            {
                let reason = pty.unreaped_child_reason();
                tracing::warn!(target: "gwt.pane.teardown", %window_id, %reason,
                    "detached PTY exit could not be proven before the finalizer deadline");
                return Err(std::io::Error::other(format!(
                    "detached PTY exit could not be proven: {reason}"
                )));
            }
            #[cfg(not(target_os = "macos"))]
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Prove a captured bound child has exited before terminalizing its exact Session.
pub fn finalize_exact_close(
    sessions_dir: &Path,
    identity: &gwt_agent::SessionExecutionIdentity,
    incarnation: u64,
    pty: &PtyHandle,
    window_id: &str,
) -> std::io::Result<Option<()>> {
    finalize_exact_close_with_restore_policy(
        sessions_dir,
        identity,
        incarnation,
        pty,
        window_id,
        Some(false),
        None,
    )
}

/// A lifetime stop preserves the latest saved restore decision under the same exact lease.
pub(crate) fn finalize_exact_close_with_restore_policy(
    sessions_dir: &Path,
    identity: &gwt_agent::SessionExecutionIdentity,
    incarnation: u64,
    pty: &PtyHandle,
    window_id: &str,
    desired_restore_on_startup: Option<bool>,
    exit_receipt: Option<&gwt_agent::SessionExitReceipt>,
) -> std::io::Result<Option<()>> {
    let proof = gwt_agent::ManualLaunchRuntimeProof {
        host_pid: std::process::id(),
        runtime_incarnation: incarnation,
    };
    crate::cli::execution_state::with_exact_owned_runtime_cleanup_lease(
        sessions_dir,
        identity,
        proof,
        || {
            let sidecar = gwt_agent::runtime_state_path_for_pid(
                sessions_dir,
                proof.host_pid,
                &identity.session_id,
            );
            let current = gwt_agent::SessionRuntimeState::load(&sidecar)?;
            let child_pid = pty
                .process_id()
                .ok_or_else(|| std::io::Error::other("captured child PID unavailable"))?;
            let child_running = pty
                .try_wait()
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .is_none();
            if current.host_started_at != crate::process::host_process_start_time(proof.host_pid)
                || current.host_started_at.is_none()
                || current.child_pid != Some(child_pid)
                || current.child_started_at.is_none_or(|started| started == 0)
                || (child_running
                    && current.child_started_at
                        != crate::process::host_process_start_time(child_pid))
            {
                return Err(std::io::Error::other(
                    "captured runtime process identity changed before close",
                ));
            }
            if !kill_and_reap(pty, window_id)? {
                return Err(std::io::Error::other(
                    "detached PTY did not exit before the finalizer deadline",
                ));
            }
            if !gwt_agent::persist_session_terminal_status_if_execution_identity_matches_under_lease(
                sessions_dir,
                identity,
                incarnation,
                gwt_agent::AgentStatus::Stopped,
            )? {
                return Err(std::io::Error::other(
                    "holder Session changed before terminal proof persistence",
                ));
            }
            if let Some(restore_on_startup) = desired_restore_on_startup {
                if !gwt_agent::persist_session_restore_window_on_startup_if_execution_identity_matches_under_lease(
                sessions_dir,
                identity,
                restore_on_startup,
            )? {
                return Err(std::io::Error::other(
                    "holder Session changed before startup-restore persistence",
                ));
            }
            }
            if let Some(receipt) = exit_receipt {
                let path = sessions_dir.join(format!("{}.toml", identity.session_id));
                let expected = gwt_agent::Session::load(&path)?;
                if gwt_agent::SessionExecutionIdentity::from_session(&expected)
                    .map_err(std::io::Error::other)?
                    .as_ref()
                    != Some(identity)
                {
                    return Err(std::io::Error::other(
                        "holder Session changed before exit receipt persistence",
                    ));
                }
                let mut updated = expected.clone();
                updated.record_process_exit(receipt);
                if !updated.save_if_unchanged_under_lease(sessions_dir, &expected)? {
                    return Err(std::io::Error::other(
                        "holder Session changed during exit receipt persistence",
                    ));
                }
            }
            Ok(())
        },
    )
}

/// Stop under the exact runtime lease without waiting on the caller's event loop.
/// A child that has not exited yet is finalized by the existing background proof path.
pub fn stop_exact_runtime(
    sessions_dir: &Path,
    expected_session: &gwt_agent::SessionExecutionIdentity,
    expected_incarnation: u64,
    pty: &PtyHandle,
    window_id: &str,
    capability: Option<(AgentCapabilityIssuer, String)>,
    retain_successor_handoff: bool,
) -> Result<(), String> {
    let proof = gwt_agent::ManualLaunchRuntimeProof {
        host_pid: std::process::id(),
        runtime_incarnation: expected_incarnation,
    };
    let stopped = crate::cli::execution_state::with_exact_active_manual_runtime_lease(
            sessions_dir,
            expected_session,
            proof,
            retain_successor_handoff,
            || {
                let reservation = capability
                    .as_ref()
                    .map(|(issuer, token)| {
                        issuer
                            .begin_manual_execution_handoff(
                                token,
                                &expected_session.execution_binding,
                            )
                            .map(|reservation| (issuer, reservation))
                    })
                    .transpose()
                    .map_err(std::io::Error::other)?;
                let stop_result = (|| {
                    tracing::info!(
                        target: "gwt.pane.teardown",
                        window_id,
                        stage = "exact_pty_kill",
                        outcome = "starting",
                        "starting exact-holder PTY teardown stage"
                    );
                    let kill_started = Instant::now();
                    let kill_result = pty.kill();
                    let kill_elapsed_ms =
                        u64::try_from(kill_started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    tracing::info!(
                        target: "gwt.pane.teardown",
                        window_id,
                        stage = "exact_pty_kill",
                        elapsed_ms = kill_elapsed_ms,
                        ok = kill_result.is_ok(),
                        outcome = if kill_result.is_ok() { "completed" } else { "failed" },
                        "exact-holder PTY teardown stage completed"
                    );
                    kill_result.map_err(|error| std::io::Error::other(error.to_string()))?;
                    // Issue #3705: never sleep on the GUI event loop waiting
                    // for reap. Persist under lease only when the child is
                    // already gone; otherwise `start_window_runtime_stop`
                    // finishes the proof on a background thread.
                    let exited = pty
                        .try_wait()
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                    if exited.is_some()
                        && !gwt_agent::persist_session_terminal_status_if_execution_identity_matches_under_lease(
                            sessions_dir,
                            expected_session,
                            expected_incarnation,
                            gwt_agent::AgentStatus::Stopped,
                        )?
                    {
                        return Err(std::io::Error::other(
                            "The holder Session changed before terminal proof persistence",
                        ));
                    }
                    Ok(())
                })();
                match stop_result {
                    Ok(()) => {
                        if let Some((issuer, reservation)) = reservation.as_ref() {
                            if !issuer.commit_manual_execution_handoff(reservation) {
                                return Err(std::io::Error::other(
                                    "The exact holder handoff reservation was lost",
                                ));
                            }
                            if !retain_successor_handoff
                                && !issuer.release_manual_execution_handoff(reservation)
                            {
                                return Err(std::io::Error::other(
                                    "The completed holder capability fence could not be released",
                                ));
                            }
                        }
                        Ok(())
                    }
                    Err(error) => {
                        if let Some((issuer, reservation)) = reservation.as_ref() {
                            if !issuer.rollback_manual_execution_handoff(reservation) {
                                return Err(std::io::Error::other(format!(
                                    "{error}; exact holder capability rollback failed"
                                )));
                            }
                        }
                        Err(error)
                    }
                }
            },
        )
        .map_err(|error| error.to_string())?;
    if stopped.is_none() {
        return Err("The exact durable holder authority changed before termination".to_string());
    }
    Ok(())
}

/// Finalize the captured unbound Session without discarding newer hook metadata.
pub fn finalize_legacy_session(
    sessions_dir: &Path,
    expected: &gwt_agent::Session,
) -> std::io::Result<bool> {
    if expected.execution_binding.is_some() {
        return Ok(false);
    }
    let mut rejected = false;
    let result = gwt_agent::update_session_if_changed(sessions_dir, &expected.id, |current| {
        if current.id != expected.id
            || current.created_at != expected.created_at
            || current.worktree_path != expected.worktree_path
            || current.branch != expected.branch
            || current.agent_id != expected.agent_id
            || current.linked_issue_number != expected.linked_issue_number
            || current.execution_binding.is_some()
            || (current.status == gwt_agent::AgentStatus::Stopped
                && !current.restore_window_on_startup)
        {
            rejected = true;
            // Abort the transaction, including migrations, for a replacement.
            return Err(std::io::Error::other(
                "legacy close is no longer applicable",
            ));
        }
        current.record_terminal_stop();
        Ok(())
    });
    match result {
        Err(error) if rejected || error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        result => result.map(|_| true),
    }
}

/// Disable restore for an exact historical view, without asserting process exit.
/// Current-Host registration and live/uncertain observations reject this path.
pub fn remove_restored_stopped_view(
    sessions_dir: &Path,
    expected: &gwt_agent::Session,
) -> std::io::Result<bool> {
    let expected_snapshot = serde_json::to_value(expected)?;
    let mut refused = false;
    let result = gwt_agent::update_session_if_changed(sessions_dir, &expected.id, |current| {
        let current_runtime =
            gwt_agent::runtime_state_path_for_pid(sessions_dir, std::process::id(), &expected.id);
        let no_local_runtime = match std::fs::symlink_metadata(current_runtime) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => return Err(error),
            Ok(_) => false,
        };
        let observations = crate::session_inventory::observe_session(current, sessions_dir);
        if serde_json::to_value(&*current)? != expected_snapshot
            || !no_local_runtime
            || !observations.sessions.is_empty()
            || !observations.uncertainties.is_empty()
        {
            refused = true;
            return Err(std::io::Error::other("stopped view removal is unconfirmed"));
        }
        current.restore_window_on_startup = false;
        Ok(())
    });
    if refused {
        Ok(false)
    } else {
        result.map(|_| true)
    }
}

/// Diagnose a slow exit without abandoning its terminal persistence.
const PANE_CLOSE_EXIT_PROOF_WAIT: Duration = Duration::from_secs(30);

/// Issue #3705 AC-3: name the pane whose teardown stalled so a hung
/// `pane.*` channel can be diagnosed from `~/.gwt/logs/` without guessing.
pub fn pane_teardown_stall_message(window_id: &str, stage: &str, elapsed_ms: u64) -> String {
    format!("PTY teardown stalled: window_id={window_id} stage={stage} elapsed_ms={elapsed_ms}")
}

struct TerminalPersistence {
    sessions_dir: PathBuf,
    identity: gwt_agent::SessionExecutionIdentity,
    incarnation: u64,
    window_id: String,
}

impl TerminalPersistence {
    fn persist(&self) -> std::io::Result<bool> {
        // Reacquire owner before Session even on the detached thread. Holding
        // only the Session lease cannot fence a new owner generation.
        crate::cli::execution_state::with_current_runtime_cleanup_session_execution_identity_lease(
            &self.sessions_dir,
            &self.identity,
            || {
                let runtime = match gwt_agent::SessionRuntimeState::load(
                    &gwt_agent::runtime_state_path(&self.sessions_dir, &self.identity.session_id),
                ) {
                    Ok(runtime) => runtime,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                    Err(error) => return Err(error),
                };
                if runtime.execution_identity.as_ref() != Some(&self.identity)
                    || runtime.runtime_incarnation != Some(self.incarnation)
                {
                    return Ok(false);
                }
                gwt_agent::persist_session_terminal_status_if_execution_identity_matches_under_lease(
                    &self.sessions_dir,
                    &self.identity,
                    self.incarnation,
                    gwt_agent::AgentStatus::Stopped,
                )
            },
        )?
        .transpose()
        .map(|result| result.unwrap_or(false))
    }

    fn record_failure(
        &self,
        exit_confirmed: bool,
        reason: String,
        last_failure: &mut Option<(bool, String)>,
    ) {
        if last_failure
            .as_ref()
            .is_some_and(|(exited, previous)| *exited == exit_confirmed && previous == &reason)
        {
            return;
        }
        // This diagnostic is independent of Session/owner mutation. It must
        // remain writable when acquiring either of their leases timed out.
        let result = (|| -> std::io::Result<()> {
            gwt_agent::validate_session_id_path_component(&self.identity.session_id)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
            let dir = self
                .sessions_dir
                .join("teardown-failures")
                .join(&self.identity.session_id);
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(format!("{}-{}.json", std::process::id(), self.incarnation));
            let mut scratch = tempfile::NamedTempFile::new_in(&dir)?;
            serde_json::to_writer(
                scratch.as_file_mut(),
                &serde_json::json!({
                    "schema_version": 1,
                    "identity": self.identity,
                    "host_pid": std::process::id(),
                    "runtime_incarnation": self.incarnation,
                    "window_id": self.window_id,
                    "exit_confirmed": exit_confirmed,
                    "reason": reason,
                    "recorded_at": chrono::Utc::now(),
                }),
            )?;
            scratch.as_file().sync_all()?;
            scratch.persist(&path).map_err(|error| error.error)?;
            #[cfg(unix)]
            std::fs::File::open(&dir)?.sync_all()?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                tracing::warn!(target: "gwt.pane.teardown", window_id = %self.window_id,
                    exit_confirmed, %reason, "terminal persistence failure recorded");
                *last_failure = Some((exit_confirmed, reason));
            }
            Err(error) => tracing::error!(target: "gwt.pane.teardown", window_id = %self.window_id,
                exit_confirmed, %reason, %error, "terminal persistence failure marker could not be saved"),
        }
    }
}

/// Commit the existing terminal proof synchronously after a captured worker reaped its child.
pub(crate) fn persist_exact_terminal_after_exit(
    pty: &PtyHandle,
    sessions_dir: &Path,
    identity: &gwt_agent::SessionExecutionIdentity,
    incarnation: u64,
    window_id: &str,
) -> std::io::Result<bool> {
    if pty
        .try_wait()
        .map_err(|error| std::io::Error::other(error.to_string()))?
        .is_none()
    {
        return Ok(false);
    }
    TerminalPersistence {
        sessions_dir: sessions_dir.into(),
        identity: identity.clone(),
        incarnation,
        window_id: window_id.into(),
    }
    .persist()
}

/// Finalize on a background thread, including an already-exited child: acquiring
/// owner/Session leases and syncing a diagnostic must never block the GUI.
/// Returns `false` because persistence is never completed synchronously.
pub fn persist_terminal_after_exit(
    pty: Arc<PtyHandle>,
    sessions_dir: PathBuf,
    identity: gwt_agent::SessionExecutionIdentity,
    incarnation: u64,
    window_id: String,
) -> bool {
    let target = TerminalPersistence {
        sessions_dir,
        identity,
        incarnation,
        window_id,
    };
    thread::spawn(move || {
        let started = Instant::now();
        let mut last_failure = None;
        let mut exited = false;
        let mut stall_reported = false;
        let mut deadline_reported = false;
        loop {
            if !exited {
                match pty.try_wait() {
                    Ok(status) => exited = status.is_some(),
                    Err(error) => target.record_failure(
                        false,
                        format!("PTY exit proof failed: {error}"),
                        &mut last_failure,
                    ),
                }
            }
            if exited {
                match target.persist() {
                    Ok(true) => return,
                    Ok(false) => {
                        target.record_failure(
                            true,
                            "exact terminal persistence was rejected".into(),
                            &mut last_failure,
                        );
                        return;
                    }
                    Err(error) => target.record_failure(true, error.to_string(), &mut last_failure),
                }
                // A lease timeout is not lost exit proof. Retry in this same
                // finalizer; a replacement identity stops the next attempt.
                thread::sleep(Duration::from_secs(1));
            } else {
                let elapsed = started.elapsed();
                if !stall_reported && elapsed >= Duration::from_secs(2) {
                    stall_reported = true;
                    let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
                    tracing::warn!(target: "gwt.pane.teardown", window_id = %target.window_id,
                        "{}", pane_teardown_stall_message(&target.window_id, "process_exit", elapsed_ms));
                }
                if !deadline_reported && elapsed >= PANE_CLOSE_EXIT_PROOF_WAIT {
                    deadline_reported = true;
                    target.record_failure(
                        false,
                        "PTY exit proof exceeded the diagnostic deadline".into(),
                        &mut last_failure,
                    );
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    });
    false
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use gwt_agent::{AgentId, AgentStatus, Session};
    use gwt_terminal::Pane;

    use super::{finalize_legacy_session, kill_and_reap};

    #[test]
    fn close_terminates_descendants_after_the_pty_leader_exits() {
        let temp = tempfile::tempdir().unwrap();
        let child_file = temp.path().join("child.pid");
        let (command, args) = if cfg!(windows) {
            (
                "powershell",
                vec![
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$child = Start-Process ping -ArgumentList '-n','61','127.0.0.1' -WindowStyle Hidden -PassThru; Set-Content -LiteralPath $env:GWT_TEST_CHILD_PID -Value $child.Id",
                ],
            )
        } else {
            (
                "/bin/sh",
                vec![
                    "-c",
                    "trap '' HUP; sleep 60 & printf '%s' \"$!\" > \"$GWT_TEST_CHILD_PID\"",
                ],
            )
        };
        let pane = Pane::new(
            "close-descendants".into(),
            command.into(),
            args.into_iter().map(str::to_owned).collect(),
            80,
            24,
            HashMap::from([(
                "GWT_TEST_CHILD_PID".into(),
                child_file.display().to_string(),
            )]),
            Some(temp.path().to_owned()),
        )
        .unwrap();
        #[cfg(windows)]
        pane.pty().write_input(b"\x1b[1;1R").unwrap();
        // test-hygiene: allow-wall-clock-deadline Bound actual OS child startup and leader exit; a virtual clock cannot observe them.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let child_pid = loop {
            if let Some(pid) = std::fs::read_to_string(&child_file)
                .ok()
                .and_then(|value| value.trim().parse::<u32>().ok())
            {
                break pid;
            }
            assert!(
                // test-hygiene: allow-wall-clock-deadline Bound the external child's PID publication, not a scheduling-order assertion.
                std::time::Instant::now() < deadline,
                "child PID not published"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        while pane.pty().try_wait().unwrap().is_none() {
            assert!(
                // test-hygiene: allow-wall-clock-deadline Observe the real PTY leader exit before exercising descendant cleanup.
                std::time::Instant::now() < deadline,
                "PTY leader did not exit"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let descendant_alive = || {
            #[cfg(windows)]
            {
                crate::process::host_process_liveness(child_pid)
                    .expect("native descendant liveness must be observable")
            }
            #[cfg(unix)]
            {
                let mut system =
                    sysinfo::System::new().expect("initialize test process observation");
                let pid = sysinfo::Pid::from_u32(child_pid);
                system.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::Some(&[pid]),
                    true,
                    sysinfo::ProcessRefreshKind::nothing(),
                );
                system
                    .process(pid)
                    .is_some_and(|process| process.status() != sysinfo::ProcessStatus::Zombie)
            }
        };
        assert!(descendant_alive());
        assert!(kill_and_reap(pane.pty(), "close-descendants").unwrap());
        let descendant_survived = descendant_alive();
        // Drop still owns the group/Job and cleans up even when the assertion fails.
        drop(pane);
        assert!(
            !descendant_survived,
            "close left descendant {child_pid} running"
        );
    }

    #[test]
    fn close_reaps_child_and_finalizes_only_the_captured_session_without_gui() {
        let temp = tempfile::tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let mut session = Session::new(temp.path(), "feature/test", AgentId::Codex);
        session.status = AgentStatus::Running;
        session.restore_window_on_startup = true;
        session.save(&sessions_dir).unwrap();
        let (command, args) = if cfg!(windows) {
            ("cmd", vec!["/D", "/S", "/C", "ping -n 31 127.0.0.1 >NUL"])
        } else {
            ("/bin/sh", vec!["-c", "sleep 30"])
        };
        let pane = Pane::new(
            "close-test".into(),
            command.into(),
            args.into_iter().map(str::to_owned).collect(),
            80,
            24,
            HashMap::new(),
            Some(temp.path().to_owned()),
        )
        .unwrap();
        assert!(kill_and_reap(pane.pty(), "close-test").unwrap());
        assert!(pane.pty().try_wait().unwrap().is_some());
        // Issue #5495: hooks update the durable Session after the GUI cached it.
        let mut hooked = session.clone();
        hooked.agent_session_id = Some("native-close-test".into());
        hooked.project_state_root = Some(temp.path().to_owned());
        hooked.record_hook_event("PostToolUse");
        hooked.save(&sessions_dir).unwrap();
        assert!(finalize_legacy_session(&sessions_dir, &session).unwrap());
        let path = sessions_dir.join(format!("{}.toml", session.id));
        let stopped = Session::load(&path).unwrap();
        assert_eq!(stopped.status, AgentStatus::Stopped);
        assert!(!stopped.restore_window_on_startup);
        assert_eq!(stopped.agent_session_id, hooked.agent_session_id);
        assert_eq!(stopped.project_state_root, hooked.project_state_root);
        assert_eq!(stopped.last_hook_event_at, hooked.last_hook_event_at);
        let bytes = std::fs::read(&path).unwrap();
        assert!(!finalize_legacy_session(&sessions_dir, &session).unwrap());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn legacy_close_preserves_same_id_successor_and_new_execution_binding() {
        let temp = tempfile::tempdir().unwrap();
        let mut captured = Session::new(temp.path(), "work/closed", AgentId::Codex);
        captured.repo_hash = Some("repo-close-test".into());
        captured.linked_issue_number = Some(5495);
        captured.restore_window_on_startup = true;
        let path = temp.path().join(format!("{}.toml", captured.id));
        for bound in [false, true] {
            let mut successor = captured.clone();
            if bound {
                successor.execution_binding = Some(gwt_agent::SessionExecutionBinding {
                    schema_version: 1,
                    session_id: captured.id.clone(),
                    repo_hash: "repo-close-test".into(),
                    owner_kind: "issue".into(),
                    owner_number: 5495,
                    identity: gwt_agent::ExecutionBindingIdentity {
                        generation_id: "successor-generation".into(),
                        binding_id: "successor-binding".into(),
                        ledger_head_hash: "successor-head".into(),
                    },
                    capability_generation: 1,
                });
            } else {
                successor.created_at += chrono::Duration::seconds(1);
            }
            successor.save(temp.path()).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert!(!finalize_legacy_session(temp.path(), &captured).unwrap());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}
