//! Read-only process observations, separate from Issue Monitor admission.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gwt_agent::{Session, SessionLaunchOrigin, SessionRuntimeState};
use serde::{Deserialize, Serialize};

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
    let resources = observe_live_candidates(candidates, &mut inventory, true);
    inventory.uncertainties.retain(|uncertainty| {
        if !uncertainty.reason.starts_with("session_unreadable:") {
            return true;
        }
        let Ok(runtime) = SessionRuntimeState::load(&uncertainty.runtime_path) else {
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
    machine_inventory(inventory, attribution, &resources)
}

#[derive(Default)]
struct MachineProcessResources {
    memory: BTreeMap<u32, u64>,
    reviews: BTreeSet<u32>,
    processes: BTreeMap<u32, u64>,
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
            let role = if crate::pm_registry::pane_is_pm(
                &project_root,
                Some(&session.worktree_path),
                Some(&session.session_id),
            ) {
                MachineSessionRole::Pm
            } else if resources.reviews.contains(&session.child_pid) {
                MachineSessionRole::Review
            } else {
                MachineSessionRole::Implementation
            };
            let resident_memory_bytes = resources
                .memory
                .get(&session.child_pid)
                .copied()
                .filter(|bytes| *bytes > 0);
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

/// Observe one Session before cross-Session process deduplication. Recovery
/// callers hold its Session lease and must also check every uncertainty.
pub(crate) fn observe_session(session: &Session, sessions_dir: &Path) -> SessionInventory {
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
    observe_live_candidates(candidates, &mut inventory, false);
    inventory
}

fn observe_live_candidates(
    candidates: Vec<RuntimeCandidate>,
    inventory: &mut SessionInventory,
    measure_resources: bool,
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
    // Read process state after the sidecars so a newly published child is not
    // compared with an OS snapshot taken before it was spawned.
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
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
    if !legacy_children.is_empty() {
        system.refresh_processes_specifics(
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
    // Retain only the Session id needed for attribution. The complete process
    // environment is never exposed in an observation or persisted anywhere.
    let legacy_processes = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let parent_pid = process.parent()?.as_u32();
            if !legacy_hosts.contains(&parent_pid) {
                return None;
            }
            let session_id = process
                .environ()
                .iter()
                .filter_map(|entry| entry.to_str())
                .find_map(|entry| entry.strip_prefix("GWT_SESSION_ID="))?;
            Some(LegacyProcessObservation {
                pid: pid.as_u32(),
                parent_pid,
                started_at: crate::process::snapshot_process_start_identity(pid.as_u32(), process),
                session_id: session_id.to_string(),
                cwd: process.cwd()?.to_path_buf(),
            })
        })
        .collect::<Vec<_>>();
    // Identity comes from the per-process start token, not the wall-clock
    // start time, which moves with the boot time on Linux (Issue #5089).
    // Only the PIDs a candidate can name are read.
    let uncertain_pids = inventory
        .uncertainties
        .iter()
        .filter(|row| row.reason.starts_with("session_unreadable:"))
        .filter_map(|row| SessionRuntimeState::load(&row.runtime_path).ok())
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
    let mut resources = MachineProcessResources::default();
    if measure_resources {
        // Only named PTY identities are queried for resource/role scalars.
        let pids = named
            .iter()
            .copied()
            .map(sysinfo::Pid::from_u32)
            .collect::<Vec<_>>();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing()
                .with_memory()
                .with_environ(UpdateKind::Always),
        );
        for (pid, process) in system
            .processes()
            .iter()
            .filter(|(pid, _)| named.contains(&pid.as_u32()))
        {
            resources.memory.insert(pid.as_u32(), process.memory());
            if process
                .environ()
                .iter()
                .any(|entry| entry == "GWT_REVIEW_DISPATCH=1")
            {
                resources.reviews.insert(pid.as_u32());
            }
        }
    }
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
    let boot_epoch = System::boot_time();
    let tokens_are_epoch = cfg!(not(target_os = "linux"));
    let candidates = candidates
        .into_iter()
        .filter(|candidate| {
            !runtime_from_previous_boot(&candidate.runtime, boot_epoch, tokens_are_epoch)
        })
        .collect();
    observe_candidates(
        candidates,
        &processes,
        &epochs,
        crate::process::is_process_group_alive,
        inventory,
        &legacy_processes,
    );
    if measure_resources {
        resources.processes = epochs;
        resources.processes.extend(processes);
        resources.legacy_processes = legacy_processes;
        resources.boot_epoch = boot_epoch;
        resources.tokens_are_epoch = tokens_are_epoch;
    }
    resources
}

#[cfg(test)]
fn observe_with_processes(
    project_root: &Path,
    sessions_dir: &Path,
    processes: &BTreeMap<u32, u64>,
    group_alive: impl Fn(u32) -> bool,
) -> SessionInventory {
    let mut inventory = SessionInventory::default();
    let candidates = read_candidates(project_root, sessions_dir, None, &mut inventory);
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

struct RuntimeCandidate {
    host_pid: u32,
    path: PathBuf,
    session: Session,
    runtime: SessionRuntimeState,
}

struct LegacyProcessObservation {
    pid: u32,
    parent_pid: u32,
    started_at: u64,
    session_id: String,
    cwd: PathBuf,
}

fn normalized(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
    let project_root = project_root.map(normalized);
    let repo_hash = project_root
        .as_deref()
        .and_then(gwt_core::repo_hash::detect_repo_identity)
        .map(|identity| identity.hash.as_str().to_string());
    let mut candidates = Vec::new();
    for namespace in read_paths(&sessions_dir.join("runtime"), inventory) {
        let Some(host_pid) = namespace
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse::<u32>().ok())
            .filter(|pid| *pid > 0)
        else {
            continue;
        };
        for path in read_paths(&namespace, inventory) {
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
            let session = match Session::load(&sessions_dir.join(format!("{id}.toml"))) {
                Ok(session) => session,
                Err(error) => {
                    inventory.uncertain(&path, Some(id), format!("session_unreadable: {error}"));
                    continue;
                }
            };
            let same_repo = repo_hash
                .as_ref()
                .is_some_and(|hash| session.repo_hash.as_ref() == Some(hash));
            let same_root = normalized(
                session
                    .project_state_root
                    .as_deref()
                    .unwrap_or(&session.worktree_path),
            );
            let same_root = project_root.as_ref().is_some_and(|root| same_root == *root);
            if project_root.is_some() && !same_repo && !same_root {
                continue;
            }
            match SessionRuntimeState::load(&path) {
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
    }
    candidates
}

fn read_paths(directory: &Path, inventory: &mut SessionInventory) -> Vec<PathBuf> {
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
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => inventory.uncertain(
                directory,
                None,
                format!("runtime_entry_unreadable: {error}"),
            ),
        }
    }
    paths.sort();
    paths
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
                        && normalized(&process.cwd) == worktree
                })
                .map(|process| (process.pid, process.started_at))
                .collect()
        };
        if children.is_empty() {
            let host_live = exact_host_identity(&runtime, processes.get(&host_pid).copied());
            if host_live
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
        let candidates = read_candidates_scope(None, sessions_dir, None, &mut inventory);
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
        let _lock = crate::env_test_lock().lock().unwrap();
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
            &MachineProcessResources::default(),
        );
        assert_eq!(inventory.sessions.len(), 1);
        assert_eq!(inventory.sessions[0].session.session_id, "live");
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
        let mut unlinked = Session::load(&sessions.join("second.toml")).unwrap();
        unlinked.linked_issue_number = None;
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
        }
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
            let candidates = read_candidates(temp.path(), &sessions, None, &mut inventory);
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
    }
}
