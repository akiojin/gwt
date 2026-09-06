//! The production subject that starts and keeps the runtime daemon alive.
//!
//! Issue #3633 (and #3505 before it): `gwt_core::daemon::resolve_bootstrap_action`
//! has always been able to say "nobody is serving this scope, start a daemon",
//! but no production caller acted on it. `cli::daemon::report_status` printed
//! `stopped`, the GUI front door persisted an `internal://gwt-front-door`
//! sentinel into the endpoint slot, and the only code path that ever reached
//! the serve loop was a human sending a `daemon.start` envelope. The result was
//! a permanently unreachable control lane (`daemon_control_unavailable`,
//! `daemon.subscribe` resolution failures) while the Issue Monitor still
//! reported a healthy snapshot.
//!
//! This module closes that gap. [`DaemonSupervisor::ensure_running`] is
//! idempotent and cheap, so the caller does not need its own supervision
//! thread, backoff schedule, or restart bookkeeping: calling it on the existing
//! Issue Monitor tick both starts the daemon and replaces one that died.
//!
//! Issue #3526 settled the Windows residency model on the same shape: the
//! daemon is a user-session child of the GUI (named-pipe transport), not a
//! Windows Service — a service could not create agent panes anyway and
//! would not share the per-user `~/.gwt` state. Only the child's signal
//! delivery differs: Unix sends `SIGTERM` so the serve loop unlinks its
//! socket, Windows terminates the child and relies on the liveness-driven
//! endpoint / authority-fence recovery that every crash already needs.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Child,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, PoisonError,
    },
};

/// What one [`DaemonSupervisor::ensure_running`] call did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonEnsureOutcome {
    /// A usable endpoint already names a live daemon for this scope.
    AlreadyRunning { pid: u32 },
    /// A daemon child was started by this call.
    Spawned { pid: u32 },
    /// A daemon child started by an earlier call is alive but has not
    /// published its endpoint yet. Starting another one here is what produces
    /// two drivers for one project.
    Starting { pid: u32 },
}

/// Everything one `gwtd` daemon child needs.
///
/// `gwtd` refuses legacy argv invocations (`gwtd daemon start` exits 2 with
/// "use stdin JSON envelope"), so the request carries the envelope rather than
/// arguments. `current_dir` matters: the daemon derives its [`RuntimeScope`]
/// from its own working directory.
///
/// [`RuntimeScope`]: gwt_core::daemon::RuntimeScope
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonSpawnRequest {
    pub program: PathBuf,
    pub current_dir: PathBuf,
    pub stdin_envelope: String,
}

/// Build the spawn request for one project's daemon.
pub fn daemon_spawn_request(gwtd_path: &Path, project_root: &Path) -> DaemonSpawnRequest {
    DaemonSpawnRequest {
        program: gwtd_path.to_path_buf(),
        current_dir: project_root.to_path_buf(),
        stdin_envelope: "{\"schema_version\":1,\"operation\":\"daemon.start\",\"params\":{}}\n"
            .to_string(),
    }
}

/// Where one daemon child is being started.
pub struct DaemonSpawnContext<'a> {
    pub project_root: &'a Path,
    /// The endpoint file this daemon is expected to publish. Also anchors the
    /// child's diagnostic log, so a daemon that dies before publishing leaves
    /// its reason next to the slot an operator is already looking at.
    pub endpoint_path: &'a Path,
}

type DaemonSpawner = Box<dyn Fn(&DaemonSpawnContext<'_>) -> std::io::Result<Child> + Send + Sync>;

/// Where a daemon child's stderr is captured.
///
/// Issue #3633: the first isolated end-to-end run of this supervisor produced
/// a child that exited instantly with no trace anywhere — the real reason
/// (`path must be shorter than SUN_LEN`, Issue #3476) went to `/dev/null`.
/// A silent daemon failure is the same class of bug this Issue is about, so
/// the reason is written where the endpoint would have been.
pub fn daemon_stderr_log_path(endpoint_path: &Path) -> PathBuf {
    let mut file_name = endpoint_path
        .file_stem()
        .map(|stem| stem.to_os_string())
        .unwrap_or_default();
    file_name.push(".daemon-stderr.log");
    endpoint_path.with_file_name(file_name)
}

/// Ambient inputs for one ensure pass.
///
/// Production reads the real gwt home and the shared liveness probe; tests
/// substitute a temporary home so the bookkeeping can be exercised without
/// touching the user's runtime directory.
pub struct DaemonEnsureInputs<'a> {
    pub gwt_home: PathBuf,
    pub is_process_alive: &'a dyn Fn(u32) -> bool,
}

