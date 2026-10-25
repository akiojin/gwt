//! Project-scoped, localized Agent identities (Issue #3824).

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::Path,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRoster {
    pub schema_version: u32,
    pub revision: u64,
    pub members: Vec<AgentMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMember {
    pub agent_member_id: String,
    pub name: String,
    pub enabled: bool,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_session_id: Option<String>,
    #[serde(default)]
    pub last_issue_number: Option<u64>,
    #[serde(default)]
    pub last_spec_number: Option<u64>,
    #[serde(default)]
    pub assignment: Option<AgentMemberAssignment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMemberAssignment {
    pub reservation_id: String,
    pub session_id: Option<String>,
    pub issue_number: Option<u64>,
    pub spec_number: Option<u64>,
    pub delivery_id: Option<String>,
    #[serde(default)]
    pub host_id: String,
    pub host_pid: u32,
    pub host_started_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentRosterAction {
    Read,
    Add,
    Rest { member_id: String },
    Resize { size: usize },
}

impl AgentRoster {
    pub fn enabled_count(&self) -> usize {
        self.members.iter().filter(|member| member.enabled).count()
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("Unsupported Agent roster schema".into());
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        if self.enabled_count() == 0 {
            return Err("At least one Agent member must remain enabled".into());
        }
        for member in &self.members {
            if member.agent_member_id.is_empty()
                || member.name.trim().is_empty()
                || !ids.insert(&member.agent_member_id)
                || !names.insert(member.name.to_lowercase())
            {
                return Err("Agent roster contains an ambiguous identity".into());
            }
        }
        Ok(())
    }

    fn add(&mut self, history: &Path) -> Result<(), String> {
        let known = self
            .members
            .iter()
            .map(|member| member.name.clone())
            .collect();
        let language = gwt_config::Settings::load()
            .map(|settings| settings.ai.effective_language())
            .unwrap_or("en");
        let name = allocate(history, language, &HashSet::new(), &known)?;
        self.members.push(AgentMember {
            agent_member_id: uuid::Uuid::new_v4().to_string(),
            name,
            enabled: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            origin_session_id: None,
            last_issue_number: None,
            last_spec_number: None,
            assignment: None,
        });
        Ok(())
    }

    /// Change future admission only. Existing assignments are retained.
    pub(crate) fn resize(&mut self, size: usize, history: &Path) -> Result<(), String> {
        if size == 0 {
            return Err("At least one Agent member must remain enabled".into());
        }
        while self.enabled_count() < size {
            if let Some(member) = self.members.iter_mut().find(|member| !member.enabled) {
                member.enabled = true;
            } else {
                self.add(history)?;
            }
        }
        while self.enabled_count() > size {
            // Prefer resting idle members, but never stop a busy member.
            let index = self
                .members
                .iter()
                .rposition(|member| member.enabled && member.assignment.is_none())
                .or_else(|| self.members.iter().rposition(|member| member.enabled))
                .expect("positive enabled count");
            self.members[index].enabled = false;
        }
        self.validate()
    }
}

fn history_for_prefs(path: &Path) -> Result<std::path::PathBuf, String> {
    path.parent()
        .and_then(Path::parent)
        .map(|project| project.join("agent-names.jsonl"))
        .ok_or_else(|| "Agent roster preferences have no project parent".into())
}

/// Normalize numerical configuration writes under the existing prefs lock.
pub(crate) fn synchronize_roster_capacity(
    prefs: &mut crate::IssueMonitorPrefs,
    prefs_path: &Path,
) -> Result<(), String> {
    if let Some(roster) = prefs.agent_roster.as_mut() {
        roster.validate()?;
        if roster.enabled_count() != prefs.max_active_agents {
            roster.resize(prefs.max_active_agents, &history_for_prefs(prefs_path)?)?;
            roster.revision = roster
                .revision
                .checked_add(1)
                .ok_or("Agent roster revision exhausted")?;
        }
    }
    Ok(())
}

fn ensure_roster(
    prefs: &mut crate::IssueMonitorPrefs,
    prefs_path: &Path,
    seed: &[AgentMember],
) -> Result<(), String> {
    if prefs.agent_roster.is_none() {
        // Legacy name history is a uniqueness ledger, never a list of members.
        let mut roster = AgentRoster {
            schema_version: 1,
            revision: 1,
            members: seed
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, member)| AgentMember {
                    enabled: index < prefs.max_active_agents.max(1),
                    ..member
                })
                .collect(),
        };
        roster.resize(
            prefs.max_active_agents.max(1),
            &history_for_prefs(prefs_path)?,
        )?;
        remember(
            &history_for_prefs(prefs_path)?,
            &roster
                .members
                .iter()
                .map(|member| member.name.clone())
                .collect(),
        )?;
        prefs.max_active_agents = roster.enabled_count();
        prefs.agent_roster = Some(roster);
    }
    prefs
        .agent_roster
        .as_ref()
        .expect("initialized roster")
        .validate()
}

fn initial_project_members(
    root: &Path,
    path: &Path,
    sessions_dir: &Path,
) -> Result<Vec<AgentMember>, String> {
    if crate::load_issue_monitor_prefs(path)
        .map_err(|error| error.to_string())?
        .agent_roster
        .is_some()
    {
        return Ok(Vec::new());
    }
    let sessions = sessions_dir;
    let inventory = crate::session_inventory::observe_machine_sessions(sessions);
    let mut members = Vec::new();
    let mut names = HashSet::new();
    for observed in inventory.sessions {
        if observed.role != crate::session_inventory::MachineSessionRole::Implementation
            || crate::issue_monitor_prefs_path_for_repo_path(&observed.project_root)
                != crate::issue_monitor_prefs_path_for_repo_path(root)
        {
            continue;
        }
        let session = gwt_agent::Session::load(
            &sessions.join(format!("{}.toml", observed.session.session_id)),
        )
        .map_err(|error| error.to_string())?;
        let Some(name) = session
            .agent_name
            .as_ref()
            .filter(|name| !name.trim().is_empty())
        else {
            continue;
        };
        if !names.insert(name.to_lowercase()) {
            return Err("Current Agent names are ambiguous; roster migration refused".into());
        }
        members.push(AgentMember {
            agent_member_id: session
                .agent_member_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: name.clone(),
            enabled: false,
            created_at: session.created_at.to_rfc3339(),
            origin_session_id: Some(session.id.clone()),
            last_issue_number: session.linked_issue_number,
            last_spec_number: session.linked_issue_number.and_then(|number| {
                crate::issue_monitor_worker::cached_agent_member_spec_owner(root, number)
            }),
            assignment: Some(AgentMemberAssignment {
                reservation_id: session.id.clone(),
                session_id: Some(session.id),
                issue_number: session.linked_issue_number,
                spec_number: None,
                delivery_id: None,
                host_id: crate::process::current_hostname(),
                host_pid: observed.session.host_pid,
                host_started_at: crate::process::host_process_start_time(observed.session.host_pid),
            }),
        });
    }
    Ok(members)
}

pub fn read_project_roster(root: &Path, sessions_dir: &Path) -> Result<AgentRoster, String> {
    mutate_project_roster(root, sessions_dir, AgentRosterAction::Read, None)
}

pub fn mutate_project_roster(
    root: &Path,
    sessions_dir: &Path,
    action: AgentRosterAction,
    expected_revision: Option<u64>,
) -> Result<AgentRoster, String> {
    let path = crate::issue_monitor_prefs_path_for_repo_path(root);
    let dead = dead_project_assignments(&path, sessions_dir)?;
    let seed = initial_project_members(root, &path, sessions_dir)?;
    let (_, roster) = crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
        ensure_roster(prefs, &path, &seed).map_err(std::io::Error::other)?;
        let roster = prefs.agent_roster.as_mut().expect("initialized roster");
        if expected_revision.is_some_and(|expected| expected != roster.revision) {
            return Err(std::io::Error::other(
                "Agent roster changed; reload before retrying",
            ));
        }
        let before = roster.clone();
        for member in &mut roster.members {
            if member
                .assignment
                .as_ref()
                .is_some_and(|assignment| dead.contains(assignment))
            {
                member.assignment = None;
            }
        }
        match action {
            AgentRosterAction::Read => {}
            AgentRosterAction::Add => roster
                .add(&history_for_prefs(&path).map_err(std::io::Error::other)?)
                .map_err(std::io::Error::other)?,
            AgentRosterAction::Resize { size } => roster
                .resize(
                    size,
                    &history_for_prefs(&path).map_err(std::io::Error::other)?,
                )
                .map_err(std::io::Error::other)?,
            AgentRosterAction::Rest { ref member_id } => {
                if roster.enabled_count() <= 1
                    && roster
                        .members
                        .iter()
                        .any(|member| member.enabled && member.agent_member_id == *member_id)
                {
                    return Err(std::io::Error::other("The last enabled Agent cannot rest"));
                }
                roster
                    .members
                    .iter_mut()
                    .find(|member| member.agent_member_id == *member_id)
                    .ok_or_else(|| std::io::Error::other("Agent member no longer exists"))?
                    .enabled = false;
            }
        }
        if *roster != before {
            roster.revision = roster
                .revision
                .checked_add(1)
                .ok_or_else(|| std::io::Error::other("Agent roster revision exhausted"))?;
        }
        prefs.max_active_agents = roster.enabled_count();
        Ok(roster.clone())
    })
    .map_err(|error| error.to_string())?;
    Ok(roster)
}

