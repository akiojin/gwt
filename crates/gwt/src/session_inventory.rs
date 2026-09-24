//! Read-only process observations, separate from Issue Monitor admission.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gwt_agent::{Session, SessionLaunchOrigin, SessionRuntimeState};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SessionObservation {
    pub session_id: String,
    pub issue_number: Option<u64>,
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

#[derive(Debug, Clone, Serialize)]
pub struct SessionObservationUncertainty {
    pub runtime_path: PathBuf,
    pub session_id: Option<String>,
    pub reason: String,
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
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut inventory = SessionInventory::default();
    let candidates = read_candidates(project_root, sessions_dir, session_id, &mut inventory);
    if candidates.is_empty() {
        return inventory;
    }
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
        .collect::<BTreeSet<_>>();
    // Read process state after the sidecars so a newly published child is not
    // compared with an OS snapshot taken before it was spawned.
    let mut system = System::new();
    let refresh = if legacy_hosts.is_empty() {
        ProcessRefreshKind::nothing()
    } else {
        ProcessRefreshKind::nothing()
            .with_environ(UpdateKind::Always)
            .with_cwd(UpdateKind::Always)
    };
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    let processes = system
        .processes()
        .iter()
        .map(|(pid, process)| (pid.as_u32(), process.start_time()))
        .collect();
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
                started_at: process.start_time(),
                session_id: session_id.to_string(),
                cwd: process.cwd()?.to_path_buf(),
            })
        })
        .collect::<Vec<_>>();
    observe_candidates(
        candidates,
        &processes,
        crate::process::is_process_group_alive,
        &mut inventory,
        &legacy_processes,
    );
    inventory
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
    observe_candidates(candidates, processes, group_alive, &mut inventory, &[]);
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

fn read_candidates(
    project_root: &Path,
    sessions_dir: &Path,
    session_id: Option<&str>,
    inventory: &mut SessionInventory,
) -> Vec<RuntimeCandidate> {
    let project_root = normalized(project_root);
    let repo_hash = gwt_core::repo_hash::detect_repo_identity(&project_root)
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
            ) == project_root;
            if !same_repo && !same_root {
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
            let host_live = processes.get(&host_pid).is_some_and(|started| {
                runtime
                    .host_started_at
                    .is_none_or(|expected| expected == *started)
            });
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
            let Some(started_at) = i64::try_from(child_started_at)
                .ok()
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