/// Owns the daemon children this process started.
///
/// Children are reaped lazily inside [`DaemonSupervisor::ensure_running`]
/// rather than by a per-child waiter thread. That keeps the supervisor free of
/// background threads and, more importantly, means a pid is never signalled
/// after it has been reaped — the [`std::process::Child`] handle is the only
/// thing that ever addresses the process.
pub struct DaemonSupervisor {
    spawn: DaemonSpawner,
    children: Mutex<HashMap<PathBuf, Child>>,
    ensure_attempts: AtomicUsize,
}

impl DaemonSupervisor {
    /// The production supervisor: starts real `gwtd` daemon children.
    pub fn gwtd() -> Self {
        Self {
            spawn: Box::new(spawn_production_daemon),
            children: Mutex::new(HashMap::new()),
            ensure_attempts: AtomicUsize::new(0),
        }
    }

    /// A supervisor with a caller-supplied child factory, for tests that need
    /// the bookkeeping without a real daemon process.
    pub fn with_spawner(
        spawn: impl Fn(&DaemonSpawnContext<'_>) -> std::io::Result<Child> + Send + Sync + 'static,
    ) -> Self {
        Self {
            spawn: Box::new(spawn),
            children: Mutex::new(HashMap::new()),
            ensure_attempts: AtomicUsize::new(0),
        }
    }

    /// A supervisor that records ensure passes but never starts a process.
    /// Test harnesses use it so an unrelated suite cannot leave real daemons
    /// running on the developer's machine.
    pub fn disabled() -> Self {
        Self {
            spawn: Box::new(|_context| {
                Err(std::io::Error::other(
                    "this supervisor is disabled and never starts a daemon",
                ))
            }),
            children: Mutex::new(HashMap::new()),
            ensure_attempts: AtomicUsize::new(0),
        }
    }

    /// How many times [`Self::ensure_running`] has been asked to keep a daemon
    /// alive. The supervision contract is "the caller keeps calling", so this
    /// counter is what proves the caller is still wired up.
    pub fn ensure_attempts(&self) -> usize {
        self.ensure_attempts.load(Ordering::SeqCst)
    }

    /// Make sure a runtime daemon is serving `project_root`, starting one if
    /// it is missing or has died.
    pub fn ensure_running(&self, project_root: &Path) -> Result<DaemonEnsureOutcome, String> {
        let result = self.ensure_running_with(
            project_root,
            DaemonEnsureInputs {
                gwt_home: gwt_core::paths::gwt_home(),
                is_process_alive: &crate::process::is_process_alive,
            },
        );
        if let Err(ref error) = result {
            crate::error_report::report_error_and_publish(
                gwt_core::error_ledger::ErrorKind::DaemonFault,
                error.clone(),
                gwt_core::error_ledger::ErrorTarget {
                    project_root: Some(project_root.display().to_string()),
                    ..gwt_core::error_ledger::ErrorTarget::default()
                },
            );
        }
        result
    }

    pub fn ensure_running_with(
        &self,
        project_root: &Path,
        inputs: DaemonEnsureInputs<'_>,
    ) -> Result<DaemonEnsureOutcome, String> {
        use gwt_core::daemon::{
            resolve_bootstrap_action, DaemonBootstrapAction, RuntimeScope, RuntimeTarget,
            DAEMON_PROTOCOL_VERSION,
        };

        self.ensure_attempts.fetch_add(1, Ordering::SeqCst);
        let DaemonEnsureInputs {
            gwt_home,
            is_process_alive,
        } = inputs;

        let scope = RuntimeScope::from_project_root(project_root, RuntimeTarget::Host)
            .map_err(|error| format!("daemon scope resolution failed: {error}"))?;
        let endpoint_path = scope.endpoint_path(&gwt_home);

        let mut children = self.children.lock().unwrap_or_else(PoisonError::into_inner);
        let started_child_pid = reap_finished_child(&mut children, &endpoint_path);

        let action =
            resolve_bootstrap_action(&gwt_home, &scope, DAEMON_PROTOCOL_VERSION, is_process_alive)
                .map_err(|error| format!("daemon bootstrap resolution failed: {error}"))?;

        match action {
            DaemonBootstrapAction::Reuse(endpoint) => {
                Ok(DaemonEnsureOutcome::AlreadyRunning { pid: endpoint.pid })
            }
            DaemonBootstrapAction::Spawn { .. } => {
                if let Some(pid) = started_child_pid {
                    return Ok(DaemonEnsureOutcome::Starting { pid });
                }
                let child = (self.spawn)(&DaemonSpawnContext {
                    project_root,
                    endpoint_path: &endpoint_path,
                })
                .map_err(|error| {
                    format!(
                        "failed to start the runtime daemon for {project_root}: {error}",
                        project_root = project_root.display()
                    )
                })?;
                let pid = child.id();
                children.insert(endpoint_path, child);
                Ok(DaemonEnsureOutcome::Spawned { pid })
            }
        }
    }

    /// Whether a daemon child started by this supervisor is still running for
    /// `project_root`. Reaps first, so a child that has already exited reports
    /// `false` instead of lingering as a zombie.
    pub fn has_live_child_for(&self, project_root: &Path, gwt_home: &Path) -> bool {
        use gwt_core::daemon::{RuntimeScope, RuntimeTarget};

        let Ok(scope) = RuntimeScope::from_project_root(project_root, RuntimeTarget::Host) else {
            return false;
        };
        let endpoint_path = scope.endpoint_path(gwt_home);
        let mut children = self.children.lock().unwrap_or_else(PoisonError::into_inner);
        reap_finished_child(&mut children, &endpoint_path).is_some()
    }

    /// Terminate every daemon this process started.
    ///
    /// The daemon is deliberately not left behind for the next launch: nothing
    /// in the endpoint contract compares `daemon_version`, so a surviving
    /// daemon from a previous build would be reused after an update. Stopping
    /// the ones we own keeps exactly one, version-matched driver per project.
    pub fn shutdown(&self) {
        let mut children = self.children.lock().unwrap_or_else(PoisonError::into_inner);
        for (endpoint_path, mut child) in children.drain() {
            // SIGTERM lets the serve loop unlink its socket and endpoint file;
            // `Child::kill` sends SIGKILL and would leave both behind.
            terminate_gracefully(&mut child);
            if let Err(error) = child.wait() {
                tracing::warn!(
                    %error,
                    endpoint = %endpoint_path.display(),
                    "failed to reap a runtime daemon during shutdown"
                );
            }
        }
    }
}

impl Default for DaemonSupervisor {
    fn default() -> Self {
        Self::gwtd()
    }
}

impl std::fmt::Debug for DaemonSupervisor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaemonSupervisor")
            .field("ensure_attempts", &self.ensure_attempts())
            .finish_non_exhaustive()
    }
}

