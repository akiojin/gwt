//! Read-only process observations, separate from Issue Monitor admission.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use gwt_agent::{Session, SessionLaunchOrigin, SessionRuntimeState};
use serde::{Deserialize, Serialize};

struct CachedRuntime {
    modified: SystemTime,
    size: u64,
    value: Result<Arc<SessionRuntimeState>, String>,
}

static RUNTIME_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedRuntime>>> = OnceLock::new();

#[cfg(test)]
thread_local! {
    static RUNTIME_PATH_STAMP_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn cached_runtime_entry(
    entry: &std::fs::DirEntry,
    path: &Path,
) -> std::io::Result<Arc<SessionRuntimeState>> {
    cached_runtime_with_stamp(path, || {
        let metadata = entry.metadata()?;
        let follow_target = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let follow_target = {
            use std::os::windows::fs::MetadataExt;
            // FILE_ATTRIBUTE_REPARSE_POINT: preserve fs::metadata's target semantics.
            follow_target || metadata.file_attributes() & 0x400 != 0
        };
        if follow_target {
            return runtime_stamp(path);
        }
        // Windows directory enumeration already supplies this metadata.
        Ok((metadata.modified()?, metadata.len()))
    })
}

fn runtime_stamp(path: &Path) -> std::io::Result<(SystemTime, u64)> {
    #[cfg(test)]
    RUNTIME_PATH_STAMP_PROBES.with(|count| count.set(count.get().wrapping_add(1)));
    let metadata = std::fs::metadata(path)?;
    Ok((metadata.modified()?, metadata.len()))
}

/// Cache decoded sidecars only. Process identity/liveness is never cached.
fn cached_runtime(path: &Path) -> std::io::Result<Arc<SessionRuntimeState>> {
    cached_runtime_with_stamp(path, || runtime_stamp(path))
}

fn cached_runtime_with_stamp(
    path: &Path,
    stamp: impl FnOnce() -> std::io::Result<(SystemTime, u64)>,
) -> std::io::Result<Arc<SessionRuntimeState>> {
    let mut cache = RUNTIME_CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (modified, size) = match stamp() {
        Ok(stamp) => stamp,
        Err(error) => {
            cache.remove(path);
            return Err(error);
        }
    };
    if let Some(hit) = cache
        .get(path)
        .filter(|hit| hit.modified == modified && hit.size == size)
    {
        return hit
            .value
            .clone()
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error));
    }
    let value = match SessionRuntimeState::load(path) {
        Ok(runtime) => Ok(Arc::new(runtime)),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => Err(error.to_string()),
        Err(error) => {
            // Retry transient I/O failures even if the file stamp is unchanged.
            cache.remove(path);
            return Err(error);
        }
    };
    cache.remove(path);
    // A writer may atomically replace the sidecar during the read. Never memoize
    // that result against a stamp that no longer describes the file.
    if runtime_stamp(path).is_ok_and(|after| after == (modified, size)) {
        cache.insert(
            path.to_path_buf(),
            CachedRuntime {
                modified,
                size,
                value: value.clone(),
            },
        );
    }
    value.map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionObservation {
    pub session_id: String,
    pub issue_number: Option<u64>,
    /// A linked Session cannot produce work without a durable binding.
    /// This reports absence only, not validity or permission to run.
    pub execution_binding_missing: bool,
    pub agent_id: String,
    pub worktree_path: PathBuf,
    pub worktree_exists: bool,
    pub host_pid: u32,
    pub child_pid: u32,
    pub child_started_at: u64,
    pub started_at: String,
    pub launch_origin: SessionLaunchOrigin,
    /// Known skill token only; launch arguments and prompt bodies stay private.
    #[serde(default)]
    pub launch_entrypoint: Option<String>,
    pub restore_source_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionObservationUncertainty {
    pub runtime_path: PathBuf,
    pub session_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineSessionRole {
    Pm,
    Implementation,
    Review,
}

/// Project attribution and named-PTY resource scalars; no process environment
/// or registration directory inventory is retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSessionObservation {
    pub session: SessionObservation,
    pub project_root: PathBuf,
    pub repo_hash: Option<String>,
    pub role: MachineSessionRole,
    pub monitor_owned: bool,
    pub resident_memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSessionInventory {
    pub sessions: Vec<MachineSessionObservation>,
    pub uncertainties: Vec<SessionObservationUncertainty>,
}

pub fn observe_machine_sessions(sessions_dir: &Path) -> MachineSessionInventory {
    let mut inventory = SessionInventory::default();
    let candidates = read_candidates_scope(None, sessions_dir, None, &mut inventory);
    let attribution = candidate_attribution(&candidates);
    let resources = observe_live_candidates(candidates, &mut inventory, true, false);
    finish_machine_inventory(inventory, attribution, &resources)
}

/// Share the capacity sampler's process census. On Windows its caller scopes CPU refresh.
pub(crate) fn observe_machine_sessions_with_system(
    sessions_dir: &Path,
    system: &mut sysinfo::System,
) -> MachineSessionInventory {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
    observe_machine_sessions_with_system_refresh(sessions_dir, system, |system| {
        let refresh_kind = ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet);
        // Linux advances process-time baselines even during metadata refresh;
        // keep its existing single CPU pass instead of splitting the sample.
        #[cfg(not(windows))]
        let refresh_kind = refresh_kind.with_cpu();
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);
    })
}

fn observe_machine_sessions_with_system_refresh(
    sessions_dir: &Path,
    system: &mut sysinfo::System,
    mut refresh: impl FnMut(&mut sysinfo::System),
) -> MachineSessionInventory {
    let mut inventory = SessionInventory::default();
    let mut process_evidence = None;
    let candidates = read_candidates_with_liveness(
        None,
        sessions_dir,
        None,
        &mut inventory,
        |host_pid, runtime| {
            // The first callback runs only after every sidecar was captured.
            // Reuse this snapshot for pruning, CPU and exact PTY observations.
            let (observed, pids) = process_evidence.get_or_insert_with(|| {
                refresh(system);
                process_liveness_evidence(system)
            });
            runtime_has_live_process(host_pid, runtime) || !*observed || pids.contains(&host_pid)
        },
    );
    if process_evidence.is_none() {
        // An empty Session inventory still needs the GUI CPU observation.
        refresh(system);
    }
    let attribution = candidate_attribution(&candidates);
    let resources = capture_process_resources_with_system(&candidates, &inventory, true, system);
    observe_candidate_resources(candidates, &mut inventory, &resources, false);
    finish_machine_inventory(inventory, attribution, &resources)
}

fn finish_machine_inventory(
    mut inventory: SessionInventory,
    attribution: BTreeMap<String, (PathBuf, Option<String>)>,
    resources: &MachineProcessResources,
) -> MachineSessionInventory {
    inventory.uncertainties.retain(|uncertainty| {
        if !uncertainty.reason.starts_with("session_unreadable:") {
            return true;
        }
        let Ok(runtime) = cached_runtime(&uncertainty.runtime_path) else {
            return true;
        };
        let Some(host_pid) = uncertainty
            .runtime_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse::<u32>().ok())
        else {
            return true;
        };
        let direct_child = resources.legacy_processes.iter().any(|process| {
            process.parent_pid == host_pid
                && uncertainty.session_id.as_deref() == Some(process.session_id.as_str())
        });
        direct_child
            || (!runtime_from_previous_boot(
                &runtime,
                resources.boot_epoch,
                resources.tokens_are_epoch,
            ) && !historical_runtime(
                &runtime,
                host_pid,
                &resources.processes,
                crate::process::is_process_group_alive,
                direct_child,
            ))
    });
    machine_inventory(inventory, attribution, resources)
}

#[derive(Default)]
struct MachineProcessResources {
    memory: BTreeMap<u32, u64>,
    reviews: BTreeSet<u32>,
    scalar_identities: BTreeMap<u32, u64>,
    processes: BTreeMap<u32, u64>,
    epochs: BTreeMap<u32, u64>,
    legacy_processes: Vec<LegacyProcessObservation>,
    boot_epoch: u64,
    tokens_are_epoch: bool,
}

fn candidate_attribution(
    candidates: &[RuntimeCandidate],
) -> BTreeMap<String, (PathBuf, Option<String>)> {
    candidates
        .iter()
        .map(|candidate| {
            (
                candidate.session.id.clone(),
                (
                    normalized(
                        candidate
                            .session
                            .project_state_root
                            .as_deref()
                            .unwrap_or(&candidate.session.worktree_path),
                    ),
                    candidate.session.repo_hash.clone(),
                ),
            )
        })
        .collect()
}

