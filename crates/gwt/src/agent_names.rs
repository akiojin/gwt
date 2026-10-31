//! Persistent, localized Agent identities and exact project assignments.

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub name: String,
    pub enabled: bool,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_session_id: Option<String>,
    #[serde(default)]
    pub last_issue_number: Option<u64>,
    #[serde(default)]
    pub last_spec_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_owner_project_key: Option<String>,
    #[serde(default)]
    pub assignment: Option<AgentMemberAssignment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMemberAssignment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_key: Option<String>,
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
    Add {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Rest {
        member_id: String,
    },
    Rename {
        member_id: String,
        name: String,
    },
    Resize {
        size: usize,
    },
}

impl AgentRoster {
    pub fn enabled_count(&self) -> usize {
        self.members.iter().filter(|member| member.enabled).count()
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if !matches!(self.schema_version, 1 | 2) {
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
                || (self.schema_version == 1 && !names.insert(member.name.trim().to_lowercase()))
            {
                return Err("Agent roster contains an ambiguous identity".into());
            }
        }
        Ok(())
    }

    fn add(
        &mut self,
        history: &Path,
        agent_id: Option<String>,
        name: Option<&str>,
    ) -> Result<(), String> {
        let known = self
            .members
            .iter()
            .map(|member| member.name.clone())
            .collect();
        let language = gwt_config::Settings::load()
            .map(|settings| settings.ai.effective_language())
            .unwrap_or("en");
        let name = if let Some(name) = name {
            let name = name.trim();
            if name.is_empty() {
                return Err("Enter a display name".into());
            }
            if self
                .members
                .iter()
                .any(|member| member.name.trim().to_lowercase() == name.to_lowercase())
            {
                return Err("This name is already registered, including resting members".into());
            }
            remember(history, &HashSet::from([name.to_string()]))?;
            name.to_string()
        } else {
            allocate(history, language, &HashSet::new(), &known)?
        };
        self.members.push(AgentMember {
            agent_member_id: uuid::Uuid::new_v4().to_string(),
            agent_id,
            name,
            enabled: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            origin_session_id: None,
            last_issue_number: None,
            last_spec_number: None,
            last_owner_project_key: None,
            assignment: None,
        });
        Ok(())
    }

