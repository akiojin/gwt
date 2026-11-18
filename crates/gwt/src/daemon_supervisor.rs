//! The production subject that starts and keeps the runtime daemon alive.
//!
//! Issue #3633 (and #3505 before it): `gwt_core::daemon::resolve_bootstrap_action`
//! has always been able to say "nobody is serving this scope, start a daemon",
//! but no production caller acted on it. `cli::daemon::report_status` printed
//! `stopped`, the GUI front door persisted an `internal://gwt-front-door`
//! sentinel into the endpoint slot, and the only code path that ever reached
//! the serve loop was a human sending a `daemon.start` envelope. The result was
//! a permanently unreachable control lane (`daemon_control_unavailable`,
//! `daemon.subscribe` resolution failures) while Issues still
//! reported a healthy snapshot.
//!
//! This module closes that gap. [`DaemonSupervisor::ensure_running`] is
//! idempotent and cheap, so the caller does not need its own supervision
//! thread, backoff schedule, or restart bookkeeping: calling it on the existing
//! Issues tick both starts the daemon and replaces one that died.
//!
//! GUI residency follows authenticated Rust subscriber leases in the daemon.
//! This supervisor only launches, authenticates incompatible-owner retirement,
//! and reaps completed children. Closing one GUI never signals a live shared
//! daemon; standalone starts remain resident without GUI presence.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Child,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, PoisonError,
    },
    time::{Duration, Instant},
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
    /// Issue #4038 (AC-6): a live daemon of another gwt version serves this
    /// scope. It was asked to terminate; nothing is spawned until its death
    /// is confirmed, so the next ensure tick starts the replacement.
    /// Issue #7541: the ask is a bounded state machine (cooperative signal,
    /// grace, forced signal, grace) advanced by later ensure ticks, not a
    /// one-shot signal that can be ignored forever.
    RetiringStale { pid: u32 },
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
pub fn daemon_spawn_request(
    gwtd_path: &Path,
    project_root: &Path,
    startup_intent: gwt_core::daemon::DaemonStartIntent,
) -> DaemonSpawnRequest {
    DaemonSpawnRequest {
        program: gwtd_path.to_path_buf(),
        current_dir: project_root.to_path_buf(),
        stdin_envelope: format!(
            "{}\n",
            serde_json::json!({"schema_version":1,"operation":"daemon.start","params":{"startup_intent":startup_intent}})
        ),
    }
}

/// Where one daemon child is being started.
pub struct DaemonSpawnContext<'a> {
    pub project_root: &'a Path,
    /// The endpoint file this daemon is expected to publish. Also anchors the
    /// child's diagnostic log, so a daemon that dies before publishing leaves
    /// its reason next to the slot an operator is already looking at.
    pub endpoint_path: &'a Path,
    pub startup_intent: gwt_core::daemon::DaemonStartIntent,
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

/// The signal one retirement step sends to a stale daemon (Issue #7541).
/// Retirement always starts with `Term`; `Kill` is sent only after the
/// cooperative grace elapsed with the captured identity still alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonRetireSignal {
    Term,
    Kill,
}

/// Ambient inputs for one ensure pass.
///
/// Production reads the real gwt home and the shared liveness probe; tests
/// substitute a temporary home so the bookkeeping can be exercised without
/// touching the user's runtime directory.
pub struct DaemonEnsureInputs<'a> {
    pub gwt_home: PathBuf,
    pub is_process_alive: &'a dyn Fn(u32) -> bool,
    /// Issue #4038 (AC-6): the version a reusable daemon must report.
    pub expected_daemon_version: &'a str,
    /// Issue #4038 (AC-6) + Issue #7541: retire a live daemon of another
    /// version. Called with [`DaemonRetireSignal::Term`] first; if the
    /// captured identity is still alive after the cooperative grace, called
    /// again with [`DaemonRetireSignal::Kill`]. Only ever called for a
    /// process whose PID + start-time identity was just verified.
    pub retire_stale_daemon: &'a dyn Fn(u32, DaemonRetireSignal),
}

/// Owns the daemon children this process started.
///
/// Children are reaped lazily inside [`DaemonSupervisor::ensure_running`]
/// rather than by a per-child waiter thread. That keeps the supervisor free of
/// background threads. Residency belongs to the daemon, not the GUI parent.
/// An external incompatible candidate must authenticate before it is signalled.
#[derive(Clone)]
struct PendingReplacement {
    endpoint: gwt_core::daemon::DaemonEndpoint,
    authority_fence: crate::IssueMonitorAuthorityFenceState,
    started_at: u64,
    stage: RetireStage,
    deadline: Instant,
}

/// One stage of the bounded retirement driven by
/// [`DaemonSupervisor::ensure_running`] (Issue #7541). The staging mirrors
/// `owner_recovery::terminate_with_grace` — cooperative request first,
/// forced termination only after its grace — but it advances across ensure
/// ticks instead of blocking one call on the wait, so a GUI tick never
/// stalls behind a daemon that ignores the cooperative signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetireStage {
    TermSent,
    KillSent,
}

/// Grace for a daemon to exit after the cooperative retirement signal
/// before the supervisor escalates (Issue #7541). Matches the 5 s
/// cooperative grace in `owner_recovery::terminate_with_grace`.
const RETIRE_TERM_GRACE: Duration = Duration::from_secs(5);
/// Grace for a daemon to exit after forced termination before the
/// supervisor reports the retirement as failed instead of spawning a
/// replacement into a still-held authority lease (Issue #7541).
const RETIRE_KILL_GRACE: Duration = Duration::from_secs(5);
struct SupervisedChild {
    project_root: PathBuf,
    child: Option<Child>,
    replacement: Option<PendingReplacement>,
}

#[derive(Default)]
struct OrphanObservations {
    generation: String,
    open_projects: HashSet<String>,
    observations: HashMap<String, OrphanObservation>,
    views: Vec<crate::protocol::OrphanDaemonView>,
}

#[derive(Clone)]
struct OrphanObservation {
    project_root: PathBuf,
    endpoint_path: PathBuf,
    endpoint: gwt_core::daemon::DaemonEndpoint,
    health: OrphanHealth,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
struct ExactRetirement {
    id: String,
    incarnation: u64,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct OrphanHealth {
    #[serde(flatten)]
    identity: crate::daemon_subscriber::ProjectOwnerIdentity,
    project_key: String,
    startup_intent: gwt_core::daemon::DaemonStartIntent,
    gui_presence_count: usize,
    #[serde(default)]
    draining: bool,
    #[serde(default)]
    reconciliation_required: bool,
    #[serde(default)]
    reconciliation: Vec<ExactRetirement>,
}

impl OrphanHealth {
    fn validate(&self, endpoint: &gwt_core::daemon::DaemonEndpoint) -> Result<(), String> {
        if self.project_key != endpoint.scope.repo_hash
            || self.identity.pid != endpoint.pid
            || self.identity.host_instance_id.is_empty()
            || self.identity.host_started_at == 0
            || crate::process::host_process_start_time(endpoint.pid)
                != Some(self.identity.host_started_at)
        {
            return Err("project daemon identity could not be verified".into());
        }
        Ok(())
    }
    fn retryable(&self) -> bool {
        self.startup_intent == gwt_core::daemon::DaemonStartIntent::GuiManaged
            && self.gui_presence_count == 0
            && self.draining
    }
}

pub struct DaemonSupervisor {
    spawn: DaemonSpawner,
    children: Mutex<HashMap<PathBuf, SupervisedChild>>,
    orphan_observations: Mutex<OrphanObservations>,
    ensure_attempts: AtomicUsize,
    in_flight: Mutex<HashSet<String>>,
    stopped: AtomicBool,
    update_in_progress: AtomicBool,
}

/// Keeps one project's queued or running ensure pass unique until its task drops.
pub struct DaemonEnsureReservation {
    supervisor: Arc<DaemonSupervisor>,
    project_key: String,
}

impl Drop for DaemonEnsureReservation {
    fn drop(&mut self) {
        self.supervisor
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.project_key);
    }
}

impl DaemonSupervisor {
    /// The production supervisor: starts real `gwtd` daemon children.
    pub fn gwtd() -> Self {
        Self {
            spawn: Box::new(spawn_production_daemon),
            children: Mutex::new(HashMap::new()),
            orphan_observations: Mutex::new(OrphanObservations::default()),
            ensure_attempts: AtomicUsize::new(0),
            in_flight: Mutex::new(HashSet::new()),
            stopped: AtomicBool::new(false),
            update_in_progress: AtomicBool::new(false),
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
            orphan_observations: Mutex::new(OrphanObservations::default()),
            ensure_attempts: AtomicUsize::new(0),
            in_flight: Mutex::new(HashSet::new()),
            stopped: AtomicBool::new(false),
            update_in_progress: AtomicBool::new(false),
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
            orphan_observations: Mutex::new(OrphanObservations::default()),
            ensure_attempts: AtomicUsize::new(0),
            in_flight: Mutex::new(HashSet::new()),
            stopped: AtomicBool::new(false),
            update_in_progress: AtomicBool::new(false),
        }
    }

    pub fn set_update_in_progress(&self, in_progress: bool) {
        self.update_in_progress.store(in_progress, Ordering::SeqCst);
    }

    pub fn update_in_progress(&self) -> bool {
        self.update_in_progress.load(Ordering::SeqCst)
    }