/// A reservation becomes reusable only after exact physical absence is known.
fn assignment_is_busy(assignment: &AgentMemberAssignment, sessions_dir: &Path) -> bool {
    if !assignment.host_id.is_empty() && assignment.host_id != crate::process::current_hostname() {
        return true;
    }
    if let Some(session_id) = &assignment.session_id {
        let sessions = sessions_dir;
        match gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml"))) {
            Ok(session) if session.id == *session_id => {
                let observed = crate::session_inventory::observe_session(&session, sessions);
                if !observed.sessions.is_empty() || !observed.uncertainties.is_empty() {
                    return true;
                }
                // Recoverable PTY errors deliberately retain the Session's last
                // hook status. An exact, exited child is stronger evidence than
                // that status, while an unpublished/preparing launch stays busy.
                let runtime_path = gwt_agent::runtime_state_path_for_pid(
                    sessions,
                    assignment.host_pid,
                    session_id,
                );
                if let Ok(runtime) = gwt_agent::SessionRuntimeState::load(&runtime_path) {
                    let exact_owner = assignment.host_started_at.is_some_and(|started| {
                        started > 0 && runtime.host_started_at == Some(started)
                    });
                    let exact_session = gwt_agent::SessionExecutionIdentity::from_session(&session)
                        .is_ok_and(|identity| runtime.execution_identity == identity);
                    let exited_child = runtime
                        .child_pid
                        .filter(|pid| *pid > 0)
                        .zip(runtime.child_started_at.filter(|started| *started > 0))
                        .is_some_and(|(pid, started)| {
                            !crate::process::exact_pty_process_tree_is_alive(pid, started)
                                && !crate::process::is_host_process_alive(pid)
                        });
                    let no_launch_in_progress = matches!(
                        std::fs::symlink_metadata(gwt_agent::active_launch_handshake_path(
                            sessions, session_id
                        )),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound
                    );
                    if exact_owner
                        && exact_session
                        && runtime.runtime_incarnation.is_some_and(|value| value > 0)
                        && exited_child
                        && no_launch_in_progress
                    {
                        return false;
                    }
                }
                // A Running Session without child proof can still be preparing.
                if !matches!(
                    session.status,
                    gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
                ) {
                    return crate::process::is_host_process_alive(assignment.host_pid)
                        && assignment.host_started_at.is_none_or(|started| {
                            crate::process::host_process_start_time(assignment.host_pid)
                                .is_none_or(|current| current == started)
                        });
                }
                false
            }
            _ => true,
        }
    } else {
        crate::process::is_host_process_alive(assignment.host_pid)
            && assignment.host_started_at.is_none_or(|started| {
                crate::process::host_process_start_time(assignment.host_pid)
                    .is_none_or(|current| current == started)
            })
    }
}

fn dead_project_assignments(
    path: &Path,
    sessions_dir: &Path,
) -> Result<Vec<AgentMemberAssignment>, String> {
    let prefs = crate::load_issue_monitor_prefs(path).map_err(|error| error.to_string())?;
    Ok(prefs
        .agent_roster
        .into_iter()
        .flat_map(|roster| roster.members)
        .filter_map(|member| member.assignment)
        .filter(|assignment| !assignment_is_busy(assignment, sessions_dir))
        .collect())
}

/// History supplies a preference, never evidence that a member is idle.
pub(crate) fn previous_spec_members(
    root: &Path,
    sessions_dir: &Path,
    requested: &std::collections::BTreeSet<u64>,
) -> std::collections::BTreeMap<u64, String> {
    let mut newest = std::collections::BTreeMap::new();
    if requested.is_empty() {
        return newest;
    }
    let Ok(sessions) = gwt_agent::session_ledger::load_sessions_shared(sessions_dir) else {
        return newest;
    };
    let normalized = |path: &Path| dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let project_root = normalized(root);
    let repo_hash = gwt_core::repo_hash::detect_repo_hash(root);
    let mut specs = std::collections::HashMap::new();
    let mut history = std::collections::BTreeMap::new();
    for session in sessions {
        let Some(member) = session.agent_member_id.as_ref().filter(|id| !id.is_empty()) else {
            continue;
        };
        let Some(issue) = session.linked_issue_number else {
            continue;
        };
        if session.review_dispatch
            || crate::pm_registry::is_canonical_pm_worktree(&session.worktree_path)
            || (!repo_hash
                .as_ref()
                .is_some_and(|hash| session.repo_hash.as_deref() == Some(hash.as_str()))
                && normalized(
                    session
                        .project_state_root
                        .as_deref()
                        .unwrap_or(&session.worktree_path),
                ) != project_root)
        {
            continue;
        }
        let owner = if requested.contains(&issue) {
            Some(issue)
        } else {
            *specs.entry(issue).or_insert_with(|| {
                crate::issue_monitor_worker::cached_agent_member_spec_owner(root, issue)
            })
        };
        if let Some(owner) = owner.filter(|owner| requested.contains(owner)) {
            let candidate = (session.created_at, session.id.clone(), member.clone());
            if history
                .get(&owner)
                .is_none_or(|existing| &candidate > existing)
            {
                history.insert(owner, candidate);
            }
        }
    }
    newest.extend(
        history
            .into_iter()
            .map(|(spec, (_, _, member))| (spec, member)),
    );
    newest
}

fn previous_spec_member(root: &Path, sessions_dir: &Path, spec_number: u64) -> Option<String> {
    previous_spec_members(
        root,
        sessions_dir,
        &std::collections::BTreeSet::from([spec_number]),
    )
    .remove(&spec_number)
}

