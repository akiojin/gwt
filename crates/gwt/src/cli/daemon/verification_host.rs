//! The `verify.run` side of daemon-hosted verification (Issue #4409).
//!
//! [`locate`] answers the one question the placement decision needs — is there
//! a daemon that can launch this workload outside the caller's process tree —
//! and [`run`] hands one command over to it.
//!
//! The connection is the leash. `run` holds it open for the child's whole
//! lifetime, so a caller that dies mid-run drops the socket and the daemon
//! reclaims the workload without anyone having to notice (AC-2).

use std::path::Path;

use gwt_core::daemon::{
    ClientFrame, DaemonEndpoint, DaemonFrame, RuntimeScope, RuntimeTarget,
    VerificationSpawnAccepted, VerificationSpawnRequest, VERIFICATION_SPAWN_MIN_PROTOCOL_VERSION,
};
use gwt_core::verification_priority::DaemonAvailability;

use super::client::DaemonClient;

/// Where a `verify.run` launches its commands from (Issue #4409).
///
/// Resolved once per run rather than per command: the answer cannot change
/// mid-run, and a per-command resolution would put a daemon lookup in front of
/// every matrix entry.
#[derive(Debug, Default)]
pub(crate) enum VerificationHost {
    /// Launch from the calling process. Chosen only when that process is
    /// already at baseline priority, so there is nothing to escape.
    ///
    /// This is the default because a caller that never resolved a host has not
    /// asked to escape anything — internal fixtures and the low-level
    /// `run_verification` primitive. Defaulting the other way would put a
    /// daemon lookup in front of every such run.
    #[default]
    Inherit,
    /// Launch from the daemon, outside the caller's process tree.
    Daemon(Box<DaemonEndpoint>),
}

impl VerificationHost {
    /// How the lease record and the run transcript name this host (AC-4).
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Daemon(_) => "daemon",
        }
    }
}

/// Decide where a run's commands are launched, or refuse the run.
///
/// A refusal is deliberate and final (AC-5): the calling process runs at a
/// degraded nice value it cannot lower, so spawning in place would hand every
/// command the agent launch policy's priority — the 54x starvation #4405
/// measured. Silently doing it anyway is the failure mode the check exists to
/// prevent, so there is no fallback.
pub(crate) fn resolve(worktree: &Path) -> Result<(VerificationHost, String), String> {
    resolve_with(
        worktree,
        gwt_core::verification_priority::LauncherPriority::current(),
    )
}

/// Set by a launcher that has already decided its priority is acceptable, so
/// gwt must not look for an escape.
///
/// This is what makes the refusal above a refusal of *implicit* fallback
/// rather than of the whole in-tree path: a harness that started gwtd itself,
/// or a test exercising the record rather than the placement, states the
/// decision instead of having gwt guess it. Every such run says so in its
/// transcript and in its lease record, so the choice never travels silently
/// with the evidence.
const SPAWN_HOST_ENV: &str = "GWT_VERIFY_SPAWN_HOST";
const SPAWN_HOST_ENV_INHERIT: &str = "inherit";