    /// Change future admission only. Existing assignments are retained.
    pub(crate) fn resize(
        &mut self,
        size: usize,
        history: &Path,
        agent_id: &str,
    ) -> Result<(), String> {
        if size == 0 {
            return Err("At least one Agent member must remain enabled".into());
        }
        while self.enabled_count() < size {
            if let Some(member) = self.members.iter_mut().find(|member| !member.enabled) {
                member.enabled = true;
            } else {
                self.add(history, Some(canonical_tool(agent_id)?), None)?;
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
    if crate::issue_monitor::machine_managed_monitor_prefs(path)
        && crate::agent_registry::is_active().map_err(|error| error.to_string())?
    {
        return Ok(crate::agent_registry::history_path());
    }
    path.parent()
        .and_then(Path::parent)
        .map(|project| project.join("agent-names.jsonl"))
        .ok_or_else(|| "Agent roster preferences have no project parent".into())
}

fn canonical_tool(value: &str) -> Result<String, String> {
    let tool = gwt_agent::resolve_agent_id(value).ok_or("Unknown Agent tool")?;
    Ok(tool.command().to_string())
}

/// Upgrade provider-only preferences under the existing project writer lock.
/// Current assignments keep their person and tool, including when their future
/// slot is rested to preserve the configured capacity for the saved primary.
fn synchronize_roster_tools(
    root: &Path,
    prefs: &mut crate::IssueMonitorPrefs,
    path: &Path,
    sessions_dir: &Path,
    requested_tool: Option<&str>,
) -> Result<(), String> {
    if prefs
        .agent_roster
        .as_ref()
        .is_some_and(|roster| roster.schema_version == 2)
    {
        let project = gwt_core::paths::project_scope_hash(root);
        for member in &prefs.agent_roster.as_ref().expect("shared roster").members {
            let Some(assignment) = member
                .assignment
                .as_ref()
                .filter(|assignment| assignment_belongs_to(root, assignment))
            else {
                continue;
            };
            let Some(id) = assignment.session_id.as_deref() else {
                continue;
            };
            let Some(session) = gwt_agent::Session::load(&sessions_dir.join(format!("{id}.toml")))
                .ok()
                .filter(|session| {
                    session.id == id
                        && gwt_core::paths::project_scope_hash(
                            session
                                .project_state_root
                                .as_deref()
                                .unwrap_or(&session.worktree_path),
                        ) == project
                })
            else {
                continue;
            };
            if let Some(fixed) = member.agent_id.as_deref() {
                let fixed = canonical_tool(fixed)?;
                let observed = session.agent_id.command();
                if fixed != observed {
                    return Err(format!(
                        "Agent {} in project {project} has fixed tool {fixed}, but its exact Session {id} records {observed}; close or restore its owning Session before retrying",
                        member.name
                    ));
                }
            }
        }
        return validate_registered_profiles(prefs, &prefs.launch_profile_pool());
    }
    let pool = prefs.launch_profile_pool();
    let tools: Vec<_> = pool
        .iter()
        .map(|profile| canonical_tool(&profile.agent_id))
        .collect::<Result<_, _>>()?;
    let Some(roster) = prefs.agent_roster.as_mut() else {
        return Ok(());
    };
    let before = roster.clone();
    let fallback = tools
        .first()
        .cloned()
        .or_else(|| requested_tool.map(str::to_string))
        .unwrap_or_else(|| "codex".into());
    for (index, member) in roster.members.iter_mut().enumerate() {
        let observed = member
            .assignment
            .as_ref()
            .and_then(|assignment| assignment.session_id.as_ref())
            .and_then(|id| {
                gwt_agent::Session::load(&sessions_dir.join(format!("{id}.toml")))
                    .ok()
                    .filter(|session| {
                        session.id == *id
                            && gwt_core::paths::project_scope_hash(
                                session
                                    .project_state_root
                                    .as_deref()
                                    .unwrap_or(&session.worktree_path),
                            ) == gwt_core::paths::project_scope_hash(root)
                    })
            })
            .map(|session| session.agent_id.command().to_string());
        if let Some(fixed) = member.agent_id.as_deref() {
            let fixed = canonical_tool(fixed)?;
            if observed.as_ref().is_some_and(|observed| observed != &fixed) {
                return Err(format!(
                    "Agent {} has a different live tool; waiting for its exact assignment",
                    member.name
                ));
            }
            member.agent_id = Some(fixed);
        } else {
            member.agent_id = Some(observed.unwrap_or_else(|| {
                tools
                    .get(index % tools.len().max(1))
                    .cloned()
                    .unwrap_or_else(|| fallback.clone())
            }));
        }
    }
    let mut migrated = Vec::new();
    let legacy = pool.iter().any(|profile| profile.agent_member_id.is_none());
    for (index, profile) in pool.iter().enumerate() {
        if profile.agent_member_id.is_some() {
            migrated.push(profile.clone());
            continue;
        }
        let tool = &tools[index];
        let mut matching: Vec<_> = roster
            .members
            .iter()
            .filter(|member| {
                member.agent_id.as_ref() == Some(tool)
                    && (member.enabled || member.assignment.is_some())
            })
            .map(|member| member.agent_member_id.clone())
            .collect();
        if matching.is_empty() {
            if let Some(member) = roster
                .members
                .iter_mut()
                .find(|member| member.agent_id.as_ref() == Some(tool))
            {
                matching.push(member.agent_member_id.clone());
            } else {
                roster.add(&history_for_prefs(path)?, Some(tool.clone()), None)?;
                let member = roster.members.last_mut().expect("added member");
                member.enabled = index == 0;
                matching.push(member.agent_member_id.clone());
            }
        }
        if index == 0
            && !roster
                .members
                .iter()
                .any(|member| matching.contains(&member.agent_member_id) && member.enabled)
        {
            roster
                .members
                .iter_mut()
                .find(|member| member.agent_member_id == matching[0])
                .expect("matching member")
                .enabled = true;
        }
        for id in matching {
            let mut bound = profile.clone();
            bound.agent_member_id = Some(id);
            bound.agent_id = tool.clone();
            migrated.push(bound);
        }
    }
    if legacy {
        while roster.enabled_count() > prefs.max_active_agents {
            let primary = migrated
                .first()
                .and_then(|profile| profile.agent_member_id.as_ref());
            let member = roster
                .members
                .iter_mut()
                .rev()
                .find(|member| member.enabled && Some(&member.agent_member_id) != primary)
                .ok_or("Cannot preserve Agent capacity during tool migration")?;
            member.enabled = false;
        }
    }
    if *roster != before {
        roster.revision = roster
            .revision
            .checked_add(1)
            .ok_or("Agent roster revision exhausted")?;
    }
    if legacy {
        prefs.set_launch_profile_pool(migrated);
    }
    validate_registered_profiles(prefs, &prefs.launch_profile_pool())
}

/// Provider is only a capability key; a bound candidate must name its owner.
pub fn validate_registered_profiles(
    prefs: &crate::IssueMonitorPrefs,
    profiles: &[crate::IssueMonitorLaunchProfile],
) -> Result<(), String> {
    for profile in profiles {
        let Some(id) = profile.agent_member_id.as_ref() else {
            continue;
        };
        let member = prefs
            .agent_roster
            .as_ref()
            .and_then(|roster| {
                roster
                    .members
                    .iter()
                    .find(|member| &member.agent_member_id == id)
            })
            .ok_or_else(|| format!("Registered Agent {id} is unavailable; reload Agents"))?;
        if member.agent_id.as_deref() != Some(canonical_tool(&profile.agent_id)?.as_str()) {
            return Err(format!(
                "Agent {} has a fixed tool; choose its registered profile",
                member.name
            ));
        }
    }
    Ok(())
}

/// Normalize numerical configuration writes under the existing prefs lock.
pub(crate) fn synchronize_roster_capacity(
    prefs: &mut crate::IssueMonitorPrefs,
    prefs_path: &Path,
) -> Result<(), String> {
    let default_tool = prefs
        .launch_profile_pool()
        .first()
        .map(|profile| profile.agent_id.clone())
        .unwrap_or_else(|| "codex".into());
    if let Some(roster) = prefs.agent_roster.as_mut() {
        roster.validate()?;
        if roster.schema_version == 2 {
            return validate_registered_profiles(prefs, &prefs.launch_profile_pool());
        }
        if roster.enabled_count() != prefs.max_active_agents {
            roster.resize(
                prefs.max_active_agents,
                &history_for_prefs(prefs_path)?,
                &default_tool,
            )?;
            roster.revision = roster
                .revision
                .checked_add(1)
                .ok_or("Agent roster revision exhausted")?;
        }
    }
    validate_registered_profiles(prefs, &prefs.launch_profile_pool())
}

/// Only explicit capacity controls may resize the shared roster. Cached state
/// and complete preference saves have no authority to change membership.
pub(crate) fn resize_shared_roster(
    prefs: &mut crate::IssueMonitorPrefs,
    size: usize,
) -> Result<(), String> {
    let tool = prefs
        .launch_profile_pool()
        .first()
        .map(|profile| profile.agent_id.clone())
        .unwrap_or_else(|| "codex".into());
    let Some(roster) = prefs
        .agent_roster
        .as_mut()
        .filter(|roster| roster.schema_version == 2)
    else {
        return Ok(());
    };
    let before = roster.clone();
    roster.resize(size, &crate::agent_registry::history_path(), &tool)?;
    if *roster != before {
        roster.revision = roster
            .revision
            .checked_add(1)
            .ok_or("Agent roster revision exhausted")?;
    }
    prefs.max_active_agents = roster.enabled_count();
    Ok(())
}

fn assignment_belongs_to(root: &Path, assignment: &AgentMemberAssignment) -> bool {
    assignment
        .project_key
        .as_deref()
        .is_none_or(|key| key == gwt_core::paths::project_scope_hash(root).as_str())
}

/// Resolve a retained Session through its exact source, without changing its bytes.
pub fn registered_session_member_id(
    session: &gwt_agent::Session,
) -> Result<Option<String>, String> {
    crate::agent_registry::SessionMemberSnapshot::load()
        .and_then(|snapshot| snapshot.resolve_session(session))
        .map_err(|error| error.to_string())
}

fn ensure_roster(
    root: &Path,
    prefs: &mut crate::IssueMonitorPrefs,
    prefs_path: &Path,
    seed: &[AgentMember],
    sessions_dir: &Path,
    requested_tool: Option<&str>,
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
        let default_tool = prefs
            .launch_profile_pool()
            .first()
            .map(|profile| profile.agent_id.clone())
            .or_else(|| requested_tool.map(str::to_string))
            .unwrap_or_else(|| "codex".into());
        roster.resize(
            prefs.max_active_agents.max(1),
            &history_for_prefs(prefs_path)?,
            &default_tool,
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
    synchronize_roster_tools(root, prefs, prefs_path, sessions_dir, requested_tool)?;
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
    Ok(observed_project_member_seeds(sessions_dir)?
        .remove(gwt_core::paths::project_scope_hash(root).as_str())
        .unwrap_or_default())
}

/// Exact currently observed implementation Sessions, before any global projection.
/// The caller acquires their Session leases before source preferences/registry locks.
pub(crate) fn observed_project_member_seeds(
    sessions_dir: &Path,
) -> Result<std::collections::BTreeMap<String, Vec<AgentMember>>, String> {
    let sessions = sessions_dir;
    let inventory = crate::session_inventory::observe_machine_sessions(sessions);
    let mut members: std::collections::BTreeMap<String, Vec<AgentMember>> = Default::default();
    let mut names: std::collections::BTreeMap<String, HashSet<String>> = Default::default();
    for observed in inventory.sessions {
        if observed.role != crate::session_inventory::MachineSessionRole::Implementation {
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
        let source = if let Some(key) = session.repo_hash.as_deref() {
            gwt_core::repo_hash::ProjectKey::parse(key).map_err(|error| error.to_string())?
        } else {
            gwt_core::paths::project_scope_hash(&observed.project_root)
        }
        .to_string();
        if !names
            .entry(source.clone())
            .or_default()
            .insert(name.to_lowercase())
        {
            return Err("Current Agent names are ambiguous; roster migration refused".into());
        }
        members
            .entry(source.clone())
            .or_default()
            .push(AgentMember {
                agent_member_id: session
                    .agent_member_id
                    .clone()
                    .unwrap_or_else(|| session.id.clone()),
                agent_id: Some(session.agent_id.command().to_string()),
                name: name.clone(),
                enabled: false,
                created_at: session.created_at.to_rfc3339(),
                origin_session_id: Some(session.id.clone()),
                last_issue_number: session.linked_issue_number,
                last_spec_number: session.linked_issue_number.and_then(|number| {
                    crate::issue_monitor_worker::cached_agent_member_spec_owner(
                        &observed.project_root,
                        number,
                    )
                }),
                last_owner_project_key: Some(source.clone()),
                assignment: Some(AgentMemberAssignment {
                    project_key: Some(source.clone()),
                    reservation_id: session.id.clone(),
                    session_id: Some(session.id),
                    issue_number: session.linked_issue_number,
                    spec_number: None,
                    delivery_id: None,
                    host_id: crate::process::current_hostname(),
                    host_pid: observed.session.host_pid,
                    host_started_at: crate::process::host_process_start_time(
                        observed.session.host_pid,
                    ),
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
    mutate_project_roster_snapshot(root, sessions_dir, action, expected_revision)
        .map(|prefs| prefs.agent_roster.expect("initialized roster"))
}

/// Read the roster and its saved manual candidates from the same locked commit.
pub fn mutate_project_roster_snapshot(
    root: &Path,
    sessions_dir: &Path,
    action: AgentRosterAction,
    expected_revision: Option<u64>,
) -> Result<crate::IssueMonitorPrefs, String> {
    if let AgentRosterAction::Add {
        agent_id: Some(id), ..
    } = &action
    {
        if let Some(gwt_agent::AgentId::Custom(id)) = gwt_agent::resolve_agent_id(id) {
            if crate::issue_monitor::custom_coding_agent_definition(&id).is_none() {
                return Err("Choose a known coding tool when registering an Agent".into());
            }
        }
        canonical_tool(id)?;
    }
    crate::agent_registry::ensure_active_for_project(root).map_err(|error| error.to_string())?;
    let path = crate::issue_monitor_prefs_path_for_repo_path(root);
    let seed = initial_project_members(root, &path, sessions_dir)?;
    let dead = dead_project_assignments(root, &path, sessions_dir)?;
    let (prefs, ()) = crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
        ensure_roster(root, prefs, &path, &seed, sessions_dir, None)
            .map_err(std::io::Error::other)?;
        let dead = unpinned_dead_assignments(prefs, &dead.assignments);
        let default_tool = prefs
            .launch_profile_pool()
            .first()
            .map(|profile| profile.agent_id.clone())
            .unwrap_or_else(|| "codex".into());
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
            AgentRosterAction::Add {
                ref agent_id,
                ref name,
            } => roster
                .add(
                    &history_for_prefs(&path).map_err(std::io::Error::other)?,
                    Some(
                        canonical_tool(agent_id.as_deref().unwrap_or(&default_tool))
                            .map_err(std::io::Error::other)?,
                    ),
                    name.as_deref(),
                )
                .map_err(std::io::Error::other)?,
            AgentRosterAction::Resize { size } => roster
                .resize(
                    size,
                    &history_for_prefs(&path).map_err(std::io::Error::other)?,
                    &default_tool,
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
            AgentRosterAction::Rename {
                ref member_id,
                ref name,
            } => {
                let index = roster
                    .members
                    .iter()
                    .position(|member| member.agent_member_id == *member_id)
                    .ok_or_else(|| std::io::Error::other("Agent member no longer exists"))?;
                let name = name.trim();
                if name.is_empty() {
                    return Err(std::io::Error::other("Enter a display name"));
                }
                if roster.members.iter().enumerate().any(|(other, member)| {
                    other != index && member.name.trim().to_lowercase() == name.to_lowercase()
                }) {
                    return Err(std::io::Error::other(
                        "This name is already registered, including resting members",
                    ));
                }
                if roster.members[index].name != name {
                    remember(
                        &history_for_prefs(&path).map_err(std::io::Error::other)?,
                        &HashSet::from([name.to_string()]),
                    )
                    .map_err(std::io::Error::other)?;
                    roster.members[index].name = name.to_string();
                }
            }
        }
        if *roster != before {
            roster.revision = roster
                .revision
                .checked_add(1)
                .ok_or_else(|| std::io::Error::other("Agent roster revision exhausted"))?;
        }
        prefs.max_active_agents = roster.enabled_count();
        Ok(())
    })
    .map_err(|error| error.to_string())?;
    drop(dead);
    Ok(prefs)
}

/// A reservation becomes reusable only after exact physical absence is known.
fn assignment_is_busy(
    root: &Path,
    assignment: &AgentMemberAssignment,
    sessions_dir: &Path,
) -> bool {
    if !assignment_belongs_to(root, assignment) {
        return true;
    }
    if !assignment.host_id.is_empty() && assignment.host_id != crate::process::current_hostname() {
        return true;
    }
    if let Some(session_id) = &assignment.session_id {
        let sessions = sessions_dir;
        match gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml"))) {
            Ok(session) if session.id == *session_id => {
                if gwt_core::paths::project_scope_hash(
                    session
                        .project_state_root
                        .as_deref()
                        .unwrap_or(&session.worktree_path),
                ) != gwt_core::paths::project_scope_hash(root)
                {
                    return true;
                }
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

struct DeadProjectAssignments {
    assignments: Vec<AgentMemberAssignment>,
    _leases: gwt_agent::SessionPathLeases,
}

fn path_is_absent(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Ok(_) => Ok(false),
        Err(error) => Err(error),
    }
}

// The caller keeps this Session's path lease through the prefs transaction.
fn missing_session_is_absent(sessions: &Path, id: &str) -> std::io::Result<bool> {
    if !path_is_absent(&sessions.join(format!("{id}.toml")))?
        || !path_is_absent(&gwt_agent::active_launch_handshake_path(sessions, id))?
    {
        return Ok(false);
    }
    let namespaces = match std::fs::read_dir(sessions.join("runtime")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error),
    };
    for namespace in namespaces {
        let namespace = namespace?;
        if namespace.file_type()?.is_dir()
            && !path_is_absent(&namespace.path().join(format!("{id}.json")))?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn dead_project_assignments(
    root: &Path,
    path: &Path,
    sessions_dir: &Path,
) -> Result<DeadProjectAssignments, String> {
    let prefs = crate::load_issue_monitor_prefs(path).map_err(|error| error.to_string())?;
    let assignments: Vec<_> = prefs
        .agent_roster
        .into_iter()
        .flat_map(|roster| roster.members)
        .filter_map(|member| member.assignment)
        .filter(|assignment| assignment_belongs_to(root, assignment))
        .collect();
    // Missing recovery is deferred inside an existing Session operation;
    // Session leases always precede the prefs writer lock.
    let host = crate::process::current_hostname();
    let local = |assignment: &AgentMemberAssignment| {
        assignment.host_id.is_empty() || assignment.host_id == host
    };
    let recover_missing = !gwt_agent::current_thread_holds_session_lease();
    let ids: std::collections::BTreeSet<_> = assignments
        .iter()
        .filter(|assignment| recover_missing && local(assignment))
        .filter_map(|assignment| assignment.session_id.as_ref())
        .filter(|id| path_is_absent(&sessions_dir.join(format!("{id}.toml"))).unwrap_or(false))
        .cloned()
        .collect();
    let leases = gwt_agent::acquire_session_path_leases(
        sessions_dir,
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .map_err(|error| error.to_string())?;
    let assignments = assignments
        .into_iter()
        .filter(|assignment| {
            !assignment_is_busy(root, assignment, sessions_dir)
                || local(assignment)
                    && assignment.session_id.as_ref().is_some_and(|id| {
                        ids.contains(id)
                            && missing_session_is_absent(sessions_dir, id).unwrap_or(false)
                    })
        })
        .collect();
    Ok(DeadProjectAssignments {
        assignments,
        _leases: leases,
    })
}

fn unpinned_dead_assignments(
    prefs: &crate::IssueMonitorPrefs,
    dead: &[AgentMemberAssignment],
) -> Vec<AgentMemberAssignment> {
    // A switch's source remains uncertain until its exact runtime exit is
    // confirmed. Recheck under the prefs writer lock: the pin can appear after
    // physical absence was observed without changing the assignment itself.
    dead.iter()
        .filter(|assignment| {
            !prefs.agent_switch_intents.values().any(|intent| {
                intent.has_admission_pin()
                    && !intent.runtime_exit_confirmed
                    && &intent.source.assignment == *assignment
            })
        })
        .cloned()
        .collect()
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
    let mut member_snapshot = None;
    let normalized = |path: &Path| dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let project_root = normalized(root);
    let repo_hash = gwt_core::repo_hash::detect_repo_hash(root);
    let mut specs = std::collections::HashMap::new();
    let mut history = std::collections::BTreeMap::new();
    for session in sessions {
        if session
            .agent_member_id
            .as_ref()
            .is_some_and(|id| id.is_empty())
        {
            continue;
        }
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
            let Ok(snapshot) = member_snapshot
                .get_or_insert_with(crate::agent_registry::SessionMemberSnapshot::load)
            else {
                continue;
            };
            let Ok(Some(member)) = snapshot.resolve_session(&session) else {
                continue;
            };
            let candidate = (session.created_at, session.id.clone(), member);
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
    reserve_project_member_inner(
        root,
        sessions_dir,
        reservation_id,
        issue_number,
        spec_number,
        delivery_id,
        inherited_member_id,
        None,
        None,
    )
}

/// The materialized tool and selected registered person must agree at reservation.
#[allow(clippy::too_many_arguments)]
pub fn reserve_project_member_for_agent(
    root: &Path,
    sessions_dir: &Path,
    reservation_id: &str,
    issue_number: Option<u64>,
    spec_number: Option<u64>,
    delivery_id: Option<&str>,
    inherited_member_id: Option<&str>,
    requested_tool: &str,
    selected_member_id: Option<&str>,
) -> Result<AgentMember, String> {
    reserve_project_member_inner(
        root,
        sessions_dir,
        reservation_id,
        issue_number,
        spec_number,
        delivery_id,
        inherited_member_id,
        Some(requested_tool),
        selected_member_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn reserve_project_member_inner(
    root: &Path,
    sessions_dir: &Path,
    reservation_id: &str,
    issue_number: Option<u64>,
    spec_number: Option<u64>,
    delivery_id: Option<&str>,
    inherited_member_id: Option<&str>,
    requested_tool: Option<&str>,
    selected_member_id: Option<&str>,
) -> Result<AgentMember, String> {
    crate::agent_registry::ensure_active_for_project(root).map_err(|error| error.to_string())?;
    let inherited = inherited_member_id
        .map(|id| crate::agent_registry::resolve_member_id(root, id))
        .transpose()
        .map_err(|error| error.to_string())?;
    let selected = selected_member_id
        .map(|id| crate::agent_registry::resolve_member_id(root, id))
        .transpose()
        .map_err(|error| error.to_string())?;
    let inherited_member_id = inherited.as_deref();
    let selected_member_id = selected.as_deref();
    let requested_tool = requested_tool.map(canonical_tool).transpose()?;
    let matches_tool = |member: &AgentMember| {
        requested_tool
            .as_ref()
            .is_none_or(|tool| member.agent_id.as_ref() == Some(tool))
            && selected_member_id.is_none_or(|id| member.agent_member_id == id)
    };
    let path = crate::issue_monitor_prefs_path_for_repo_path(root);
    let host = crate::process::current_hostname();
    let seed = initial_project_members(root, &path, sessions_dir)?;
    let historical_member = if inherited_member_id.is_none() {
        spec_number.and_then(|spec| previous_spec_member(root, sessions_dir, spec))
    } else {
        None
    };
    let dead = dead_project_assignments(root, &path, sessions_dir)?;
    let (_, member) = crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
        ensure_roster(root, prefs, &path, &seed, sessions_dir, requested_tool.as_deref()).map_err(std::io::Error::other)?;
        let dead = unpinned_dead_assignments(prefs, &dead.assignments);
        if selected_member_id.is_some_and(|id| !prefs.launch_profile_pool().iter().any(|profile| profile.agent_member_id.as_deref() == Some(id) && requested_tool.as_ref() == Some(&profile.agent_id))) {
            return Err(std::io::Error::other("Selected registered Agent profile changed; wait and reload before launching"));
        }
        let selected_member_id = selected_member_id.map(str::to_string).or_else(|| requested_tool.as_ref().and_then(|tool| prefs.launch_profile_pool().into_iter()
            .find(|profile| &profile.agent_id == tool && profile.agent_member_id.as_ref().is_some_and(|id| prefs.agent_roster.as_ref().is_some_and(|roster| roster.members.iter().any(|member| &member.agent_member_id == id && member.enabled && member.assignment.is_none()))))
            .or_else(|| prefs.launch_profile_pool().into_iter().find(|profile| &profile.agent_id == tool))
            .and_then(|profile| profile.agent_member_id)));
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
                assignment_belongs_to(root, assignment)
                    && (assignment.reservation_id == reservation_id
                    || delivery_id.is_some_and(|id| assignment.delivery_id.as_deref() == Some(id)))
            })
        });
        if let Some(index) = existing {
            if !matches_tool(&roster.members[index]) {
                return Err(std::io::Error::other("Exact Agent reservation has a different registered person or fixed tool; waiting instead of switching tools"));
            }
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
        } else { None }.or(selected_member_id);
        let pinned_elsewhere = |member_id: &str| switch_exclusions.contains_key(member_id) || prefs.pending_launch_deliveries.iter().any(|entry| entry.agent_member_id.as_deref() == Some(member_id) && Some(entry.issue_number) != issue_number);
        let index = if let Some(member_id) = inherited_member_id {
            roster.members.iter().position(|member| {
                member.agent_member_id == member_id && member.assignment.is_none()
            })
        } else if let Some(member_id) = scheduled.as_deref() {
            Some(roster.members.iter().position(|member| member.agent_member_id == member_id && member.enabled && matches_tool(member) && member.assignment.is_none() && !pinned_elsewhere(&member.agent_member_id)).ok_or_else(|| std::io::Error::other(format!("Scheduled Agent {member_id} is not enabled and idle on this host; read issue.monitor.schedule.read and wait or reassign")))?)
        } else {
            roster
                .members
                .iter()
                .position(|member| {
                    member.enabled
                        && matches_tool(member)
                        && member.assignment.is_none()
                        && !pinned_elsewhere(&member.agent_member_id)
                        && historical_member.as_deref() == Some(member.agent_member_id.as_str())
                })
                .or_else(|| {
                    roster.members.iter().position(|member| {
                        member.enabled
                            && matches_tool(member)
                            && member.assignment.is_none()
                            && !pinned_elsewhere(&member.agent_member_id)
                            && spec_number.is_some()
                            && member.last_spec_number == spec_number
                            && member.last_owner_project_key.as_deref().is_none_or(|key| {
                                key == gwt_core::paths::project_scope_hash(root).as_str()
                            })
                    })
                })
                .or_else(|| {
                    roster
                        .members
                        .iter()
                        .position(|member| member.enabled && matches_tool(member) && member.assignment.is_none() && !pinned_elsewhere(&member.agent_member_id))
                })
        }
        .ok_or_else(|| std::io::Error::other("No idle Agent member is available"))?;
        if !matches_tool(&roster.members[index]) {
            return Err(std::io::Error::other("Selected Agent has a different registered person or fixed tool; waiting for its exact registered profile"));
        }
        let member = &mut roster.members[index];
        member.assignment = Some(AgentMemberAssignment {
            project_key: Some(gwt_core::paths::project_scope_hash(root).to_string()),
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
        member.last_owner_project_key = Some(gwt_core::paths::project_scope_hash(root).to_string());
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
    drop(dead);
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
    let expected_project = gwt_core::paths::project_scope_hash(root);
    if session.repo_hash.as_deref() != Some(expected_project.as_str())
        && crate::issue_monitor_prefs_path_for_repo_path(
            session
                .project_state_root
                .as_deref()
                .unwrap_or(&session.worktree_path),
        ) != crate::issue_monitor_prefs_path_for_repo_path(root)
    {
        return Err(format!(
            "Agent Session belongs to another project: expected {expected_project}, recorded {:?}; restore from its own project",
            session.repo_hash
        ));
    }
    if session.agent_member_id.is_some() || registered_session_member_id(session)?.is_some() {
        let roster = read_project_roster(root, sessions_dir)?;
        let resolved = registered_session_member_id(session)?
            .ok_or("Registered Agent identity is unavailable")?;
        let member = roster
            .members
            .iter()
            .find(|member| member.agent_member_id == resolved)
            .ok_or("Registered Agent is missing")?;
        if member.agent_id.as_deref() != Some(session.agent_id.command()) {
            return Err("Exact restored Agent has a different fixed tool".into());
        }
        return Ok(Some(resolved));
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
        agent_id: Some(session.agent_id.command().to_string()),
        name: name.clone(),
        enabled: false,
        created_at: session.created_at.to_rfc3339(),
        origin_session_id: Some(session.id.clone()),
        last_issue_number: session.linked_issue_number,
        last_spec_number: session.linked_issue_number.and_then(|number| {
            crate::issue_monitor_worker::cached_agent_member_spec_owner(root, number)
        }),
        last_owner_project_key: Some(gwt_core::paths::project_scope_hash(root).to_string()),
        assignment: inventory
            .sessions
            .first()
            .map(|observed| AgentMemberAssignment {
                project_key: Some(gwt_core::paths::project_scope_hash(root).to_string()),
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
        ensure_roster(root, prefs, &path, &seed, sessions_dir, None)
            .map_err(std::io::Error::other)?;
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
    let member_id = crate::agent_registry::resolve_member_id(root, member_id)
        .map_err(|error| error.to_string())?;
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
        if !assignment_belongs_to(root, assignment) {
            return Err(std::io::Error::other(format!(
                "Agent reservation belongs to project {:?}, current project {}; bind from its owning project",
                assignment.project_key, gwt_core::paths::project_scope_hash(root)
            )));
        }
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
    let member_id = crate::agent_registry::resolve_member_id(root, member_id)
        .map_err(|error| error.to_string())?;
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
    if !assignment_belongs_to(root, &observed) {
        return Err(format!(
            "Agent reservation belongs to project {:?}, current project {}; release from its owning project",
            observed.project_key, gwt_core::paths::project_scope_hash(root)
        ));
    }
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

/// The physical owner may release only its exact reaped reservation while
/// holding the captured Session path lease, including an already removed Session.
pub(crate) fn release_reaped_project_member(
    root: &Path,
    sessions_dir: &Path,
    captured: &gwt_agent::Session,
    reservation_id: &str,
    pty: &gwt_terminal::PtyHandle,
) -> Result<(), String> {
    let Some(member_id) = captured.agent_member_id.as_deref() else {
        return Ok(());
    };
    if !pty.tree_exit_confirmed() || pty.process_id().is_none_or(|pid| pid == 0) {
        return Err("Agent reservation has no exact reaped project process".into());
    }
    // The real owner captured the Session before Git metadata could disappear.
    // Do not redirect its close into a new path-derived project store.
    let project_key = match captured.repo_hash.as_deref() {
        Some(key) => {
            gwt_core::repo_hash::ProjectKey::parse(key).map_err(|error| error.to_string())?
        }
        None if root.exists() => gwt_core::paths::project_scope_hash(root),
        None => return Ok(()),
    };
    if root.exists() && project_key != gwt_core::paths::project_scope_hash(root) {
        return Err("Agent reservation belongs to another project".into());
    }
    let prefs_path =
        gwt_core::paths::gwt_project_dir(&project_key).join("project-state/issue-monitor.json");
    let member_id = crate::agent_registry::resolve_member_id_in_source(&project_key, member_id)
        .map_err(|error| error.to_string())?;
    gwt_agent::with_session_path_lease(sessions_dir, &captured.id, |state| {
        match state {
            gwt_agent::SessionPathState::Missing => {
                if !missing_session_is_absent(sessions_dir, &captured.id)? {
                    return Err(std::io::Error::other(
                        "Agent reservation retains unconfirmed runtime evidence",
                    ));
                }
            }
            gwt_agent::SessionPathState::Present(current)
                if current.created_at == captured.created_at
                    && current.agent_member_id == captured.agent_member_id
                    && current.execution_binding == captured.execution_binding
                    && current.worktree_path == captured.worktree_path
                    && current.agent_id == captured.agent_id =>
            {
                let observed = crate::session_inventory::observe_session(&current, sessions_dir);
                if !observed.sessions.is_empty() || !observed.uncertainties.is_empty() {
                    return Err(std::io::Error::other(
                        "Agent reservation retains live or uncertain runtime evidence",
                    ));
                }
            }
            gwt_agent::SessionPathState::Present(_) => return Ok(()),
            gwt_agent::SessionPathState::Error(error) => return Err(error),
        }
        crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
            let Some(roster) = prefs.agent_roster.as_mut() else {
                return Ok(());
            };
            let Some(member) = roster.members.iter_mut().find(|member| {
                member.agent_member_id == member_id
                    && member.assignment.as_ref().is_some_and(|assignment| {
                        assignment
                            .project_key
                            .as_deref()
                            .is_none_or(|key| key == project_key.as_str())
                            && assignment.reservation_id == reservation_id
                            && assignment.session_id.as_deref() == Some(captured.id.as_str())
                            && assignment.host_id == crate::process::current_hostname()
                            && assignment.host_pid == std::process::id()
                            && assignment.host_started_at.is_some_and(|started| {
                                started > 0
                                    && Some(started)
                                        == crate::process::host_process_start_time(
                                            std::process::id(),
                                        )
                            })
                    })
            }) else {
                return Ok(());
            };
            if prefs.agent_switch_intents.values().any(|intent| {
                intent.has_admission_pin()
                    && !intent.runtime_exit_confirmed
                    && member.assignment.as_ref() == Some(&intent.source.assignment)
            }) {
                return Ok(());
            }
            member.assignment = None;
            roster.revision = roster
                .revision
                .checked_add(1)
                .ok_or_else(|| std::io::Error::other("Agent roster revision exhausted"))?;
            Ok(())
        })?;
        Ok(())
    })
    .map_err(|error| error.to_string())
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
    fn rename_preserves_member_and_session_facts_and_rejects_invalid_or_stale_drafts() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let original = read_project_roster(&root, &sessions).unwrap();
        let member =
            reserve_project_member(&root, &sessions, "rename", None, None, None, None).unwrap();
        let mut session = gwt_agent::Session::new(&root, "main", gwt_agent::AgentId::Codex);
        session.agent_member_id = Some(member.agent_member_id.clone());
        session.agent_name = Some(member.name.clone());
        session.save(&sessions).unwrap();
        bind_project_member_session(&root, &member.agent_member_id, "rename", &session.id).unwrap();
        let before = read_project_roster(&root, &sessions).unwrap();
        let session_path = sessions.join(format!("{}.toml", session.id));
        let session_bytes = std::fs::read(&session_path).unwrap();
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        let prefs_before = crate::load_issue_monitor_prefs(&prefs_path).unwrap();
        let rename = |name: &str, revision| {
            let action = serde_json::from_value(serde_json::json!({
                "kind": "rename", "member_id": member.agent_member_id, "name": name,
            }))
            .expect("registered members accept rename actions");
            mutate_project_roster(&root, &sessions, action, Some(revision))
        };
        let renamed = rename("  Maple  ", before.revision).unwrap();
        let mut expected = before.clone();
        expected.revision += 1;
        expected
            .members
            .iter_mut()
            .find(|m| m.agent_member_id == member.agent_member_id)
            .unwrap()
            .name = "Maple".into();
        assert_eq!(renamed, expected);
        assert_eq!(read_project_roster(&root, &sessions).unwrap(), renamed);
        assert_eq!(std::fs::read(&session_path).unwrap(), session_bytes);
        let prefs_after = crate::load_issue_monitor_prefs(&prefs_path).unwrap();
        assert_eq!(
            prefs_after.launch_profile_pool(),
            prefs_before.launch_profile_pool()
        );
        assert_eq!(
            prefs_after.max_active_agents,
            prefs_before.max_active_agents
        );
        assert_eq!(rename(" Maple ", renamed.revision).unwrap(), renamed);
        for (name, revision) in [
            ("", renamed.revision),
            ("  ", renamed.revision),
            ("Cedar", original.revision),
        ] {
            assert!(rename(name, revision).is_err());
            assert_eq!(read_project_roster(&root, &sessions).unwrap(), renamed);
        }
        let added = mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Add {
                agent_id: None,
                name: Some("Cedar".into()),
            },
            Some(renamed.revision),
        )
        .unwrap();
        let rested = mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Rest {
                member_id: added.members.last().unwrap().agent_member_id.clone(),
            },
            Some(added.revision),
        )
        .unwrap();
        assert!(rename(" CEDAR ", rested.revision).is_err());
        assert_eq!(read_project_roster(&root, &sessions).unwrap(), rested);
        assert_eq!(std::fs::read(session_path).unwrap(), session_bytes);
    }

    #[test]
    fn explicit_registration_preserves_name_identity_and_rejects_invalid_or_stale_writes() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let original = read_project_roster(&root, &sessions).unwrap();
        // Deserialize the actual wire shape, including legacy optional fields.
        let register = |name: &str, revision| {
            let action = serde_json::from_value(serde_json::json!({
                "kind": "add", "agent_id": "claude", "name": name,
            }))
            .unwrap();
            mutate_project_roster(&root, &sessions, action, Some(revision))
        };
        let added = register("  Maple  ", original.revision).unwrap();
        let member = added.members.last().unwrap();
        assert_eq!(member.name, "Maple");
        assert_eq!(member.agent_id.as_deref(), Some("claude"));
        assert!(uuid::Uuid::parse_str(&member.agent_member_id).is_ok());
        assert!(!original
            .members
            .iter()
            .any(|old| old.agent_member_id == member.agent_member_id));
        assert_eq!(read_project_roster(&root, &sessions).unwrap(), added);
        assert!(locked_history(
            &history_for_prefs(&crate::issue_monitor_prefs_path_for_repo_path(&root)).unwrap()
        )
        .unwrap()
        .1
        .contains("maple"));
        for name in ["", "  ", "maple", " MAPLE "] {
            assert!(
                register(name, added.revision).is_err(),
                "invalid name {name:?}"
            );
            assert_eq!(read_project_roster(&root, &sessions).unwrap(), added);
        }
        assert!(register("Cedar", original.revision).is_err());
        assert_eq!(read_project_roster(&root, &sessions).unwrap(), added);
        let rested = mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Rest {
                member_id: member.agent_member_id.clone(),
            },
            Some(added.revision),
        )
        .unwrap();
        assert!(
            register(" Maple ", rested.revision).is_err(),
            "resting keeps the registered name"
        );
        assert_eq!(read_project_roster(&root, &sessions).unwrap(), rested);
        let legacy = serde_json::from_value(serde_json::json!({"kind": "add"})).unwrap();
        let legacy =
            mutate_project_roster(&root, &sessions, legacy, Some(rested.revision)).unwrap();
        assert_ne!(legacy.members.last().unwrap().name.to_lowercase(), "maple");
    }

    #[test]
    fn issue_6413_missing_session_cannot_be_recreated_during_roster_commit() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Resize { size: 2 },
            None,
        )
        .unwrap();
        let mut removed = Vec::new();
        for token in ["removed-a", "removed-b"] {
            let member =
                reserve_project_member(&root, &sessions, token, None, None, None, None).unwrap();
            let session = gwt_agent::Session::new(&root, "main", gwt_agent::AgentId::Codex);
            session.save(&sessions).unwrap();
            bind_project_member_session(&root, &member.agent_member_id, token, &session.id)
                .unwrap();
            removed.push(session);
        }
        for session in &removed {
            std::fs::remove_file(sessions.join(format!("{}.toml", session.id))).unwrap();
        }
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        gwt_agent::with_session_path_lease(&sessions, "another-session", |_| {
            assert!(
                dead_project_assignments(&root, &path, &sessions)
                    .unwrap()
                    .assignments
                    .is_empty(),
                "nested Session operations defer Missing recovery"
            );
            Ok(())
        })
        .unwrap();
        let dead = dead_project_assignments(&root, &path, &sessions).unwrap();
        assert_eq!(dead.assignments.len(), 2);
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            for session in &removed {
                let concurrent = std::thread::scope(|scope| scope.spawn(|| {
                    gwt_agent::with_session_path_lease_wait(
                        &sessions, &session.id, std::time::Duration::ZERO, |_| Ok(()),
                    )
                }).join().unwrap());
                assert!(matches!(concurrent, Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock),
                    "Session genesis must stay fenced until the prefs transaction commits");
            }
            for member in &mut prefs.agent_roster.as_mut().unwrap().members {
                member.assignment = None;
            }
            Ok(())
        }).unwrap();
        drop(dead);
        for session in removed {
            session
                .save(&sessions)
                .expect("Session lease is released after the roster commit");
        }
    }

    #[test]
    fn issue_6413_new_switch_pin_fences_an_older_dead_assignment_observation() {
        let mut prefs = crate::IssueMonitorPrefs::default();
        let intent = serde_json::from_value(serde_json::json!({
            "request":{"operation_id":"switch","member_id":"member","expected_roster_revision":1,"target_issue_number":42},
            "source":{"assignment":{"reservation_id":"source","session_id":"removed-session",
                "issue_number":41,"spec_number":null,"delivery_id":null,"host_id":"host","host_pid":1,"host_started_at":2},
                "window_id":"window","claim_id":"claim",
                "confirmation":{"window_id":"window","claim_id":"claim","confirmed_at":"2026-10-26T00:00:00Z"}},
            "stage":"checkpoint_verified","runtime_exit_confirmed":false
        })).unwrap();
        prefs.agent_switch_intents.insert("switch".into(), intent);
        let observed_dead = vec![prefs.agent_switch_intents["switch"]
            .source
            .assignment
            .clone()];
        assert_eq!(
            unpinned_dead_assignments(&prefs, &observed_dead),
            observed_dead
        );

        // The switch can advance after absence is observed without changing
        // the assignment. The writer must consult this newer pin state.
        prefs.agent_switch_intents.get_mut("switch").unwrap().stage =
            crate::issue_monitor::AgentSwitchStage::StoppingCurrent;
        assert!(unpinned_dead_assignments(&prefs, &observed_dead).is_empty());
        prefs
            .agent_switch_intents
            .get_mut("switch")
            .unwrap()
            .runtime_exit_confirmed = true;
        assert_eq!(
            unpinned_dead_assignments(&prefs, &observed_dead),
            observed_dead
        );
    }

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
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        let registered = mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Add {
                agent_id: Some("claude".into()),
                name: Some("Next contributor".into()),
            },
            None,
        )
        .unwrap();
        let next_member = registered.members.last().unwrap().agent_member_id.clone();
        mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Rest {
                member_id: member.agent_member_id.clone(),
            },
            None,
        )
        .unwrap();
        crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
            prefs.set_launch_profile_pool(vec![serde_json::from_value(
                serde_json::json!({"agent_id":"claude","agent_member_id":next_member,"model":"sonnet"}),
            )
            .unwrap()]);
            Ok(())
        })
        .unwrap();
        let migrated = read_project_roster(&root, &sessions).unwrap();
        let active = migrated
            .members
            .iter()
            .find(|entry| entry.agent_member_id == member.agent_member_id)
            .unwrap();
        assert_eq!(active.agent_id.as_deref(), Some("codex"));
        assert_eq!(
            active.assignment.as_ref().unwrap().session_id.as_deref(),
            Some(session.id.as_str())
        );
        assert!(
            !active.enabled,
            "future slot transfers without stopping the old assignment"
        );
        assert_eq!(migrated.enabled_count(), 1);
        assert!(migrated
            .members
            .iter()
            .any(|member| member.enabled && member.agent_id.as_deref() == Some("claude")));
        assert!(reserve_project_member_for_agent(
            &root,
            &sessions,
            "primary-while-live",
            None,
            None,
            None,
            None,
            "claude",
            None
        )
        .unwrap_err()
        .contains("capacity is occupied"));
        assert_eq!(read_project_roster(&root, &sessions).unwrap(), migrated);
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
        assert!(assignment_is_busy(&root, &assignment, &sessions));
        runtime.runtime_incarnation = Some(7);
        runtime.save(&path).unwrap();
        let handshake = gwt_agent::active_launch_handshake_path(&sessions, &session.id);
        std::fs::create_dir_all(handshake.parent().unwrap()).unwrap();
        std::fs::write(&handshake, b"preparing or unreadable").unwrap();
        assert!(assignment_is_busy(&root, &assignment, &sessions));
        std::fs::remove_file(handshake).unwrap();
        let foreign = temp.path().join("foreign-project");
        std::fs::create_dir_all(&foreign).unwrap();
        session.project_state_root = Some(foreign);
        session.agent_id = gwt_agent::AgentId::ClaudeCode;
        session.save(&sessions).unwrap();
        let mut legacy = crate::load_issue_monitor_prefs(&prefs_path).unwrap();
        legacy.agent_roster.as_mut().unwrap().schema_version = 1;
        let index = legacy
            .agent_roster
            .as_ref()
            .unwrap()
            .members
            .iter()
            .position(|entry| entry.agent_member_id == member.agent_member_id)
            .unwrap();
        legacy.agent_roster.as_mut().unwrap().members[index].agent_id = None;
        let mut profile = legacy.launch_profile_pool()[0].clone();
        profile.agent_id = "codex".into();
        profile.agent_member_id = Some(member.agent_member_id.clone());
        legacy.set_launch_profile_pool(vec![profile]);
        synchronize_roster_tools(&root, &mut legacy, &prefs_path, &sessions, None).unwrap();
        let legacy_member = &legacy.agent_roster.as_ref().unwrap().members[index];
        assert_eq!(legacy_member.agent_id.as_deref(), Some("codex"));
        assert_eq!(legacy_member.assignment.as_ref(), Some(&assignment));
        assert!(
            read_project_roster(&root, &sessions)
                .unwrap()
                .members
                .iter()
                .find(|entry| entry.agent_member_id == member.agent_member_id)
                .unwrap()
                .assignment
                .is_some(),
            "a same-id Session belonging to another project must retain the assignment"
        );
        session.project_state_root = None;
        session.save(&sessions).unwrap();
        assert!(
            read_project_roster(&root, &sessions).is_err(),
            "a same-project Session with a different tool still refuses synchronization"
        );
        session.agent_id = gwt_agent::AgentId::Codex;
        session.save(&sessions).unwrap();
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
        assert_eq!(session.agent_member_id.as_ref(), Some(&id));
        assert_eq!(
            Some(&roster.members[0].agent_member_id),
            registered_session_member_id(&session).unwrap().as_ref()
        );
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
            AgentRosterAction::Add {
                agent_id: None,
                name: None,
            },
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
            AgentRosterAction::Add {
                agent_id: None,
                name: None
            },
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
                project_key: Some(gwt_core::paths::project_scope_hash(&root).to_string()),
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
        assert_eq!(
            read_project_roster(&another_root, &gwt_core::paths::gwt_sessions_dir())
                .unwrap()
                .members,
            persisted.members,
            "another project observes the same people and exact assignments"
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
        assert!(roster
            .members
            .iter()
            .all(|member| member.agent_id.as_deref() == Some("codex")));
        let saved =
            crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(&root))
                .unwrap();
        assert_eq!(saved.agent_roster.as_ref(), Some(&roster));
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
            AgentRosterAction::Add {
                agent_id: None,
                name: None,
            },
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
        let second_root = temp.path().join("other-repo");
        std::fs::create_dir_all(&second_root).unwrap();
        let second =
            read_project_roster(&second_root, &gwt_core::paths::gwt_sessions_dir()).unwrap();
        assert_eq!(
            roster.members, second.members,
            "both projects share one registered roster"
        );
        let gate = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = [(&root, "first"), (&second_root, "second")]
                .into_iter()
                .map(|(root, reservation)| {
                    let (home, gate) = (&home, &gate);
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
    fn common_registry_resolves_old_session_identity_without_rebinding_or_foreign_release() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path().join("home"));
        let root = temp.path().join("project-a");
        let other = temp.path().join("project-b");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let old_id = "44444444-4444-4444-8444-444444444444";
        let mut session = gwt_agent::Session::new(&root, "main", gwt_agent::AgentId::Codex);
        session.agent_member_id = Some(old_id.into());
        session.agent_name = Some("Maple".into());
        session.linked_issue_number = Some(42);
        session.save(&sessions).unwrap();
        let session_path = sessions.join(format!("{}.toml", session.id));
        let original_session = std::fs::read(&session_path).unwrap();
        let mut value = serde_json::to_value(crate::IssueMonitorPrefs::default()).unwrap();
        value["agent_roster"] = serde_json::json!({
            "schema_version":1,"revision":3,"members":[{
                "agent_member_id":old_id,"agent_id":"codex","name":"Maple","enabled":true,
                "created_at":"2026-10-01T00:00:00Z","assignment":{
                    "reservation_id":"legacy-reservation","session_id":session.id,
                    "issue_number":42,"spec_number":42,"delivery_id":null,
                    "host_id":"unobservable-imported-host","host_pid":77,"host_started_at":null
                }
            }]
        });
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        crate::save_issue_monitor_prefs(&path, &serde_json::from_value(value).unwrap()).unwrap();
        let roster = read_project_roster(&root, &sessions).unwrap();
        let global = &roster.members[0].agent_member_id;
        assert_ne!(
            global, old_id,
            "old source identity is retained through its explicit alias"
        );
        assert_eq!(
            adopt_project_member_session(&root, &sessions, &mut session)
                .unwrap()
                .as_ref(),
            Some(global)
        );
        assert_eq!(session.agent_member_id.as_deref(), Some(old_id));
        assert_eq!(std::fs::read(&session_path).unwrap(), original_session);
        assert_eq!(
            previous_spec_member(&root, &sessions, 42).as_ref(),
            Some(global)
        );
        let other_roster = read_project_roster(&other, &sessions).unwrap();
        assert!(other_roster
            .members
            .iter()
            .any(|member| { &member.agent_member_id == global && member.assignment.is_some() }));
        let other_path = crate::issue_monitor_prefs_path_for_repo_path(&other);
        let before = (
            std::fs::read(&path).unwrap(),
            std::fs::read(&other_path).unwrap(),
        );
        assert!(adopt_project_member_session(&other, &sessions, &mut session).is_err());
        assert!(release_project_member(&other, &sessions, global, "legacy-reservation").is_err());
        assert!(
            bind_project_member_session(&other, global, "legacy-reservation", &session.id).is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before.0);
        assert_eq!(std::fs::read(&other_path).unwrap(), before.1);
        assert_eq!(std::fs::read(&session_path).unwrap(), original_session);
    }

    fn with_rosterless_live_sources(
        check: impl FnOnce(&Path, &Path, &mut [(std::path::PathBuf, gwt_agent::Session, Vec<u8>)]),
    ) {
        struct OwnedChild(std::process::Child);
        impl Drop for OwnedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let first = temp.path().join("project-with-roster");
        std::fs::create_dir_all(&first).unwrap();
        let first_path = crate::issue_monitor_prefs_path_for_repo_path(&first);
        std::fs::create_dir_all(first_path.parent().unwrap()).unwrap();
        let mut initial = serde_json::to_value(crate::IssueMonitorPrefs::default()).unwrap();
        initial["agent_roster"] = serde_json::json!({"schema_version":1,"revision":1,
            "members":[{"agent_member_id":"44444444-4444-4444-8444-444444444444",
                "agent_id":"codex","name":"Maple","enabled":true,
                "created_at":"2026-10-01T00:00:00Z"}]});
        std::fs::write(&first_path, serde_json::to_vec(&initial).unwrap()).unwrap();
        let mut children = Vec::new();
        let mut retained = Vec::new();
        for (name, old_id) in [
            ("Cedar", Some("55555555-5555-4555-8555-555555555555")),
            ("Pine", None),
        ] {
            let root = temp.path().join(name);
            std::fs::create_dir_all(&root).unwrap();
            let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                serde_json::to_vec(&crate::IssueMonitorPrefs::default()).unwrap(),
            )
            .unwrap();
            let mut session = gwt_agent::Session::new(&root, "main", gwt_agent::AgentId::Codex);
            session.agent_member_id = old_id.map(str::to_string);
            session.agent_name = Some(name.into());
            session.linked_issue_number = Some(42);
            session.update_status(gwt_agent::AgentStatus::Idle);
            session.save(&sessions).unwrap();
            let child = OwnedChild(
                gwt_core::process::resolved_command(gwt_core::process::ProcessPlanRequest::new(std::env::current_exe().unwrap()))
                    .unwrap()
                    .args(["--exact", "agent_names::tests::an_exited_physical_child_releases_an_idle_session_for_its_member_restart"])
                    .env("GWT_ROSTER_CHILD_WAIT", "1")
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn().unwrap(),
            );
            let mut runtime = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Idle);
            runtime.host_started_at = crate::process::host_process_start_time(std::process::id());
            runtime.child_pid = Some(child.0.id());
            runtime.child_started_at = crate::process::host_process_start_time(child.0.id());
            runtime.runtime_incarnation = Some(7);
            assert!(runtime.host_started_at.is_some() && runtime.child_started_at.is_some());
            runtime
                .save(&gwt_agent::runtime_state_path(&sessions, &session.id))
                .unwrap();
            let bytes = std::fs::read(sessions.join(format!("{}.toml", session.id))).unwrap();
            children.push(child);
            retained.push((root, session, bytes));
        }
        let observed = observed_project_member_seeds(&sessions).unwrap();
        for (root, session, _) in &retained {
            assert!(
                observed[&gwt_core::paths::project_scope_hash(root).to_string()]
                    .iter()
                    .any(|member| member.origin_session_id.as_deref() == Some(session.id.as_str())),
                "the fixture has an independently observed live implementation Session"
            );
        }
        check(&first, &sessions, &mut retained);
        drop(children);
    }

