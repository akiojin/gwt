//! Physical pane launches shared by project runtime owners.
//!
//! These functions return the owned pane. Callers install it and retain the
//! handshake guard through publication or uncertain-launch reconciliation.
use gwt_terminal::{
    pty::{ProcessPolicy, SpawnConfig},
    Pane,
};
use std::path::{Path, PathBuf};

/// Apply an agent resource policy to a gated launch without letting a
/// rejection fail it.
///
/// Issue #3942: the tree-wide priority of SPEC #1921 Phase 86 (#3813) is an
/// optimization for GUI responsiveness, not a precondition for running an
/// agent. `setpriority` returns EPERM on every host where the launcher may not
/// renice the target's process group, and propagating that turned an ordinary
/// launch into a user-facing `PTY creation failed: setpriority permission
/// denied`. This returns nothing on purpose: the spawn routes never hold an
/// outcome they could propagate, panic on, or drop silently.
fn best_effort_apply_policy(
    window_id: &str,
    pending: &gwt_terminal::PendingPane,
    policy: gwt_terminal::pty::ProcessPolicy,
) {
    note_unapplied_agent_resource_policy(window_id, pending.apply_policy(policy));
}

/// Warn about a rejected policy and let the target run at the inherited
/// priority. Split from `best_effort_apply_policy` so the warning contract is
/// testable without a live PTY.
fn note_unapplied_agent_resource_policy(
    window_id: &str,
    outcome: Result<(), gwt_terminal::TerminalError>,
) {
    if let Err(error) = outcome {
        tracing::warn!(
            window_id = %window_id,
            error = %error,
            "agent resource policy was not applied; continuing launch at the inherited priority"
        );
    }
}

#[cfg(any(test, feature = "test-gh-guard"))]
#[doc(hidden)]
pub fn note_unapplied_agent_resource_policy_for_test(
    id: &str,
    outcome: Result<(), gwt_terminal::TerminalError>,
) {
    note_unapplied_agent_resource_policy(id, outcome);
}

pub struct ActiveLaunchHandshakeCleanup {
    sessions_dir: PathBuf,
    handshake: Option<gwt_agent::SessionActiveLaunchHandshake>,
    retain_on_drop: bool,
}

impl ActiveLaunchHandshakeCleanup {
    pub fn new(
        sessions_dir: PathBuf,
        handshake: Option<gwt_agent::SessionActiveLaunchHandshake>,
    ) -> Self {
        Self {
            sessions_dir,
            handshake,
            retain_on_drop: false,
        }
    }

    pub fn child_started(&self) -> bool {
        matches!(
            self.handshake.as_ref().map(|marker| &marker.phase),
            Some(gwt_agent::SessionActiveLaunchPhase::ChildSpawned { .. })
        )
    }

    pub fn finish(&mut self) -> Result<(), String> {
        let Some(handshake) = self.handshake.as_ref() else {
            return Ok(());
        };
        if !crate::cli::execution_state::finish_active_session_launch_handshake(
            &self.sessions_dir,
            handshake,
        )
        .map_err(|error| error.to_string())?
        {
            return Err("Active launch handshake changed before Running publication".to_string());
        }
        self.handshake = None;
        self.retain_on_drop = false;
        Ok(())
    }

    pub fn retain_for_reconciliation(&mut self) {
        self.retain_on_drop = true;
    }
}

impl Drop for ActiveLaunchHandshakeCleanup {
    fn drop(&mut self) {
        if self.retain_on_drop {
            return;
        }
        if let Some(handshake) = self.handshake.as_ref() {
            let _ = crate::cli::execution_state::finish_active_session_launch_handshake(
                &self.sessions_dir,
                handshake,
            );
        }
    }
}