/// [`resolve`] with the launcher's priority supplied rather than observed, so
/// the refusal can be tested without renicing the test runner.
fn resolve_with(
    worktree: &Path,
    launcher: gwt_core::verification_priority::LauncherPriority,
) -> Result<(VerificationHost, String), String> {
    use gwt_core::verification_priority::{decide_placement, SpawnPlacement};

    match std::env::var(SPAWN_HOST_ENV) {
        Ok(declared) if declared == SPAWN_HOST_ENV_INHERIT => {
            return Ok((
                VerificationHost::Inherit,
                format!(
                    "spawn-host: inherit (declared via {SPAWN_HOST_ENV}; the launcher runs at \
                     nice {} and its commands inherit that)\n",
                    launcher
                        .nice
                        .map(|nice| nice.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                ),
            ));
        }
        Ok(declared) => {
            return Err(format!(
                "{SPAWN_HOST_ENV} is set to '{declared}', which is not a spawn host. The only \
                 accepted value is '{SPAWN_HOST_ENV_INHERIT}', which declares that the launcher's \
                 own priority is acceptable for verification. Unset it to let gwt choose."
            ));
        }
        Err(_) => {}
    }

    let (availability, endpoint) = locate(worktree);
    match decide_placement(launcher, &availability) {
        SpawnPlacement::Reject { message } => Err(message),
        SpawnPlacement::Inherit { reason } => Ok((
            VerificationHost::Inherit,
            format!("spawn-host: inherit ({reason})\n"),
        )),
        SpawnPlacement::Delegate => {
            let endpoint = endpoint.ok_or_else(|| {
                "the daemon was reported available without an endpoint".to_string()
            })?;
            let note = format!(
                "spawn-host: daemon pid {} (launcher runs at nice {}; commands are launched \
                 outside this process tree so they do not inherit it)\n",
                endpoint.pid,
                launcher
                    .nice
                    .map(|nice| nice.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
            );
            Ok((VerificationHost::Daemon(Box::new(endpoint)), note))
        }
    }
}

/// The lease record's answer to "is this holder's workload inside the agent
/// tree, and at what priority" (Issue #4409 AC-4).
///
/// `refused` is a real answer, not an error: a holder that cannot escape the
/// tree will not run anything, and a waiter deciding whether to keep queueing
/// needs to see that.
pub(crate) fn describe_for_lease(worktree: &Path) -> (String, Option<i32>) {
    let nice = gwt_core::verification_priority::LauncherPriority::current().nice;
    let label = match resolve(worktree) {
        Ok((host, _)) => host.label().to_string(),
        Err(_) => "refused".to_string(),
    };
    (label, nice)
}

/// What a delegated command produced.
pub(crate) struct DelegatedRun {
    pub exit_code: i32,
    pub accepted: VerificationSpawnAccepted,
    /// The daemon had to kill descendants that outlived the runner.
    pub reclaimed_survivors: bool,
}

/// Find a daemon that can host verification for `worktree`.
///
/// The exact endpoint for this worktree is preferred, but any live daemon in
/// the same project will do: the request carries its own working directory, so
/// a daemon is a launcher for the project rather than for one checkout. That
/// distinction is what makes the feature deliverable — one GUI owns a single
/// daemon while a project can have hundreds of agent worktrees, and requiring
/// a per-worktree daemon would refuse verification almost everywhere.
///
/// A running daemon that is too old is reported as
/// [`DaemonAvailability::Incompatible`] rather than folded into "absent",
/// because the two need different advice: one is started, the other upgraded.
pub(crate) fn locate(worktree: &Path) -> (DaemonAvailability, Option<DaemonEndpoint>) {
    let Ok(scope) = RuntimeScope::from_project_root(worktree, RuntimeTarget::Host) else {
        return (DaemonAvailability::Absent, None);
    };
    let gwt_home = gwt_core::paths::gwt_home();
    let exact_path = scope.endpoint_path(&gwt_home);
    let Ok(entries) = std::fs::read_dir(scope.daemon_dir(&gwt_home)) else {
        return (DaemonAvailability::Absent, None);
    };

    let mut best: Option<(bool, DaemonEndpoint)> = None;
    let mut outdated: Option<DaemonEndpoint> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Ok(payload) = std::fs::read(&path) else {
            continue;
        };
        let Ok(endpoint) = serde_json::from_slice::<DaemonEndpoint>(&payload) else {
            continue;
        };
        // Liveness is checked apart from the protocol version on purpose: a
        // live daemon of the wrong version is a different answer from no
        // daemon at all, and `is_usable` collapses the two.
        if endpoint.scope.repo_hash != scope.repo_hash
            || endpoint.scope.target != scope.target
            || endpoint.bind.trim().is_empty()
            || endpoint.auth_token.trim().is_empty()
            || !endpoint.has_live_owner(crate::process::is_process_alive)
        {
            continue;
        }
        if endpoint.protocol_version < VERIFICATION_SPAWN_MIN_PROTOCOL_VERSION {
            outdated.get_or_insert(endpoint);
            continue;
        }
        let exact = path == exact_path;
        if exact {
            return (DaemonAvailability::Available, Some(endpoint));
        }
        best.get_or_insert((exact, endpoint));
    }

    match (best, outdated) {
        (Some((_, endpoint)), _) => (DaemonAvailability::Available, Some(endpoint)),
        (None, Some(endpoint)) => (
            DaemonAvailability::Incompatible {
                protocol_version: endpoint.protocol_version,
            },
            Some(endpoint),
        ),
        (None, None) => (DaemonAvailability::Absent, None),
    }
}

/// Run one verification command on the daemon and wait for it to finish.
pub(crate) fn run(
    endpoint: &DaemonEndpoint,
    request: &VerificationSpawnRequest,
) -> Result<DelegatedRun, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("tokio runtime build failed: {err}"))?;
    runtime.block_on(async {
        let mut client = DaemonClient::connect(endpoint).await?;
        client
            .send_frame(&ClientFrame::SpawnVerification(request.clone()))
            .await?;

        let accepted = match client.read_frame::<DaemonFrame>().await? {
            DaemonFrame::VerificationAccepted(accepted) => accepted,
            DaemonFrame::Error { message } => {
                return Err(format!("daemon refused the verification spawn: {message}"))
            }
            other => return Err(format!("expected VerificationAccepted, got: {other:?}")),
        };

        // No timeout: a verification matrix legitimately runs for an hour, and
        // the daemon already bounds the child by this connection's lifetime.
        loop {
            match client.read_frame::<DaemonFrame>().await? {
                DaemonFrame::VerificationFinished(finished) => {
                    return Ok(DelegatedRun {
                        exit_code: finished.exit_code,
                        accepted,
                        reclaimed_survivors: finished.reclaimed_survivors,
                    })
                }
                DaemonFrame::Error { message } => {
                    return Err(format!("daemon reported a spawn failure: {message}"))
                }
                // The daemon may fan unrelated control frames down any
                // connection; none of them ends this run.
                _ => continue,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::verification_priority::LauncherPriority;

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    /// Hold the env lock with `GWT_VERIFY_SPAWN_HOST` cleared.
    ///
    /// `resolve_with` consults that variable before anything else, so a test
    /// asserting what gwt *chooses* is otherwise decided by whoever launched
    /// the test binary. That is not hypothetical: an agent worktree cannot run
    /// `verify.run` on a host without a current daemon unless it declares
    /// `inherit`, and under that declaration the refusal test below inverts —
    /// it was observed failing for exactly this reason. The placement tests
    /// have to pin the variable rather than inherit it.
    fn without_declared_spawn_host() -> (
        std::sync::MutexGuard<'static, ()>,
        gwt_core::test_support::ScopedEnvVar,
    ) {
        let guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cleared = gwt_core::test_support::ScopedEnvVar::unset(SPAWN_HOST_ENV);
        (guard, cleared)
    }

    /// AC-5: a degraded launcher with no daemon to escape to is refused. The
    /// refusal is the point — an in-tree fallback here would silently hand the
    /// matrix the agent's nice value, which is the defect #4409 exists to fix.
    #[test]
    fn a_degraded_launcher_without_a_daemon_is_refused() {
        let _env = without_declared_spawn_host();
        let dir = scratch();
        let error = resolve_with(dir.path(), LauncherPriority { nice: Some(10) })
            .expect_err("no daemon can host this worktree");
        assert!(error.contains("nice 10"), "{error}");
        assert!(error.contains("no gwt daemon is reachable"), "{error}");
    }

    /// The same launcher at baseline priority runs in place: there is nothing
    /// to escape, so requiring a daemon would break every plain terminal and
    /// CI invocation for no gain.
    #[test]
    fn a_baseline_launcher_runs_in_place_without_a_daemon() {
        let _env = without_declared_spawn_host();
        let dir = scratch();
        let (host, note) =
            resolve_with(dir.path(), LauncherPriority { nice: Some(0) }).expect("no escape needed");
        assert_eq!(host.label(), "inherit");
        assert!(note.starts_with("spawn-host: inherit"), "{note}");
    }

    /// AC-4 feeds the lease record from the same decision, so a waiter never
    /// sees a spawn host that disagrees with what the run actually did.
    #[test]
    fn the_lease_description_names_the_same_host_the_run_uses() {
        let _env = without_declared_spawn_host();
        let dir = scratch();
        let (label, _) = describe_for_lease(dir.path());
        assert!(
            matches!(label.as_str(), "inherit" | "daemon" | "refused"),
            "unexpected lease spawn-host label: {label}"
        );
    }

    /// A worktree with no endpoint descriptor at all is `Absent`, not a
    /// crash — the ordinary case for a project whose GUI is not running.
    #[test]
    fn a_worktree_without_a_daemon_endpoint_reports_absent() {
        let dir = scratch();
        let (availability, endpoint) = locate(dir.path());
        assert_eq!(availability, DaemonAvailability::Absent);
        assert!(endpoint.is_none());
    }

    /// Measured on a live host before this was allowed: 455 agent worktrees
    /// shared 2 daemon endpoints, because one GUI owns one daemon for the
    /// project rather than one per checkout. Requiring the worktree's own
    /// endpoint would have refused verification in every agent worktree, so a
    /// daemon is a launcher for the project — the request carries its own
    /// working directory.
    #[test]
    fn a_sibling_worktrees_daemon_can_host_this_worktree() {
        let home = scratch();
        let project = scratch();
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
            .expect("project scope");
        let mut sibling = scope.clone();
        sibling.worktree_hash = "a-different-checkout".to_string();
        let endpoint = DaemonEndpoint::new(
            sibling.clone(),
            std::process::id(),
            "/tmp/sibling.sock".to_string(),
            "token".to_string(),
            "9.99.0".to_string(),
        );
        let gwt_home = home.path().join(".gwt");
        std::fs::create_dir_all(sibling.daemon_dir(&gwt_home)).expect("daemon dir");
        std::fs::write(
            sibling.endpoint_path(&gwt_home),
            serde_json::to_vec(&endpoint).expect("serialize endpoint"),
        )
        .expect("write sibling endpoint");

        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let (availability, found) = locate(project.path());
        assert_eq!(availability, DaemonAvailability::Available);
        assert_eq!(
            found.expect("sibling endpoint").scope.worktree_hash,
            "a-different-checkout"
        );
    }
}