    #[test]
    fn rosterless_live_members_are_seeded_before_another_project_activates() {
        with_rosterless_live_sources(|first, sessions, retained| {
            let roster = read_project_roster(first, sessions).unwrap();
            assert_eq!(
                roster.members.len(),
                3,
                "activating one project must retain rosterless live people in the others"
            );
            for (root, session, bytes) in retained.iter_mut() {
                let member = roster
                    .members
                    .iter()
                    .find(|member| member.origin_session_id.as_deref() == Some(session.id.as_str()))
                    .unwrap();
                assert_eq!(
                    member.assignment.as_ref().unwrap().session_id.as_deref(),
                    Some(session.id.as_str())
                );
                let original_id = session.agent_member_id.clone();
                assert_eq!(
                    registered_session_member_id(session).unwrap().as_ref(),
                    Some(&member.agent_member_id)
                );
                assert_eq!(
                    previous_spec_member(root, sessions, 42).as_ref(),
                    Some(&member.agent_member_id),
                    "proven legacy identities retain their source-qualified history preference"
                );
                assert_eq!(
                    adopt_project_member_session(root, sessions, session)
                        .unwrap()
                        .as_ref(),
                    Some(&member.agent_member_id)
                );
                let restored = crate::session_launch::launch_config_from_persisted_session(session);
                assert_eq!(
                restored.env_vars.get("GWT_AGENT_MEMBER_ID"),
                Some(&member.agent_member_id),
                "restoring a proven legacy Session carries its exact common person, including an absent old member ID"
            );
                assert_eq!(session.agent_member_id, original_id);
                assert_eq!(
                    std::fs::read(sessions.join(format!("{}.toml", session.id))).unwrap(),
                    *bytes
                );
            }
        });
    }