fn machine_inventory(
    mut inventory: SessionInventory,
    attribution: BTreeMap<String, (PathBuf, Option<String>)>,
    resources: &MachineProcessResources,
) -> MachineSessionInventory {
    let sessions = inventory
        .sessions
        .into_iter()
        .map(|session| {
            let (project_root, repo_hash) = attribution[&session.session_id].clone();
            let scalars_match = session.child_started_at > 0
                && resources.scalar_identities.get(&session.child_pid)
                    == Some(&session.child_started_at);
            let role = if crate::pm_registry::pane_is_pm(
                &project_root,
                Some(&session.worktree_path),
                Some(&session.session_id),
            ) {
                MachineSessionRole::Pm
            } else if scalars_match && resources.reviews.contains(&session.child_pid) {
                MachineSessionRole::Review
            } else {
                MachineSessionRole::Implementation
            };
            let resident_memory_bytes = resources
                .memory
                .get(&session.child_pid)
                .copied()
                .filter(|bytes| scalars_match && *bytes > 0);
            if resident_memory_bytes.is_none() {
                inventory.uncertainties.push(SessionObservationUncertainty {
                    runtime_path: PathBuf::new(),
                    session_id: Some(session.session_id.clone()),
                    reason: "child_resident_memory_unavailable".to_string(),
                });
            }
            let monitor_owned =
                crate::cli::execution_state::session_launch_route(Some(&session.session_id))
                    == Some(gwt_agent::LaunchRoute::Autonomous);
            MachineSessionObservation {
                session,
                project_root,
                repo_hash,
                role,
                monitor_owned,
                resident_memory_bytes,
            }
        })
        .collect();
    MachineSessionInventory {
        sessions,
        uncertainties: inventory.uncertainties,
    }
}

#[derive(Debug, Default)]
pub struct SessionInventory {
    pub sessions: Vec<SessionObservation>,
    pub uncertainties: Vec<SessionObservationUncertainty>,
}

#[derive(Debug, Serialize)]
pub struct WorktreeSessionCount {
    pub worktree_path: PathBuf,
    pub worktree_exists: bool,
    pub session_count: usize,
    pub session_ids: Vec<String>,
}

pub fn observe_sessions(project_root: &Path, sessions_dir: &Path) -> SessionInventory {
    observe_sessions_filtered(project_root, sessions_dir, None)
}

/// A single tick's read-only census, shared by project roots using the same
/// Session directory. Deduplication and uncertain evidence stay project-scoped.
/// Never reuse this across ticks or for stop authorization.
pub struct ProjectSessionCensus {
    candidates: Vec<RuntimeCandidate>,
    uncertainties: Vec<(SessionObservationUncertainty, Option<Session>)>,
    resources: MachineProcessResources,
}

impl ProjectSessionCensus {
    pub fn capture(sessions_dir: &Path) -> Self {
        let mut inventory = SessionInventory::default();
        let mut snapshot = None;
        let candidates = read_candidates_scope_with_snapshot(
            None,
            sessions_dir,
            None,
            &mut inventory,
            &mut snapshot,
        );
        let resources = capture_process_resources(&candidates, &inventory, false, snapshot);
        inventory.check_deadline(&sessions_dir.join("runtime"));
        let uncertainties = inventory
            .uncertainties
            .into_iter()
            .map(|uncertainty| {
                // Only runtime decode errors have a known, scoped Session. Ledger
                // and directory failures must continue to fence every project.
                let scope = uncertainty
                    .reason
                    .starts_with("runtime_unreadable:")
                    .then(|| {
                        uncertainty.session_id.as_ref().and_then(|id| {
                            Session::load(&sessions_dir.join(format!("{id}.toml"))).ok()
                        })
                    })
                    .flatten();
                (uncertainty, scope)
            })
            .collect();
        Self {
            candidates,
            uncertainties,
            resources,
        }
    }

    pub fn observe(&self, project_root: &Path) -> SessionInventory {
        let root = normalized(project_root);
        let hash = gwt_core::repo_hash::detect_repo_identity(&root)
            .map(|identity| identity.hash.as_str().to_string());
        let matches = |session: &Session| session_matches_project(session, &root, hash.as_deref());
        let mut inventory = SessionInventory {
            sessions: Vec::new(),
            uncertainties: self
                .uncertainties
                .iter()
                .filter(|(_, scope)| scope.as_ref().is_none_or(matches))
                .map(|(uncertainty, _)| uncertainty.clone())
                .collect(),
        };
        observe_candidate_resources(
            self.candidates
                .iter()
                .filter(|row| matches(&row.session))
                .cloned()
                .collect(),
            &mut inventory,
            &self.resources,
            false,
        );
        inventory
    }
}

/// Observe one Session before cross-Session process deduplication. Recovery
/// callers hold its Session lease and must also check every uncertainty.
/// A live host without recorded identity remains uncertain for recovery.
pub fn observe_session(session: &Session, sessions_dir: &Path) -> SessionInventory {
    observe_sessions_filtered(
        session
            .project_state_root
            .as_deref()
            .unwrap_or(&session.worktree_path),
        sessions_dir,
        Some(&session.id),
    )
}

fn observe_sessions_filtered(
    project_root: &Path,
    sessions_dir: &Path,
    session_id: Option<&str>,
) -> SessionInventory {
    let mut inventory = SessionInventory::default();
    let candidates = read_candidates(project_root, sessions_dir, session_id, &mut inventory);
    observe_live_candidates(candidates, &mut inventory, false, session_id.is_some());
    inventory
}

fn observe_live_candidates(
    candidates: Vec<RuntimeCandidate>,
    inventory: &mut SessionInventory,
    measure_resources: bool,
    require_host_absence: bool,
) -> MachineProcessResources {
    if !inventory.check_deadline(Path::new("runtime")) {
        return MachineProcessResources::default();
    }
    let resources = capture_process_resources(&candidates, inventory, measure_resources, None);
    observe_candidate_resources(candidates, inventory, &resources, require_host_absence);
    inventory.check_deadline(Path::new("runtime"));
    resources
}

fn capture_process_resources(
    candidates: &[RuntimeCandidate],
    inventory: &SessionInventory,
    measure_resources: bool,
    snapshot: Option<sysinfo::System>,
) -> MachineProcessResources {
    if candidates.is_empty() && inventory.uncertainties.is_empty() {
        return MachineProcessResources::default();
    }
    // Read process state after the sidecars so a newly published child is not
    // compared with an OS snapshot taken before it was spawned.
    let system = snapshot.unwrap_or_else(process_snapshot);
    capture_process_resources_with_system(candidates, inventory, measure_resources, &system)
}