/// Drop a finished child from the table and return the pid of the one that is
/// still running, if any.
fn reap_finished_child(
    children: &mut HashMap<PathBuf, Child>,
    endpoint_path: &Path,
) -> Option<u32> {
    let child = children.get_mut(endpoint_path)?;
    let pid = child.id();
    match child.try_wait() {
        Ok(None) => Some(pid),
        Ok(Some(status)) => {
            tracing::warn!(
                pid,
                %status,
                endpoint = %endpoint_path.display(),
                stderr_log = %daemon_stderr_log_path(endpoint_path).display(),
                reason = %last_daemon_stderr_line(endpoint_path).unwrap_or_default(),
                "the runtime daemon exited; the next ensure pass will start a replacement"
            );
            children.remove(endpoint_path);
            None
        }
        Err(error) => {
            tracing::warn!(
                pid,
                %error,
                endpoint = %endpoint_path.display(),
                "could not read runtime daemon exit status; forgetting the child"
            );
            children.remove(endpoint_path);
            None
        }
    }
}

/// The last non-empty line the exited daemon wrote, so the warning that
/// announces the exit also carries the reason instead of only a status code.
fn last_daemon_stderr_line(endpoint_path: &Path) -> Option<String> {
    const MAX_REPORTED_BYTES: usize = 4096;
    let captured = std::fs::read_to_string(daemon_stderr_log_path(endpoint_path)).ok()?;
    let tail = captured
        .char_indices()
        .rev()
        .take(MAX_REPORTED_BYTES)
        .last()
        .map_or(captured.as_str(), |(index, _)| &captured[index..]);
    tail.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::to_string)
}