    #[test]
    fn prepared_refresh_keeps_rosterless_live_claims_and_every_observed_legacy_raw() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        with_rosterless_live_sources(|first, sessions, retained| {
            let mut paths: Vec<_> = std::iter::once(first.to_path_buf())
                .chain(retained.iter().map(|(root, _, _)| root.clone()))
                .map(|root| {
                    (
                        gwt_core::paths::project_scope_hash(&root).to_string(),
                        crate::issue_monitor_prefs_path_for_repo_path(&root),
                    )
                })
                .collect();
            paths.sort_by(|left, right| left.0.cmp(&right.0));
            let first_path = &paths[0].1;
            let (root, session, _) = retained
                .iter()
                .find(|(root, _, _)| {
                    gwt_core::paths::project_scope_hash(root).as_str() != paths[0].0
                })
                .unwrap();
            let key = gwt_core::paths::project_scope_hash(root).to_string();
            let path = crate::issue_monitor_prefs_path_for_repo_path(root);
            let original_raw = std::fs::read_to_string(&path).unwrap();
            let marker = first_path.with_extension("parent-sync-fail-once");
            std::fs::write(&marker, b"fail once").unwrap();
            let failure = gwt_core::test_support::ScopedEnvVar::set(
                "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
                first_path,
            );
            assert!(crate::agent_registry::ensure_active_for_project(first).is_err());
            drop(failure);
            let registry_path = crate::agent_registry::path();
            let prepared: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&registry_path).unwrap()).unwrap();
            assert_eq!(prepared["phase"], "prepared");
            let source_id = session.agent_member_id.as_deref().unwrap_or(&session.id);
            let global = prepared["sources"][&key]["aliases"][source_id]
                .as_str()
                .unwrap()
                .to_string();
            let original_member = prepared["roster"]["members"]
                .as_array()
                .unwrap()
                .iter()
                .find(|member| member["agent_member_id"] == global)
                .unwrap()
                .clone();
            let mut observed_raws = vec![original_raw.clone()];
            for step in [1, 2] {
                let mut drift: serde_json::Value = serde_json::from_str(&original_raw).unwrap();
                drift["fixture_refresh_step"] = serde_json::json!(step);
                let drift = serde_json::to_string_pretty(&drift).unwrap();
                std::fs::write(&path, &drift).unwrap();
                let status = crate::agent_registry::status(Some(&key)).unwrap();
                let observed = &status.projects[0];
                let request = crate::agent_registry::RegistryReconcileRequest {
                    expected_phase: crate::agent_registry::RegistryPhase::Prepared,
                    expected_registry_revision: status.registry_revision,
                    source_key: key.clone(),
                    expected_transaction_id: observed.transaction_id.clone(),
                    expected_source_sha256: observed.observed_sha256.clone(),
                    expected_floor_sha256: Some(observed.floor_observed_sha256.clone()),
                    action: crate::agent_registry::RegistryReconcileAction::RefreshPreparedSource,
                };
                // Keep the same real Prepared migration interrupted after its
                // updated intent commits, so a second observed drift is reachable.
                std::fs::write(&marker, b"fail once").unwrap();
                let failure = gwt_core::test_support::ScopedEnvVar::set(
                    "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
                    first_path,
                );
                assert!(crate::agent_registry::reconcile(&request).is_err());
                drop(failure);
                let bytes = std::fs::read_to_string(&registry_path).unwrap();
                let refreshed: serde_json::Value = serde_json::from_str(&bytes).unwrap();
                assert_eq!(refreshed["phase"], "prepared");
                let current = refreshed["roster"]["members"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|member| member["agent_member_id"] == global);
                assert!(
                    current.is_some(),
                    "refresh retains the currently observed live source person"
                );
                assert_eq!(
                    current.unwrap()["assignment"],
                    original_member["assignment"]
                );
                for raw in &observed_raws {
                    assert!(
                        bytes.contains(&serde_json::to_string(raw).unwrap()),
                        "every prior observed rosterless raw remains losslessly recorded"
                    );
                }
                observed_raws.push(drift);
            }
            let mut changed_shape: serde_json::Value =
                serde_json::from_str(observed_raws.last().unwrap()).unwrap();
            changed_shape["agent_roster"] = serde_json::json!({
                "schema_version":1,"revision":7,"members":[{
                    "agent_member_id":"66666666-6666-4666-8666-666666666666",
                    "agent_id":"codex","name":"Other prospective member","enabled":true,
                    "created_at":"2026-10-01T00:00:00Z"
                }]
            });
            let unknown = serde_json::to_string_pretty(&changed_shape).unwrap();
            std::fs::write(&path, &unknown).unwrap();
            let request_for_current = || {
                let status = crate::agent_registry::status(Some(&key)).unwrap();
                let current = &status.projects[0];
                crate::agent_registry::RegistryReconcileRequest {
                    expected_phase: crate::agent_registry::RegistryPhase::Prepared,
                    expected_registry_revision: status.registry_revision,
                    source_key: key.clone(),
                    expected_transaction_id: current.transaction_id.clone(),
                    expected_source_sha256: current.observed_sha256.clone(),
                    expected_floor_sha256: Some(current.floor_observed_sha256.clone()),
                    action: crate::agent_registry::RegistryReconcileAction::RefreshPreparedSource,
                }
            };
            let before = std::fs::read(&registry_path).unwrap();
            let imports_before = std::fs::read(crate::agent_registry_import::path()).ok();
            let floor_path = path.with_extension("committed.json");
            let floor_before = std::fs::read(&floor_path).ok();
            assert!(
                crate::agent_registry::reconcile(&request_for_current()).is_err(),
                "a new roster cannot drop a currently observed live person during refresh"
            );
            assert_eq!(std::fs::read(&registry_path).unwrap(), before);
            assert_eq!(
                std::fs::read(crate::agent_registry_import::path()).ok(),
                imports_before
            );
            assert_eq!(std::fs::read(&floor_path).ok(), floor_before);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), unknown);

            // A legitimate roster addition retaining the recorded person and
            // exact assignment must reuse its already allocated common UUID.
            let mut preserved = original_member.clone();
            preserved["agent_member_id"] = serde_json::json!(source_id);
            changed_shape["agent_roster"]["members"] = serde_json::json!([preserved]);
            std::fs::write(&path, serde_json::to_vec_pretty(&changed_shape).unwrap()).unwrap();
            assert_eq!(
                crate::agent_registry::reconcile(&request_for_current())
                    .unwrap()
                    .phase,
                crate::agent_registry::RegistryPhase::Active
            );
            let final_record = std::fs::read_to_string(&registry_path).unwrap();
            for raw in &observed_raws {
                assert!(final_record.contains(&serde_json::to_string(raw).unwrap()));
            }
            crate::agent_registry::ensure_active_for_project(first).unwrap();
            assert_eq!(
                read_project_roster(first, sessions).unwrap().members.len(),
                3
            );
            assert_eq!(
                registered_session_member_id(session).unwrap().as_deref(),
                Some(global.as_str())
            );
            for (_, session, original) in retained.iter() {
                assert_eq!(
                    std::fs::read(sessions.join(format!("{}.toml", session.id))).unwrap(),
                    *original
                );
            }
        });
    }

    #[test]
    fn project_roster_survives_preferences_roundtrip() {
        let mut value = serde_json::to_value(crate::IssueMonitorPrefs::default()).unwrap();
        value["agent_roster"] = serde_json::json!({
            "schema_version": 1, "revision": 1,
            "members": [{"agent_member_id": "member-one", "name": "Ada", "enabled": true,
                "agent_id": "codex",
                "created_at": "2026-10-25T00:00:00Z"}]
        });
        let prefs: crate::IssueMonitorPrefs = serde_json::from_value(value).unwrap();
        let saved = serde_json::to_value(prefs).unwrap();
        assert_eq!(
            saved["agent_roster"]["members"][0]["agent_member_id"],
            "member-one"
        );
        assert_eq!(saved["agent_roster"]["members"][0]["agent_id"], "codex");
    }

    #[test]
    fn legacy_roster_binds_the_saved_primary_tool_without_increasing_capacity() {
        let temp = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let root = temp.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        let prefs = crate::IssueMonitorPrefs {
            max_active_agents: 4,
            launch_profile: Some(
                serde_json::from_value(serde_json::json!({"agent_id":"claude", "model":"sonnet"}))
                    .unwrap(),
            ),
            ..Default::default()
        };
        crate::save_issue_monitor_prefs(&path, &prefs).unwrap();
        let roster = read_project_roster(&root, &gwt_core::paths::gwt_sessions_dir()).unwrap();
        let saved = crate::load_issue_monitor_prefs(&path).unwrap();
        let member = &serde_json::to_value(&roster).unwrap()["members"][0];
        assert_eq!(member["agent_id"], "claude");
        assert_eq!(saved.max_active_agents, 4);
        let pool = saved.launch_profile_pool();
        assert_eq!(
            pool.len(),
            4,
            "a legacy provider retains all four named slots"
        );
        assert!(pool
            .iter()
            .all(|profile| profile.model.as_deref() == Some("sonnet")));
        assert_eq!(
            serde_json::to_value(&saved.launch_profile).unwrap()["agent_member_id"],
            member["agent_member_id"]
        );
        assert_eq!(
            read_project_roster(&root, &gwt_core::paths::gwt_sessions_dir()).unwrap(),
            roster
        );
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let first = reserve_project_member_for_agent(
            &root, &sessions, "one", None, None, None, None, "claude", None,
        )
        .unwrap();
        let second = reserve_project_member_for_agent(
            &root, &sessions, "two", None, None, None, None, "claude", None,
        )
        .unwrap();
        assert_ne!(first.agent_member_id, second.agent_member_id);
        assert!(reserve_project_member_for_agent(
            &root, &sessions, "one", None, None, None, None, "codex", None
        )
        .unwrap_err()
        .contains("fixed tool"));
        release_project_member(&root, &sessions, &first.agent_member_id, "one").unwrap();
        mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Rest {
                member_id: first.agent_member_id.clone(),
            },
            None,
        )
        .unwrap();
        assert!(reserve_project_member_for_agent(
            &root,
            &sessions,
            "wrong",
            None,
            None,
            None,
            Some(&first.agent_member_id),
            "codex",
            None
        )
        .unwrap_err()
        .contains("fixed tool"));
        let restored = reserve_project_member_for_agent(
            &root,
            &sessions,
            "restore",
            None,
            None,
            None,
            Some(&first.agent_member_id),
            "claude",
            None,
        )
        .unwrap();
        assert_eq!(
            restored.agent_member_id, first.agent_member_id,
            "Rest preserves exact same-tool restoration"
        );
        crate::try_mutate_issue_monitor_prefs(&path, |prefs| {
            prefs.max_active_agents = 5;
            resize_shared_roster(prefs, 5).map_err(std::io::Error::other)?;
            Ok(())
        })
        .unwrap();
        let saved = crate::load_issue_monitor_prefs(&path).unwrap();
        let roster = saved.agent_roster.as_ref().unwrap();
        assert_eq!(roster.enabled_count(), 5);
        assert!(roster
            .members
            .iter()
            .all(|member| member.agent_id.as_deref() == Some("claude")));
        assert_eq!(
            saved.launch_profile_pool().len(),
            4,
            "bound selections never expand on resize"
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
