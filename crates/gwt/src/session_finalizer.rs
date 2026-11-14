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
    if pty
        .try_wait()
        .map_err(|error| std::io::Error::other(error.to_string()))?
        .is_some()
    {
        return Ok(true);
    }
    tracing::info!(
        target: "gwt.pane.teardown",
        window_id = %window_id,
        stage = "pty_kill",
        outcome = "starting",
        "starting detached PTY teardown stage"
    );
    let kill_started = Instant::now();
    let kill_result = pty.kill();
    let kill_elapsed_ms = u64::try_from(kill_started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        target: "gwt.pane.teardown",
        window_id = %window_id,
        stage = "pty_kill",
        elapsed_ms = kill_elapsed_ms,
        ok = kill_result.is_ok(),
        outcome = if kill_result.is_ok() { "completed" } else { "failed" },
        "detached PTY teardown stage completed"
    );
    kill_result.map_err(|error| std::io::Error::other(error.to_string()))?;
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
    let proof = gwt_agent::ManualLaunchRuntimeProof {
        host_pid: std::process::id(),
        runtime_incarnation: incarnation,
    };
    crate::cli::execution_state::with_exact_active_manual_runtime_lease(
        sessions_dir,
        identity,
        proof,
        false,
        || {
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
            if !gwt_agent::persist_session_restore_window_on_startup_if_execution_identity_matches_under_lease(
                sessions_dir,
                identity,
                false,
            )? {
                return Err(std::io::Error::other(
                    "holder Session changed before startup-restore persistence",
                ));
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

/// Finalize an unbound legacy Session only while its captured snapshot is unchanged.
pub fn finalize_legacy_session(
    sessions_dir: &Path,
    expected: &gwt_agent::Session,
) -> std::io::Result<bool> {
    let mut stopped = expected.clone();
    stopped.update_status(gwt_agent::AgentStatus::Stopped);
    stopped.restore_window_on_startup = false;
    stopped.save_if_unchanged(sessions_dir, expected)
}

/// How long a pane close keeps waiting for its PTY child to exit before it
/// stops trying to record the terminal status (Issue #4643 AC-1).
const PANE_CLOSE_EXIT_PROOF_WAIT: Duration = Duration::from_secs(30);

/// Issue #3705 AC-3: name the pane whose teardown stalled so a hung
/// `pane.*` channel can be diagnosed from `~/.gwt/logs/` without guessing.
pub fn pane_teardown_stall_message(window_id: &str, stage: &str, elapsed_ms: u64) -> String {
    format!("PTY teardown stalled: window_id={window_id} stage={stage} elapsed_ms={elapsed_ms}")
}

/// Persist exact terminal status immediately or after background child exit proof.
pub fn persist_terminal_after_exit(
    pty: Arc<PtyHandle>,
    sessions_dir: PathBuf,
    identity: gwt_agent::SessionExecutionIdentity,
    incarnation: u64,
    window_id: String,
) -> bool {
    let exited = pty.try_wait().ok().flatten().is_some();
    if exited {
        return gwt_agent::persist_session_terminal_status_if_execution_identity_matches(
            &sessions_dir,
            &identity,
            incarnation,
            gwt_agent::AgentStatus::Stopped,
        )
        .unwrap_or(false);
    } else {
        thread::spawn(move || {
            // Issue #4643: an agent routinely needs more than the
            // stall threshold to exit after SIGHUP. Giving up at
            // the threshold left the closed pane's sidecar live
            // forever, so keep waiting for the exit proof and
            // only report the stall once.
            let started = Instant::now();
            let stall_after = Duration::from_secs(2);
            let deadline = started + PANE_CLOSE_EXIT_PROOF_WAIT;
            let mut stall_reported = false;
            let mut exited = false;
            while Instant::now() < deadline {
                if pty.try_wait().ok().flatten().is_some() {
                    exited = true;
                    break;
                }
                if !stall_reported && started.elapsed() >= stall_after {
                    stall_reported = true;
                    let elapsed_ms =
                        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    tracing::warn!(
                        target: "gwt.pane.teardown",
                        window_id = %window_id,
                        elapsed_ms,
                        "{}",
                        pane_teardown_stall_message(&window_id, "process_exit", elapsed_ms)
                    );
                }
                thread::sleep(Duration::from_millis(10));
            }
            if exited {
                let _ = gwt_agent::persist_session_terminal_status_if_execution_identity_matches(
                    &sessions_dir,
                    &identity,
                    incarnation,
                    gwt_agent::AgentStatus::Stopped,
                );
            } else {
                // The sidecar keeps its PTY child identity, so the
                // worktree sweep still sees this launch end the
                // moment the child does (Issue #4643 AC-2).
                tracing::warn!(
                    target: "gwt.pane.teardown",
                    window_id = %window_id,
                    "closed pane's PTY child did not exit; its runtime sidecar was not terminalized"
                );
            }
        });
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use gwt_agent::{AgentId, AgentStatus, Session};
    use gwt_terminal::Pane;

    use super::{finalize_legacy_session, kill_and_reap};

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
        assert!(finalize_legacy_session(&sessions_dir, &session).unwrap());
        let path = sessions_dir.join(format!("{}.toml", session.id));
        let stopped = Session::load(&path).unwrap();
        assert_eq!(stopped.status, AgentStatus::Stopped);
        assert!(!stopped.restore_window_on_startup);
        let bytes = std::fs::read(&path).unwrap();
        assert!(!finalize_legacy_session(&sessions_dir, &session).unwrap());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