    /// Reserve an off-thread ensure using the same project identity as discovery.
    pub fn reserve_ensure(
        self: &Arc<Self>,
        project_root: &Path,
    ) -> Result<Option<DaemonEnsureReservation>, String> {
        if self.stopped.load(Ordering::SeqCst) || self.update_in_progress() {
            return Ok(None);
        }
        let scope = gwt_core::daemon::RuntimeScope::from_project_root(
            project_root,
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .map_err(|error| format!("daemon scope resolution failed: {error}"))?;
        let mut in_flight = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !in_flight.insert(scope.repo_hash.clone()) {
            return Ok(None);
        }
        Ok(Some(DaemonEnsureReservation {
            supervisor: Arc::clone(self),
            project_key: scope.repo_hash,
        }))
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
                expected_daemon_version: env!("CARGO_PKG_VERSION"),
                retire_stale_daemon: &terminate_pid_with_signal,
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
        use gwt_core::daemon::{DaemonBootstrapAction, RuntimeScope, RuntimeTarget};

        self.ensure_attempts.fetch_add(1, Ordering::SeqCst);
        let DaemonEnsureInputs {
            gwt_home,
            is_process_alive,
            expected_daemon_version,
            retire_stale_daemon,
        } = inputs;

        let scope = RuntimeScope::from_project_root(project_root, RuntimeTarget::Host)
            .map_err(|error| format!("daemon scope resolution failed: {error}"))?;
        let endpoint_path = scope.endpoint_path(&gwt_home);

        let mut children = self.children.lock().unwrap_or_else(PoisonError::into_inner);
        if self.stopped.load(Ordering::SeqCst) {
            return Err("daemon supervisor is shut down".to_string());
        }
        if self.update_in_progress() {
            return Err("Project owner is preparing for a GWT update".into());
        }
        let child_endpoint = child_endpoint_for_project(&children, &endpoint_path);
        let started_child_pid = reap_finished_child(&mut children, &child_endpoint);

        // Retirement outlives the old daemon's transports and descriptor.
        // Once authenticated, progress only the captured process identity;
        // resolving or authenticating again can fail during its shutdown.
        if let Some(previous) = children
            .get(&child_endpoint)
            .and_then(|slot| slot.replacement.clone())
        {
            let still_alive = is_process_alive(previous.endpoint.pid);
            let current_birth = crate::process::host_process_start_time(previous.endpoint.pid);
            if still_alive && current_birth.is_none() {
                return Err("pending daemon process identity could not be verified".into());
            }
            if still_alive && current_birth == Some(previous.started_at) {
                verify_pending_retirement_authority(&gwt_home, &previous)?;
                let now = Instant::now();
                if now >= previous.deadline {
                    match previous.stage {
                        RetireStage::TermSent => {
                            // Filesystem checks above can race process exit/PID reuse.
                            if !is_process_alive(previous.endpoint.pid)
                                || crate::process::host_process_start_time(previous.endpoint.pid)
                                    != Some(previous.started_at)
                            {
                                return Ok(DaemonEnsureOutcome::RetiringStale {
                                    pid: previous.endpoint.pid,
                                });
                            }
                            let pending = children
                                .get_mut(&child_endpoint)
                                .expect("pending retirement")
                                .replacement
                                .as_mut()
                                .expect("captured identity");
                            pending.stage = RetireStage::KillSent;
                            pending.deadline = now + RETIRE_KILL_GRACE;
                            retire_stale_daemon(previous.endpoint.pid, DaemonRetireSignal::Kill);
                        }
                        RetireStage::KillSent => {
                            return Err(format!(
                                "daemon (pid {}) did not exit after forced termination; replacement is paused until it exits",
                                previous.endpoint.pid
                            ));
                        }
                    }
                }
                return Ok(DaemonEnsureOutcome::RetiringStale {
                    pid: previous.endpoint.pid,
                });
            }
        }

        let action = crate::daemon_publisher::resolve_project_daemon_bootstrap(
            &gwt_home,
            &scope,
            Some(expected_daemon_version),
            is_process_alive,
        )?;

        let retirement_candidate = |action: &DaemonBootstrapAction| match action {
            DaemonBootstrapAction::RetireStaleVersion { endpoint } => Some(endpoint.clone()),
            DaemonBootstrapAction::Reuse(endpoint)
                if endpoint
                    .project_transport_url
                    .as_deref()
                    .is_none_or(|url| url.trim().is_empty()) =>
            {
                Some(endpoint.clone())
            }
            _ => None,
        };
        if let Some(endpoint) = retirement_candidate(&action) {
            let authority_fence = retirement_authority_fence(&gwt_home, &endpoint.scope)?;
            let started_at = crate::process::host_process_start_time(endpoint.pid);
            drop(children);
            let Some(_authenticated) = authenticate_retirement_candidate(&endpoint)? else {
                // Issue #7463: nothing serves the bind, so no daemon is behind
                // this descriptor (a live PID is a reused one). Clear it and
                // start a replacement; never signal the unproven PID.
                remove_unreachable_endpoint(&gwt_home, &endpoint)?;
                tracing::warn!(pid = endpoint.pid, bind = %endpoint.bind, project_root = %project_root.display(), "removed unreachable project daemon descriptor before replacement");
                let mut children = self.children.lock().unwrap_or_else(PoisonError::into_inner);
                if self.stopped.load(Ordering::SeqCst) || self.update_in_progress() {
                    return Err("daemon supervisor shut down during retirement verification".into());
                }
                let action = crate::daemon_publisher::resolve_project_daemon_bootstrap(
                    &gwt_home,
                    &scope,
                    Some(expected_daemon_version),
                    is_process_alive,
                )?;
                if retirement_candidate(&action).is_some() {
                    return Err("daemon authority changed during retirement verification".into());
                }
                return self.start_or_reuse(
                    project_root,
                    &endpoint_path,
                    &child_endpoint,
                    &mut children,
                    action,
                    started_child_pid,
                );
            };
            let started_at = started_at
                .filter(|started_at| {
                    Some(*started_at) == crate::process::host_process_start_time(endpoint.pid)
                })
                .ok_or_else(|| "daemon retirement PID birth was not confirmed".to_string())?;
            let mut children = self.children.lock().unwrap_or_else(PoisonError::into_inner);
            if self.stopped.load(Ordering::SeqCst) || self.update_in_progress() {
                return Err("daemon supervisor shut down during retirement verification".into());
            }
            let current = crate::daemon_publisher::resolve_project_daemon_bootstrap(
                &gwt_home,
                &scope,
                Some(expected_daemon_version),
                is_process_alive,
            )?;
            if !matches!(current, DaemonBootstrapAction::RetireStaleVersion { endpoint: current } | DaemonBootstrapAction::Reuse(current) if current == endpoint)
                || retirement_authority_fence(&gwt_home, &endpoint.scope)? != authority_fence
            {
                return Err("daemon authority changed during retirement verification".into());
            }
            if !is_process_alive(endpoint.pid)
                || crate::process::host_process_start_time(endpoint.pid) != Some(started_at)
            {
                return Err("daemon retirement PID birth changed before signaling".into());
            }
            let slot = children.entry(child_endpoint).or_insert(SupervisedChild {
                project_root: project_root.to_path_buf(),
                child: None,
                replacement: None,
            });
            // A concurrent ensure may have authenticated this same owner.
            if !slot.replacement.as_ref().is_some_and(|previous| {
                previous.endpoint == endpoint && previous.started_at == started_at
            }) {
                slot.replacement = Some(PendingReplacement {
                    endpoint: endpoint.clone(),
                    authority_fence,
                    started_at,
                    stage: RetireStage::TermSent,
                    deadline: Instant::now() + RETIRE_TERM_GRACE,
                });
                tracing::warn!(pid = endpoint.pid, project_root = %project_root.display(), "retiring authenticated incompatible project daemon before replacement");
                retire_stale_daemon(endpoint.pid, DaemonRetireSignal::Term);
            }
            return Ok(DaemonEnsureOutcome::RetiringStale { pid: endpoint.pid });
        }
        self.start_or_reuse(
            project_root,
            &endpoint_path,
            &child_endpoint,
            &mut children,
            action,
            started_child_pid,
        )
    }

    /// Reuse the resolved daemon or start a new child once no retirement is
    /// pending for this project.
    fn start_or_reuse(
        &self,
        project_root: &Path,
        endpoint_path: &Path,
        child_endpoint: &Path,
        children: &mut HashMap<PathBuf, SupervisedChild>,
        action: gwt_core::daemon::DaemonBootstrapAction,
        started_child_pid: Option<u32>,
    ) -> Result<DaemonEnsureOutcome, String> {
        use gwt_core::daemon::DaemonBootstrapAction;

        match action {
            DaemonBootstrapAction::Reuse(endpoint) => {
                Ok(DaemonEnsureOutcome::AlreadyRunning { pid: endpoint.pid })
            }
            DaemonBootstrapAction::RetireStaleVersion { .. } => {
                unreachable!("retirement handled above")
            }
            DaemonBootstrapAction::Spawn { .. } => {
                // Quit can stop admission while discovery is doing filesystem
                // work. Do not create a new child after that decision.
                if self.stopped.load(Ordering::SeqCst) || self.update_in_progress() {
                    return Err("daemon supervisor shut down during discovery".into());
                }
                if let Some(pid) = started_child_pid {
                    return Ok(DaemonEnsureOutcome::Starting { pid });
                }
                let startup_intent = children
                    .get(child_endpoint)
                    .and_then(|slot| slot.replacement.as_ref())
                    .map(|previous| previous.endpoint.startup_intent)
                    .unwrap_or(gwt_core::daemon::DaemonStartIntent::GuiManaged);
                let child = (self.spawn)(&DaemonSpawnContext {
                    project_root,
                    endpoint_path,
                    startup_intent,
                })
                .map_err(|error| {
                    format!(
                        "failed to start the runtime daemon for {project_root}: {error}",
                        project_root = project_root.display()
                    )
                })?;
                let pid = child.id();
                children.remove(child_endpoint);
                children.insert(
                    endpoint_path.to_path_buf(),
                    SupervisedChild {
                        project_root: project_root.to_path_buf(),
                        child: Some(child),
                        replacement: None,
                    },
                );
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
        let child_endpoint = child_endpoint_for_project(&children, &endpoint_path);
        reap_finished_child(&mut children, &child_endpoint).is_some()
    }

    /// Synchronize the current GUI projection; reopening invalidates every observation token.
    pub fn update_orphan_context(&self, generation: String, open_projects: HashSet<String>) {
        let mut state = self
            .orphan_observations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.generation != generation || state.open_projects != open_projects {
            *state = OrphanObservations {
                generation,
                open_projects,
                ..Default::default()
            };
        }
    }

    pub fn orphan_views(&self) -> Vec<crate::protocol::OrphanDaemonView> {
        self.orphan_observations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .views
            .clone()
    }

    /// Only already-known child slots and supplied Recent roots are inspected.
    pub fn observe_orphans(&self, generation: &str, mut roots: Vec<PathBuf>) -> Result<(), String> {
        let children = self.children.lock().unwrap_or_else(PoisonError::into_inner);
        let known = children
            .iter()
            .map(|(path, slot)| (slot.project_root.clone(), path.clone()))
            .collect::<HashMap<_, _>>();
        roots.extend(known.keys().cloned());
        drop(children);
        roots.sort();
        roots.dedup();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let mut observations = HashMap::new();
        let mut views = Vec::new();
        let mut seen = HashSet::new();
        for root in roots {
            self.check_orphan_generation(generation, None)?;
            let endpoint = crate::daemon_publisher::resolve_project_daemon_endpoint(&root)
                .ok()
                .flatten()
                .or_else(|| {
                    known
                        .get(&root)
                        .and_then(|path| read_orphan_endpoint(path).ok())
                });
            let Some(endpoint) = endpoint else {
                continue;
            };
            if !seen.insert(endpoint.scope.repo_hash.clone())
                || self
                    .check_orphan_generation(generation, Some(&endpoint.scope.repo_hash))
                    .is_err()
            {
                continue;
            }
            if !crate::process::is_process_alive(endpoint.pid) {
                continue;
            }
            let endpoint_path = endpoint.scope.endpoint_path(&gwt_core::paths::gwt_home());
            let inspected = runtime.block_on(inspect_orphan_health(&endpoint));
            let title = root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| endpoint.scope.repo_hash.clone());
            let inspection_failed = inspected.is_err();
            let (detail, observation_token) = match inspected {
                Ok(health) => {
                    let detail = if health.startup_intent
                        == gwt_core::daemon::DaemonStartIntent::Standalone
                    {
                        "Running as a standalone daemon"
                    } else if health.gui_presence_count != 0 {
                        "Another GUI is connected"
                    } else if !health.draining {
                        "Waiting for reconnection"
                    } else if health.reconciliation_required {
                        "Shutdown failed. Retry termination to recover this project."
                    } else {
                        "Shutting down"
                    }
                    .to_string();
                    let token = if health.retryable() {
                        let token = uuid::Uuid::new_v4().to_string();
                        observations.insert(
                            token.clone(),
                            OrphanObservation {
                                project_root: root.clone(),
                                endpoint_path,
                                endpoint: endpoint.clone(),
                                health,
                            },
                        );
                        Some(token)
                    } else {
                        None
                    };
                    (detail, token)
                }
                Err(_) => (
                    "The project connection could not be verified. Termination cannot be retried safely.".into(),
                    None,
                ),
            };
            if inspection_failed || observation_token.is_some() {
                report_orphan_fault(&root, &detail);
            }
            views.push(crate::protocol::OrphanDaemonView {
                project_key: endpoint.scope.repo_hash,
                title,
                detail,
                observation_token,
            });
        }
        let mut state = self
            .orphan_observations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.generation != generation {
            return Err("This observation expired. Refresh and try again.".into());
        }
        state.observations = observations;
        state.views = views;
        Ok(())
    }

    fn check_orphan_generation(
        &self,
        generation: &str,
        project_key: Option<&str>,
    ) -> Result<(), String> {
        let state = self
            .orphan_observations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.generation != generation
            || project_key.is_some_and(|key| state.open_projects.contains(key))
        {
            return Err("This observation expired. Refresh and try again.".into());
        }
        Ok(())
    }

    /// Tokens are consumed before network I/O, including every refusal/failure.
    pub fn reap_orphan(&self, generation: &str, token: &str) -> Result<(), String> {
        let observation = {
            let mut state = self
                .orphan_observations
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let observation = state
                .observations
                .remove(token)
                .ok_or("This observation expired. Refresh and try again.")?;
            for view in &mut state.views {
                if view.observation_token.as_deref() == Some(token) {
                    view.observation_token = None;
                }
            }
            observation
        };
        self.check_orphan_generation(generation, Some(&observation.endpoint.scope.repo_hash))?;
        let result = self.retry_orphan(generation, &observation);
        if let Err(error) = &result {
            report_orphan_fault(&observation.project_root, error);
        }
        result
    }

    fn retry_orphan(
        &self,
        generation: &str,
        observation: &OrphanObservation,
    ) -> Result<(), String> {
        self.check_orphan_generation(generation, Some(&observation.endpoint.scope.repo_hash))?;
        let current = read_orphan_endpoint(&observation.endpoint_path)?;
        if !same_orphan_endpoint(&current, &observation.endpoint) {
            return Err("The project owner changed. Refresh and try again.".into());
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime.block_on(async {
            let health = inspect_orphan_health(&current).await?;
            if !health.retryable()
                || health.identity != observation.health.identity
                || health.reconciliation != observation.health.reconciliation
            {
                return Err("The project state changed. Refresh and try again.".into());
            }
            self.check_orphan_generation(generation, Some(&current.scope.repo_hash))?;
            if !same_orphan_endpoint(&read_orphan_endpoint(&observation.endpoint_path)?, &current) {
                return Err("The project owner changed. Refresh and try again.".into());
            }
            let client = crate::project_runtime::client::ProjectRuntimeClient::new(&current)?;
            for exact in health.reconciliation {
                // A draining actor cannot admit new GUI leases or launches.
                // Authenticate and prove the same owner immediately before every exact retry.
                let latest = inspect_orphan_health(&current).await?;
                if !latest.retryable()
                    || latest.identity != observation.health.identity
                    || !latest.reconciliation.contains(&exact)
                    || !same_orphan_endpoint(
                        &read_orphan_endpoint(&observation.endpoint_path)?,
                        &current,
                    )
                {
                    return Err("The project state changed. Refresh and try again.".into());
                }
                self.check_orphan_generation(generation, Some(&current.scope.repo_hash))?;
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    client.command(crate::project_runtime::events::OwnerCommand::Close {
                        id: exact.id,
                        incarnation: exact.incarnation,
                    }),
                )
                .await
                .map_err(|_| "Termination retry timed out".to_string())??;
            }
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while crate::process::host_process_start_time(current.pid)
                    == Some(health.identity.host_started_at)
                {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            })
            .await
            .map_err(|_| "Shutdown is still in progress. Refresh and try again.".to_string())?;
            Ok::<(), String>(())
        })?;
        self.reap_finished_children();
        Ok(())
    }

    /// Stop GUI ensure work. Live daemon residency is governed by authenticated GUI leases.
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.reap_finished_children();
    }