/// Spawn a shell or unbound agent, recording physical observation when supplied.
pub fn spawn_unbound_pane(
    id: &str,
    spawn_config: SpawnConfig,
    policy_gate: Option<(ProcessPolicy, PathBuf, Vec<String>)>,
    incarnation: u64,
    observation: Option<(&Path, &str)>,
) -> Result<Pane, String> {
    let record_observation = |child_pid: Option<u32>| {
        let Some((sessions_dir, session_id)) = observation else {
            return;
        };
        // Unbound launches and automatic restores need the same physical
        // process observation as producing launches. This does not grant
        // execution authority or reserve an Issue Monitor slot.
        let observation = (|| -> std::io::Result<()> {
            let child_pid = child_pid
                .ok_or_else(|| std::io::Error::other("agent PTY process id is unavailable"))?;
            let child_started_at = crate::process::host_process_start_time(child_pid)
                .ok_or_else(|| std::io::Error::other("agent PTY start time is unavailable"))?;
            let host_started_at = crate::process::host_process_start_time(std::process::id())
                .ok_or_else(|| std::io::Error::other("agent Host start time is unavailable"))?;
            gwt_agent::with_session_path_lease(sessions_dir, session_id, |state| {
                match state {
                    gwt_agent::SessionPathState::Present(_) => {}
                    gwt_agent::SessionPathState::Missing => {
                        return Err(std::io::Error::other("agent Session is missing"))
                    }
                    gwt_agent::SessionPathState::Error(error) => return Err(error),
                }
                let path = gwt_agent::runtime_state_path(sessions_dir, session_id);
                let mut runtime = match gwt_agent::SessionRuntimeState::load(&path) {
                    Ok(runtime) => runtime,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
                    }
                    Err(error) => return Err(error),
                };
                runtime.execution_identity = None;
                runtime.runtime_incarnation = Some(incarnation);
                runtime.host_started_at = Some(host_started_at);
                runtime.child_pid = Some(child_pid);
                runtime.child_started_at = Some(child_started_at);
                runtime.save(&path)
            })
        })();
        if let Err(error) = observation {
            tracing::warn!(window_id = %id, session_id = %session_id, %error,
                    "agent process observation could not be persisted");
        }
    };
    // SPEC #1921 Phase 86 (#3813): a policy-bearing AgentBootstrap launch
    // goes through the start gate so priority / Job limits exist before
    // the target can create its first descendant. Shell panes stay direct.
    let pane = if let Some((policy, gate_program, gate_args)) = policy_gate {
        let pending = Pane::new_pending_with_spawn_config(
            id.to_string(),
            spawn_config,
            gate_program,
            gate_args,
            uuid::Uuid::new_v4().to_string(),
        )
        .map_err(|error| error.to_string())?;
        best_effort_apply_policy(id, &pending, policy);
        // Finish the Session lease transaction before SessionStart can run.
        record_observation(pending.process_id());
        pending.release().map_err(|error| error.to_string())?
    } else {
        let pane = Pane::new_with_spawn_config(id.to_string(), spawn_config)
            .map_err(|error| error.to_string())?;
        record_observation(pane.pty().process_id());
        pane
    };
    Ok(pane)
}

/// Publish exact Session proof before releasing the gated producing child.
pub fn spawn_bound_pane(
    id: &str,
    spawn_config: SpawnConfig,
    resource_policy: Option<ProcessPolicy>,
    gate: (PathBuf, Vec<String>),
    incarnation: u64,
    expected: &gwt_agent::SessionExecutionIdentity,
    handshake_cleanup: &mut ActiveLaunchHandshakeCleanup,
) -> Result<Pane, String> {
    let (gate_program, gate_args) = gate;
    let pending = Pane::new_pending_with_spawn_config(
        id.to_string(),
        spawn_config,
        gate_program,
        gate_args,
        uuid::Uuid::new_v4().to_string(),
    )
    .map_err(|error| error.to_string())?;
    let child_pid = pending.process_id().ok_or_else(|| {
        "bound launch gate identity was unavailable before runtime proof publication".to_string()
    })?;
    let child_started_at = crate::process::host_process_start_time(child_pid).ok_or_else(|| {
        "bound launch gate start time was unavailable before runtime proof publication".to_string()
    })?;
    let host_started_at =
        crate::process::host_process_start_time(std::process::id()).ok_or_else(|| {
            "bound launch Host identity was unavailable before runtime proof publication"
                .to_string()
        })?;

    if let Some(handshake) = handshake_cleanup.handshake.as_ref() {
        let updated =
            crate::cli::execution_state::mark_active_session_launch_handshake_child_spawned(
                &handshake_cleanup.sessions_dir,
                handshake,
                child_pid,
                child_started_at,
            )
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "bound launch lost its exact child-spawned authority fence".to_string()
            })?;
        handshake_cleanup.handshake = Some(updated);
    }
    if !gwt_agent::persist_session_running_state_if_execution_identity_matches(
        &handshake_cleanup.sessions_dir,
        expected,
        incarnation,
        host_started_at,
        child_pid,
        child_started_at,
    )
    .map_err(|error| error.to_string())?
    {
        return Err(
            "bound launch Session identity changed before runtime proof publication".to_string(),
        );
    }

    // SPEC #1921 Phase 86 (#3813): the policy lands on the gated tree
    // after identity proof and before release, so the target never runs
    // ungoverned.
    if let Some(policy) = resource_policy {
        best_effort_apply_policy(id, &pending, policy);
    }
    let pane = pending.release().map_err(|error| error.to_string())?;
    Ok(pane)
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(unix)]
    fn physical_spawn_completes_without_gui_runtime() {
        let config = gwt_terminal::pty::SpawnConfig {
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "exit 0".to_string()],
            cols: 80,
            rows: 24,
            env: Default::default(),
            remove_env: Vec::new(),
            cwd: None,
        };
        let mut pane = super::spawn_unbound_pane("shared-spawn", config, None, 1, None)
            .expect("spawn without AppRuntime or event loop");
        assert!(pane.pty().process_id().is_some());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if *pane.check_status().expect("check child status")
                != gwt_terminal::PaneStatus::Running
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "child did not exit");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(
            pane.last_exit().is_some(),
            "spawn retains the process exit receipt"
        );
    }
}