/// Reserve one exact assignment; retries reuse the delivery's existing member.
pub fn reserve_project_member(
    root: &Path,
    sessions_dir: &Path,
    reservation_id: &str,
    issue_number: Option<u64>,
    spec_number: Option<u64>,
    delivery_id: Option<&str>,
    inherited_member_id: Option<&str>,
) -> Result<AgentMember, String> {
    let path = crate::issue_monitor_prefs_path_for_repo_path(root);
    let host = crate::process::current_hostname();
    let dead = dead_project_assignments(&path, sessions_dir)?;
    let seed = initial_project_members(root, &path, sessions_dir)?;
    let historical_member = if inherited_member_id.is_none() {
        spec_number.and_then(|spec| previous_spec_member(root, sessions_dir, spec))
    } else {
        None
    };
    let (_, member) = crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
        ensure_roster(prefs, &path, &seed).map_err(std::io::Error::other)?;
        let switch_exclusions: std::collections::BTreeMap<_, _> = prefs.agent_roster.as_ref().expect("initialized roster").members.iter().filter_map(|member| {
            prefs.agent_switch_admission_exclusion(issue_number, &member.agent_member_id,
                inherited_member_id == Some(member.agent_member_id.as_str()), &host).map(|reason| (member.agent_member_id.clone(), reason))
        }).collect();
        if let Some(reason) = inherited_member_id.and_then(|member| switch_exclusions.get(member)) {
            return Err(std::io::Error::other(reason.clone()));
        }
        let roster = prefs.agent_roster.as_mut().expect("initialized roster");
        let existing = roster.members.iter().position(|member| {
            member.assignment.as_ref().is_some_and(|assignment| {
                assignment.reservation_id == reservation_id
                    || delivery_id.is_some_and(|id| assignment.delivery_id.as_deref() == Some(id))
            })
        });
        if let Some(index) = existing {
            if let Some(reason) = switch_exclusions.get(&roster.members[index].agent_member_id) {
                return Err(std::io::Error::other(reason.clone()));
            }
            let assignment = roster.members[index]
                .assignment
                .as_mut()
                .expect("matched assignment");
            if assignment.issue_number != issue_number {
                return Err(std::io::Error::other(
                    "Agent reservation belongs to another Issue",
                ));
            }
            if dead.contains(assignment) {
                // A replay keeps its member but changes the reservation token,
                // fencing late failure/ACK callbacks from the old materializer.
                assignment.reservation_id = reservation_id.to_string();
                assignment.session_id = None;
                assignment.host_pid = std::process::id();
                assignment.host_id = crate::process::current_hostname();
                assignment.host_started_at =
                    crate::process::host_process_start_time(std::process::id());
                roster.revision = roster
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| std::io::Error::other("Agent roster revision exhausted"))?;
            } else if assignment.host_pid != std::process::id()
                || (!assignment.host_id.is_empty()
                    && assignment.host_id != crate::process::current_hostname())
            {
                return Err(std::io::Error::other(
                    "Agent member is reserved by another host",
                ));
            }
            let result = roster.members[index].clone();
            prefs.abandon_agent_switch_for_source_resume(issue_number, &result.agent_member_id, inherited_member_id == Some(result.agent_member_id.as_str()), &host);
            return Ok(result);
        }
        for member in &mut roster.members {
            if member
                .assignment
                .as_ref()
                .is_some_and(|assignment| dead.contains(assignment))
            {
                member.assignment = None;
            }
        }
        if inherited_member_id.is_none()
            && roster
                .members
                .iter()
                .filter(|member| member.assignment.is_some())
                .count()
                >= roster.enabled_count()
        {
            return Err(std::io::Error::other(
                "Agent roster capacity is occupied, including resting members",
            ));
        }
        let scheduled = if inherited_member_id.is_none() {
            if issue_number.is_some_and(|number| prefs.agent_issue_schedule.reservations.iter().any(|entry| entry.issue_number == number && entry.host_id != host)) {
                return Err(std::io::Error::other("Issue is scheduled on another host; read issue.monitor.schedule.read on its queue owner"));
            }
            delivery_id.and_then(|id| prefs.pending_launch_deliveries.iter().find(|entry| entry.delivery_id == id && Some(entry.issue_number) == issue_number).and_then(|entry| entry.agent_member_id.clone()))
                .or_else(|| prefs.agent_issue_schedule.reservations.iter().find(|entry| Some(entry.issue_number) == issue_number && entry.host_id == host && prefs.terminal_queues.get(&host).is_some_and(|queue| queue.entries.iter().any(|queued| queued.number == entry.issue_number))).map(|entry| entry.agent_member_id.clone()))
        } else { None };
        let pinned_elsewhere = |member_id: &str| switch_exclusions.contains_key(member_id) || prefs.pending_launch_deliveries.iter().any(|entry| entry.agent_member_id.as_deref() == Some(member_id) && Some(entry.issue_number) != issue_number);
        let index = if let Some(member_id) = inherited_member_id {
            roster.members.iter().position(|member| {
                member.agent_member_id == member_id && member.assignment.is_none()
            })
        } else if let Some(member_id) = scheduled.as_deref() {
            Some(roster.members.iter().position(|member| member.agent_member_id == member_id && member.enabled && member.assignment.is_none() && !pinned_elsewhere(&member.agent_member_id)).ok_or_else(|| std::io::Error::other(format!("Scheduled Agent {member_id} is not enabled and idle on this host; read issue.monitor.schedule.read and wait or reassign")))?)
        } else {
            roster
                .members
                .iter()
                .position(|member| {
                    member.enabled
                        && member.assignment.is_none()
                        && !pinned_elsewhere(&member.agent_member_id)
                        && historical_member.as_deref() == Some(member.agent_member_id.as_str())
                })
                .or_else(|| {
                    roster.members.iter().position(|member| {
                        member.enabled
                            && member.assignment.is_none()
                            && !pinned_elsewhere(&member.agent_member_id)
                            && spec_number.is_some()
                            && member.last_spec_number == spec_number
                    })
                })
                .or_else(|| {
                    roster
                        .members
                        .iter()
                        .position(|member| member.enabled && member.assignment.is_none() && !pinned_elsewhere(&member.agent_member_id))
                })
        }
        .ok_or_else(|| std::io::Error::other("No idle Agent member is available"))?;
        let member = &mut roster.members[index];
        member.assignment = Some(AgentMemberAssignment {
            reservation_id: reservation_id.to_string(),
            session_id: None,
            issue_number,
            spec_number,
            delivery_id: delivery_id.map(str::to_string),
            host_id: crate::process::current_hostname(),
            host_pid: std::process::id(),
            host_started_at: crate::process::host_process_start_time(std::process::id()),
        });
        member.last_issue_number = issue_number;
        member.last_spec_number = spec_number;
        let result = member.clone();
        roster.revision = roster
            .revision
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("Agent roster revision exhausted"))?;
        if let Some(number) = issue_number { prefs.agent_issue_schedule.consume(&host, number)?; }
        prefs.abandon_agent_switch_for_source_resume(issue_number, &result.agent_member_id, inherited_member_id == Some(result.agent_member_id.as_str()), &host);
        Ok(result)
    })
    .map_err(|error| error.to_string())?;
    Ok(member)
}

pub fn bind_project_member_session(
    root: &Path,
    member_id: &str,
    reservation_id: &str,
    session_id: &str,
) -> Result<(), String> {
    update_member_assignment(root, member_id, reservation_id, |assignment| {
        assignment.session_id = Some(session_id.to_string())
    })
}