    /// Fixture cleanup for exact captured children; never called by the GUI lifecycle.
    #[cfg(any(test, feature = "test-gh-guard"))]
    #[doc(hidden)]
    pub fn cleanup_test_children(&self) {
        let mut children = self.children.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, mut slot) in children.drain() {
            if let Some(mut child) = slot.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// Reap completed children even when the last project tab has closed.
    pub fn reap_finished_children(&self) {
        // GUI ticks must never wait behind a project's blocking ensure work.
        let mut children = match self.children.try_lock() {
            Ok(children) => children,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return,
        };
        let endpoints = children.keys().cloned().collect::<Vec<_>>();
        for endpoint in endpoints {
            reap_finished_child(&mut children, &endpoint);
        }
    }
}

type UpdateGuiConnection = std::pin::Pin<
    Box<
        dyn futures_util::Sink<
            tokio_tungstenite::tungstenite::Message,
            Error = tokio_tungstenite::tungstenite::Error,
        >,
    >,
>;

struct UpdateDaemonTarget {
    root: PathBuf,
    prefs: PathBuf,
    owner: Option<(
        gwt_core::daemon::DaemonEndpoint,
        u64,
        crate::IssueMonitorAuthorityFenceState,
    )>,
}

/// Capture authenticated owners without spawning a replacement. The explicit
/// Quit is sent on these same connections, even if discovery changes later.
pub struct UpdateDaemonRetirement<'a> {
    home: PathBuf,
    runtime: tokio::runtime::Runtime,
    targets: Vec<UpdateDaemonTarget>,
    connections: Vec<UpdateGuiConnection>,
    supervisor: Option<&'a DaemonSupervisor>,
}

impl<'a> UpdateDaemonRetirement<'a> {
    /// Call from an update worker, or before the native event loop starts.
    /// Bootstrap has no GUI presence to detach and requires idle projects.
    pub fn capture(
        roots: &[PathBuf],
        supervisor: Option<&'a DaemonSupervisor>,
    ) -> Result<Self, String> {
        let home = gwt_core::paths::gwt_home();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let mut targets = Vec::new();
        let mut connections = Vec::<UpdateGuiConnection>::new();
        let mut seen = HashSet::new();
        for root in roots {
            let prefs = crate::issue_monitor_prefs_path_for_repo_path(root);
            if !seen.insert(prefs.clone()) {
                continue;
            }
            let fail = |reason: &str| {
                format!(
                    "Could not retire project daemon for {}: {reason}",
                    root.display()
                )
            };
            let endpoint = crate::daemon_publisher::resolve_project_daemon_endpoint(root)
                .map_err(|error| fail(&error))?;
            let owner = if let Some(endpoint) = endpoint {
                if supervisor.is_none() {
                    return Err(fail("a live owner is still running"));
                }
                let fence = retirement_authority_fence(&home, &endpoint.scope)
                    .map_err(|error| fail(&error))?;
                let (health, connection) = runtime
                    .block_on(async {
                        let health = inspect_orphan_health(&endpoint).await?;
                        if health.startup_intent != gwt_core::daemon::DaemonStartIntent::GuiManaged
                        {
                            return Err("a standalone owner is still running".to_string());
                        }
                        let client =
                            crate::project_runtime::client::ProjectRuntimeClient::new(&endpoint)?;
                        let connection =
                            tokio::time::timeout(Duration::from_secs(2), client.gui_events())
                                .await
                                .map_err(|_| "authenticated GUI lease timed out".to_string())??;
                        Ok::<_, String>((health, connection))
                    })
                    .map_err(|error| fail(&error))?;
                verify_retirement_authority(&home, &endpoint, &fence)
                    .map_err(|error| fail(&error))?;
                connections.push(Box::pin(connection));
                Some((endpoint, health.identity.host_started_at, fence))
            } else {
                let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                    gwt_core::operation_deadline::now() + Duration::from_secs(1),
                );
                if crate::issue_monitor::issue_monitor_daemon_lease_is_held(&prefs).map_err(
                    |error| fail(&format!("authority lease could not be verified: {error}")),
                )? {
                    return Err(fail(
                        "the authority lease is held without an authenticated owner",
                    ));
                }
                if supervisor.is_some_and(|supervisor| supervisor.has_live_child_for(root, &home)) {
                    return Err(fail("a starting owner has not registered yet"));
                }
                None
            };
            targets.push(UpdateDaemonTarget {
                root: root.clone(),
                prefs,
                owner,
            });
        }
        Ok(Self {
            home,
            runtime,
            targets,
            connections,
            supervisor,
        })
    }