#[cfg(unix)]
fn terminate_gracefully(child: &mut Child) {
    let pid = child.id();
    if pid == 0 || pid > i32::MAX as u32 {
        return;
    }
    // SAFETY: SIGTERM to a pid this process owns as a child. The child has not
    // been reaped yet (we still hold its handle), so the pid cannot have been
    // recycled by another process.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

/// Windows has no cross-process cooperative signal for a windowless child
/// (console control events need a shared console, which a `CREATE_NO_WINDOW`
/// child does not have), so the supervisor terminates the daemon outright.
/// The endpoint descriptor and authority fence it leaves behind are exactly
/// what a crashed daemon leaves, and both are reclaimed by the liveness
/// probes in `resolve_bootstrap_action` and the fence recovery (Issue #3526).
#[cfg(windows)]
fn terminate_gracefully(child: &mut Child) {
    if let Err(error) = child.kill() {
        tracing::warn!(pid = child.id(), %error, "failed to terminate a runtime daemon");
    }
}

/// Start a real `gwtd` daemon for one project.
fn spawn_production_daemon(context: &DaemonSpawnContext<'_>) -> std::io::Result<Child> {
    use std::{io::Write, process::Stdio};

    use gwt_core::process::{resolved_command, ProcessPlanRequest};

    let gwtd_path = crate::cli::gwtd_resolver::resolve_gwtd_path().ok_or_else(|| {
        std::io::Error::other(
            "no gwtd binary could be resolved; set GWT_BIN_PATH or install gwtd on PATH",
        )
    })?;
    let request = daemon_spawn_request(&gwtd_path, context.project_root);
    let stderr_log_path = daemon_stderr_log_path(context.endpoint_path);
    if let Some(parent) = stderr_log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A file rather than a pipe: nothing in this process reads the daemon's
    // stderr, and a full pipe would block the daemon itself. Truncating on
    // each spawn keeps the file describing the current attempt.
    let stderr_log = std::fs::File::create(&stderr_log_path)?;

    // Go through the shared resolver rather than `Command::new` so the daemon
    // child gets the same executable resolution and window-hiding rules as
    // every other process gwt starts (Issues #3290, #3293).
    let mut command = resolved_command(
        ProcessPlanRequest::new(&request.program).current_dir(&request.current_dir),
    )
    .map_err(|error| std::io::Error::other(error.to_string()))?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_log));
    // Leave the launcher's process group. A daemon that shares it dies
    // with any signal aimed at the foreground job — which is how the
    // manually started daemons observed in Issue #3633 kept vanishing.
    // Windows children already run under `CREATE_NO_WINDOW` through
    // `resolved_command`, so no console signal can reach them.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;

    // gwtd's only sanctioned non-hook transport is one newline-delimited
    // stdin envelope; argv invocations are refused with exit 2.
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("gwtd child stdin was not captured"))?;
    stdin.write_all(request.stdin_envelope.as_bytes())?;
    stdin.flush()?;
    drop(stdin);

    tracing::info!(
        pid = child.id(),
        gwtd = %request.program.display(),
        project_root = %context.project_root.display(),
        stderr_log = %stderr_log_path.display(),
        "started a runtime daemon child"
    );
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_request_targets_the_project_it_serves() {
        let request = daemon_spawn_request(Path::new("/usr/local/bin/gwtd"), Path::new("/repo"));
        assert_eq!(request.program, Path::new("/usr/local/bin/gwtd"));
        assert_eq!(request.current_dir, Path::new("/repo"));
        assert!(request.stdin_envelope.contains("\"daemon.start\""));
    }

    #[test]
    fn the_default_supervisor_is_the_production_one() {
        // A `Default` that never starts anything would reproduce Issue #3505
        // exactly: a supervisor that exists, is called, and does nothing.
        assert!(
            supervisor_source_between("pub fn gwtd()", "pub fn with_spawner")
                .contains("spawn_production_daemon")
        );
        assert!(supervisor_source_between(
            "impl Default for DaemonSupervisor",
            "impl std::fmt::Debug"
        )
        .contains("Self::gwtd()"));
    }

    /// Issue #3633 AC-7: the recurrence guard for the spawn subject itself.
    /// #3505 was closed while no production code path started a daemon, so the
    /// regression to catch is "the subject degraded back into a no-op".
    #[test]
    fn the_production_spawner_starts_a_real_gwtd_daemon() {
        let spawner = supervisor_source_between("fn spawn_production_daemon", "\n#[cfg(test)]");
        assert!(
            spawner.contains("resolve_gwtd_path()"),
            "the production spawner must resolve a real gwtd binary"
        );
        assert!(
            spawner.contains("daemon_spawn_request"),
            "the production spawner must ask gwtd to start the daemon"
        );
        assert!(
            spawner.contains(".spawn()?"),
            "the production spawner must actually start a process"
        );
    }

    /// The exit warning is only useful if it carries the reason. A daemon that
    /// dies during bind writes one diagnostic line and nothing else, and that
    /// line is the whole difference between "it exited" and a diagnosis.
    #[test]
    fn the_exit_reason_is_the_last_thing_the_daemon_managed_to_say() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let endpoint_path = temp.path().join("worktree-hash.json");
        let log_path = daemon_stderr_log_path(&endpoint_path);

        assert_eq!(
            last_daemon_stderr_line(&endpoint_path),
            None,
            "a daemon that never wrote anything reports no reason rather than an empty one"
        );

        std::fs::write(
            &log_path,
            "gwtd daemon start: bind=/tmp/x.sock\n\
             gwtd daemon.start: failed to bind daemon socket: path must be shorter than SUN_LEN\n\
             \n   \n",
        )
        .expect("write stderr log");

        assert_eq!(
            last_daemon_stderr_line(&endpoint_path).as_deref(),
            Some(
                "gwtd daemon.start: failed to bind daemon socket: path must be shorter than SUN_LEN"
            ),
            "trailing blank lines must not hide the reason"
        );
    }

    /// A daemon that logs for weeks must not turn one warning into a
    /// multi-megabyte line.
    #[test]
    fn a_large_diagnostic_log_still_reports_only_its_last_line() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let endpoint_path = temp.path().join("worktree-hash.json");
        let mut captured = "noise\n".repeat(20_000);
        captured.push_str("the reason it died\n");
        std::fs::write(daemon_stderr_log_path(&endpoint_path), captured).expect("write log");

        assert_eq!(
            last_daemon_stderr_line(&endpoint_path).as_deref(),
            Some("the reason it died")
        );
    }

    fn supervisor_source_between(start: &str, end: &str) -> &'static str {
        let source = include_str!("daemon_supervisor.rs");
        let after = source
            .split_once(start)
            .unwrap_or_else(|| panic!("source marker not found: {start}"))
            .1;
        // Leak-free: `after` borrows from the `'static` `include_str!` slice.
        let (body, _) = after
            .split_once(end)
            .unwrap_or_else(|| panic!("source marker not found: {end}"));
        body
    }
}