/// Migrate only a proven current Session, never a historical name-only row.
pub fn adopt_project_member_session(
    root: &Path,
    sessions_dir: &Path,
    session: &mut gwt_agent::Session,
) -> Result<Option<String>, String> {
    if let Some(member_id) = &session.agent_member_id {
        return Ok(Some(member_id.clone()));
    }
    let Some(name) = session
        .agent_name
        .as_ref()
        .filter(|name| !name.trim().is_empty())
    else {
        return Ok(None);
    };
    let inventory = crate::session_inventory::observe_session(session, sessions_dir);
    let persisted = gwt_agent::Session::load(&sessions_dir.join(format!("{}.toml", session.id)))
        .is_ok_and(|saved| {
            serde_json::to_value(saved).ok() == serde_json::to_value(&*session).ok()
        });
    if inventory.sessions.is_empty() && !persisted {
        return Ok(None);
    }
    let path = crate::issue_monitor_prefs_path_for_repo_path(root);
    let candidate = AgentMember {
        agent_member_id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        enabled: false,
        created_at: session.created_at.to_rfc3339(),
        origin_session_id: Some(session.id.clone()),
        last_issue_number: session.linked_issue_number,
        last_spec_number: session.linked_issue_number.and_then(|number| {
            crate::issue_monitor_worker::cached_agent_member_spec_owner(root, number)
        }),
        assignment: inventory
            .sessions
            .first()
            .map(|observed| AgentMemberAssignment {
                reservation_id: session.id.clone(),
                session_id: Some(session.id.clone()),
                issue_number: session.linked_issue_number,
                spec_number: None,
                delivery_id: None,
                host_id: crate::process::current_hostname(),
                host_pid: observed.host_pid,
                host_started_at: crate::process::host_process_start_time(observed.host_pid),
            }),
    };
    let mut seed = initial_project_members(root, &path, sessions_dir)?;
    if !seed
        .iter()
        .any(|member| member.origin_session_id == candidate.origin_session_id)
    {
        // The exact Session being restored is also a current member candidate,
        // even when its previous process has already stopped.
        seed.push(candidate.clone());
    }
    let (_, member_id) = crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
        ensure_roster(prefs, &path, &seed).map_err(std::io::Error::other)?;
        let roster = prefs.agent_roster.as_mut().expect("initialized roster");
        if let Some(member) = roster.members.iter().find(|member| {
            member.origin_session_id.as_deref() == Some(session.id.as_str())
                && member.created_at == candidate.created_at
        }) {
            return Ok(member.agent_member_id.clone());
        }
        if roster
            .members
            .iter()
            .any(|member| member.name.eq_ignore_ascii_case(name))
        {
            return Err(std::io::Error::other(
                "Legacy Agent name is ambiguous; member identity cannot be inferred",
            ));
        }
        let member_id = candidate.agent_member_id.clone();
        // Existing identities are immutable. An excess restored member remains
        // retained without raising the enabled capacity.
        roster.members.push(candidate);
        remember(
            &history_for_prefs(&path).map_err(std::io::Error::other)?,
            &HashSet::from([name.clone()]),
        )
        .map_err(std::io::Error::other)?;
        roster.revision = roster
            .revision
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("Agent roster revision exhausted"))?;
        Ok(member_id)
    })
    .map_err(|error| error.to_string())?;
    let previous = session.clone();
    session.agent_member_id = Some(member_id.clone());
    if !session
        .save_if_unchanged(sessions_dir, &previous)
        .map_err(|error| error.to_string())?
    {
        return Err("Session changed during Agent member migration".into());
    }
    Ok(Some(member_id))
}

fn update_member_assignment(
    root: &Path,
    member_id: &str,
    reservation_id: &str,
    update: impl FnOnce(&mut AgentMemberAssignment),
) -> Result<(), String> {
    let path = crate::issue_monitor_prefs_path_for_repo_path(root);
    crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
        let roster = prefs
            .agent_roster
            .as_mut()
            .ok_or_else(|| std::io::Error::other("Agent roster is missing"))?;
        let assignment = roster
            .members
            .iter_mut()
            .find(|member| member.agent_member_id == member_id)
            .and_then(|member| member.assignment.as_mut())
            .filter(|assignment| assignment.reservation_id == reservation_id)
            .ok_or_else(|| std::io::Error::other("Agent member reservation changed"))?;
        update(assignment);
        roster.revision = roster
            .revision
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("Agent roster revision exhausted"))?;
        Ok(())
    })
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn release_project_member(
    root: &Path,
    sessions_dir: &Path,
    member_id: &str,
    reservation_id: &str,
) -> Result<(), String> {
    let path = crate::issue_monitor_prefs_path_for_repo_path(root);
    let observed = crate::load_issue_monitor_prefs(&path)
        .map_err(|error| error.to_string())?
        .agent_roster
        .and_then(|roster| {
            roster
                .members
                .into_iter()
                .find(|member| member.agent_member_id == member_id)
                .and_then(|member| member.assignment)
        })
        .filter(|assignment| assignment.reservation_id == reservation_id);
    let Some(observed) = observed else {
        return Ok(());
    };
    if (!observed.host_id.is_empty() && observed.host_id != crate::process::current_hostname())
        || observed.host_pid != std::process::id()
    {
        return Err("Only the exact reserving host may release an Agent member".into());
    }
    if let Some(session_id) = observed.session_id.as_deref() {
        let sessions = sessions_dir;
        let session = gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml")))
            .map_err(|error| format!("Cannot release an unobserved Agent Session: {error}"))?;
        let inventory = crate::session_inventory::observe_session(&session, sessions);
        if !inventory.sessions.is_empty() || !inventory.uncertainties.is_empty() {
            return Err("Agent member retains a live or uncertain Session".into());
        }
    }
    crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
        if let Some(roster) = prefs.agent_roster.as_mut() {
            if let Some(member) = roster.members.iter_mut().find(|member| {
                member.agent_member_id == member_id && member.assignment.as_ref() == Some(&observed)
            }) {
                member.assignment = None;
                roster.revision = roster
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| std::io::Error::other("Agent roster revision exhausted"))?;
            }
        }
        Ok(())
    })
    .map_err(|error| error.to_string())?;
    Ok(())
}