    /// Release this GUI's existing subscriptions, then close only the exact
    /// authenticated leases. A standalone or shared owner retains its normal
    /// lifetime. Hold the released authority locks through helper spawn.
    pub fn retire(
        self,
        detach_gui: impl FnOnce(),
        cancelled: impl Fn() -> bool,
    ) -> Result<Vec<crate::IssueMonitorAuthorityLease>, String> {
        let check_cancelled = || {
            if cancelled() {
                Err("GWT quit was requested before the update could apply".to_string())
            } else {
                Ok(())
            }
        };
        check_cancelled()?;
        detach_gui();
        self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                for connection in self.connections {
                    crate::daemon_subscriber::close_project_owner_for_quit(connection).await?;
                }
                Ok::<_, String>(())
            })
            .await
            .map_err(|_| "Project daemon GUI detach timed out".to_string())?
        })?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            check_cancelled()?;
            if let Some(supervisor) = self.supervisor {
                supervisor.reap_finished_children();
            }
            let mut leases = Vec::with_capacity(self.targets.len());
            let mut waiting = None;
            for target in &self.targets {
                match target.lease_after_exit(&self.home) {
                    Ok(Some(lease)) => leases.push(lease),
                    Ok(None) => {
                        waiting = Some(&target.root);
                        break;
                    }
                    Err(error) => {
                        return Err(format!(
                            "Could not retire project daemon for {}: {error}",
                            target.root.display()
                        ))
                    }
                }
            }
            let Some(root) = waiting else {
                check_cancelled()?;
                return Ok(leases);
            };
            if Instant::now() >= deadline {
                return Err(format!(
                    "Project daemon for {} did not exit and release its authority lease. Close other GWT instances using this project and retry the staged update.",
                    root.display()
                ));
            }
            drop(leases);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl UpdateDaemonTarget {
    fn lease_after_exit(
        &self,
        home: &Path,
    ) -> Result<Option<crate::IssueMonitorAuthorityLease>, String> {
        if let Some((endpoint, started_at, fence)) = &self.owner {
            verify_retirement_authority(home, endpoint, fence)?;
            match crate::process::host_process_liveness(endpoint.pid) {
                Some(false) => {}
                Some(true) => match crate::process::host_process_start_time(endpoint.pid) {
                    Some(current) if current != *started_at => {}
                    Some(_) => return Ok(None),
                    None => return Err("daemon process birth could not be verified".into()),
                },
                None => return Err("daemon process exit could not be verified".into()),
            }
        }
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            gwt_core::operation_deadline::now() + Duration::from_secs(1),
        );
        let lease = match crate::issue_monitor::acquire_issue_monitor_daemon_lease(&self.prefs) {
            Ok(lease) => lease,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(format!("authority lease could not be verified: {error}")),
        };
        if let Some((endpoint, _, fence)) = &self.owner {
            verify_retirement_authority(home, endpoint, fence)?;
        } else if crate::daemon_publisher::resolve_project_daemon_endpoint(&self.root)?.is_some() {
            return Err("a new daemon appeared before the update could commit".into());
        }
        Ok(Some(lease))
    }
}

fn read_orphan_endpoint(path: &Path) -> Result<gwt_core::daemon::DaemonEndpoint, String> {
    serde_json::from_slice(
        &std::fs::read(path)
            .map_err(|_| "The project endpoint changed. Refresh and try again.".to_string())?,
    )
    .map_err(|_| "The project endpoint could not be verified".into())
}

fn same_orphan_endpoint(
    left: &gwt_core::daemon::DaemonEndpoint,
    right: &gwt_core::daemon::DaemonEndpoint,
) -> bool {
    left.scope == right.scope
        && left.pid == right.pid
        && left.bind == right.bind
        && left.auth_token == right.auth_token
        && left.project_transport_url == right.project_transport_url
        && left.startup_intent == right.startup_intent
}

async fn inspect_orphan_health(
    endpoint: &gwt_core::daemon::DaemonEndpoint,
) -> Result<OrphanHealth, String> {
    let client = crate::project_runtime::client::ProjectRuntimeClient::new(endpoint)?;
    let value = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.command(crate::project_runtime::events::OwnerCommand::Health),
    )
    .await
    .map_err(|_| "project daemon health timed out".to_string())??;
    let health: OrphanHealth = serde_json::from_value(value)
        .map_err(|_| "project daemon identity unavailable".to_string())?;
    health.validate(endpoint)?;
    Ok(health)
}

fn report_orphan_fault(project_root: &Path, detail: &str) {
    crate::error_report::report_error_and_publish(
        gwt_core::error_ledger::ErrorKind::DaemonFault,
        detail.to_string(),
        gwt_core::error_ledger::ErrorTarget {
            project_root: Some(project_root.display().to_string()),
            ..Default::default()
        },
    );
}

fn retirement_authority_fence(
    gwt_home: &Path,
    scope: &gwt_core::daemon::RuntimeScope,
) -> Result<crate::IssueMonitorAuthorityFenceState, String> {
    let prefs_path = gwt_home
        .join("projects")
        .join(&scope.repo_hash)
        .join("project-state/issue-monitor.json");
    crate::load_issue_monitor_authority_fence(&prefs_path)
        .map_err(|error| format!("daemon retirement authority could not be verified: {error}"))
}

fn verify_pending_retirement_authority(
    gwt_home: &Path,
    pending: &PendingReplacement,
) -> Result<(), String> {
    verify_retirement_authority(gwt_home, &pending.endpoint, &pending.authority_fence)
}

fn verify_retirement_authority(
    gwt_home: &Path,
    endpoint: &gwt_core::daemon::DaemonEndpoint,
    fence: &crate::IssueMonitorAuthorityFenceState,
) -> Result<(), String> {
    let current = retirement_authority_fence(gwt_home, &endpoint.scope)?;
    if matches!(current, crate::IssueMonitorAuthorityFenceState::Active(_)) && &current != fence {
        return Err("daemon authority changed during retirement".into());
    }
    let descriptor = match std::fs::read(endpoint.scope.endpoint_path(gwt_home)) {
        Ok(payload) => serde_json::from_slice::<gwt_core::daemon::DaemonEndpoint>(&payload)
            .map_err(|error| {
                format!("daemon retirement descriptor could not be verified: {error}")
            })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "daemon retirement descriptor could not be read: {error}"
            ))
        }
    };
    if !same_orphan_endpoint(&descriptor, endpoint)
        || descriptor.protocol_version != endpoint.protocol_version
        || descriptor.daemon_version != endpoint.daemon_version
    {
        return Err("daemon endpoint changed during retirement".into());
    }
    Ok(())
}

/// `Ok(None)` means nothing serves the candidate's bind (Issue #7463): the
/// descriptor is stale. Any other failure, including a live daemon refusing
/// the handshake, stays an error.
fn authenticate_retirement_candidate(
    endpoint: &gwt_core::daemon::DaemonEndpoint,
) -> Result<
    Option<(
        tokio::runtime::Runtime,
        crate::cli::daemon::client::DaemonClient,
    )>,
    String,
> {
    use crate::cli::daemon::client::{DaemonClient, DaemonConnectError};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("retirement verifier runtime failed: {error}"))?;
    let connected = runtime.block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            DaemonClient::connect_classified(endpoint),
        )
        .await
        .map_err(|_| {
            DaemonConnectError::Failed("daemon retirement authentication timed out".to_string())
        })?
    });
    match connected {
        // Keep the authenticated connection alive through the retirement callback.
        Ok(client) => Ok(Some((runtime, client))),
        Err(DaemonConnectError::Unreachable(_)) => Ok(None),
        Err(DaemonConnectError::Failed(error)) => Err(format!(
            "daemon retirement candidate was not authenticated: {error}"
        )),
    }
}

/// Remove an unreachable candidate's descriptor, but only while it still
/// names that candidate: a replacement may already have rewritten it.
fn remove_unreachable_endpoint(
    gwt_home: &Path,
    endpoint: &gwt_core::daemon::DaemonEndpoint,
) -> Result<(), String> {
    let path = endpoint.scope.endpoint_path(gwt_home);
    let current = match std::fs::read(&path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "stale daemon descriptor {} is unreadable: {error}",
                path.display()
            ))
        }
    };
    if serde_json::from_slice::<gwt_core::daemon::DaemonEndpoint>(&current)
        .ok()
        .as_ref()
        != Some(endpoint)
    {
        return Ok(());
    }
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "stale daemon descriptor {} could not be removed: {error}",
            path.display()
        )),
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

/// Endpoint slots retain the owner's real worktree, but their parent directory
/// is already keyed by ProjectKey. Reuse a starting child across that project
/// before it has published an authority fence or endpoint.
fn child_endpoint_for_project(
    children: &HashMap<PathBuf, SupervisedChild>,
    endpoint: &Path,
) -> PathBuf {
    children
        .keys()
        .find(|candidate| candidate.parent() == endpoint.parent())
        .cloned()
        .unwrap_or_else(|| endpoint.to_path_buf())
}