fn capture_process_resources_with_system(
    candidates: &[RuntimeCandidate],
    inventory: &SessionInventory,
    measure_resources: bool,
    system: &sysinfo::System,
) -> MachineProcessResources {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    if candidates.is_empty() && inventory.uncertainties.is_empty() {
        return MachineProcessResources::default();
    }
    let orphan_hosts = inventory
        .uncertainties
        .iter()
        .filter(|row| row.reason.starts_with("session_unreadable:"))
        .filter_map(|row| {
            row.runtime_path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .and_then(|name| name.parse::<u32>().ok())
        })
        .collect::<BTreeSet<_>>();
    let legacy_hosts = candidates
        .iter()
        .filter(|candidate| {
            candidate
                .runtime
                .child_pid
                .zip(candidate.runtime.child_started_at)
                .is_none_or(|(pid, started)| pid == 0 || started == 0)
        })
        .map(|candidate| candidate.host_pid)
        .chain(orphan_hosts.iter().copied())
        .collect::<BTreeSet<_>>();
    let legacy_children = system
        .processes()
        .iter()
        .filter(|(_, process)| {
            process
                .parent()
                .is_some_and(|pid| legacy_hosts.contains(&pid.as_u32()))
        })
        .map(|(pid, _)| *pid)
        .collect::<Vec<_>>();
    let legacy_identities = legacy_children
        .iter()
        .filter_map(|pid| {
            system.process(*pid).map(|process| {
                (
                    pid.as_u32(),
                    crate::process::snapshot_process_start_identity(pid.as_u32(), process),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    // Resource refreshes retain old values on access failure. Keep those probes
    // transient so an earlier positive RSS/environment never survives a tick.
    let mut scalar_system = System::new();
    if !legacy_children.is_empty() {
        scalar_system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&legacy_children),
            true,
            ProcessRefreshKind::nothing()
                .with_environ(UpdateKind::Always)
                .with_cwd(UpdateKind::Always),
        );
    }
    let epochs = system
        .processes()
        .iter()
        .map(|(pid, process)| (pid.as_u32(), process.start_time()))
        .collect::<BTreeMap<_, _>>();
    let legacy_processes =
        legacy_process_observations(system, &scalar_system, &legacy_hosts, &legacy_identities);
    // Identity comes from the per-process start token, not the wall-clock
    // start time, which moves with the boot time on Linux (Issue #5089).
    // Only the PIDs a candidate can name are read.
    let uncertain_pids = inventory
        .uncertainties
        .iter()
        .filter(|row| row.reason.starts_with("session_unreadable:"))
        .filter_map(|row| cached_runtime(&row.runtime_path).ok())
        .filter_map(|runtime| runtime.child_pid)
        .collect::<Vec<_>>();
    let named = candidates
        .iter()
        .flat_map(|candidate| [Some(candidate.host_pid), candidate.runtime.child_pid])
        .flatten()
        .chain(legacy_processes.iter().map(|process| process.pid))
        .chain(uncertain_pids)
        .chain(orphan_hosts)
        .collect::<BTreeSet<_>>();
    let processes = system
        .processes()
        .iter()
        .filter(|(pid, _)| named.contains(&pid.as_u32()))
        .map(|(pid, process)| {
            (
                pid.as_u32(),
                crate::process::snapshot_process_start_identity(pid.as_u32(), process),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut resources = MachineProcessResources::default();
    if measure_resources {
        // Only named PTY identities are queried for resource/role scalars.
        let pids = named
            .iter()
            .copied()
            .map(sysinfo::Pid::from_u32)
            .collect::<Vec<_>>();
        scalar_system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing()
                .with_memory()
                .with_environ(UpdateKind::Always),
        );
        for (pid, process) in scalar_system
            .processes()
            .iter()
            .filter(|(pid, _)| named.contains(&pid.as_u32()))
        {
            resources.memory.insert(pid.as_u32(), process.memory());
            resources.scalar_identities.insert(
                pid.as_u32(),
                crate::process::snapshot_process_start_identity(pid.as_u32(), process),
            );
            if process
                .environ()
                .iter()
                .any(|entry| entry == "GWT_REVIEW_DISPATCH=1")
            {
                resources.reviews.insert(pid.as_u32());
            }
        }
    }
    let boot_epoch = System::boot_time();
    let tokens_are_epoch = cfg!(not(target_os = "linux"));
    resources.processes = epochs.clone();
    resources.processes.extend(processes);
    resources.epochs = epochs;
    resources.legacy_processes = legacy_processes;
    resources.boot_epoch = boot_epoch;
    resources.tokens_are_epoch = tokens_are_epoch;
    resources
}

fn observe_candidate_resources(
    candidates: Vec<RuntimeCandidate>,
    inventory: &mut SessionInventory,
    resources: &MachineProcessResources,
    require_host_absence: bool,
) {
    let candidates = candidates
        .into_iter()
        .filter(|candidate| {
            !runtime_from_previous_boot(
                &candidate.runtime,
                resources.boot_epoch,
                resources.tokens_are_epoch,
            )
        })
        .collect::<Vec<_>>();
    if require_host_absence {
        // Census absence alone does not authorize abandoning another Session.
        for candidate in &candidates {
            if candidate
                .runtime
                .host_started_at
                .filter(|started| *started > 0)
                .is_none()
                && candidate
                    .runtime
                    .child_pid
                    .zip(candidate.runtime.child_started_at)
                    .filter(|(pid, started)| *pid > 0 && *started > 0)
                    .is_none()
                && resources.processes.contains_key(&candidate.host_pid)
            {
                inventory.uncertain(
                    &candidate.path,
                    Some(&candidate.session.id),
                    "host_identity_missing_for_recovery".to_string(),
                );
            }
        }
    }
    observe_candidates(
        candidates,
        &resources.processes,
        &resources.epochs,
        crate::process::is_process_group_alive,
        inventory,
        &resources.legacy_processes,
    );
}

fn process_snapshot() -> sysinfo::System {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    system
}

fn legacy_process_observations(
    system: &sysinfo::System,
    scalar_system: &sysinfo::System,
    legacy_hosts: &BTreeSet<u32>,
    legacy_identities: &BTreeMap<u32, u64>,
) -> Vec<LegacyProcessObservation> {
    // Preserve snapshot existence independently of readable attribution.
    // Only matching fresh identities may supply the Session id and cwd.
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let parent_pid = process.parent()?.as_u32();
            if !legacy_hosts.contains(&parent_pid) {
                return None;
            }
            let started_at = legacy_identities
                .get(&pid.as_u32())
                .copied()
                .unwrap_or_default();
            let scalar = scalar_system.process(*pid).filter(|scalar| {
                started_at > 0
                    && crate::process::snapshot_process_start_identity(pid.as_u32(), scalar)
                        == started_at
            });
            let session_id = scalar
                .and_then(|scalar| {
                    scalar
                        .environ()
                        .iter()
                        .filter_map(|entry| entry.to_str())
                        .find_map(|entry| entry.strip_prefix("GWT_SESSION_ID="))
                })
                .unwrap_or_default();
            Some(LegacyProcessObservation {
                pid: pid.as_u32(),
                parent_pid,
                parent_may_be_host: system
                    .process(sysinfo::Pid::from_u32(parent_pid))
                    .is_none_or(|parent| {
                        let name = Path::new(parent.name())
                            .file_stem()
                            .and_then(|name| name.to_str());
                        name.is_none_or(|name| {
                            name.eq_ignore_ascii_case("gwt") || name.eq_ignore_ascii_case("gwtd")
                        })
                    }),
                started_at,
                session_id: session_id.to_string(),
                // Retain present children even when attribution is unreadable.
                cwd: scalar
                    .and_then(sysinfo::Process::cwd)
                    .map(Path::to_path_buf)
                    .unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
fn observe_with_processes(
    project_root: &Path,
    sessions_dir: &Path,
    processes: &BTreeMap<u32, u64>,
    group_alive: impl Fn(u32) -> bool,
) -> SessionInventory {
    let mut inventory = SessionInventory::default();
    let candidates = read_candidates_with_liveness(
        Some(project_root),
        sessions_dir,
        None,
        &mut inventory,
        |host_pid, runtime| {
            processes.contains_key(&host_pid)
                || runtime
                    .child_pid
                    .is_some_and(|pid| processes.contains_key(&pid) || group_alive(pid))
        },
    );
    observe_candidates(
        candidates,
        processes,
        processes,
        group_alive,
        &mut inventory,
        &[],
    );
    inventory
}

#[derive(Clone)]
struct RuntimeCandidate {
    host_pid: u32,
    path: PathBuf,
    session: Session,
    runtime: Arc<SessionRuntimeState>,
}

struct LegacyProcessObservation {
    pid: u32,
    parent_pid: u32,
    parent_may_be_host: bool,
    started_at: u64,
    session_id: String, // Empty when the environment is unavailable.
    cwd: PathBuf,       // Empty when the working directory is unavailable.
}

fn normalized(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn session_matches_project(session: &Session, root: &Path, repo_hash: Option<&str>) -> bool {
    repo_hash.is_some_and(|hash| session.repo_hash.as_deref() == Some(hash))
        || normalized(
            session
                .project_state_root
                .as_deref()
                .unwrap_or(&session.worktree_path),
        ) == root
}

fn exact_host_identity(runtime: &SessionRuntimeState, observed: Option<u64>) -> bool {
    observed.is_some_and(|started| {
        started > 0
            && runtime
                .host_started_at
                .is_some_and(|expected| expected == started)
    })
}

fn historical_runtime(
    runtime: &SessionRuntimeState,
    host_pid: u32,
    processes: &BTreeMap<u32, u64>,
    group_alive: impl Fn(u32) -> bool,
    exact_direct_child: bool,
) -> bool {
    if exact_direct_child {
        return false;
    }
    if let Some(pid) = runtime.child_pid.filter(|pid| *pid > 0) {
        return match processes.get(&pid) {
            None => !group_alive(pid),
            Some(started) if *started > 0 => runtime
                .child_started_at
                .filter(|expected| *expected > 0)
                .is_some_and(|expected| expected != *started),
            Some(_) => false,
        };
    }
    !exact_direct_child && !exact_host_identity(runtime, processes.get(&host_pid).copied())
}

fn runtime_from_previous_boot(
    runtime: &SessionRuntimeState,
    boot: u64,
    tokens_are_epoch: bool,
) -> bool {
    if !tokens_are_epoch || boot == 0 {
        return false;
    }
    runtime
        .child_started_at
        .filter(|started| *started > 0)
        .or(runtime.host_started_at)
        .is_some_and(|started| started > 0 && started < boot)
}

fn read_candidates(
    project_root: &Path,
    sessions_dir: &Path,
    session_id: Option<&str>,
    inventory: &mut SessionInventory,
) -> Vec<RuntimeCandidate> {
    read_candidates_scope(Some(project_root), sessions_dir, session_id, inventory)
}

fn read_candidates_scope(
    project_root: Option<&Path>,
    sessions_dir: &Path,
    session_id: Option<&str>,
    inventory: &mut SessionInventory,
) -> Vec<RuntimeCandidate> {
    read_candidates_scope_with_snapshot(
        project_root,
        sessions_dir,
        session_id,
        inventory,
        &mut None,
    )
}

fn read_candidates_scope_with_snapshot(
    project_root: Option<&Path>,
    sessions_dir: &Path,
    session_id: Option<&str>,
    inventory: &mut SessionInventory,
    snapshot: &mut Option<sysinfo::System>,
) -> Vec<RuntimeCandidate> {
    let mut process_evidence = None;
    read_candidates_with_liveness(
        project_root,
        sessions_dir,
        session_id,
        inventory,
        |host_pid, runtime| {
            if runtime_has_live_process(host_pid, runtime) {
                return true;
            }
            // A legacy orphan may have no recorded child PID. Preserve the
            // parent-PID evidence used by missing-ledger recovery, including
            // Windows, where a child's parent PID survives its parent's exit.
            // All sidecars have been read before this callback. Retain live
            // PIDs too: a Host can spawn a child and exit after the snapshot
            // but before its direct probe. Never collect environments.
            let (observed, pids) = process_evidence.get_or_insert_with(|| {
                let system = snapshot.get_or_insert_with(process_snapshot);
                process_liveness_evidence(system)
            });
            !*observed || pids.contains(&host_pid)
        },
    )
}

fn runtime_has_live_process(host_pid: u32, runtime: &SessionRuntimeState) -> bool {
    crate::process::is_host_process_alive(host_pid)
        || runtime.child_pid.filter(|pid| *pid > 0).is_some_and(|pid| {
            crate::process::is_host_process_alive(pid)
                || crate::process::is_process_group_alive(pid)
        })
}

fn process_liveness_evidence(system: &sysinfo::System) -> (bool, BTreeSet<u32>) {
    (
        !system.processes().is_empty(),
        system
            .processes()
            .keys()
            .map(|pid| pid.as_u32())
            .chain(
                system
                    .processes()
                    .values()
                    .filter_map(|process| process.parent().map(|pid| pid.as_u32())),
            )
            .collect(),
    )
}

fn read_candidates_with_liveness(
    project_root: Option<&Path>,
    sessions_dir: &Path,
    session_id: Option<&str>,
    inventory: &mut SessionInventory,
    mut may_be_live: impl FnMut(u32, &SessionRuntimeState) -> bool,
) -> Vec<RuntimeCandidate> {
    let project_root = project_root.map(normalized);
    let repo_hash = project_root
        .as_deref()
        .and_then(gwt_core::repo_hash::detect_repo_identity)
        .map(|identity| identity.hash.as_str().to_string());
    let mut runtimes = Vec::new();
    let runtime_root = sessions_dir.join("runtime");
    for entry in read_entries(&runtime_root, inventory) {
        let namespace = entry.path();
        if !inventory.check_deadline(&runtime_root) {
            break;
        }
        let Some(host_pid) = namespace
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse::<u32>().ok())
            .filter(|pid| *pid > 0)
        else {
            continue;
        };
        for entry in read_entries(&namespace, inventory) {
            let path = entry.path();
            if !inventory.check_deadline(&runtime_root) {
                break;
            }
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            // Older bridge receipts used a JSON companion beside the runtime.
            // Their payload is not a SessionRuntimeState and has no Session ledger.
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".bridge.json"))
            {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };
            if session_id.is_some_and(|expected| expected != id) {
                continue;
            }
            // Read every small sidecar before taking any process snapshot so
            // children published during the scan are not compared with an
            // older parent-PID inventory. No Session ledger is retained here.
            let runtime = cached_runtime_entry(&entry, &path);
            runtimes.push((host_pid, path, runtime));
        }
    }
    if session_id.is_none()
        && gwt_core::operation_deadline::ensure_remaining("session census").is_ok()
    {
        // Full sweeps drop files that GC or a Host removed. A filtered recovery
        // observation must not evict other Sessions' entries.
        let root = sessions_dir.join("runtime");
        let seen = runtimes
            .iter()
            .map(|(_, path, _)| path.as_path())
            .collect::<HashSet<_>>();
        if let Some(cache) = RUNTIME_CACHE.get() {
            cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|path, _| !path.starts_with(&root) || seen.contains(path.as_path()));
        }
    }
    let mut candidates = Vec::new();
    for (host_pid, path, runtime) in runtimes {
        if !inventory.check_deadline(&runtime_root) {
            break;
        }
        let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        if runtime
            .as_ref()
            .is_ok_and(|runtime| !may_be_live(host_pid, runtime))
        {
            continue;
        }
        // Preserve the original ledger/scope/error priority for uncertain rows.
        let session = match Session::load(&sessions_dir.join(format!("{id}.toml"))) {
            Ok(session) => session,
            Err(error) => {
                inventory.uncertain(&path, Some(id), format!("session_unreadable: {error}"));
                continue;
            }
        };
        if project_root
            .as_ref()
            .is_some_and(|root| !session_matches_project(&session, root, repo_hash.as_deref()))
        {
            continue;
        }
        match runtime {
            Ok(runtime) => candidates.push(RuntimeCandidate {
                host_pid,
                path,
                session,
                runtime,
            }),
            Err(error) => {
                inventory.uncertain(&path, Some(id), format!("runtime_unreadable: {error}"))
            }
        }
    }
    inventory.check_deadline(&runtime_root);
    candidates
}

fn read_entries(directory: &Path, inventory: &mut SessionInventory) -> Vec<std::fs::DirEntry> {
    if !inventory.check_deadline(directory) {
        return Vec::new();
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            inventory.uncertain(
                directory,
                None,
                format!("runtime_directory_unreadable: {error}"),
            );
            return Vec::new();
        }
    };
    let mut captured = Vec::new();
    for entry in entries {
        if !inventory.check_deadline(directory) {
            break;
        }
        match entry {
            Ok(entry) => captured.push(entry),
            Err(error) => inventory.uncertain(
                directory,
                None,
                format!("runtime_entry_unreadable: {error}"),
            ),
        }
    }
    // Every entry shares this parent; sorting file names preserves path order.
    captured.sort_by_cached_key(std::fs::DirEntry::file_name);
    captured
}

fn observe_candidates(
    candidates: Vec<RuntimeCandidate>,
    processes: &BTreeMap<u32, u64>,
    epochs: &BTreeMap<u32, u64>,
    group_alive: impl Fn(u32) -> bool,
    inventory: &mut SessionInventory,
    legacy_processes: &[LegacyProcessObservation],
) {
    let mut seen = BTreeSet::new();
    for RuntimeCandidate {
        host_pid,
        path,
        session,
        runtime,
    } in candidates
    {
        let child = runtime
            .child_pid
            .zip(runtime.child_started_at)
            .filter(|(pid, started)| *pid > 0 && *started > 0);
        let host_live = processes.get(&host_pid).is_some_and(|started| {
            *started > 0
                && runtime
                    .host_started_at
                    .is_none_or(|expected| expected == *started)
        });
        let children = if let Some(child) = child {
            vec![child]
        } else {
            let worktree = normalized(&session.worktree_path);
            legacy_processes
                .iter()
                .filter(|process| {
                    host_live
                        && process.parent_pid == host_pid
                        && process.session_id == session.id
                        && process.pid > 0
                        && process.started_at > 0
                        && !process.cwd.as_os_str().is_empty()
                        && normalized(&process.cwd) == worktree
                })
                .map(|process| (process.pid, process.started_at))
                .collect()
        };
        if children.is_empty() {
            let host_may_match = processes.get(&host_pid).is_some_and(|started| {
                *started == 0
                    || runtime
                        .host_started_at
                        .is_none_or(|expected| expected == *started)
            });
            let unidentified_child = host_may_match
                && legacy_processes.iter().any(|process| {
                    process.parent_pid == host_pid
                        && processes.contains_key(&process.pid)
                        && (process.session_id.is_empty() || process.session_id == session.id)
                        && (!process.session_id.is_empty() || process.parent_may_be_host)
                });
            // A legacy namespace PID may now belong to an unrelated process.
            // Require Host provenance or actual child evidence, not PID alone.
            if exact_host_identity(&runtime, processes.get(&host_pid).copied())
                || unidentified_child
                || runtime
                    .child_pid
                    .is_some_and(|pid| processes.contains_key(&pid) || group_alive(pid))
            {
                inventory.uncertain(
                    &path,
                    Some(&session.id),
                    "child_identity_missing".to_string(),
                );
            }
            continue;
        }
        for (child_pid, child_started_at) in children {
            match processes.get(&child_pid) {
                Some(started) if *started == child_started_at => {}
                Some(0) => {
                    inventory.uncertain(
                        &path,
                        Some(&session.id),
                        "child_start_time_unavailable".to_string(),
                    );
                    continue;
                }
                Some(_) => continue, // Recycled PID, not this PTY.
                None => {
                    if group_alive(child_pid) {
                        inventory.uncertain(
                            &path,
                            Some(&session.id),
                            "child_exited_with_live_process_group".to_string(),
                        );
                    }
                    continue;
                }
            }
            let Some(started_at) = epochs
                .get(&child_pid)
                .filter(|started| **started > 0)
                .and_then(|started| i64::try_from(*started).ok())
                .and_then(|started| chrono::DateTime::<chrono::Utc>::from_timestamp(started, 0))
            else {
                inventory.uncertain(
                    &path,
                    Some(&session.id),
                    "child_start_time_invalid".to_string(),
                );
                continue;
            };
            if !seen.insert((child_pid, child_started_at)) {
                continue;
            }
            let entrypoint = gwt_agent::permission_mode::entrypoint_from_launch_args(
                &session.launch_args,
                false,
            );
            inventory.sessions.push(SessionObservation {
                session_id: session.id.clone(),
                issue_number: session.linked_issue_number,
                execution_binding_missing: session.linked_issue_number.is_some()
                    && session.execution_binding.is_none(),
                agent_id: session.agent_id.command().to_string(),
                worktree_exists: session.worktree_path.is_dir(),
                worktree_path: session.worktree_path.clone(),
                host_pid,
                child_pid,
                child_started_at,
                started_at: started_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                launch_origin: session.launch_origin,
                launch_entrypoint: gwt_skills::assets::CLAUDE_SKILLS
                    .dirs()
                    .any(|skill| {
                        skill.path().file_name().and_then(|name| name.to_str())
                            == Some(entrypoint.as_str())
                    })
                    .then_some(entrypoint),
                restore_source_session_id: session.restore_source_session_id.clone(),
            });
        }
    }
    let mut session_counts = BTreeMap::new();
    for row in &inventory.sessions {
        *session_counts.entry(row.session_id.clone()).or_insert(0) += 1;
    }
    for row in &mut inventory.sessions {
        if session_counts[&row.session_id] > 1 {
            // The ledger describes only the latest use of a Session id; it
            // cannot attribute one origin to multiple distinct live PTYs.
            row.launch_origin = SessionLaunchOrigin::Unknown;
            row.launch_entrypoint = None;
            row.restore_source_session_id = None;
        }
    }
    inventory.sessions.sort_by(|left, right| {
        (
            &left.worktree_path,
            left.child_started_at,
            &left.session_id,
            left.child_pid,
        )
            .cmp(&(
                &right.worktree_path,
                right.child_started_at,
                &right.session_id,
                right.child_pid,
            ))
    });
}

impl SessionInventory {
    fn check_deadline(&mut self, path: &Path) -> bool {
        match gwt_core::operation_deadline::ensure_remaining("session census") {
            Ok(_) => true,
            Err(error) => {
                // No Session attribution: missing rows in a partial census are
                // uncertain for every owner, never proof of process absence.
                self.uncertain(path, None, error.to_string());
                false
            }
        }
    }

    fn uncertain(&mut self, path: &Path, session_id: Option<&str>, reason: String) {
        self.uncertainties.push(SessionObservationUncertainty {
            runtime_path: path.to_path_buf(),
            session_id: session_id.map(str::to_string),
            reason,
        });
    }

    pub fn worktree_sessions(&self) -> Vec<WorktreeSessionCount> {
        let mut groups: BTreeMap<PathBuf, WorktreeSessionCount> = BTreeMap::new();
        for session in &self.sessions {
            let group = groups
                .entry(session.worktree_path.clone())
                .or_insert_with(|| WorktreeSessionCount {
                    worktree_path: session.worktree_path.clone(),
                    worktree_exists: session.worktree_exists,
                    session_count: 0,
                    session_ids: Vec::new(),
                });
            group.session_count += 1;
            group.session_ids.push(session.session_id.clone());
        }
        groups.into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_agent::{AgentId, AgentStatus};

    #[test]
    fn issue_5644_expired_inventory_is_uncertain_instead_of_empty() {
        let temp = tempfile::tempdir().unwrap();
        let deadline = std::time::Instant::now();
        let _clock = gwt_core::operation_deadline::ScopedOperationClock::set(deadline);
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline);
        let inventory = observe_sessions(temp.path(), &temp.path().join("sessions"));
        assert!(inventory.sessions.is_empty());
        assert!(
            inventory
                .uncertainties
                .iter()
                .any(|row| row.reason.contains("deadline expired")),
            "a partial census must not authorize releasing a running owner"
        );
    }

    #[test]
    fn project_census_keeps_scopes_and_uncertainties_without_rereading_the_tick() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let roots = [temp.path().join("first"), temp.path().join("second")];
        let pid = std::process::id();
        let started = crate::process::host_process_start_time(pid).unwrap();
        for (index, root) in roots.iter().enumerate() {
            std::fs::create_dir(root).unwrap();
            // The same PTY may have aliases in different project scopes. A
            // global deduplication must not hide either project's observation.
            save_runtime(
                &sessions,
                root,
                root,
                &format!("live-{index}"),
                Some((pid, started)),
            );
            save_runtime(&sessions, root, root, &format!("bad-{index}"), None);
            let bad =
                gwt_agent::runtime_state_path_for_pid(&sessions, 100, &format!("bad-{index}"));
            std::fs::write(bad, b"invalid runtime").unwrap();
        }
        std::fs::write(
            sessions.join("runtime/100/unknown.json"),
            b"invalid runtime",
        )
        .unwrap();
        let expected = roots
            .each_ref()
            .map(|root| observe_sessions(root, &sessions));
        let census = ProjectSessionCensus::capture(&sessions);
        // No later sidecar/ledger changes may turn one tick into five censuses.
        std::fs::remove_dir_all(&sessions).unwrap();
        for (root, expected) in roots.iter().zip(expected) {
            let actual = census.observe(root);
            assert_eq!(actual.sessions, expected.sessions);
            assert_eq!(actual.sessions.len(), 1, "deduplication is project-scoped");
            assert_eq!(actual.uncertainties, expected.uncertainties);
            assert_eq!(
                actual.uncertainties.len(),
                2,
                "scoped and unknown failures survive"
            );
        }
    }

    #[test]
    fn warm_runtime_entry_reuses_directory_stamp_without_path_probe() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime.json");
        SessionRuntimeState::new(AgentStatus::Running)
            .save(&path)
            .unwrap();
        let entry = std::fs::read_dir(temp.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let before_cold = RUNTIME_PATH_STAMP_PROBES.with(std::cell::Cell::get);
        let first = cached_runtime_entry(&entry, &path).unwrap();
        assert_eq!(
            RUNTIME_PATH_STAMP_PROBES.with(std::cell::Cell::get) - before_cold,
            1,
            "cold entry must validate the path stamp after reading"
        );
        let before = RUNTIME_PATH_STAMP_PROBES.with(std::cell::Cell::get);
        let second = cached_runtime_entry(&entry, &path).unwrap();
        let probes = RUNTIME_PATH_STAMP_PROBES.with(std::cell::Cell::get) - before;
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            probes, 0,
            "warm regular entry must reuse its directory stamp"
        );
    }

    #[cfg(unix)]
    #[test]
    fn runtime_entry_symlink_rechecks_the_target_stamp() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.json");
        let link = temp.path().join("link.json");
        let mut runtime = SessionRuntimeState::new(AgentStatus::Running);
        runtime.save(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let entry = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap())
            .find(|entry| entry.path() == link)
            .unwrap();
        assert!(cached_runtime_entry(&entry, &link)
            .unwrap()
            .source_event
            .is_none());
        runtime.source_event = Some("changed target".to_string());
        runtime.save(&target).unwrap();
        assert_eq!(
            cached_runtime_entry(&entry, &link)
                .unwrap()
                .source_event
                .as_deref(),
            Some("changed target")
        );
    }

    #[test]
    fn runtime_cache_reuses_only_matching_mtime_and_size() {
        use std::{
            fs,
            sync::Arc,
            time::{Duration, SystemTime},
        };

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime.json");
        let mut runtime = SessionRuntimeState::new(AgentStatus::Running);
        runtime.save(&path).unwrap();
        let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let set_modified = |stamp| {
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(stamp))
                .unwrap();
        };
        set_modified(stamp);
        let first = cached_runtime(&path).unwrap();
        assert!(
            Arc::ptr_eq(&first, &cached_runtime(&path).unwrap()),
            "unchanged runtime must not be reparsed"
        );

        set_modified(stamp + Duration::from_secs(1));
        let touched = cached_runtime(&path).unwrap();
        assert!(
            !Arc::ptr_eq(&first, &touched),
            "mtime changes invalidate the cache"
        );
        runtime.source_event = Some("size changed".to_string());
        runtime.save(&path).unwrap();
        set_modified(stamp + Duration::from_secs(1));
        let resized = cached_runtime(&path).unwrap();
        assert_eq!(resized.source_event.as_deref(), Some("size changed"));
        assert!(
            !Arc::ptr_eq(&touched, &resized),
            "size changes invalidate even with preserved mtime"
        );

        fs::write(&path, b"invalid runtime").unwrap();
        assert!(
            cached_runtime(&path).is_err(),
            "malformed replacements must not reuse a success"
        );
        runtime.save(&path).unwrap();
        assert_eq!(
            cached_runtime(&path).unwrap().source_event,
            runtime.source_event
        );
        fs::remove_file(&path).unwrap();
        assert_eq!(
            cached_runtime(&path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn cached_runtime_still_rechecks_child_and_group_liveness() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        save_runtime(
            &sessions,
            temp.path(),
            temp.path(),
            "orphan",
            Some((201, 20)),
        );
        let live =
            observe_with_processes(temp.path(), &sessions, &BTreeMap::from([(201, 20)]), |_| {
                false
            });
        assert_eq!(live.sessions.len(), 1);
        let group =
            observe_with_processes(temp.path(), &sessions, &BTreeMap::new(), |pid| pid == 201);
        assert_eq!(
            group.uncertainties[0].reason,
            "child_exited_with_live_process_group"
        );
        let dead = observe_with_processes(temp.path(), &sessions, &BTreeMap::new(), |_| false);
        assert!(dead.sessions.is_empty());
        assert!(dead.uncertainties.is_empty());
    }

    #[test]
    fn session_inventory_691_namespaces_cpu() {
        use std::time::Instant;
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let mut runtime = SessionRuntimeState::new(AgentStatus::Stopped);
        runtime.host_started_at = Some(1);
        runtime.source_event = Some("x".repeat(2400));
        let json = serde_json::to_vec_pretty(&runtime).unwrap();
        for index in 0..4047 {
            let path = gwt_agent::runtime_state_path_for_pid(
                &sessions,
                2_000_000_000 + index % 691,
                &format!("history-{index}"),
            );
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, &json).unwrap();
        }
        let pid = sysinfo::Pid::from_u32(std::process::id());
        let mut system = System::new();
        let mut cpu_ms = || {
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing().with_cpu(),
            );
            system.process(pid).unwrap().accumulated_cpu_time()
        };
        for pass in ["cold", "warm", "warm-repeat"] {
            let cpu_before = cpu_ms();
            let started = Instant::now();
            let inventory = observe_sessions(temp.path(), &sessions);
            let wall = started.elapsed();
            let cpu = cpu_ms() - cpu_before;
            assert!(inventory.sessions.is_empty());
            assert!(inventory.uncertainties.is_empty());
            eprintln!(
                "691 folders / 4047 files / {} bytes: {pass} observe CPU={cpu}ms wall={wall:?}",
                json.len() * 4047
            );
        }
    }

    #[test]
    fn absent_host_and_child_skip_session_ledger_without_removing_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let path = gwt_agent::runtime_state_path_for_pid(&sessions, 2_000_000_000, "dead");
        let mut runtime = SessionRuntimeState::new(AgentStatus::Stopped);
        runtime.child_pid = Some(2_000_000_001);
        runtime.child_started_at = Some(20);
        runtime.save(&path).unwrap();
        std::fs::write(sessions.join("dead.toml"), "invalid Session TOML").unwrap();

        let mut inventory = SessionInventory::default();
        let candidates = read_candidates_scope(None, &sessions, None, &mut inventory);

        assert!(candidates.is_empty());
        assert!(
            inventory.uncertainties.is_empty(),
            "a definitively dead namespace must not decode its Session ledger: {:?}",
            inventory.uncertainties
        );
        assert!(path.exists(), "census is read-only; GC belongs to #5384");
    }

    #[test]
    fn all_sidecars_are_read_before_the_liveness_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        save_runtime(&sessions, temp.path(), temp.path(), "first", None);
        save_runtime(
            &sessions,
            temp.path(),
            temp.path(),
            "later",
            Some((201, 20)),
        );
        let later = gwt_agent::runtime_state_path_for_pid(&sessions, 100, "later");
        let mut inventory = SessionInventory::default();
        let candidates =
            read_candidates_with_liveness(None, &sessions, None, &mut inventory, |_, runtime| {
                if runtime.child_pid.is_none() {
                    // Model publication after the first liveness observation.
                    // The later sidecar must already have been captured.
                    std::fs::write(&later, b"changed after observation").unwrap();
                }
                runtime.child_pid == Some(201)
            });
        assert!(
            inventory.uncertainties.is_empty(),
            "{:?}",
            inventory.uncertainties
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].session.id, "later");
    }

    #[test]
    fn machine_capacity_reads_all_sidecars_before_one_shared_process_refresh() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        for id in ["first", "later"] {
            save_runtime(&sessions, temp.path(), temp.path(), id, None);
        }
        let mut system = sysinfo::System::new();
        let mut refreshes = 0;
        let inventory =
            observe_machine_sessions_with_system_refresh(&sessions, &mut system, |_| {
                refreshes += 1;
                for id in ["first", "later"] {
                    let path = gwt_agent::runtime_state_path_for_pid(&sessions, 100, id);
                    std::fs::write(path, b"changed after process observation").unwrap();
                }
            });
        assert_eq!(refreshes, 1, "all machine observations share one refresh");
        assert!(
            inventory.uncertainties.is_empty(),
            "sidecars must already be captured before the refresh: {:?}",
            inventory.uncertainties
        );
        observe_machine_sessions_with_system_refresh(
            &temp.path().join("empty"),
            &mut system,
            |_| refreshes += 1,
        );
        assert_eq!(refreshes, 2, "GUI CPU still refreshes without any Sessions");
    }

    #[test]
    fn orphan_child_keeps_live_count_and_missing_identity_uncertainty() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let mut session = Session::new(temp.path(), "orphan", AgentId::Codex);
        session.id = "orphan".to_string();
        session.project_state_root = Some(temp.path().to_path_buf());
        session.save(&sessions).unwrap();
        let path = gwt_agent::runtime_state_path_for_pid(&sessions, 2_000_000_000, &session.id);
        let mut runtime = SessionRuntimeState::new(AgentStatus::Running);
        runtime.child_pid = Some(std::process::id());
        runtime.child_started_at = crate::process::host_process_start_time(std::process::id());
        assert!(runtime.child_started_at.is_some());
        runtime.save(&path).unwrap();

        let live = observe_sessions(temp.path(), &sessions);
        assert_eq!(live.sessions.len(), 1);
        assert_eq!(live.sessions[0].session_id, session.id);
        assert!(live.uncertainties.is_empty());

        runtime.child_started_at = None;
        runtime.save(&path).unwrap();
        let uncertain = observe_sessions(temp.path(), &sessions);
        assert!(uncertain.sessions.is_empty());
        assert_eq!(uncertain.uncertainties.len(), 1);
        assert_eq!(uncertain.uncertainties[0].reason, "child_identity_missing");
    }

    #[test]
    fn orphan_process_group_remains_uncertain_after_early_filtering() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        save_runtime(
            &sessions,
            temp.path(),
            temp.path(),
            "orphan-group",
            Some((201, 20)),
        );
        let inventory =
            observe_with_processes(temp.path(), &sessions, &BTreeMap::new(), |pid| pid == 201);
        assert!(inventory.sessions.is_empty());
        assert_eq!(inventory.uncertainties.len(), 1);
        assert_eq!(
            inventory.uncertainties[0].reason,
            "child_exited_with_live_process_group"
        );
    }

    #[test]
    fn namespace_pid_presence_without_recorded_host_identity_is_not_live_session_proof() {
        let mut runtime = SessionRuntimeState::new(AgentStatus::Stopped);
        assert!(
            !exact_host_identity(&runtime, Some(100)),
            "old namespace PID may be an unrelated OS process"
        );
        runtime.host_started_at = Some(100);
        assert!(exact_host_identity(&runtime, Some(100)));
        assert!(!exact_host_identity(&runtime, Some(101)));
    }

    #[test]
    fn unreadable_ledger_with_dead_or_recycled_runtime_is_historical_but_unknown_live_is_retained()
    {
        let mut runtime = SessionRuntimeState::new(AgentStatus::Stopped);
        runtime.child_pid = Some(201);
        runtime.child_started_at = Some(20);
        assert!(
            !historical_runtime(&runtime, 100, &BTreeMap::new(), |_| false, true),
            "a current exact direct child must retain missing-ledger uncertainty"
        );
        assert!(historical_runtime(
            &runtime,
            100,
            &BTreeMap::new(),
            |_| false,
            false
        ));
        assert!(historical_runtime(
            &runtime,
            100,
            &BTreeMap::from([(201, 99)]),
            |_| false,
            false
        ));
        assert!(!historical_runtime(
            &runtime,
            100,
            &BTreeMap::from([(201, 20)]),
            |_| false,
            false
        ));
        assert!(!historical_runtime(
            &runtime,
            100,
            &BTreeMap::from([(201, 0)]),
            |_| false,
            false
        ));
        assert!(!historical_runtime(
            &runtime,
            100,
            &BTreeMap::new(),
            |_| true,
            false
        ));
        runtime.child_pid = None;
        runtime.child_started_at = None;
        assert!(historical_runtime(
            &runtime,
            100,
            &BTreeMap::from([(100, 999)]),
            |_| false,
            false
        ));
        assert!(!historical_runtime(
            &runtime,
            100,
            &BTreeMap::from([(100, 999)]),
            |_| false,
            true
        ));
    }

    #[test]
    fn epoch_identity_from_before_current_boot_is_historical_even_with_reused_unknown_pid() {
        let mut runtime = SessionRuntimeState::new(AgentStatus::Stopped);
        runtime.child_started_at = Some(20);
        assert!(runtime_from_previous_boot(&runtime, 100, true));
        assert!(
            !runtime_from_previous_boot(&runtime, 100, false),
            "Linux start ticks are opaque tokens"
        );
        runtime.child_started_at = Some(120);
        assert!(!runtime_from_previous_boot(&runtime, 100, true));
        runtime.child_started_at = None;
        runtime.host_started_at = Some(20);
        assert!(runtime_from_previous_boot(&runtime, 100, true));
        assert!(!runtime_from_previous_boot(&runtime, 0, true));
    }

    #[test]
    fn legacy_bridge_companion_is_not_a_runtime_but_malformed_actual_runtime_stays_uncertain() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let namespace = sessions.join("runtime/100");
        std::fs::create_dir_all(&namespace).unwrap();
        std::fs::write(
            namespace.join("3ae813ff-3954-469f-a3ee-7c124ffc9863.bridge.json"),
            br#"{"bridges":[],"launch":null}"#,
        )
        .unwrap();
        let mut inventory = SessionInventory::default();
        let candidates = read_candidates_scope(None, &sessions, None, &mut inventory);
        assert!(candidates.is_empty());
        assert!(
            inventory.uncertainties.is_empty(),
            "legacy bridge receipt is outside the runtime census"
        );
        std::fs::write(namespace.join("actual.json"), b"invalid runtime").unwrap();
        read_candidates_scope(None, &sessions, None, &mut inventory);
        assert_eq!(
            inventory.uncertainties.len(),
            1,
            "unreadable actual runtime remains fail-closed"
        );
    }

    fn machine_fixture(
        sessions_dir: &Path,
        processes: &BTreeMap<u32, u64>,
        resources: &MachineProcessResources,
    ) -> MachineSessionInventory {
        let mut inventory = SessionInventory::default();
        let candidates = read_candidates_with_liveness(
            None,
            sessions_dir,
            None,
            &mut inventory,
            |host_pid, runtime| {
                processes.contains_key(&host_pid)
                    || runtime
                        .child_pid
                        .is_some_and(|pid| processes.contains_key(&pid))
            },
        );
        let attribution = candidate_attribution(&candidates);
        observe_candidates(
            candidates,
            processes,
            processes,
            |_| false,
            &mut inventory,
            &[],
        );
        machine_inventory(inventory, attribution, resources)
    }

    #[test]
    fn machine_inventory_counts_foreign_live_ptys_without_project_registration_scan() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        for (id, project, pid) in [("own", "own", 201), ("other", "other", 202)] {
            let project = temp.path().join(project);
            save_runtime(&sessions, &project, &project, id, Some((pid, 20)));
        }
        assert_eq!(
            machine_fixture(
                &sessions,
                &BTreeMap::from([(201, 20), (202, 20)]),
                &MachineProcessResources {
                    memory: BTreeMap::from([(201, 100), (202, 100)]),
                    scalar_identities: BTreeMap::from([(201, 20), (202, 20)]),
                    ..MachineProcessResources::default()
                }
            )
            .sessions
            .len(),
            2
        );
    }

    #[test]
    fn machine_roles_use_exact_pm_and_monitor_provenance_and_named_review_scalar() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let project = temp.path().join("project");
        let pm = gwt_core::paths::gwt_projects_dir().join("hash/pm/worktree");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&pm).unwrap();
        for (id, worktree, pid) in [
            ("pm", &pm, 201),
            ("review", &project, 202),
            ("manual", &project, 203),
        ] {
            save_runtime(&sessions, &project, worktree, id, Some((pid, 20)));
        }
        let mut review = Session::load(&sessions.join("review.toml")).unwrap();
        review.launch_route = gwt_agent::LaunchRoute::Manual;
        review.launch_args =
            vec![crate::issue_monitor::ISSUE_MONITOR_LAUNCH_PROVENANCE.to_string()];
        review.save(&sessions).unwrap();
        let inventory = machine_fixture(
            &sessions,
            &BTreeMap::from([(201, 20), (202, 20), (203, 20)]),
            &MachineProcessResources {
                memory: BTreeMap::from([(201, 10), (202, 20), (203, 30)]),
                reviews: BTreeSet::from([202]),
                scalar_identities: BTreeMap::from([(201, 20), (202, 20), (203, 20)]),
                ..MachineProcessResources::default()
            },
        );
        assert!(inventory.uncertainties.is_empty());
        let get = |id| {
            inventory
                .sessions
                .iter()
                .find(|row| row.session.session_id == id)
                .unwrap()
        };
        assert_eq!(get("pm").role, MachineSessionRole::Pm);
        assert_eq!(get("review").role, MachineSessionRole::Review);
        assert!(
            get("review").monitor_owned,
            "legacy Manual Monitor stamps use the canonical resolver"
        );
        assert_eq!(get("manual").role, MachineSessionRole::Implementation);
        assert!(!get("manual").monitor_owned);
        assert_eq!(get("review").resident_memory_bytes, Some(20));
        let wire = serde_json::to_string(&inventory).unwrap();
        assert!(!wire.contains("GWT_REVIEW_DISPATCH"));
        assert!(!wire.contains("environment"));
    }

    #[test]
    fn machine_inventory_deduplicates_ptys_and_diagnoses_unknown_rss_without_counting_dead_rows() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        for (id, child) in [
            ("live", (201, 20)),
            ("dead", (202, 21)),
            ("recycled", (203, 22)),
        ] {
            save_runtime(&sessions, temp.path(), temp.path(), id, Some(child));
        }
        let original = gwt_agent::runtime_state_path_for_pid(&sessions, 100, "live");
        let duplicate = gwt_agent::runtime_state_path_for_pid(&sessions, 101, "live");
        std::fs::create_dir_all(duplicate.parent().unwrap()).unwrap();
        std::fs::copy(original, duplicate).unwrap();
        let inventory = machine_fixture(
            &sessions,
            &BTreeMap::from([(201, 20), (203, 99)]),
            &MachineProcessResources {
                memory: BTreeMap::from([(201, 100)]),
                reviews: BTreeSet::from([201]),
                scalar_identities: BTreeMap::from([(201, 99)]),
                ..MachineProcessResources::default()
            },
        );
        assert_eq!(inventory.sessions.len(), 1);
        assert_eq!(inventory.sessions[0].session.session_id, "live");
        assert_eq!(inventory.sessions[0].resident_memory_bytes, None);
        assert_eq!(
            inventory.sessions[0].role,
            MachineSessionRole::Implementation,
            "a scalar observation from another PID incarnation must not mark a review"
        );
        assert_eq!(inventory.uncertainties.len(), 1);
        assert_eq!(
            inventory.uncertainties[0].reason,
            "child_resident_memory_unavailable"
        );
    }

    fn save_runtime(
        sessions_dir: &Path,
        project_root: &Path,
        worktree: &Path,
        id: &str,
        child: Option<(u32, u64)>,
    ) {
        let mut session = Session::new(worktree, "issue-4305", AgentId::Codex);
        session.id = id.to_string();
        session.project_state_root = Some(project_root.to_path_buf());
        session.linked_issue_number = Some(4305);
        session.launch_origin = SessionLaunchOrigin::AutomaticRestore;
        session.restore_source_session_id = Some("source".to_string());
        session.save(sessions_dir).expect("save Session");
        // A historical status is deliberately not the liveness authority.
        let mut runtime = SessionRuntimeState::new(AgentStatus::Stopped);
        runtime.host_started_at = Some(10);
        runtime.child_pid = child.map(|(pid, _)| pid);
        runtime.child_started_at = child.map(|(_, start)| start);
        runtime
            .save(&gwt_agent::runtime_state_path_for_pid(
                sessions_dir,
                100,
                id,
            ))
            .expect("save runtime");
    }

    #[test]
    fn counts_live_ptys_separately_with_worktree_totals_and_actual_start_times() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let sessions = temp.path().join("sessions");
        std::fs::create_dir_all(&project).expect("project");
        let missing = project.join("missing-worktree");
        for (id, worktree, child) in [
            ("first", &project, (201, 20)),
            ("second", &project, (202, 21)),
            ("missing", &missing, (203, 22)),
            ("reused", &project, (204, 23)),
            ("dead", &project, (205, 24)),
        ] {
            save_runtime(&sessions, &project, worktree, id, Some(child));
        }
        let mut launched = Session::load(&sessions.join("first.toml")).unwrap();
        launched.launch_args =
            vec!["$gwt-execute #4305\nPrivate launch instructions must not be exposed".to_string()];
        launched.save(&sessions).unwrap();
        let mut unlinked = Session::load(&sessions.join("second.toml")).unwrap();
        unlinked.linked_issue_number = None;
        unlinked.launch_args = vec![
            "--model".to_string(),
            "$gwt-private-token-do-not-publish".to_string(),
        ];
        unlinked.save(&sessions).unwrap();
        let mut bound = Session::load(&sessions.join("missing.toml")).unwrap();
        bound.execution_binding = Some(gwt_agent::SessionExecutionBinding {
            schema_version: 1,
            session_id: bound.id.clone(),
            repo_hash: "repo".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 4305,
            identity: gwt_agent::ExecutionBindingIdentity {
                binding_id: "binding".to_string(),
                generation_id: "generation".to_string(),
                ledger_head_hash: "head".to_string(),
            },
            capability_generation: 1,
        });
        bound.save(&sessions).unwrap();
        let duplicate = gwt_agent::runtime_state_path_for_pid(&sessions, 100, "first");
        let duplicate_target = gwt_agent::runtime_state_path_for_pid(&sessions, 101, "first");
        std::fs::create_dir_all(duplicate_target.parent().unwrap()).unwrap();
        std::fs::copy(duplicate, duplicate_target).unwrap();
        save_runtime(
            &sessions,
            &temp.path().join("foreign"),
            &project,
            "foreign",
            Some((206, 25)),
        );
        let processes = BTreeMap::from([
            (100, 10),
            (201, 20),
            (202, 21),
            (203, 22),
            (204, 99),
            (206, 25),
        ]);

        let observed = observe_with_processes(&project, &sessions, &processes, |_| false);

        assert_eq!(
            observed.sessions.len(),
            3,
            "count each live PTY, not distinct owners"
        );
        assert!(observed.uncertainties.is_empty());
        // Issue #4788: this inventory is serialized as active_sessions by
        // issue.monitor.status; an unbound linked window must be identifiable.
        let status_sessions = serde_json::to_value(&observed.sessions).unwrap();
        for row in status_sessions.as_array().unwrap() {
            assert_eq!(
                row["execution_binding_missing"],
                serde_json::json!(row["session_id"] == "first"),
                "only the linked unbound Session lacks producing authority: {row}"
            );
            assert_eq!(
                row["launch_entrypoint"],
                if row["session_id"] == "first" {
                    serde_json::json!("gwt-execute")
                } else {
                    serde_json::Value::Null
                },
                "active_sessions exposes only the known launch entrypoint: {row}"
            );
        }
        assert!(!status_sessions
            .to_string()
            .contains("Private launch instructions"));
        let first = observed
            .sessions
            .iter()
            .find(|row| row.session_id == "first")
            .unwrap();
        assert_eq!(first.started_at, "1970-01-01T00:00:20Z");
        assert_eq!(first.launch_origin, SessionLaunchOrigin::AutomaticRestore);
        assert_eq!(first.restore_source_session_id.as_deref(), Some("source"));
        let groups = observed.worktree_sessions();
        assert_eq!(
            groups
                .iter()
                .map(|group| group.session_count)
                .sum::<usize>(),
            3
        );
        assert_eq!(
            groups
                .iter()
                .find(|group| group.worktree_path == project)
                .unwrap()
                .session_count,
            2
        );
        assert!(
            !groups
                .iter()
                .find(|group| group.worktree_path == missing)
                .unwrap()
                .worktree_exists
        );
    }

    #[test]
    fn restarted_legacy_sidecars_without_host_proof_or_children_are_historical() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        for (index, status) in [
            AgentStatus::Running,
            AgentStatus::Idle,
            AgentStatus::Stopped,
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("old-{index}");
            save_runtime(&sessions, temp.path(), temp.path(), &id, None);
            let path = gwt_agent::runtime_state_path_for_pid(&sessions, 100, &id);
            let mut runtime = SessionRuntimeState::load(&path).unwrap();
            runtime.host_started_at = None;
            runtime.status = status; // Historical status is not proof.
            runtime.save(&path).unwrap();
        }
        save_runtime(&sessions, temp.path(), temp.path(), "live", Some((201, 20)));
        let mut observed = SessionInventory::default();
        let candidates = read_candidates_with_liveness(
            Some(temp.path()),
            &sessions,
            None,
            &mut observed,
            |_, _| true,
        );
        let processes = BTreeMap::from([(100, 99), (201, 20), (202, 0)]);
        observe_candidates(
            candidates,
            &processes,
            &processes,
            |_| false,
            &mut observed,
            &[LegacyProcessObservation {
                pid: 202,
                parent_pid: 100,
                parent_may_be_host: false,
                started_at: 0,
                session_id: String::new(),
                cwd: PathBuf::new(),
            }],
        );
        assert_eq!(observed.sessions.len(), 1);
        assert_eq!(observed.sessions[0].session_id, "live");
        assert!(
            observed.uncertainties.is_empty(),
            "namespace PID presence is not Host proof"
        );
    }

    #[test]
    fn present_legacy_child_with_unreadable_metadata_remains_uncertain() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        save_runtime(&sessions, temp.path(), temp.path(), "legacy", None);
        let path = gwt_agent::runtime_state_path_for_pid(&sessions, 100, "legacy");
        let mut runtime = SessionRuntimeState::load(&path).unwrap();
        runtime.host_started_at = None;
        runtime.save(&path).unwrap();
        let mut inventory = SessionInventory::default();
        let candidates = read_candidates_with_liveness(
            Some(temp.path()),
            &sessions,
            None,
            &mut inventory,
            |_, _| true,
        );
        let processes = BTreeMap::from([(100, 10), (201, 0)]);
        observe_candidates(
            candidates,
            &processes,
            &processes,
            |_| false,
            &mut inventory,
            &[LegacyProcessObservation {
                pid: 201,
                parent_pid: 100,
                parent_may_be_host: true,
                started_at: 0,
                session_id: String::new(),
                cwd: PathBuf::new(),
            }],
        );
        assert!(inventory.sessions.is_empty());
        assert_eq!(
            inventory.uncertainties.len(),
            1,
            "unreadable child is not absent"
        );
    }

    #[test]
    fn legacy_snapshot_presence_survives_unavailable_scalar_observation() {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
        let pid = sysinfo::Pid::from_u32(std::process::id());
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        let process = system.process(pid).expect("current process snapshot");
        let parent = process.parent().expect("current process parent").as_u32();
        let started_at = crate::process::snapshot_process_start_identity(pid.as_u32(), process);
        let identities = BTreeMap::from([(pid.as_u32(), started_at)]);
        let rows = legacy_process_observations(
            &system,
            &System::new(),
            &BTreeSet::from([parent]),
            &identities,
        );
        assert_eq!(
            rows.len(),
            1,
            "unreadable scalars must not erase a present child"
        );
        assert_eq!(
            (rows[0].pid, rows[0].parent_pid, rows[0].started_at),
            (pid.as_u32(), parent, started_at)
        );
        assert!(rows[0].session_id.is_empty());
        assert!(rows[0].cwd.as_os_str().is_empty());
    }

    #[test]
    fn legacy_live_host_without_child_identity_reports_uncertainty() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions = temp.path().join("sessions");
        save_runtime(&sessions, temp.path(), temp.path(), "legacy", None);
        let observed =
            observe_with_processes(temp.path(), &sessions, &BTreeMap::from([(100, 10)]), |_| {
                false
            });
        assert!(observed.sessions.is_empty());
        assert_eq!(
            observed.uncertainties.len(),
            1,
            "unknown must not become a confident zero"
        );
        assert_eq!(
            observed.uncertainties[0].session_id.as_deref(),
            Some("legacy")
        );
    }

    #[test]
    fn legacy_pty_uses_matching_host_session_and_cwd_without_counting_inherited_helpers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions = temp.path().join("sessions");
        save_runtime(&sessions, temp.path(), temp.path(), "legacy", None);
        let path = gwt_agent::runtime_state_path_for_pid(&sessions, 100, "legacy");
        let mut runtime = SessionRuntimeState::load(&path).unwrap();
        runtime.host_started_at = None;
        runtime.save(&path).unwrap();
        let mut session = Session::load(&sessions.join("legacy.toml")).unwrap();
        session.launch_origin = SessionLaunchOrigin::Unknown;
        session.restore_source_session_id = None;
        session.save(&sessions).unwrap();
        let foreign = temp.path().join("foreign-project");
        save_runtime(&sessions, &foreign, &foreign, "foreign", None);
        let legacy_processes = [
            (201, 100, "legacy", temp.path()),
            (202, 201, "legacy", temp.path()), // npm's agent child
            (203, 1, "legacy", temp.path()),   // detached helper/daemon
            (204, 100, "legacy", foreign.as_path()), // inherited id, other cwd
            (205, 100, "foreign", foreign.as_path()),
        ]
        .into_iter()
        .map(
            |(pid, parent_pid, session_id, cwd)| LegacyProcessObservation {
                pid,
                parent_pid,
                parent_may_be_host: true,
                started_at: 20,
                session_id: session_id.to_string(),
                cwd: cwd.to_path_buf(),
            },
        )
        .collect::<Vec<_>>();
        let mut processes = BTreeMap::from([
            (100, 10),
            (201, 20),
            (202, 20),
            (203, 20),
            (204, 20),
            (205, 20),
        ]);
        let observe = |processes: &BTreeMap<u32, u64>| {
            let mut inventory = SessionInventory::default();
            let candidates = read_candidates_with_liveness(
                Some(temp.path()),
                &sessions,
                None,
                &mut inventory,
                |_, _| true,
            );
            observe_candidates(
                candidates,
                processes,
                processes,
                |_| false,
                &mut inventory,
                &legacy_processes,
            );
            inventory
        };

        let observed = observe(&processes);
        assert_eq!(
            observed.sessions.len(),
            1,
            "one legacy PTY, not its inherited helper processes"
        );
        assert_eq!(observed.sessions[0].child_pid, 201);
        assert_eq!(observed.sessions[0].started_at, "1970-01-01T00:00:20Z");
        assert_eq!(
            observed.sessions[0].launch_origin,
            SessionLaunchOrigin::Unknown
        );
        assert!(observed.uncertainties.is_empty());

        runtime.host_started_at = Some(10);
        runtime.save(&path).unwrap();
        processes.insert(100, 99);
        assert!(
            observe(&processes).sessions.is_empty(),
            "a recycled Host PID cannot supply legacy PTY identity"
        );

        save_runtime(
            &sessions,
            temp.path(),
            temp.path(),
            "legacy",
            Some((206, 21)),
        );
        processes.insert(206, 21);
        let current = observe(&processes);
        assert_eq!(current.sessions.len(), 1);
        assert_eq!(
            current.sessions[0].child_pid, 206,
            "persisted PID/start proof takes precedence over legacy discovery"
        );
    }

    #[test]
    fn reused_session_id_counts_each_pty_without_attributing_new_origin_to_both() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions = temp.path().join("sessions");
        save_runtime(
            &sessions,
            temp.path(),
            temp.path(),
            "reused-session",
            Some((201, 20)),
        );
        let mut session = Session::load(&sessions.join("reused-session.toml")).unwrap();
        session.launch_args = vec!["$gwt-execute #4305".to_string()];
        session.save(&sessions).unwrap();
        let mut successor = SessionRuntimeState::new(AgentStatus::Running);
        successor.child_pid = Some(202);
        successor.child_started_at = Some(21);
        successor
            .save(&gwt_agent::runtime_state_path_for_pid(
                &sessions,
                101,
                "reused-session",
            ))
            .unwrap();

        let observed = observe_with_processes(
            temp.path(),
            &sessions,
            &BTreeMap::from([(201, 20), (202, 21)]),
            |_| false,
        );

        assert_eq!(observed.sessions.len(), 2);
        assert!(observed
            .sessions
            .iter()
            .all(|row| row.launch_origin == SessionLaunchOrigin::Unknown
                && row.restore_source_session_id.is_none()));
        let rows = serde_json::to_value(&observed.sessions).unwrap();
        assert!(rows
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["launch_entrypoint"].is_null()));
    }
}