// Keep the catalog in the same language as the visible identity. Each catalog
// contains over 200 real given names, spread across initials, sounds and lengths.
const EN_NAMES: &str = "
Ada Alice Amelia Anna Audrey Arthur Asher Austin
Beatrice Bella Beth Blake Bonnie Brandon Brian Brooke
Caleb Camilla Cara Casey Charles Chloe Clara Cole
Daisy Daniel Daphne David Dean Delia Diana Dylan
Eden Edgar Edith Eleanor Elena Eli Emma Ethan
Faith Faye Felix Finn Fiona Flora Frances Frank
Gabriel Gail Gemma George Gideon Grace Grant Gwen
Hannah Harriet Harvey Hazel Hector Henry Holly Hugo
Ian Ida Imogen India Ingrid Irene Iris Isaac
Jack Jacob Jade James Jane Jasper Jonah Julia
Kate Keira Kelly Kelsey Kenneth Kevin Kira Kyle
Lara Laura Leo Liam Lily Logan Louis Lucy
Mabel Maeve Marcus Maria Maya Miles Molly Morgan
Nadia Naomi Nathan Nell Nicholas Nina Noah Nora
Olive Oliver Olivia Omar Opal Orla Oscar Owen
Paige Parker Patrick Paul Pearl Penny Peter Phoebe
Quentin Quinn Quincy Quintin Quinlan Quinton Queenie Quiana
Rachel Ralph Rebecca Reid Rhys Riley Rose Ruby
Sadie Sam Sarah Scott Seth Simon Sophie Stella
Tara Taylor Tessa Theo Thomas Tobias Toby Tristan
Una Ulysses Ulric Upton Ursula Uri Uriah Unity
Valerie Vanessa Vera Vernon Victor Victoria Vince Violet
Wade Walter Warren Wendy Wesley Whitney William Willow
Xander Xavier Xena Xenia Ximena Xiomara Xavion Xyla
Yale Yasmin Yara Yvette Yolanda York Yusuf Yvonne
Zachary Zara Zane Zelda Zena Zia Zoe Zora
";
const JA_NAMES: &str = "
アイ アオイ アカネ アキ アキラ アサヒ アスカ アズサ アツシ アヤ アユミ アラタ アリサ
イオリ イクミ イズミ イチカ イッセイ イツキ イブキ イヨ イロハ
ウタ ウミ ウララ ウキョウ ウメ ウノ
エイジ エイタ エツコ エナ エマ エミ エリ エリカ
オト オトハ オサム オウジ オウカ オウタ オリエ
カイ カエデ カオリ カズキ カズハ カナ カナタ カナメ カホ カリン
キイ キキ キコ キョウ キョウカ キョウヘイ キヨ キヨミ キララ
クミ クミコ クルミ クニオ クニコ クオン クララ
ケイ ケイコ ケイタ ケイゴ ケント ケン ケンジ
コウ コウタ コウキ コウヘイ ココ ココナ コト コトネ コハル コマチ
サエ サキ サクラ サチ サチコ サト サトシ サナ サヤ サラ
シオリ シズカ シノ シノブ シホ シュウ シュン ショウ ショウタ シン シンタロウ
スイ スズ スズカ スズネ スバル スミ スミレ スナオ
セイ セイジ セイナ セイラ セツ セナ セリ セリナ
ソウ ソウタ ソウマ ソノ ソノカ ソラ
タイガ タイチ タクミ タケシ タケル タツキ タツヤ タマキ
チアキ チカ チサ チトセ チナ チハル チヒロ
ツカサ ツキ ツバキ ツバサ ツムギ ツヨシ
テツ テツオ テツヤ テル テルミ テン テンマ
トウカ トウマ トキ トシオ トモ トモキ トモヤ トワ
ナオ ナオキ ナギ ナギサ ナツ ナツキ ナツミ ナナ ナナミ ナル ナルミ
ニイナ ニコ ニナ ニジ ニジカ ニシキ
ネネ ネイロ ネム ノア ノゾミ ノドカ ノノ ノリ ノリコ
ハジメ ハナ ハノン ハヤテ ハヤト ハル ハルカ ハルキ ハルト ハルナ
ヒカリ ヒカル ヒサコ ヒトミ ヒナ ヒナタ ヒビキ ヒマリ ヒロ ヒロキ
フウ フウカ フウタ フク フミ フミカ フミヤ フユ フユカ
ヘイジ ヘイタ ホノカ ホタル ホシ ホクト ホナミ ホマレ
マイ マオ マキ マコ マコト マサキ マサト マナ マナミ マホ マユ マリ
ミオ ミカ ミキ ミク ミサ ミズキ ミツキ ミドリ ミナ ミノリ ミホ ミヤビ
ムギ ムツキ ムツミ ムネオ メイ メグ メグミ メイカ
モエ モトキ モナ モミジ モモ モモカ モリオ
ヤエ ヤス ヤスコ ヤスヒロ ヤマト ヤヨイ
ユイ ユウ ユウカ ユウキ ユウタ ユウナ ユキ ユズ ユズキ ユナ ユメ ユリ
ヨウ ヨウコ ヨシ ヨシキ ヨシノ ヨリ ヨリコ
ライ ライト ライム ラン ランカ ララ
リオ リク リサ リツ リト リナ リホ リョウ リン リンカ
ルイ ルカ ルナ ルリ ルミ ルミコ
レイ レイカ レイコ レオ レナ レン レンカ
ロイ ロカ ロミ ロミナ ロン ワカ ワカナ ワコ ワタル
";

// Expand exhausted given-name catalogs into localized full names rather than
// recycling a historical name with a number. Additional family components can
// expand the catalog again if an entire composed catalog has also been used.
const EN_FAMILY_NAMES: &str = "
Adams Bell Clark Davis Evans Ford Green Hall Irving Jones King Lane Moore
Nash Owen Price Quinn Reed Stone Turner Underwood Vale Walker Xu Young Zane
";
const JA_FAMILY_NAMES: &str = "
アサノ イノウエ ウエダ エンドウ オガワ カトウ キムラ クドウ コバヤシ
サイトウ シミズ スズキ セキ ソノダ タカハシ チバ ツジ テラダ トミタ
ナカムラ ニシダ ノダ ハヤシ ヒラノ フクダ ホンダ マツダ ミウラ
ムラタ モリ ヤマダ ユアサ ヨシダ ワタナベ
";

fn initial(name: &str) -> Option<char> {
    name.chars().next().map(|ch| ch.to_ascii_lowercase())
}

pub fn ensure_available_initial(name: &str, active: &HashSet<String>) -> Result<(), String> {
    if active.iter().any(|other| initial(other) == initial(name)) {
        return Err(format!(
            "Agent name {name}'s initial is occupied; close the conflicting Agent window and retry"
        ));
    }
    Ok(())
}

fn choose(
    language: &str,
    used: &HashSet<String>,
    active: &HashSet<String>,
) -> Result<String, String> {
    let catalog = if language == "ja" { JA_NAMES } else { EN_NAMES };
    let mut candidates: Vec<String> = catalog
        .split_whitespace()
        .filter(|name| !active.iter().any(|other| initial(other) == initial(name)))
        .map(str::to_string)
        .collect();
    if candidates.is_empty() {
        return Err(
            "All Agent name initials are occupied; close an inactive Agent window and retry".into(),
        );
    }
    let (families, separator) = if language == "ja" {
        (JA_FAMILY_NAMES, "・")
    } else {
        (EN_FAMILY_NAMES, " ")
    };
    loop {
        let fresh: Vec<_> = candidates
            .iter()
            .filter(|name| !used.contains(&name.to_lowercase()))
            .collect();
        if !fresh.is_empty() {
            // The first UUID field is 32 random bits (no version/variant bits).
            // Rejection sampling avoids modulo bias for catalogs of any size.
            let count = fresh.len() as u32;
            let limit = u32::MAX - u32::MAX % count;
            let index = loop {
                let random = uuid::Uuid::new_v4().as_fields().0;
                if random < limit {
                    break (random % count) as usize;
                }
            };
            return Ok(fresh[index].clone());
        }
        candidates = candidates
            .iter()
            .flat_map(|name| {
                families
                    .split_whitespace()
                    .map(move |family| format!("{name}{separator}{family}"))
            })
            .collect();
    }
}

struct HistoryLock(std::fs::File);

impl Drop for HistoryLock {
    fn drop(&mut self) {
        // A fork can retain this open file description until exec. Closing only
        // the parent's descriptor would leave its completed operation locked.
        let _ = FileExt::unlock(&self.0);
    }
}

fn locked_history(path: &Path) -> Result<(HistoryLock, HashSet<String>), String> {
    let parent = path.parent().ok_or("Agent name history has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.try_lock_exclusive()
        .map_err(|error| format!("Agent name history is busy: {error}; retry assignment"))?;
    let file = HistoryLock(file);
    let mut used = HashSet::new();
    for line in BufReader::new(&file.0).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let name: String = serde_json::from_str(&line)
            .map_err(|error| format!("Agent name history is corrupt: {error}"))?;
        if name.trim().is_empty() {
            return Err("Agent name history contains an empty identity".into());
        }
        used.insert(name.to_lowercase());
    }
    Ok((file, used))
}