/// Drop a finished child from the table and return the pid of the one that is
/// still running, if any.
fn reap_finished_child(
    children: &mut HashMap<PathBuf, SupervisedChild>,
    endpoint_path: &Path,
) -> Option<u32> {
    let child = children.get_mut(endpoint_path)?.child.as_mut()?;
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
            if children
                .get(endpoint_path)
                .is_some_and(|slot| slot.replacement.is_some())
            {
                children.get_mut(endpoint_path).expect("known slot").child = None;
            } else {
                children.remove(endpoint_path);
            }
            None
        }
        Err(error) => {
            tracing::warn!(
                pid,
                %error,
                endpoint = %endpoint_path.display(),
                "could not read runtime daemon exit status; forgetting the child"
            );
            if children
                .get(endpoint_path)
                .is_some_and(|slot| slot.replacement.is_some())
            {
                children.get_mut(endpoint_path).expect("known slot").child = None;
            } else {
                children.remove(endpoint_path);
            }
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

/// Issue #4038 (AC-6): signal a daemon named by a live endpoint descriptor so
/// its serve loop unlinks the socket and descriptor. Liveness and identity
/// were just verified by the caller through the same descriptor, which is the
/// strongest identity the endpoint contract offers for a process this GUI did
/// not spawn. Issue #7541: the signal is one stage of the bounded retirement
/// state machine — `Term` first, `Kill` only after the cooperative grace
/// elapsed with the captured identity still alive.
#[cfg(unix)]
fn terminate_pid_with_signal(pid: u32, signal: DaemonRetireSignal) {
    if pid == 0 || pid > i32::MAX as u32 {
        return;
    }
    let signo = match signal {
        DaemonRetireSignal::Term => libc::SIGTERM,
        DaemonRetireSignal::Kill => libc::SIGKILL,
    };
    // SAFETY: plain signal send; `kill` has no memory-safety preconditions.
    unsafe {
        libc::kill(pid as libc::pid_t, signo);
    }
}

/// Windows has no cross-process cooperative signal for a windowless child
/// (console control events need a shared console, which a `CREATE_NO_WINDOW`
/// child does not have), so the supervisor terminates the daemon outright at
/// either retirement stage. The endpoint descriptor and authority fence it
/// leaves behind are exactly what a crashed daemon leaves, and both are
/// reclaimed by the liveness probes in `resolve_bootstrap_action` and the
/// fence recovery (Issue #3526). Issue #4038 (AC-6): Windows counterpart for
/// a stale-version daemon this GUI did not spawn (no `Child` handle).
#[cfg(windows)]
fn terminate_pid_with_signal(pid: u32, _signal: DaemonRetireSignal) {
    if pid == 0 {
        return;
    }
    match gwt_core::process::hidden_command("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output()
    {
        Ok(output) if output.status.success() => {}
        Ok(output) => tracing::warn!(
            pid,
            stderr = %String::from_utf8_lossy(&output.stderr).trim(),
            "failed to terminate a stale-version runtime daemon"
        ),
        Err(error) => tracing::warn!(
            pid,
            %error,
            "failed to invoke taskkill for a stale-version runtime daemon"
        ),
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
    let request = daemon_spawn_request(&gwtd_path, context.project_root, context.startup_intent);
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
    fn update_retirement_requires_both_process_exit_and_authority_release() {
        use gwt_core::daemon::{DaemonEndpoint, RuntimeScope, RuntimeTarget};
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host).unwrap();
        let pid = std::process::id();
        let birth = crate::process::host_process_start_time(pid).unwrap();
        let endpoint =
            DaemonEndpoint::new(scope, pid, "unused".into(), "fixture".into(), "1".into());
        let prefs = home
            .path()
            .join("projects/fixture/project-state/issue-monitor.json");
        let mut target = UpdateDaemonTarget {
            root: project.path().into(),
            prefs: prefs.clone(),
            owner: Some((
                endpoint.clone(),
                birth,
                crate::IssueMonitorAuthorityFenceState::Missing,
            )),
        };

        assert!(target.lease_after_exit(home.path()).unwrap().is_none());
        let holder = crate::issue_monitor::acquire_issue_monitor_daemon_lease(&prefs).unwrap();
        // Model the original process exiting and the OS reusing its PID.
        target.owner = Some((
            endpoint,
            birth.wrapping_add(1),
            crate::IssueMonitorAuthorityFenceState::Missing,
        ));
        assert!(target.lease_after_exit(home.path()).unwrap().is_none());
        drop(holder);
        let lease = target.lease_after_exit(home.path()).unwrap().unwrap();
        assert!(crate::issue_monitor::issue_monitor_daemon_lease_is_held(&prefs).unwrap());
        drop(lease);
        assert!(!crate::issue_monitor::issue_monitor_daemon_lease_is_held(&prefs).unwrap());
    }

    #[test]
    fn quit_during_discovery_prevents_a_pending_spawn() {
        use gwt_core::daemon::{persist_endpoint, DaemonEndpoint, RuntimeScope, RuntimeTarget};
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host).unwrap();
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            12345,
            "unused".into(),
            "quit-discovery-fixture".into(),
            env!("CARGO_PKG_VERSION").into(),
        );
        persist_endpoint(&scope.endpoint_path(home.path()), &endpoint).unwrap();
        let supervisor = DaemonSupervisor::with_spawner(|_| panic!("Quit must prevent spawn"));
        let observed = |_: u32| {
            supervisor.shutdown();
            false
        };
        let result = supervisor.ensure_running_with(
            project.path(),
            DaemonEnsureInputs {
                gwt_home: home.path().into(),
                is_process_alive: &observed,
                expected_daemon_version: env!("CARGO_PKG_VERSION"),
                retire_stale_daemon: &|_, _| panic!("Quit must not retire an owner"),
            },
        );
        assert_eq!(
            result.unwrap_err(),
            "daemon supervisor shut down during discovery"
        );
    }

    #[test]
    fn orphan_observation_rejects_reuse_reopen_replacement_and_unproved_birth() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated home");
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        use gwt_core::daemon::{DaemonEndpoint, DaemonStartIntent, RuntimeScope, RuntimeTarget};
        let root = tempfile::tempdir().expect("root");
        let endpoint = DaemonEndpoint::new(
            RuntimeScope::new(
                "project",
                "worktree",
                root.path().to_path_buf(),
                RuntimeTarget::Host,
            )
            .expect("scope"),
            std::process::id(),
            "private-test".into(),
            "never-browser".into(),
            "test".into(),
        );
        let mut health = OrphanHealth {
            identity: crate::daemon_subscriber::ProjectOwnerIdentity {
                pid: endpoint.pid,
                host_started_at: crate::process::host_process_start_time(endpoint.pid)
                    .expect("physical birth proof"),
                host_instance_id: "fixed-instance".into(),
            },
            project_key: endpoint.scope.repo_hash.clone(),
            startup_intent: DaemonStartIntent::GuiManaged,
            gui_presence_count: 0,
            draining: true,
            reconciliation_required: true,
            reconciliation: vec![ExactRetirement {
                id: "pane".into(),
                incarnation: 7,
            }],
        };
        health.validate(&endpoint).expect("physical identity");
        assert!(health.retryable());
        health.startup_intent = DaemonStartIntent::Standalone;
        assert!(
            !health.retryable(),
            "explicit resident owners cannot be reaped"
        );
        health.startup_intent = DaemonStartIntent::GuiManaged;
        health.identity.host_started_at += 1;
        assert!(
            health.validate(&endpoint).is_err(),
            "unproved birth must fail closed"
        );
        health.identity.host_started_at -= 1;
        let mut replacement = endpoint.clone();
        replacement.auth_token = "replacement-secret".into();
        assert!(!same_orphan_endpoint(&endpoint, &replacement));
        let supervisor = DaemonSupervisor::disabled();
        supervisor.update_orphan_context("generation-1".into(), HashSet::new());
        let observation = OrphanObservation {
            project_root: root.path().to_path_buf(),
            endpoint_path: root.path().join("endpoint.json"),
            endpoint,
            health,
        };
        supervisor
            .orphan_observations
            .lock()
            .expect("observations")
            .observations
            .insert("once".into(), observation.clone());
        assert!(supervisor.reap_orphan("stale-generation", "once").is_err());
        assert!(
            !supervisor
                .orphan_observations
                .lock()
                .expect("observations")
                .observations
                .contains_key("once"),
            "a refused token is still consumed"
        );
        assert!(supervisor.reap_orphan("generation-1", "once").is_err());
        supervisor
            .orphan_observations
            .lock()
            .expect("observations")
            .observations
            .insert("reopened".into(), observation);
        supervisor.update_orphan_context("generation-2".into(), HashSet::from(["project".into()]));
        assert!(
            supervisor.reap_orphan("generation-2", "reopened").is_err(),
            "reopening invalidates old observations"
        );
    }

    #[test]
    fn spawn_request_targets_the_project_it_serves() {
        let request = daemon_spawn_request(
            Path::new("/usr/local/bin/gwtd"),
            Path::new("/repo"),
            gwt_core::daemon::DaemonStartIntent::GuiManaged,
        );
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

    /// Issue #4038 (AC-6): a live daemon whose `daemon_version` differs from
    /// this build is retired instead of adopted, and nothing is spawned until
    /// it has gone (the next ensure tick finds no endpoint and spawns).
    #[cfg(unix)]
    #[test]
    fn ensure_running_retires_a_live_daemon_of_another_version() {
        use gwt_core::daemon::{persist_endpoint, DaemonEndpoint, RuntimeScope, RuntimeTarget};

        let gwt_home = gwt_core::test_support::socket_tempdir().expect("gwt home");
        let project = tempfile::TempDir::new().expect("project");
        let scope =
            RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host).expect("scope");
        let socket_path = gwt_home.path().join("retire.sock");
        let listener = std::os::unix::net::UnixListener::bind(
            gwt_core::test_support::checked_socket_path(&socket_path),
        )
        .unwrap();
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            socket_path.display().to_string(),
            "secret-token".into(),
            "0.0.1-stale".into(),
        );
        persist_endpoint(&scope.endpoint_path(gwt_home.path()), &endpoint).expect("persist");

        let server_scope = scope.clone();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: gwt_core::daemon::IpcHandshakeRequest =
                serde_json::from_str(&line).unwrap();
            assert_eq!(request.scope, server_scope);
            assert_eq!(request.auth_token, "secret-token");
            writeln!(
                stream,
                "{}",
                serde_json::to_string(&gwt_core::daemon::IpcHandshakeResponse {
                    protocol_version: request.protocol_version,
                    daemon_version: "0.0.1-stale".into(),
                    accepted: true,
                    rejection_reason: None,
                })
                .unwrap()
            )
            .unwrap();
            let mut tail = Vec::new();
            std::io::Read::read_to_end(&mut stream, &mut tail).unwrap();
        });

        let supervisor = DaemonSupervisor::with_spawner(move |_context| {
            Err(std::io::Error::other(
                "spawn must not run while a stale daemon is alive",
            ))
        });
        let retired = std::sync::Mutex::new(Vec::new());
        let outcome = supervisor
            .ensure_running_with(
                project.path(),
                DaemonEnsureInputs {
                    gwt_home: gwt_home.path().to_path_buf(),
                    is_process_alive: &|pid| pid == std::process::id(),
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &|pid, signal| retired.lock().unwrap().push((pid, signal)),
                },
            )
            .expect("ensure succeeds");

        server.join().unwrap();
        assert_eq!(
            outcome,
            DaemonEnsureOutcome::RetiringStale {
                pid: std::process::id()
            }
        );
        assert_eq!(
            retired.lock().unwrap().as_slice(),
            &[(std::process::id(), DaemonRetireSignal::Term)],
            "Issue #7541: retirement always opens with the cooperative signal"
        );
        assert!(
            scope.endpoint_path(gwt_home.path()).exists(),
            "the live daemon unlinks its own descriptor on SIGTERM; the supervisor does not"
        );
    }

    /// Issue #7541 fixture: a live stale-version daemon. The current test
    /// process poses as the daemon (the supervisor never signals it for
    /// real; every effect goes through the injected inputs), a loopback
    /// listener answers the retirement authentication handshake
    /// `connections` times, and the persisted descriptor names version
    /// "0.0.1-stale".
    #[cfg(unix)]
    struct StaleDaemonFixture {
        home: tempfile::TempDir,
        project: tempfile::TempDir,
        endpoint_path: PathBuf,
        server: std::thread::JoinHandle<()>,
    }

    #[cfg(unix)]
    fn stale_daemon_fixture(connections: usize) -> StaleDaemonFixture {
        use gwt_core::daemon::{persist_endpoint, DaemonEndpoint, RuntimeScope, RuntimeTarget};

        let home = gwt_core::test_support::socket_tempdir().expect("gwt home");
        let project = tempfile::TempDir::new().expect("project");
        let scope =
            RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host).expect("scope");
        let socket_path = home.path().join("retire.sock");
        let listener = std::os::unix::net::UnixListener::bind(
            gwt_core::test_support::checked_socket_path(&socket_path),
        )
        .unwrap();
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            socket_path.display().to_string(),
            "secret-token".into(),
            "0.0.1-stale".into(),
        );
        let endpoint_path = scope.endpoint_path(home.path());
        persist_endpoint(&endpoint_path, &endpoint).expect("persist");
        let server_scope = scope.clone();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            for _ in 0..connections {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut line = String::new();
                std::io::BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: gwt_core::daemon::IpcHandshakeRequest =
                    serde_json::from_str(&line).unwrap();
                assert_eq!(request.scope, server_scope);
                assert_eq!(request.auth_token, "secret-token");
                writeln!(
                    stream,
                    "{}",
                    serde_json::to_string(&gwt_core::daemon::IpcHandshakeResponse {
                        protocol_version: request.protocol_version,
                        daemon_version: "0.0.1-stale".into(),
                        accepted: true,
                        rejection_reason: None,
                    })
                    .unwrap()
                )
                .unwrap();
                let mut tail = Vec::new();
                std::io::Read::read_to_end(&mut stream, &mut tail).unwrap();
            }
        });
        StaleDaemonFixture {
            home,
            project,
            endpoint_path,
            server,
        }
    }

    /// Move the pending retirement's deadline into the past so the next
    /// ensure tick observes the elapsed grace instead of waiting for it.
    #[cfg(unix)]
    fn backdate_retirement_deadline(supervisor: &DaemonSupervisor) {
        let mut children = supervisor
            .children
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let slot = children
            .values_mut()
            .find(|slot| slot.replacement.is_some())
            .expect("a retirement is pending");
        slot.replacement.as_mut().expect("replacement").deadline =
            // test-hygiene: allow-wall-clock-deadline the deadline is supervisor state, not a wait: backdating it makes the next ensure tick observe the elapsed grace deterministically
            Instant::now() - Duration::from_secs(1);
    }

    #[cfg(unix)]
    fn spawn_sleep_child(_context: &DaemonSpawnContext<'_>) -> std::io::Result<Child> {
        gwt_core::process::resolved_command(gwt_core::process::ProcessPlanRequest::new("sleep"))
            .map_err(std::io::Error::other)?
            .arg("30")
            .spawn()
    }

    /// Issue #7541 (D1): a stale daemon that honours the cooperative signal
    /// is replaced on the next ensure tick; no forced signal is sent.
    #[cfg(unix)]
    #[test]
    fn retire_term_success_then_replacement_spawns() {
        let fixture = stale_daemon_fixture(1);
        let live = Arc::new(AtomicBool::new(true));
        let signals = Arc::new(Mutex::new(Vec::new()));
        let supervisor = DaemonSupervisor::with_spawner(spawn_sleep_child);
        let endpoint_path = fixture.endpoint_path.clone();
        let retire_live = Arc::clone(&live);
        let retire_signals = Arc::clone(&signals);
        let retire = move |pid: u32, signal: DaemonRetireSignal| {
            retire_signals.lock().unwrap().push((pid, signal));
            if signal == DaemonRetireSignal::Term {
                // A cooperative daemon unlinks its own descriptor and exits.
                let _ = std::fs::remove_file(&endpoint_path);
                retire_live.store(false, Ordering::SeqCst);
            }
        };
        let observe_live = Arc::clone(&live);
        let observe =
            move |pid: u32| pid == std::process::id() && observe_live.load(Ordering::SeqCst);
        let first = supervisor
            .ensure_running_with(
                fixture.project.path(),
                DaemonEnsureInputs {
                    gwt_home: fixture.home.path().to_path_buf(),
                    is_process_alive: &observe,
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &retire,
                },
            )
            .expect("first ensure succeeds");
        assert_eq!(
            first,
            DaemonEnsureOutcome::RetiringStale {
                pid: std::process::id()
            }
        );
        let second = supervisor
            .ensure_running_with(
                fixture.project.path(),
                DaemonEnsureInputs {
                    gwt_home: fixture.home.path().to_path_buf(),
                    is_process_alive: &observe,
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &retire,
                },
            )
            .expect("second ensure succeeds");
        fixture.server.join().unwrap();
        assert!(
            matches!(second, DaemonEnsureOutcome::Spawned { .. }),
            "the replacement spawns once the cooperative exit is observed, got: {second:?}"
        );
        assert_eq!(
            signals.lock().unwrap().as_slice(),
            &[(std::process::id(), DaemonRetireSignal::Term)],
            "a cooperative exit never escalates to the forced signal"
        );
        supervisor.cleanup_test_children();
    }

    /// Issue #7541 (D2 + D5 + D7): a stale daemon that ignores the
    /// cooperative signal is escalated to the forced signal once the grace
    /// elapses — repeated ensures advance the stage instead of re-sending
    /// the cooperative signal forever — and once its death is confirmed the
    /// replacement spawns and later ensures report the new child, not the
    /// retirement.
    #[cfg(unix)]
    #[test]
    fn retire_escalates_to_kill_after_term_grace_then_spawns() {
        let fixture = stale_daemon_fixture(1);
        let live = Arc::new(AtomicBool::new(true));
        let signals = Arc::new(Mutex::new(Vec::new()));
        let supervisor = DaemonSupervisor::with_spawner(spawn_sleep_child);
        let retire_signals = Arc::clone(&signals);
        let retire = move |pid: u32, signal: DaemonRetireSignal| {
            // The daemon ignores every signal; only the test's explicit
            // death simulation below ends it.
            retire_signals.lock().unwrap().push((pid, signal));
        };
        let observe_live = Arc::clone(&live);
        let observe =
            move |pid: u32| pid == std::process::id() && observe_live.load(Ordering::SeqCst);
        let ensure = || {
            supervisor.ensure_running_with(
                fixture.project.path(),
                DaemonEnsureInputs {
                    gwt_home: fixture.home.path().to_path_buf(),
                    is_process_alive: &observe,
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &retire,
                },
            )
        };
        assert!(matches!(
            ensure().expect("first ensure succeeds"),
            DaemonEnsureOutcome::RetiringStale { .. }
        ));
        // A newly published authority cannot inherit a capture made with no fence.
        let scope = gwt_core::daemon::RuntimeScope::from_project_root(
            fixture.project.path(),
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .unwrap();
        let prefs_path = fixture
            .home
            .path()
            .join("projects")
            .join(&scope.repo_hash)
            .join("project-state/issue-monitor.json");
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default()).unwrap();
        crate::persist_issue_monitor_authority_fence(
            &prefs_path,
            &crate::IssueMonitorAuthorityFence::current_process(),
        )
        .unwrap();
        assert!(ensure().unwrap_err().contains("authority changed"));
        assert_eq!(
            *signals.lock().unwrap(),
            vec![(std::process::id(), DaemonRetireSignal::Term)]
        );
        std::fs::remove_file(crate::issue_monitor_authority_fence_path(&prefs_path)).unwrap();
        assert!(matches!(
            ensure().expect("second ensure succeeds"),
            DaemonEnsureOutcome::RetiringStale { .. }
        ));
        assert_eq!(
            signals.lock().unwrap().as_slice(),
            &[(std::process::id(), DaemonRetireSignal::Term)],
            "within the grace the stage is kept, not restarted (D5)"
        );
        backdate_retirement_deadline(&supervisor);
        assert!(matches!(
            ensure().expect("third ensure succeeds"),
            DaemonEnsureOutcome::RetiringStale { .. }
        ));
        assert_eq!(
            signals.lock().unwrap().as_slice(),
            &[
                (std::process::id(), DaemonRetireSignal::Term),
                (std::process::id(), DaemonRetireSignal::Kill)
            ],
            "an ignored cooperative signal escalates to the forced signal (D2)"
        );
        // The daemon finally dies and unlinks its descriptor.
        live.store(false, Ordering::SeqCst);
        std::fs::remove_file(&fixture.endpoint_path).unwrap();
        assert!(
            matches!(
                ensure().expect("fourth ensure succeeds"),
                DaemonEnsureOutcome::Spawned { .. }
            ),
            "the replacement spawns only after the death is confirmed (D2)"
        );
        assert!(
            matches!(
                ensure().expect("fifth ensure succeeds"),
                DaemonEnsureOutcome::Starting { .. }
            ),
            "after the generation change, ensure reports the new child (D7)"
        );
        fixture.server.join().unwrap();
        supervisor.cleanup_test_children();
    }

    #[cfg(unix)]
    #[test]
    fn pending_retirement_survives_lost_ipc_and_descriptor_with_held_authority() {
        let fixture = stale_daemon_fixture(1);
        let mut replacement: gwt_core::daemon::DaemonEndpoint =
            serde_json::from_slice(&std::fs::read(&fixture.endpoint_path).unwrap()).unwrap();
        let prefs_path = fixture
            .home
            .path()
            .join("projects")
            .join(&replacement.scope.repo_hash)
            .join("project-state/issue-monitor.json");
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default()).unwrap();
        let fence = crate::IssueMonitorAuthorityFence::current_process();
        let (_, lease) =
            crate::establish_issue_monitor_authority_fence(&prefs_path, &fence, |_| true).unwrap();
        let supervisor =
            DaemonSupervisor::with_spawner(|_| panic!("a live retiring owner forbids replacement"));
        let signals = Mutex::new(Vec::new());
        let retire = |pid, signal| {
            signals.lock().unwrap().push((pid, signal));
            if signal == DaemonRetireSignal::Term {
                std::fs::remove_file(&fixture.endpoint_path).unwrap();
            }
        };
        let ensure = || {
            supervisor.ensure_running_with(
                fixture.project.path(),
                DaemonEnsureInputs {
                    gwt_home: fixture.home.path().to_path_buf(),
                    is_process_alive: &|_| true,
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &retire,
                },
            )
        };
        assert!(matches!(
            ensure().unwrap(),
            DaemonEnsureOutcome::RetiringStale { .. }
        ));
        fixture.server.join().unwrap();
        backdate_retirement_deadline(&supervisor);
        // A new generation must not inherit the old retirement authority.
        crate::persist_issue_monitor_authority_fence(
            &prefs_path,
            &crate::IssueMonitorAuthorityFence::current_process(),
        )
        .unwrap();
        assert!(ensure().unwrap_err().contains("authority changed"));
        crate::persist_issue_monitor_authority_fence(&prefs_path, &fence).unwrap();
        replacement.auth_token = "a-new-owner".into();
        gwt_core::daemon::persist_endpoint(&fixture.endpoint_path, &replacement).unwrap();
        assert!(ensure().unwrap_err().contains("endpoint changed"));
        assert_eq!(
            *signals.lock().unwrap(),
            vec![(std::process::id(), DaemonRetireSignal::Term)]
        );
        std::fs::remove_file(&fixture.endpoint_path).unwrap();
        assert!(matches!(
            ensure().expect("captured retirement must outlive IPC and descriptor loss"),
            DaemonEnsureOutcome::RetiringStale { .. }
        ));
        assert_eq!(
            *signals.lock().unwrap(),
            vec![
                (std::process::id(), DaemonRetireSignal::Term),
                (std::process::id(), DaemonRetireSignal::Kill),
            ]
        );
        // A normal shutdown may revoke its fence before the process exits.
        std::fs::remove_file(crate::issue_monitor_authority_fence_path(&prefs_path)).unwrap();
        assert!(matches!(
            ensure().unwrap(),
            DaemonEnsureOutcome::RetiringStale { .. }
        ));
        drop(lease);
    }

    /// Issue #7541 (D3): a daemon that survives even the forced signal is
    /// reported as an explicit error — the supervisor never spawns into its
    /// still-held authority lease and never pretends to be Ready — and the
    /// pending retirement stays retryable, so a later ensure recovers once
    /// the process is actually gone.
    #[cfg(unix)]
    #[test]
    fn retire_reports_error_when_daemon_survives_forced_termination() {
        let fixture = stale_daemon_fixture(1);
        let live = Arc::new(AtomicBool::new(true));
        let signals = Arc::new(Mutex::new(Vec::new()));
        let spawns = Arc::new(AtomicUsize::new(0));
        let counted_spawns = Arc::clone(&spawns);
        let supervisor = DaemonSupervisor::with_spawner(move |context| {
            counted_spawns.fetch_add(1, Ordering::SeqCst);
            spawn_sleep_child(context)
        });
        let retire_signals = Arc::clone(&signals);
        let retire = move |pid: u32, signal: DaemonRetireSignal| {
            retire_signals.lock().unwrap().push((pid, signal));
        };
        let observe_live = Arc::clone(&live);
        let observe =
            move |pid: u32| pid == std::process::id() && observe_live.load(Ordering::SeqCst);
        let ensure = || {
            supervisor.ensure_running_with(
                fixture.project.path(),
                DaemonEnsureInputs {
                    gwt_home: fixture.home.path().to_path_buf(),
                    is_process_alive: &observe,
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &retire,
                },
            )
        };
        assert!(matches!(
            ensure().expect("first ensure succeeds"),
            DaemonEnsureOutcome::RetiringStale { .. }
        ));
        backdate_retirement_deadline(&supervisor);
        assert!(matches!(
            ensure().expect("second ensure succeeds"),
            DaemonEnsureOutcome::RetiringStale { .. }
        ));
        backdate_retirement_deadline(&supervisor);
        let error = ensure().expect_err("an unkillable daemon is an explicit error");
        assert!(
            error.contains("did not exit after forced termination"),
            "got: {error}"
        );
        assert_eq!(
            spawns.load(Ordering::SeqCst),
            0,
            "no replacement is spawned while the old daemon survives"
        );
        // Once the process is actually gone, the same pending retirement
        // lets a later ensure recover.
        live.store(false, Ordering::SeqCst);
        std::fs::remove_file(&fixture.endpoint_path).unwrap();
        assert!(matches!(
            ensure().expect("fourth ensure succeeds"),
            DaemonEnsureOutcome::Spawned { .. }
        ));
        fixture.server.join().unwrap();
        supervisor.cleanup_test_children();
    }

    /// Issue #7541 (D4): a pending retirement whose captured start time no
    /// longer matches the live process means the PID was reused by an
    /// unrelated process. That process is never signalled, and the
    /// replacement spawns.
    #[cfg(unix)]
    #[test]
    fn reused_pid_is_never_signalled_and_replacement_spawns() {
        use gwt_core::daemon::{DaemonStartIntent, RuntimeScope, RuntimeTarget};

        let home = gwt_core::test_support::socket_tempdir().expect("gwt home");
        let project = tempfile::tempdir().expect("project");
        let scope =
            RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host).expect("scope");
        let endpoint_path = scope.endpoint_path(home.path());
        let supervisor = DaemonSupervisor::with_spawner(spawn_sleep_child);
        let real_started = crate::process::host_process_start_time(std::process::id())
            .expect("test process start time");
        {
            let mut children = supervisor
                .children
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            children.insert(
                endpoint_path,
                SupervisedChild {
                    project_root: project.path().to_path_buf(),
                    child: None,
                    replacement: Some(PendingReplacement {
                        endpoint: gwt_core::daemon::DaemonEndpoint {
                            startup_intent: DaemonStartIntent::GuiManaged,
                            ..gwt_core::daemon::DaemonEndpoint::new(
                                scope,
                                std::process::id(),
                                String::new(),
                                String::new(),
                                env!("CARGO_PKG_VERSION").into(),
                            )
                        },
                        authority_fence: crate::IssueMonitorAuthorityFenceState::Missing,
                        started_at: real_started + 1,
                        stage: RetireStage::KillSent,
                        // test-hygiene: allow-wall-clock-deadline the deadline is supervisor state, not a wait: an already-elapsed deadline makes the next ensure tick observe the forced stage deterministically
                        deadline: Instant::now() - Duration::from_secs(1),
                    }),
                },
            );
        }
        let signals = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&signals);
        let outcome = supervisor
            .ensure_running_with(
                project.path(),
                DaemonEnsureInputs {
                    gwt_home: home.path().to_path_buf(),
                    is_process_alive: &|_| true,
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &move |pid, signal| {
                        recorded.lock().unwrap().push((pid, signal))
                    },
                },
            )
            .expect("ensure succeeds");
        assert!(
            matches!(outcome, DaemonEnsureOutcome::Spawned { .. }),
            "a reused PID ends the retirement and the replacement spawns, got: {outcome:?}"
        );
        assert!(
            signals.lock().unwrap().is_empty(),
            "a reused PID is never signalled"
        );
        supervisor.cleanup_test_children();
    }

    /// Issue #7541 (D6): shutting the supervisor down — the normal GUI Quit
    /// path — never signals a daemon, including one whose retirement is
    /// pending and overdue; residency stays governed by the daemon's own
    /// authenticated GUI leases.
    #[cfg(unix)]
    #[test]
    fn shutdown_never_signals_a_pending_retirement() {
        use gwt_core::daemon::{DaemonStartIntent, RuntimeScope, RuntimeTarget};

        let home = gwt_core::test_support::socket_tempdir().expect("gwt home");
        let project = tempfile::tempdir().expect("project");
        let scope =
            RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host).expect("scope");
        let endpoint_path = scope.endpoint_path(home.path());
        let supervisor = DaemonSupervisor::with_spawner(spawn_sleep_child);
        let real_started = crate::process::host_process_start_time(std::process::id())
            .expect("test process start time");
        {
            let mut children = supervisor
                .children
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            children.insert(
                endpoint_path,
                SupervisedChild {
                    project_root: project.path().to_path_buf(),
                    child: None,
                    replacement: Some(PendingReplacement {
                        endpoint: gwt_core::daemon::DaemonEndpoint {
                            startup_intent: DaemonStartIntent::GuiManaged,
                            ..gwt_core::daemon::DaemonEndpoint::new(
                                scope,
                                std::process::id(),
                                String::new(),
                                String::new(),
                                env!("CARGO_PKG_VERSION").into(),
                            )
                        },
                        authority_fence: crate::IssueMonitorAuthorityFenceState::Missing,
                        started_at: real_started,
                        stage: RetireStage::KillSent,
                        // test-hygiene: allow-wall-clock-deadline the deadline is supervisor state, not a wait: an already-elapsed deadline makes the next ensure tick observe the forced stage deterministically
                        deadline: Instant::now() - Duration::from_secs(1),
                    }),
                },
            );
        }
        supervisor.shutdown();
        let signals = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&signals);
        let error = supervisor
            .ensure_running_with(
                project.path(),
                DaemonEnsureInputs {
                    gwt_home: home.path().to_path_buf(),
                    is_process_alive: &|_| true,
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &move |pid, signal| {
                        recorded.lock().unwrap().push((pid, signal))
                    },
                },
            )
            .expect_err("a shut-down supervisor refuses ensure work");
        assert!(error.contains("shut down"), "got: {error}");
        assert!(
            signals.lock().unwrap().is_empty(),
            "Quit never signals the shared daemon"
        );
        supervisor.cleanup_test_children();
    }

    #[cfg(unix)]
    #[test]
    fn same_version_legacy_retirement_preserves_standalone_and_waits_for_exit() {
        use gwt_core::daemon::{
            persist_endpoint, DaemonEndpoint, DaemonStartIntent, RuntimeScope, RuntimeTarget,
        };
        let home = gwt_core::test_support::socket_tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host).unwrap();
        let socket_path = home.path().join("legacy.sock");
        let listener = std::os::unix::net::UnixListener::bind(
            gwt_core::test_support::checked_socket_path(&socket_path),
        )
        .unwrap();
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            socket_path.display().to_string(),
            "secret".into(),
            env!("CARGO_PKG_VERSION").into(),
        );
        persist_endpoint(&scope.endpoint_path(home.path()), &endpoint).unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: gwt_core::daemon::IpcHandshakeRequest =
                serde_json::from_str(&line).unwrap();
            assert_eq!(request.auth_token, "secret");
            writeln!(
                stream,
                "{}",
                serde_json::to_string(&gwt_core::daemon::IpcHandshakeResponse {
                    protocol_version: request.protocol_version,
                    daemon_version: env!("CARGO_PKG_VERSION").into(),
                    accepted: true,
                    rejection_reason: None
                })
                .unwrap()
            )
            .unwrap();
            let mut tail = Vec::new();
            std::io::Read::read_to_end(&mut stream, &mut tail).unwrap();
        });
        let intents = Arc::new(Mutex::new(Vec::new()));
        let spawned_intents = intents.clone();
        let supervisor = DaemonSupervisor::with_spawner(move |context| {
            spawned_intents.lock().unwrap().push(context.startup_intent);
            gwt_core::process::resolved_command(gwt_core::process::ProcessPlanRequest::new("sleep"))
                .map_err(std::io::Error::other)?
                .arg("30")
                .spawn()
        });
        let live = AtomicBool::new(true);
        let observe = |_: u32| live.load(Ordering::SeqCst);
        let retire = |_: u32, _: DaemonRetireSignal| {
            live.store(false, Ordering::SeqCst);
        };
        let first = supervisor
            .ensure_running_with(
                project.path(),
                DaemonEnsureInputs {
                    gwt_home: home.path().into(),
                    is_process_alive: &observe,
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &retire,
                },
            )
            .unwrap();
        server.join().unwrap();
        assert!(matches!(first, DaemonEnsureOutcome::RetiringStale { .. }));
        assert!(
            intents.lock().unwrap().is_empty(),
            "old owner must be authenticated and retired before spawning"
        );
        let second = supervisor
            .ensure_running_with(
                project.path(),
                DaemonEnsureInputs {
                    gwt_home: home.path().into(),
                    is_process_alive: &observe,
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &retire,
                },
            )
            .unwrap();
        assert!(matches!(second, DaemonEnsureOutcome::Spawned { .. }));
        assert_eq!(
            *intents.lock().unwrap(),
            vec![DaemonStartIntent::Standalone]
        );
        supervisor.shutdown();
        assert!(
            supervisor.has_live_child_for(project.path(), home.path()),
            "GUI shutdown cannot signal a live shared daemon"
        );
        supervisor.cleanup_test_children();
    }

    /// Issue #7463: a retirement candidate whose bind nobody serves (Unix:
    /// stale socket file → ECONNREFUSED; Windows: no pipe → NotFound) is a
    /// stale descriptor, not a refusal. The launch proceeds to a new daemon,
    /// the descriptor is removed, and the unproven PID is never signalled.
    #[test]
    fn unreachable_retirement_candidate_is_cleared_and_replaced() {
        use gwt_core::daemon::{persist_endpoint, DaemonEndpoint, RuntimeScope, RuntimeTarget};

        let gwt_home = gwt_core::test_support::socket_tempdir().expect("gwt home");
        let project = tempfile::TempDir::new().expect("project");
        let scope =
            RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host).expect("scope");
        let endpoint_path = scope.endpoint_path(gwt_home.path());
        let bind = gwt_core::daemon::resolve_daemon_socket_path(
            &gwt_home.path().join("refused-daemon.json"),
        )
        .expect("bind")
        .path;
        #[cfg(unix)]
        drop(
            std::os::unix::net::UnixListener::bind(gwt_core::test_support::checked_socket_path(
                &bind,
            ))
            .expect("bind then abandon the socket"),
        );
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            bind.display().to_string(),
            "secret-token".into(),
            "0.0.1-stale".into(),
        );
        persist_endpoint(&endpoint_path, &endpoint).expect("persist");

        let supervisor = DaemonSupervisor::with_spawner(move |_context| {
            Err(std::io::Error::other("replacement-spawn-reached"))
        });
        let retired = std::sync::Mutex::new(Vec::new());
        let error = supervisor
            .ensure_running_with(
                project.path(),
                DaemonEnsureInputs {
                    gwt_home: gwt_home.path().to_path_buf(),
                    is_process_alive: &|pid| pid == std::process::id(),
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &|pid, _| retired.lock().unwrap().push(pid),
                },
            )
            .expect_err("the fake spawner reports that the replacement was started");

        assert!(
            error.contains("replacement-spawn-reached"),
            "launch must proceed to the replacement daemon, got: {error}"
        );
        assert!(
            retired.lock().unwrap().is_empty(),
            "an unauthenticated PID is never signalled"
        );
        assert!(
            !endpoint_path.exists(),
            "the stale descriptor is removed so the next launch does not repeat the failure"
        );
    }

    /// Issue #7463: a live candidate that refuses authentication keeps the
    /// pre-existing fail-closed behaviour.
    #[cfg(unix)]
    #[test]
    fn rejected_retirement_candidate_still_blocks_launch() {
        use gwt_core::daemon::{persist_endpoint, DaemonEndpoint, RuntimeScope, RuntimeTarget};

        let gwt_home = gwt_core::test_support::socket_tempdir().expect("gwt home");
        let project = tempfile::TempDir::new().expect("project");
        let scope =
            RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host).expect("scope");
        let endpoint_path = scope.endpoint_path(gwt_home.path());
        let socket_path = gwt_home.path().join("reject.sock");
        let listener = std::os::unix::net::UnixListener::bind(
            gwt_core::test_support::checked_socket_path(&socket_path),
        )
        .unwrap();
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            socket_path.display().to_string(),
            "secret-token".into(),
            "0.0.1-stale".into(),
        );
        persist_endpoint(&endpoint_path, &endpoint).expect("persist");
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: gwt_core::daemon::IpcHandshakeRequest =
                serde_json::from_str(&line).unwrap();
            writeln!(
                stream,
                "{}",
                serde_json::to_string(&gwt_core::daemon::IpcHandshakeResponse {
                    protocol_version: request.protocol_version,
                    daemon_version: "0.0.1-stale".into(),
                    accepted: false,
                    rejection_reason: Some("auth token mismatch".into()),
                })
                .unwrap()
            )
            .unwrap();
        });

        let supervisor = DaemonSupervisor::with_spawner(move |_context| {
            Err(std::io::Error::other("spawn must not run"))
        });
        let error = supervisor
            .ensure_running_with(
                project.path(),
                DaemonEnsureInputs {
                    gwt_home: gwt_home.path().to_path_buf(),
                    is_process_alive: &|pid| pid == std::process::id(),
                    expected_daemon_version: env!("CARGO_PKG_VERSION"),
                    retire_stale_daemon: &|_, _| panic!("a rejected candidate is never signalled"),
                },
            )
            .expect_err("a live candidate that refuses authentication blocks the launch");
        server.join().unwrap();

        assert!(error.contains("was not authenticated"), "got: {error}");
        assert!(endpoint_path.exists(), "a live daemon's descriptor is kept");
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