fn reserve(
    file: &mut std::fs::File,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<bool, String> {
    if !used.insert(name.to_lowercase()) {
        return Ok(false);
    }
    serde_json::to_writer(&mut *file, name).map_err(|error| error.to_string())?;
    file.write_all(b"\n").map_err(|error| error.to_string())?;
    Ok(true)
}

/// Seed pre-upgrade/restored names before a window can disappear.
pub fn remember(history_path: &Path, names: &HashSet<String>) -> Result<(), String> {
    if names.is_empty() {
        return Ok(());
    }
    let (mut file, mut used) = locked_history(history_path)?;
    let mut changed = false;
    for name in names {
        changed |= reserve(&mut file.0, &mut used, name)?;
    }
    if changed {
        file.0.sync_all().map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Reserve the name durably before publishing it. Busy/corrupt history fails
/// closed; callers may retry without inventing or reusing a visible identity.
pub fn allocate(
    history_path: &Path,
    language: &str,
    active: &HashSet<String>,
    known: &HashSet<String>,
) -> Result<String, String> {
    let (mut file, mut used) = locked_history(history_path)?;
    for name in known {
        reserve(&mut file.0, &mut used, name)?;
    }
    let name = choose(language, &used, active)?;
    reserve(&mut file.0, &mut used, &name)?;
    file.0.sync_all().map_err(|error| error.to_string())?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exited_physical_child_releases_an_idle_session_for_its_member_restart() {
        const TEST: &str = "agent_names::tests::an_exited_physical_child_releases_an_idle_session_for_its_member_restart";
        if std::env::var_os("GWT_ROSTER_CHILD_WAIT").is_some() {
            use std::io::Read;
            let _ = std::io::stdin().read_to_end(&mut Vec::new());
            return;
        }
        struct OwnedChild(std::process::Child);
        impl Drop for OwnedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let member =
            reserve_project_member(&root, &sessions, "physical-child", None, None, None, None)
                .unwrap();
        let mut session = gwt_agent::Session::new(&root, "main", gwt_agent::AgentId::Codex);
        session.agent_member_id = Some(member.agent_member_id.clone());
        session.agent_name = Some(member.name);
        session.update_status(gwt_agent::AgentStatus::Idle);
        session.save(&sessions).unwrap();
        bind_project_member_session(
            &root,
            &member.agent_member_id,
            "physical-child",
            &session.id,
        )
        .unwrap();
        let mut child = OwnedChild(
            gwt_core::process::resolved_command(gwt_core::process::ProcessPlanRequest::new(
                std::env::current_exe().unwrap(),
            ))
            .unwrap()
            .args(["--exact", TEST])
            .env("GWT_ROSTER_CHILD_WAIT", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
        );
        let mut runtime = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Idle);
        runtime.host_started_at = crate::process::host_process_start_time(std::process::id());
        runtime.child_pid = Some(child.0.id());
        runtime.child_started_at = crate::process::host_process_start_time(child.0.id());
        runtime.runtime_incarnation = Some(7);
        assert!(runtime.host_started_at.is_some() && runtime.child_started_at.is_some());
        let path = gwt_agent::runtime_state_path(&sessions, &session.id);
        runtime.save(&path).unwrap();
        assert!(reserve_project_member(
            &root,
            &sessions,
            "while-live",
            None,
            None,
            None,
            Some(&member.agent_member_id)
        )
        .is_err());
        drop(child.0.stdin.take());
        assert!(child.0.wait().unwrap().success());
        let assignment =
            crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(&root))
                .unwrap()
                .agent_roster
                .unwrap()
                .members
                .into_iter()
                .find(|entry| entry.agent_member_id == member.agent_member_id)
                .unwrap()
                .assignment
                .unwrap();
        runtime.runtime_incarnation = None;
        runtime.save(&path).unwrap();
        assert!(assignment_is_busy(&assignment, &sessions));
        runtime.runtime_incarnation = Some(7);
        runtime.save(&path).unwrap();
        let handshake = gwt_agent::active_launch_handshake_path(&sessions, &session.id);
        std::fs::create_dir_all(handshake.parent().unwrap()).unwrap();
        std::fs::write(&handshake, b"preparing or unreadable").unwrap();
        assert!(assignment_is_busy(&assignment, &sessions));
        std::fs::remove_file(handshake).unwrap();
        let restarted = reserve_project_member(
            &root,
            &sessions,
            "restart",
            None,
            None,
            None,
            Some(&member.agent_member_id),
        )
        .unwrap();
        assert_eq!(restarted.agent_member_id, member.agent_member_id);
        assert_eq!(
            gwt_agent::Session::load(&sessions.join(format!("{}.toml", session.id)))
                .unwrap()
                .status,
            gwt_agent::AgentStatus::Idle
        );
    }

    #[test]
    fn legacy_adoption_retries_the_exact_session_after_interrupted_save() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let project = temp.path().join("repo");
        std::fs::create_dir_all(&project).unwrap();
        read_project_roster(&project, &gwt_core::paths::gwt_sessions_dir()).unwrap();
        let mut original =
            gwt_agent::Session::new(&project, "work/legacy", gwt_agent::AgentId::Codex);
        original.agent_name = Some("Legacy named contributor".into());
        original.update_status(gwt_agent::AgentStatus::Stopped);
        original.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let mut first = original.clone();
        let id = adopt_project_member_session(
            &project,
            &gwt_core::paths::gwt_sessions_dir(),
            &mut first,
        )
        .unwrap()
        .unwrap();
        // Reproduce the durable half-commit: roster saved, Session CAS did not.
        original.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let recovered = adopt_project_member_session(
            &project,
            &gwt_core::paths::gwt_sessions_dir(),
            &mut original,
        )
        .unwrap()
        .unwrap();
        assert_eq!(recovered, id);
        let mut unrelated = original.clone();
        unrelated.id = uuid::Uuid::new_v4().to_string();
        unrelated.agent_member_id = None;
        unrelated
            .save(&gwt_core::paths::gwt_sessions_dir())
            .unwrap();
        assert!(
            adopt_project_member_session(
                &project,
                &gwt_core::paths::gwt_sessions_dir(),
                &mut unrelated
            )
            .is_err(),
            "a same-name different Session cannot inherit the member"
        );
    }

    #[test]
    fn scheduled_issue_uses_its_exact_member_after_preferences_reload() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let roster = mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Resize { size: 2 },
            None,
        )
        .unwrap();
        let target = &roster.members[1];
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        let mut value =
            serde_json::to_value(crate::load_issue_monitor_prefs(&path).unwrap()).unwrap();
        value["terminal_queues"] = serde_json::json!({crate::process::current_hostname(): {
            "entries": [{"number":42,"queued_at":"2026-10-25T00:00:00Z","queued_by":"test"}],
            "last_seen_at":null
        }});
        value["agent_issue_schedule"] = serde_json::json!({"revision":1,"reservations":[{
            "host_id":crate::process::current_hostname(),"agent_member_id":target.agent_member_id,
            "issue_number":42,"reason":"assigned_by_you"
        }]});
        let prefs = serde_json::from_value(value).unwrap();
        crate::save_issue_monitor_prefs(&path, &prefs).unwrap();
        let selected = reserve_project_member(
            &root,
            &sessions,
            "scheduled-launch",
            Some(42),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            selected.agent_member_id, target.agent_member_id,
            "saved reservation must select the scheduled member instead of the first idle member"
        );
        assert_eq!(selected.name, target.name);
    }

    #[test]
    fn restored_legacy_session_seeds_the_configured_member_without_an_extra_identity() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let project = temp.path().join("repo");
        std::fs::create_dir_all(&project).unwrap();
        let mut session =
            gwt_agent::Session::new(&project, "work/legacy", gwt_agent::AgentId::Codex);
        session.agent_name = Some("Ada".into());
        session.update_status(gwt_agent::AgentStatus::Stopped);
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let id = adopt_project_member_session(
            &project,
            &gwt_core::paths::gwt_sessions_dir(),
            &mut session,
        )
        .unwrap()
        .unwrap();
        let roster = read_project_roster(&project, &gwt_core::paths::gwt_sessions_dir()).unwrap();
        assert_eq!(roster.members.len(), 1);
        assert_eq!(roster.members[0].agent_member_id, id);
        assert_eq!(roster.members[0].name, "Ada");
        assert!(roster.members[0].enabled);
    }

    #[test]
    fn roster_identity_and_capacity_survive_process_restart() {
        const TEST: &str =
            "agent_names::tests::roster_identity_and_capacity_survive_process_restart";
        if let Ok(stage) = std::env::var("GWT_ROSTER_TEST_STAGE") {
            let root = std::path::PathBuf::from(std::env::var_os("GWT_ROSTER_TEST_ROOT").unwrap());
            let _home = gwt_core::test_support::ScopedGwtHome::set(root.join("home"));
            let project = root.join("repo");
            std::fs::create_dir_all(&project).unwrap();
            let sessions = gwt_core::paths::gwt_sessions_dir();
            if stage == "write" {
                mutate_project_roster(
                    &project,
                    &gwt_core::paths::gwt_sessions_dir(),
                    AgentRosterAction::Resize { size: 2 },
                    None,
                )
                .unwrap();
                let member = reserve_project_member(
                    &project,
                    &gwt_core::paths::gwt_sessions_dir(),
                    "restart-reservation",
                    Some(4777),
                    Some(4777),
                    None,
                    None,
                )
                .unwrap();
                let mut session =
                    gwt_agent::Session::new(&project, "work/restart", gwt_agent::AgentId::Codex);
                session.id = "restart-member".into();
                session.agent_name = Some(member.name);
                session.agent_member_id = Some(member.agent_member_id.clone());
                session.update_status(gwt_agent::AgentStatus::Stopped);
                session.save(&sessions).unwrap();
                bind_project_member_session(
                    &project,
                    &member.agent_member_id,
                    "restart-reservation",
                    &session.id,
                )
                .unwrap();
            } else {
                let roster =
                    read_project_roster(&project, &gwt_core::paths::gwt_sessions_dir()).unwrap();
                let session =
                    gwt_agent::Session::load(&sessions.join("restart-member.toml")).unwrap();
                let member = roster
                    .members
                    .iter()
                    .find(|member| {
                        Some(&member.agent_member_id) == session.agent_member_id.as_ref()
                    })
                    .unwrap();
                assert_eq!(Some(&member.name), session.agent_name.as_ref());
                assert_eq!(roster.enabled_count(), 2);
                let config = crate::session_launch::launch_config_from_persisted_session(&session);
                assert_eq!(
                    config.env_vars.get("GWT_AGENT_MEMBER_ID"),
                    session.agent_member_id.as_ref()
                );
            }
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let run = |stage: &str| {
            let output = gwt_core::process::resolved_command(
                gwt_core::process::ProcessPlanRequest::new(std::env::current_exe().unwrap()),
            )
            .unwrap()
            .args(["--exact", TEST, "--nocapture"])
            .env("GWT_ROSTER_TEST_STAGE", stage)
            .env("GWT_ROSTER_TEST_ROOT", temp.path())
            .output()
            .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run("write");
        let path = crate::issue_monitor_prefs_path_for_repo_path(&temp.path().join("repo"));
        let before = crate::load_issue_monitor_prefs(&path)
            .unwrap()
            .agent_roster
            .unwrap();
        run("read");
        let after = crate::load_issue_monitor_prefs(&path)
            .unwrap()
            .agent_roster
            .unwrap();
        let identities = |roster: AgentRoster| {
            roster
                .members
                .into_iter()
                .map(|member| (member.agent_member_id, member.name, member.enabled))
                .collect::<Vec<_>>()
        };
        assert_eq!(identities(before), identities(after));
    }

    #[test]
    fn roster_transactions_reuse_members_fence_replays_and_preserve_busy_rest() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let first = read_project_roster(&root, &gwt_core::paths::gwt_sessions_dir()).unwrap();
        assert_eq!(first.enabled_count(), 1);
        let two = mutate_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            AgentRosterAction::Add,
            Some(first.revision),
        )
        .unwrap();
        assert_eq!(two.enabled_count(), 2);
        let selected = reserve_project_member(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            "one",
            Some(1),
            Some(9),
            Some("delivery"),
            None,
        )
        .unwrap();
        let replay = reserve_project_member(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            "different",
            Some(1),
            Some(9),
            Some("delivery"),
            None,
        )
        .unwrap();
        assert_eq!(
            selected, replay,
            "response loss keeps the exact reservation"
        );
        let other = reserve_project_member(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            "two",
            Some(2),
            None,
            None,
            None,
        )
        .unwrap();
        assert_ne!(selected.agent_member_id, other.agent_member_id);
        assert!(reserve_project_member(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            "three",
            Some(3),
            None,
            None,
            None
        )
        .is_err());
        let rested = mutate_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            AgentRosterAction::Rest {
                member_id: selected.agent_member_id.clone(),
            },
            None,
        )
        .unwrap();
        assert_eq!(rested.enabled_count(), 1);
        assert!(rested
            .members
            .iter()
            .find(|member| member.agent_member_id == selected.agent_member_id)
            .unwrap()
            .assignment
            .is_some());
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        assert_eq!(
            crate::load_issue_monitor_prefs(&path)
                .unwrap()
                .max_active_agents,
            1
        );
        let before = std::fs::read(&path).unwrap();
        assert!(mutate_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            AgentRosterAction::Rest {
                member_id: other.agent_member_id.clone()
            },
            None
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(mutate_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            AgentRosterAction::Add,
            Some(first.revision)
        )
        .is_err());
        release_project_member(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            &selected.agent_member_id,
            "stale-reservation",
        )
        .unwrap();
        assert!(
            read_project_roster(&root, &gwt_core::paths::gwt_sessions_dir())
                .unwrap()
                .members[0]
                .assignment
                .is_some()
        );
        release_project_member(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            &other.agent_member_id,
            "two",
        )
        .unwrap();
        assert!(
            reserve_project_member(
                &root,
                &gwt_core::paths::gwt_sessions_dir(),
                "too-soon",
                Some(3),
                None,
                None,
                None
            )
            .is_err(),
            "resting a busy member cannot admit above the reduced capacity"
        );
        release_project_member(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            &selected.agent_member_id,
            "one",
        )
        .unwrap();
        let restored = mutate_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            AgentRosterAction::Resize { size: 2 },
            None,
        )
        .unwrap();
        assert_eq!(restored.members[0].name, first.members[0].name);
        let preferred = reserve_project_member(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            "next",
            Some(4),
            Some(9),
            None,
            None,
        )
        .unwrap();
        assert_eq!(preferred.agent_member_id, selected.agent_member_id);
        release_project_member(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            &preferred.agent_member_id,
            "next",
        )
        .unwrap();

        // Another SPEC must not erase a member's earlier SPEC ownership.
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let held =
            reserve_project_member(&root, &sessions, "held-z", Some(90), Some(90), None, None)
                .unwrap();
        let x = reserve_project_member(&root, &sessions, "history-x", Some(9), Some(9), None, None)
            .unwrap();
        assert_ne!(x.agent_member_id, restored.members[0].agent_member_id);
        let mut x_session = gwt_agent::Session::new(&root, "work/x", gwt_agent::AgentId::Codex);
        x_session.project_state_root = Some(root.clone());
        x_session.agent_member_id = Some(x.agent_member_id.clone());
        x_session.agent_name = Some(x.name.clone());
        x_session.linked_issue_number = Some(9);
        x_session.update_status(gwt_agent::AgentStatus::Stopped);
        x_session.save(&sessions).unwrap();
        bind_project_member_session(&root, &x.agent_member_id, "history-x", &x_session.id).unwrap();
        release_project_member(&root, &sessions, &x.agent_member_id, "history-x").unwrap();
        let y = reserve_project_member(
            &root,
            &sessions,
            "history-y",
            Some(10),
            Some(10),
            None,
            None,
        )
        .unwrap();
        assert_eq!(y.agent_member_id, x.agent_member_id);
        release_project_member(&root, &sessions, &y.agent_member_id, "history-y").unwrap();
        release_project_member(&root, &sessions, &held.agent_member_id, "held-z").unwrap();
        let x_again = reserve_project_member(
            &root,
            &sessions,
            "history-x-again",
            Some(9),
            Some(9),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            x_again.agent_member_id, x.agent_member_id,
            "X -> Y -> idle -> X must prefer X's previous member over the first roster row"
        );
        release_project_member(
            &root,
            &sessions,
            &x_again.agent_member_id,
            "history-x-again",
        )
        .unwrap();
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            let member = prefs
                .agent_roster
                .as_mut()
                .unwrap()
                .members
                .iter_mut()
                .find(|member| member.agent_member_id == other.agent_member_id)
                .unwrap();
            member.assignment = Some(AgentMemberAssignment {
                reservation_id: "lost-host".into(),
                session_id: None,
                issue_number: Some(8),
                spec_number: None,
                delivery_id: Some("recover-delivery".into()),
                host_id: crate::process::current_hostname(),
                host_pid: i32::MAX as u32,
                host_started_at: None,
            });
            Ok(())
        })
        .unwrap();
        let recovered = reserve_project_member(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            "recovery",
            Some(8),
            None,
            Some("recover-delivery"),
            None,
        )
        .unwrap();
        assert_eq!(
            recovered.agent_member_id, other.agent_member_id,
            "a dead materializer's delivery retains its selected member"
        );
        let persisted = crate::load_issue_monitor_prefs(&path)
            .unwrap()
            .agent_roster
            .unwrap();
        assert_eq!(
            persisted,
            read_project_roster(&root, &gwt_core::paths::gwt_sessions_dir()).unwrap()
        );
        let another_root = temp.path().join("another");
        std::fs::create_dir_all(&another_root).unwrap();
        assert_ne!(
            read_project_roster(&another_root, &gwt_core::paths::gwt_sessions_dir())
                .unwrap()
                .members[0]
                .agent_member_id,
            selected.agent_member_id
        );
    }

    #[test]
    fn adding_after_rest_creates_a_new_member_without_reusing_retired_identity() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let roster = mutate_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            AgentRosterAction::Resize { size: 2 },
            None,
        )
        .unwrap();
        let retired = &roster.members[1].agent_member_id;
        mutate_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            AgentRosterAction::Rest {
                member_id: retired.clone(),
            },
            None,
        )
        .unwrap();
        let added = mutate_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            AgentRosterAction::Add,
            None,
        )
        .unwrap();
        assert_eq!(added.members.len(), 3);
        assert!(
            !added
                .members
                .iter()
                .find(|member| &member.agent_member_id == retired)
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn competing_reservations_cannot_allocate_the_same_member() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let _home = gwt_core::test_support::ScopedGwtHome::set(&home);
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let roster = read_project_roster(&root, &gwt_core::paths::gwt_sessions_dir()).unwrap();
        let gate = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = ["first", "second"]
                .into_iter()
                .map(|reservation| {
                    let (root, home, gate) = (&root, &home, &gate);
                    scope.spawn(move || {
                        let _home = gwt_core::test_support::ScopedGwtHome::set(home);
                        gate.wait();
                        reserve_project_member(
                            root,
                            &gwt_core::paths::gwt_sessions_dir(),
                            reservation,
                            None,
                            None,
                            None,
                            None,
                        )
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            read_project_roster(&root, &gwt_core::paths::gwt_sessions_dir())
                .unwrap()
                .members[0]
                .agent_member_id,
            roster.members[0].agent_member_id
        );
    }

    #[test]
    fn project_roster_survives_preferences_roundtrip() {
        let mut value = serde_json::to_value(crate::IssueMonitorPrefs::default()).unwrap();
        value["agent_roster"] = serde_json::json!({
            "schema_version": 1, "revision": 1,
            "members": [{"agent_member_id": "member-one", "name": "Ada", "enabled": true,
                "created_at": "2026-10-25T00:00:00Z"}]
        });
        let prefs: crate::IssueMonitorPrefs = serde_json::from_value(value).unwrap();
        let saved = serde_json::to_value(prefs).unwrap();
        assert_eq!(
            saved["agent_roster"]["members"][0]["agent_member_id"],
            "member-one"
        );
    }

    #[cfg(unix)]
    #[test]
    fn history_lock_releases_while_an_inherited_descriptor_remains_open() {
        let temp = tempfile::tempdir().unwrap();
        let history = temp.path().join("names.jsonl");
        let (file, _) = locked_history(&history).unwrap();
        // dup and fork share the open file description until the child execs.
        let inherited = file.0.try_clone().unwrap();
        drop(file);
        let allocation = allocate(&history, "en", &HashSet::new(), &HashSet::new());
        drop(inherited);
        assert!(allocation.is_ok(), "{allocation:?}");
    }

    #[test]
    fn localized_names_are_never_reused_and_initials_are_distributed() {
        for language in ["ja", "en"] {
            let catalog: Vec<_> = if language == "ja" { JA_NAMES } else { EN_NAMES }
                .split_whitespace()
                .collect();
            assert!(catalog.len() >= 200);
            assert_eq!(catalog.len(), catalog.iter().collect::<HashSet<_>>().len());
            let temp = tempfile::tempdir().unwrap();
            let history = temp.path().join("names.jsonl");
            let mut used = HashSet::new();
            let mut initials = std::collections::HashMap::new();
            for _ in 0..1000 {
                let name = allocate(&history, language, &HashSet::new(), &HashSet::new()).unwrap();
                assert!(
                    !name.chars().any(|ch| ch.is_ascii_digit()),
                    "numeric name: {name}"
                );
                if language == "ja" {
                    assert!(name.chars().next().unwrap() >= 'ァ', "{name}");
                } else {
                    assert!(name.starts_with(|c: char| c.is_ascii_uppercase()), "{name}");
                }
                *initials.entry(name.chars().next().unwrap()).or_insert(0) += 1;
                assert!(used.insert(name), "reused a historical identity");
            }
            assert!(*initials.values().max().unwrap() <= 150, "{initials:?}");
        }
    }

    #[test]
    fn exhausted_catalog_expands_without_reusing_legacy_names_or_active_initials() {
        for (language, catalog, active_name) in
            [("ja", JA_NAMES, "ミドリ-2"), ("en", EN_NAMES, "Flora-2")]
        {
            let temp = tempfile::tempdir().unwrap();
            let history = temp.path().join("names.jsonl");
            let mut known: HashSet<String> =
                catalog.split_whitespace().map(str::to_string).collect();
            known.insert(active_name.into());
            let active = HashSet::from([active_name.to_string()]);
            let name = allocate(&history, language, &active, &known).unwrap();
            assert!(!name.chars().any(|ch| ch.is_ascii_digit()), "{name}");
            assert!(!known.contains(&name));
            assert_ne!(initial(&name), initial(active_name));
            // A later window reopens the same history without the live window.
            assert_ne!(
                allocate(&history, language, &HashSet::new(), &known).unwrap(),
                name
            );
        }
    }

    #[test]
    fn active_initials_and_known_legacy_names_are_reserved() {
        let temp = tempfile::tempdir().unwrap();
        let history = temp.path().join("names.jsonl");
        let mut active = HashSet::new();
        let known = HashSet::from(["Unassigned".into()]);
        for _ in 0..10 {
            let name = allocate(&history, "en", &active, &known).unwrap();
            assert!(!known.contains(&name));
            assert!(!active
                .iter()
                .any(|old: &String| old.starts_with(name.chars().next().unwrap())));
            active.insert(name);
        }
        // Reopening the ledger after every allocation models separate launches.
        let name = allocate(&history, "unknown", &HashSet::new(), &HashSet::new()).unwrap();
        assert!(!active.contains(&name));
        assert!(!known.contains(&name));
    }

    #[test]
    fn corrupt_or_busy_history_refuses_without_reusing_an_identity() {
        use fs2::FileExt;
        let temp = tempfile::tempdir().unwrap();
        let history = temp.path().join("names.jsonl");
        std::fs::write(&history, "broken record\n").unwrap();
        assert!(allocate(&history, "en", &HashSet::new(), &HashSet::new()).is_err());
        std::fs::write(&history, "").unwrap();
        let lock = std::fs::File::open(&history).unwrap();
        lock.lock_exclusive().unwrap();
        assert!(allocate(&history, "en", &HashSet::new(), &HashSet::new()).is_err());
    }
}
