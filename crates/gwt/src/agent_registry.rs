//! Shared Agent ownership. Project preference files are recoverable execution mirrors.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use gwt_core::{operation_deadline::NamedFileLock, repo_hash::ProjectKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    agent_names::{AgentMember, AgentRoster},
    IssueMonitorLaunchProfile, IssueMonitorPrefs,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryPhase {
    Inactive,
    Prepared,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileCore {
    pub agent_id: String,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub version: Option<String>,
    pub fast_mode: bool,
}

impl AgentProfileCore {
    fn from_profile(profile: &IssueMonitorLaunchProfile) -> Self {
        Self {
            agent_id: profile.agent_id.clone(),
            model: profile.model.clone(),
            reasoning: profile.reasoning.clone(),
            version: profile.version.clone(),
            fast_mode: profile.fast_mode,
        }
    }

    fn apply(&self, profile: &mut IssueMonitorLaunchProfile) {
        profile.agent_id.clone_from(&self.agent_id);
        profile.model.clone_from(&self.model);
        profile.reasoning.clone_from(&self.reasoning);
        profile.version.clone_from(&self.version);
        profile.fast_mode = self.fast_mode;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSnapshot {
    sha256: String,
    raw: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    was_active: bool,
    aliases: BTreeMap<String, String>,
    session_aliases: BTreeMap<String, String>,
    transaction_id: String,
    before_sha256: String,
    before_json: Option<String>,
    floor_before_sha256: String,
    floor_before_json: Option<String>,
    after_sha256: String,
    after_json: String,
    // Rosterless sources are not represented in the inactive identity importer.
    // Keep every observed legacy snapshot here instead of losing migration evidence.
    rosterless_snapshots: Vec<RawSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema_version: u32,
    phase: RegistryPhase,
    revision: u64,
    roster: Option<AgentRoster>,
    profiles: BTreeMap<String, AgentProfileCore>,
    sources: BTreeMap<String, Source>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryReconcileAction {
    RefreshPreparedSource,
    QuarantineReplayMirror,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryReconcileRequest {
    pub expected_phase: RegistryPhase,
    pub expected_registry_revision: u64,
    pub source_key: String,
    pub expected_transaction_id: String,
    pub expected_source_sha256: String,
    #[serde(default)]
    pub expected_floor_sha256: Option<String>,
    pub action: RegistryReconcileAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryProjectStatus {
    pub source_key: String,
    pub transaction_id: String,
    pub before_sha256: String,
    pub after_sha256: String,
    pub observed_sha256: String,
    pub floor_before_sha256: String,
    pub floor_observed_sha256: String,
    pub floor_pending: bool,
    pub floor_conflict: bool,
    pub pending: bool,
    pub conflict: bool,
    pub recovery_action: Option<RegistryReconcileAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryStatus {
    pub phase: RegistryPhase,
    pub registry_revision: u64,
    pub projects: Vec<RegistryProjectStatus>,
    pub assigned_members: usize,
    pub unknown_assignments: usize,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub fn path() -> PathBuf {
    gwt_core::paths::gwt_home().join("machine-state/agent-registry.json")
}

pub fn history_path() -> PathBuf {
    gwt_core::paths::gwt_home().join("machine-state/agent-names.jsonl")
}

pub(crate) fn fingerprint(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

fn raw(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(Some(raw)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn raw_hash(raw: Option<&str>) -> String {
    raw.map(fingerprint).unwrap_or_else(|| "missing".into())
}

fn prefs_path(key: &str) -> io::Result<PathBuf> {
    Ok(gwt_core::paths::gwt_project_dir(
        &ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?,
    )
    .join("project-state/issue-monitor.json"))
}

pub(crate) fn source_key(path: &Path) -> Option<String> {
    let relative = path
        .strip_prefix(gwt_core::paths::gwt_projects_dir())
        .ok()?;
    if relative.components().count() != 3 || !relative.ends_with("project-state/issue-monitor.json")
    {
        return None;
    }
    let key = relative.components().next()?.as_os_str().to_str()?;
    ProjectKey::parse(key).ok().map(|key| key.to_string())
}

fn lock() -> io::Result<NamedFileLock> {
    let path = path();
    gwt_config::home::assert_test_path(&path);
    fs::create_dir_all(path.parent().expect("machine state parent"))?;
    NamedFileLock::acquire(&path.with_extension("lock"), "shared Agent registry")
}

fn load() -> io::Result<Option<Registry>> {
    let Some(raw) = raw(&path())? else {
        return Ok(None);
    };
    let registry: Registry = serde_json::from_str(&raw).map_err(io::Error::other)?;
    validate(&registry)?;
    Ok(Some(registry))
}

fn save(registry: &Registry) -> io::Result<()> {
    validate(registry)?;
    crate::issue_monitor::durable_atomic_write(
        &path(),
        &serde_json::to_vec_pretty(registry).map_err(io::Error::other)?,
    )
}

fn confirm_durable() -> io::Result<()> {
    let registry_path = path();
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&registry_path)?
        .sync_all()?;
    #[cfg(test)]
    if std::env::var_os("GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE")
        .is_some_and(|target| Path::new(&target) == registry_path)
    {
        let marker = registry_path.with_extension("parent-sync-fail-once");
        if marker.exists() {
            fs::remove_file(marker)?;
            return Err(io::Error::other(
                "injected Agent registry durability confirmation failure",
            ));
        }
    }
    #[cfg(unix)]
    fs::File::open(registry_path.parent().expect("machine state parent"))?.sync_all()?;
    Ok(())
}

fn ensure_cores(registry: &mut Registry) {
    for member in registry
        .roster
        .as_ref()
        .into_iter()
        .flat_map(|roster| &roster.members)
    {
        if let Some(tool) = &member.agent_id {
            registry
                .profiles
                .entry(member.agent_member_id.clone())
                .or_insert_with(|| AgentProfileCore {
                    agent_id: tool.clone(),
                    model: None,
                    reasoning: None,
                    version: None,
                    fast_mode: false,
                });
        }
    }
}

fn settings_revision(registry: &Registry) -> u64 {
    registry.roster.as_ref().map_or(0, |roster| roster.revision)
}

fn validate(registry: &Registry) -> io::Result<()> {
    if registry.schema_version != 1
        || !matches!(
            registry.phase,
            RegistryPhase::Prepared | RegistryPhase::Active
        )
    {
        return Err(invalid("Unsupported shared Agent registry schema or phase"));
    }
    let mut imported = BTreeSet::new();
    for (key, source) in &registry.sources {
        ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?;
        if fingerprint(&source.after_json) != source.after_sha256
            || raw_hash(source.before_json.as_deref()) != source.before_sha256
            || raw_hash(source.floor_before_json.as_deref()) != source.floor_before_sha256
            || source.transaction_id.is_empty()
        {
            return Err(invalid(format!("Agent registry commit mismatch for {key}")));
        }
        for observation in &source.rosterless_snapshots {
            if raw_hash(observation.raw.as_deref()) != observation.sha256 {
                return Err(invalid(format!(
                    "Agent legacy observation fingerprint mismatch for {key}"
                )));
            }
        }
        let prefs = decode(Some(&source.after_json))?;
        if prefs.schema_version != 2 || prefs.agent_roster.is_some() {
            return Err(invalid(format!("Invalid Agent execution mirror for {key}")));
        }
        if registry.phase == RegistryPhase::Active && !source.was_active {
            return Err(invalid("Active Agent registry has an unfenced enrollment"));
        }
        for id in source.aliases.values() {
            uuid::Uuid::parse_str(id).map_err(|error| invalid(error.to_string()))?;
            if !imported.insert(id.as_str()) {
                return Err(invalid(
                    "Agent registry source aliases share a global identity",
                ));
            }
        }
        for (session_id, id) in &source.session_aliases {
            if !source.aliases.values().any(|alias| alias == id)
                || registry
                    .roster
                    .as_ref()
                    .and_then(|roster| {
                        roster
                            .members
                            .iter()
                            .find(|member| &member.agent_member_id == id)
                    })
                    .is_some_and(|member| member.origin_session_id.as_deref() != Some(session_id))
            {
                return Err(invalid(
                    "Agent migration Session proof has no exact source member",
                ));
            }
        }
    }
    if let Some(roster) = &registry.roster {
        roster.validate().map_err(invalid)?;
        if roster.schema_version != 2 {
            return Err(invalid("Shared Agent roster must use schema2"));
        }
        let mut names: BTreeMap<String, Vec<&AgentMember>> = BTreeMap::new();
        for member in &roster.members {
            uuid::Uuid::parse_str(&member.agent_member_id)
                .map_err(|error| invalid(error.to_string()))?;
            names
                .entry(member.name.trim().to_lowercase())
                .or_default()
                .push(member);
            if let Some(assignment) = &member.assignment {
                let Some(key) = assignment.project_key.as_deref() else {
                    return Err(invalid("Shared Agent assignment has no source project"));
                };
                if !registry.sources.contains_key(key) {
                    return Err(invalid(
                        "Shared Agent assignment has an unknown source project",
                    ));
                }
            }
        }
        for members in names.values().filter(|members| members.len() > 1) {
            if members
                .iter()
                .any(|member| !imported.contains(member.agent_member_id.as_str()))
            {
                return Err(invalid(
                    "Duplicate Agent names require distinct imported provenance",
                ));
            }
        }
        for (id, core) in &registry.profiles {
            let member = roster
                .members
                .iter()
                .find(|member| &member.agent_member_id == id)
                .ok_or_else(|| invalid("Saved Agent core has no registered member"))?;
            if member.agent_id.as_ref() != Some(&core.agent_id) {
                return Err(invalid("Saved Agent core has a different fixed tool"));
            }
        }
    } else if !registry.profiles.is_empty() {
        return Err(invalid("Saved Agent cores require a registered roster"));
    }
    Ok(())
}

fn decode(raw: Option<&str>) -> io::Result<IssueMonitorPrefs> {
    match raw {
        Some(raw) => crate::update_settings::decode_project_prefs(raw).map_err(|error| {
            io::Error::new(
                if error.is_syntax() || error.is_eof() {
                    io::ErrorKind::InvalidData
                } else {
                    io::ErrorKind::InvalidInput
                },
                error,
            )
        }),
        None => Ok(IssueMonitorPrefs::default()),
    }
}

fn resolve(registry: &Registry, key: &str, id: &str) -> io::Result<String> {
    if let Some(global) = registry
        .sources
        .get(key)
        .and_then(|source| source.aliases.get(id))
    {
        if global != id
            && registry.roster.as_ref().is_some_and(|roster| {
                roster
                    .members
                    .iter()
                    .any(|member| member.agent_member_id == id)
            })
        {
            return Err(invalid(format!("Agent identity {key}/{id} is ambiguous between legacy and global namespaces; use the exact recorded assignment")));
        }
        return Ok(global.clone());
    }
    if registry.roster.as_ref().is_some_and(|roster| {
        roster
            .members
            .iter()
            .any(|member| member.agent_member_id == id)
    }) {
        return Ok(id.into());
    }
    Err(invalid(format!(
        "Registered Agent identity {key}/{id} is unavailable; reload Agents"
    )))
}

pub fn resolve_member_id(root: &Path, observed_id: &str) -> io::Result<String> {
    resolve_member_id_in_source(&gwt_core::paths::project_scope_hash(root), observed_id)
}

pub fn is_active() -> io::Result<bool> {
    load().map(|registry| registry.is_some_and(|registry| registry.phase == RegistryPhase::Active))
}

pub fn resolve_member_id_in_source(source: &ProjectKey, observed_id: &str) -> io::Result<String> {
    SessionMemberSnapshot::load()?.resolve_member_id(source, observed_id)
}

/// One immutable, validated registry read for a batch of retained Sessions.
/// It carries no write authority and never refreshes itself during a view.
pub struct SessionMemberSnapshot {
    registry: Option<Registry>,
}

impl SessionMemberSnapshot {
    pub fn load() -> io::Result<Self> {
        Ok(Self { registry: load()? })
    }

    fn resolve_member_id(&self, source: &ProjectKey, observed_id: &str) -> io::Result<String> {
        let Some(registry) = &self.registry else {
            return Ok(observed_id.into());
        };
        if registry.phase != RegistryPhase::Active {
            return Err(invalid(
                "Agent registry is prepared; inspect agent.registry.status and reconcile",
            ));
        }
        resolve(registry, source.as_str(), observed_id)
    }

    fn legacy_session_member_id(
        &self,
        source: &ProjectKey,
        session_id: &str,
    ) -> io::Result<Option<String>> {
        let Some(registry) = self
            .registry
            .as_ref()
            .filter(|registry| registry.phase == RegistryPhase::Active)
        else {
            return Ok(None);
        };
        let Some(source) = registry.sources.get(source.as_str()) else {
            return Ok(None);
        };
        let mut matches = BTreeSet::new();
        for id in [
            source.session_aliases.get(session_id),
            source.aliases.get(session_id),
        ]
        .into_iter()
        .flatten()
        {
            if registry.roster.as_ref().is_some_and(|roster| {
                roster.members.iter().any(|member| {
                    &member.agent_member_id == id
                        && member.origin_session_id.as_deref() == Some(session_id)
                })
            }) {
                matches.insert(id.clone());
            }
        }
        if matches.len() > 1 {
            return Err(invalid(
                "Legacy Session has ambiguous source-qualified Agent proofs",
            ));
        }
        Ok(matches.into_iter().next())
    }

    pub fn resolve_session(&self, session: &gwt_agent::Session) -> io::Result<Option<String>> {
        let source = if let Some(key) = session.repo_hash.as_deref() {
            ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?
        } else {
            gwt_core::paths::project_scope_hash(
                session
                    .project_state_root
                    .as_deref()
                    .unwrap_or(&session.worktree_path),
            )
        };
        match session.agent_member_id.as_deref() {
            Some(id) => self.resolve_member_id(&source, id).map(Some),
            None => self.legacy_session_member_id(&source, &session.id),
        }
    }
}

fn bind_project_profiles(registry: &Registry, key: &str, prefs: &mut IssueMonitorPrefs) {
    let mut profiles = prefs.launch_profile_pool();
    for profile in &mut profiles {
        if let Some(id) = profile.agent_member_id.as_deref() {
            if let Ok(global) = resolve(registry, key, id) {
                profile.agent_member_id = Some(global);
            }
        } else if let Some(roster) = &registry.roster {
            let source = registry.sources.get(key);
            let matching =
                |member: &&AgentMember| member.agent_id.as_ref() == Some(&profile.agent_id);
            let local = roster.members.iter().filter(matching).find(|member| {
                source.is_some_and(|source| {
                    source
                        .aliases
                        .values()
                        .any(|id| id == &member.agent_member_id)
                })
            });
            if let Some(member) = local.or_else(|| {
                roster.members.iter().find(|member| {
                    member.enabled && member.agent_id.as_ref() == Some(&profile.agent_id)
                })
            }) {
                profile.agent_member_id = Some(member.agent_member_id.clone());
            }
        }
    }
    prefs.set_launch_profile_pool(profiles);
}

fn project_profiles(registry: &Registry, key: &str, prefs: &mut IssueMonitorPrefs) {
    bind_project_profiles(registry, key, prefs);
    let mut profiles = prefs.launch_profile_pool();
    for profile in &mut profiles {
        if let Some(core) = profile
            .agent_member_id
            .as_ref()
            .and_then(|id| registry.profiles.get(id))
        {
            core.apply(profile);
        }
    }
    prefs.set_launch_profile_pool(profiles);
}

fn project_identity(
    registry: &Registry,
    key: &str,
    prefs: &mut IssueMonitorPrefs,
) -> io::Result<()> {
    // Every persisted project pin is qualified by this exact source store.
    for entry in &mut prefs.agent_issue_schedule.reservations {
        entry.agent_member_id = resolve(registry, key, &entry.agent_member_id)?;
    }
    for delivery in &mut prefs.pending_launch_deliveries {
        if let Some(id) = &mut delivery.agent_member_id {
            *id = resolve(registry, key, id)?;
        }
    }
    for intent in prefs.agent_switch_intents.values_mut() {
        intent.request.member_id = resolve(registry, key, &intent.request.member_id)?;
        intent.source.assignment.project_key = Some(key.into());
    }
    project_profiles(registry, key, prefs);
    Ok(())
}

fn project(registry: &Registry, key: &str, raw: Option<&str>) -> io::Result<IssueMonitorPrefs> {
    let mut prefs = decode(raw)?;
    project_identity(registry, key, &mut prefs)?;
    prefs.agent_roster = registry.roster.clone();
    if let Some(roster) = &prefs.agent_roster {
        prefs.max_active_agents = roster.enabled_count();
    }
    prefs.schema_version = 2;
    prefs.agent_registry_revision = Some(settings_revision(registry));
    prefs.agent_registry_project_key = Some(key.into());
    prefs.agent_registry_profiles = registry.profiles.clone();
    Ok(prefs)
}

fn mirror_json(prefs: &IssueMonitorPrefs, original: Option<&str>) -> io::Result<String> {
    let mut mirror = prefs.clone();
    mirror.schema_version = 2;
    mirror.agent_roster = None;
    let mut profiles = mirror.launch_profile_pool();
    for profile in &mut profiles {
        if profile.agent_member_id.is_none() {
            continue;
        }
        profile.model = None;
        profile.reasoning = None;
        profile.version = None;
        profile.fast_mode = false;
    }
    mirror.set_launch_profile_pool(profiles);
    let mut value = original
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()
        .map_err(io::Error::other)?
        .unwrap_or_else(|| serde_json::json!({}));
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid("Project preferences must be an object"))?;
    let mut known = serde_json::to_value(IssueMonitorPrefs::default()).map_err(io::Error::other)?;
    if let Some(original) = original {
        let before = serde_json::to_value(decode(Some(original))?).map_err(io::Error::other)?;
        known
            .as_object_mut()
            .expect("prefs object")
            .extend(before.as_object().expect("prefs object").clone());
    }
    for key in known.as_object().expect("prefs object").keys() {
        object.remove(key);
    }
    object.remove("agent_roster");
    object.extend(
        serde_json::to_value(mirror)
            .map_err(io::Error::other)?
            .as_object()
            .expect("prefs object")
            .clone(),
    );
    serde_json::to_string_pretty(&value).map_err(io::Error::other)
}

fn source_from(
    registry: &Registry,
    key: &str,
    prefs: &IssueMonitorPrefs,
    original: Option<&str>,
    aliases: BTreeMap<String, String>,
) -> io::Result<Source> {
    let after = mirror_json(prefs, original)?;
    let floor = raw(&prefs_path(key)?.with_extension("committed.json"))?;
    Ok(Source {
        was_active: registry
            .sources
            .get(key)
            .is_some_and(|source| source.was_active),
        aliases,
        session_aliases: registry
            .sources
            .get(key)
            .map(|source| source.session_aliases.clone())
            .unwrap_or_default(),
        transaction_id: uuid::Uuid::new_v4().to_string(),
        before_sha256: raw_hash(original),
        before_json: original.map(str::to_string),
        floor_before_sha256: raw_hash(floor.as_deref()),
        floor_before_json: floor,
        after_sha256: fingerprint(&after),
        after_json: after,
        rosterless_snapshots: registry
            .sources
            .get(key)
            .map(|source| source.rosterless_snapshots.clone())
            .unwrap_or_default(),
    })
}

fn assert_known(registry: &Registry, key: &str, observed: Option<&str>) -> io::Result<()> {
    let source = registry
        .sources
        .get(key)
        .ok_or_else(|| invalid(format!("Agent source {key} is not enrolled")))?;
    let hash = raw_hash(observed);
    if hash != source.after_sha256 && hash != source.before_sha256 {
        return Err(invalid(format!(
            "Agent mirror conflict: project={key}, phase={:?}, revision={}, transaction={}, before={}, after={}, observed={hash}; inspect agent.registry.status, then explicitly quarantine_replay_mirror",
            registry.phase, registry.revision, source.transaction_id, source.before_sha256, source.after_sha256,
        )));
    }
    Ok(())
}

fn assert_known_floor(source: &Source, path: &Path) -> io::Result<()> {
    let observed = raw_hash(raw(&path.with_extension("committed.json"))?.as_deref());
    if observed != source.floor_before_sha256 && observed != source.after_sha256 {
        return Err(invalid(format!(
            "Agent recovery floor conflict: project={}, transaction={}, before={}, after={}, observed={observed}; read agent.registry.status and explicitly reconcile with expected_floor_sha256",
            source_key(path).as_deref().unwrap_or("unknown"), source.transaction_id,
            source.floor_before_sha256, source.after_sha256,
        )));
    }
    Ok(())
}

fn publish(source: &Source, path: &Path) -> io::Result<()> {
    assert_known_floor(source, path)?;
    publish_verified(source, path)
}

fn publish_verified(source: &Source, path: &Path) -> io::Result<()> {
    crate::issue_monitor::durable_atomic_write(path, source.after_json.as_bytes())?;
    // The schema1 recovery writer must not be able to restore an old schema1
    // floor after the canonical schema2 file is damaged.
    crate::issue_monitor::durable_atomic_write(
        &path.with_extension("committed.json"),
        source.after_json.as_bytes(),
    )
}

fn quarantine(path: &Path, contents: &str) -> io::Result<()> {
    let destination = path.with_file_name(format!(
        "{}.registry-quarantine-{}.json",
        path.file_name().expect("prefs file").to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    crate::issue_monitor::durable_atomic_write(&destination, contents.as_bytes())
}

#[cfg(test)]
thread_local! {
    static READ_PROJECT_AFTER_SOURCE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Read one stable registry/raw/registry generation without acquiring writer locks.
pub(crate) fn read_project(path: &Path) -> io::Result<Option<IssueMonitorPrefs>> {
    let Some(key) = source_key(path) else {
        return Ok(None);
    };
    let deadline = gwt_core::operation_deadline::current()
        .unwrap_or_else(|| gwt_core::operation_deadline::now() + std::time::Duration::from_secs(5));
    loop {
        if gwt_core::operation_deadline::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Agent registry changed during read; retry the same operation",
            ));
        }
        let Some(before) = load()? else {
            return Ok(None);
        };
        if before.phase != RegistryPhase::Active {
            return Err(invalid(
                "Agent registry is prepared; inspect agent.registry.status and reconcile",
            ));
        }
        let observed = raw(path)?;
        let floor = raw(&path.with_extension("committed.json"))?;
        #[cfg(test)]
        if let Some(observer) = READ_PROJECT_AFTER_SOURCE.with(|slot| slot.borrow_mut().take()) {
            observer();
        }
        let Some(after) = load()? else { continue };
        if before != after {
            std::thread::yield_now();
            continue;
        }
        if let Some(source) = after.sources.get(&key) {
            assert_known(&after, &key, observed.as_deref())?;
            let floor_hash = raw_hash(floor.as_deref());
            if floor_hash != source.floor_before_sha256 && floor_hash != source.after_sha256 {
                return Err(invalid(format!("Agent recovery floor conflict for {key}: observed={floor_hash}; read agent.registry.status and explicitly reconcile with expected_floor_sha256")));
            }
            return project(&after, &key, Some(&source.after_json)).map(Some);
        }
        let prefs = decode(observed.as_deref())?;
        if prefs.schema_version != 1 || prefs.agent_roster.is_some() {
            return Err(invalid(format!(
                "Agent source {key} needs enrollment; read Agents before retrying"
            )));
        }
        // A new rosterless project can read the shared snapshot before its first
        // mutation enrolls its execution mirror under source -> registry locks.
        return project(&after, &key, observed.as_deref()).map(Some);
    }
}

pub fn member_profile(
    prefs: &IssueMonitorPrefs,
    member_id: &str,
) -> Option<IssueMonitorLaunchProfile> {
    if let Some(profile) = prefs
        .launch_profile_pool()
        .into_iter()
        .find(|profile| profile.agent_member_id.as_deref() == Some(member_id))
    {
        return Some(profile);
    }
    let core = prefs.agent_registry_profiles.get(member_id)?;
    let mut profile = prefs
        .launch_profile_pool()
        .into_iter()
        .next()
        .unwrap_or_else(|| IssueMonitorLaunchProfile {
            agent_id: core.agent_id.clone(),
            agent_member_id: None,
            model: None,
            reasoning: None,
            version: None,
            session_mode: Default::default(),
            skip_permissions: false,
            fast_mode: false,
            runtime_target: Default::default(),
            docker_service: None,
            docker_lifecycle_intent: Default::default(),
            windows_shell: None,
            prefer_for: Vec::new(),
        });
    profile.agent_member_id = Some(member_id.into());
    profile.prefer_for.clear();
    core.apply(&mut profile);
    Some(profile)
}

/// Held strictly after a project prefs lock. No method takes another source lock.
pub(crate) struct RegistryWrite {
    _lock: Option<NamedFileLock>,
    registry: Option<Registry>,
    key: Option<String>,
    before: Option<IssueMonitorPrefs>,
}

impl RegistryWrite {
    pub(crate) fn acquire(path: &Path) -> io::Result<Self> {
        let Some(key) = source_key(path) else {
            return Ok(Self {
                _lock: None,
                registry: None,
                key: None,
                before: None,
            });
        };
        let guard = lock()?;
        let mut registry = load()?;
        if let Some(current) = &mut registry {
            if current.phase != RegistryPhase::Active {
                return Err(invalid(
                    "Agent registry is prepared; inspect agent.registry.status and reconcile",
                ));
            }
            confirm_durable()?;
            if !current.sources.contains_key(&key) {
                if path.try_exists()?
                    || crate::agent_names::observed_project_member_seeds(
                        &gwt_core::paths::gwt_sessions_dir(),
                    )
                    .map_err(io::Error::other)?
                    .contains_key(&key)
                {
                    return Err(io::Error::new(io::ErrorKind::WouldBlock, "Legacy Agent source needs enrollment before its prefs lock; retry the same command after preflight"));
                }
                enroll(current, &key, None, &[])?;
                save(current)?;
            }
            let observed = raw(path)?;
            assert_known(current, &key, observed.as_deref())?;
            let source = &current.sources[&key];
            assert_known_floor(source, path)?;
            if raw_hash(observed.as_deref()) != source.after_sha256
                || raw(&path.with_extension("committed.json"))?.as_deref()
                    != Some(source.after_json.as_str())
            {
                publish(source, path)?;
            }
        }
        Ok(Self {
            _lock: Some(guard),
            registry,
            key: Some(key),
            before: None,
        })
    }

    pub(crate) fn load(&mut self, path: &Path) -> io::Result<Option<IssueMonitorPrefs>> {
        let (Some(registry), Some(key)) = (&self.registry, &self.key) else {
            return Ok(None);
        };
        let prefs = project(registry, key, Some(&registry.sources[key].after_json))?;
        self.before = Some(prefs.clone());
        let _ = path;
        Ok(Some(prefs))
    }

    pub(crate) fn commit(
        &mut self,
        path: &Path,
        prefs: &IssueMonitorPrefs,
        mutation: bool,
    ) -> io::Result<Option<IssueMonitorPrefs>> {
        let (Some(registry), Some(key)) = (&self.registry, &self.key) else {
            return Ok(None);
        };
        let mut next = registry.clone();
        let latest = project(registry, key, Some(&registry.sources[key].after_json))?;
        let mut candidate = prefs.clone();
        if mutation {
            if prefs.agent_registry_revision != Some(settings_revision(registry))
                || prefs.agent_registry_project_key.as_deref() != Some(key)
            {
                return Err(invalid(
                    "Agent registry changed; reload the exact project before retrying",
                ));
            }
            if prefs.agent_roster != latest.agent_roster {
                next.roster = prefs.agent_roster.clone();
            }
            bind_project_profiles(&next, key, &mut candidate);
            for profile in candidate.launch_profile_pool() {
                let Some(id) = profile.agent_member_id.as_ref() else {
                    continue;
                };
                let changed = self
                    .before
                    .as_ref()
                    .and_then(|before| {
                        before
                            .launch_profile_pool()
                            .into_iter()
                            .find(|before| before.agent_member_id.as_ref() == Some(id))
                    })
                    .is_none_or(|before| {
                        AgentProfileCore::from_profile(&before)
                            != AgentProfileCore::from_profile(&profile)
                    });
                if changed || !next.profiles.contains_key(id) {
                    next.profiles
                        .insert(id.clone(), AgentProfileCore::from_profile(&profile));
                }
            }
        }
        ensure_cores(&mut next);
        candidate.agent_roster = next.roster.clone();
        if let Some(roster) = &candidate.agent_roster {
            candidate.max_active_agents = roster.enabled_count();
        }
        project_profiles(&next, key, &mut candidate);
        crate::agent_names::validate_registered_profiles(
            &candidate,
            &candidate.launch_profile_pool(),
        )
        .map_err(invalid)?;
        let source = &registry.sources[key];
        let after_json = mirror_json(&candidate, Some(&source.after_json))?;
        if next.roster == registry.roster
            && next.profiles == registry.profiles
            && after_json == source.after_json
        {
            return Ok(Some(latest));
        }
        let global_changed = next.roster != registry.roster || next.profiles != registry.profiles;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("Agent registry revision exhausted"))?;
        if let Some(roster) = &mut next.roster {
            roster.schema_version = 2;
            if global_changed {
                roster.revision = settings_revision(registry)
                    .checked_add(1)
                    .ok_or_else(|| invalid("Agent settings revision exhausted"))?;
            }
        }
        let committed = source_from(
            &next,
            key,
            &candidate,
            Some(&source.after_json),
            source.aliases.clone(),
        )?;
        next.sources.insert(key.clone(), committed);
        save(&next)?; // The sole logical commit point for an Active transaction.
        self.registry = Some(next.clone());
        publish(&next.sources[key], path)?;
        let saved = project(&next, key, Some(&next.sources[key].after_json))?;
        self.before = Some(saved.clone());
        Ok(Some(saved))
    }
}

fn add_source(
    registry: &mut Registry,
    key: &str,
    raw: Option<&str>,
    mut aliases: BTreeMap<String, String>,
    seeds: &[AgentMember],
) -> io::Result<()> {
    let mut prefs = decode(raw)?;
    if prefs.schema_version != 1 {
        return Err(invalid(format!("New Agent source {key} is not schema1")));
    }
    let rosterless = prefs.agent_roster.is_none();
    let mut rosterless_snapshots = registry
        .sources
        .get(key)
        .map(|source| source.rosterless_snapshots.clone())
        .unwrap_or_default();
    if rosterless || !rosterless_snapshots.is_empty() {
        let observation = RawSnapshot {
            sha256: raw_hash(raw),
            raw: raw.map(str::to_string),
        };
        if rosterless_snapshots.last() != Some(&observation) {
            rosterless_snapshots.push(observation);
        }
    }
    if let Some(mut roster) = prefs.agent_roster.take() {
        roster.validate().map_err(invalid)?;
        let pool = prefs.launch_profile_pool();
        let canonical = |tool: &str| {
            gwt_agent::resolve_agent_id(tool)
                .map(|tool| tool.command().to_string())
                .ok_or_else(|| invalid("Unknown legacy Agent tool"))
        };
        let tools: Vec<_> = pool
            .iter()
            .map(|profile| canonical(&profile.agent_id))
            .collect::<io::Result<_>>()?;
        for (index, member) in roster.members.iter_mut().enumerate() {
            let old = member.agent_member_id.clone();
            let observed = member
                .assignment
                .as_ref()
                .and_then(|assignment| assignment.session_id.as_ref())
                .and_then(|id| {
                    if let Some(seed) = seeds
                        .iter()
                        .find(|seed| seed.origin_session_id.as_ref() == Some(id))
                    {
                        return seed.agent_id.clone();
                    }
                    let session = gwt_agent::Session::load(
                        &gwt_core::paths::gwt_sessions_dir().join(format!("{id}.toml")),
                    )
                    .ok()?;
                    let root = session
                        .project_state_root
                        .as_deref()
                        .unwrap_or(&session.worktree_path);
                    let exact_project = if root.exists() {
                        gwt_core::paths::project_scope_hash(root).as_str() == key
                    } else {
                        session.repo_hash.as_deref() == Some(key)
                    };
                    (session.id == *id && exact_project)
                        .then(|| session.agent_id.command().to_string())
                });
            let fixed = member.agent_id.as_deref().map(canonical).transpose()?;
            if fixed
                .as_ref()
                .zip(observed.as_ref())
                .is_some_and(|(fixed, observed)| fixed != observed)
            {
                return Err(invalid(format!("Agent {} has a different exact live tool; retain its assignment before retrying", member.name)));
            }
            member.agent_id = Some(
                fixed
                    .or(observed)
                    .or_else(|| {
                        pool.iter()
                            .find(|profile| profile.agent_member_id.as_deref() == Some(&old))
                            .map(|profile| profile.agent_id.clone())
                    })
                    .or_else(|| tools.get(index % tools.len().max(1)).cloned())
                    .unwrap_or_else(|| "codex".into()),
            );
            member.agent_member_id = aliases
                .get(&old)
                .ok_or_else(|| invalid("Imported Agent alias missing"))?
                .clone();
            if let Some(assignment) = &mut member.assignment {
                assignment.project_key = Some(key.into());
            }
            if member.last_issue_number.is_some() || member.last_spec_number.is_some() {
                member.last_owner_project_key = Some(key.into());
            }
        }
        roster.schema_version = 2;
        match &mut registry.roster {
            Some(global) => global.members.extend(roster.members),
            None => registry.roster = Some(roster),
        }
    } else if !seeds.is_empty() {
        let mut seeds = seeds.to_vec();
        seeds.sort_by(|left, right| {
            (&left.created_at, &left.origin_session_id)
                .cmp(&(&right.created_at, &right.origin_session_id))
        });
        for (index, member) in seeds.iter_mut().enumerate() {
            member.agent_member_id = aliases
                .entry(member.agent_member_id.clone())
                .or_insert_with(|| uuid::Uuid::new_v4().to_string())
                .clone();
            member.enabled = index < prefs.max_active_agents.max(1);
            member.last_owner_project_key = Some(key.into());
            if let Some(assignment) = &mut member.assignment {
                assignment.project_key = Some(key.into());
            }
        }
        match &mut registry.roster {
            Some(roster) => roster.members.extend(seeds),
            None => {
                registry.roster = Some(AgentRoster {
                    schema_version: 2,
                    revision: 1,
                    members: seeds,
                })
            }
        }
    }
    // Insert aliases before translating project pins and saved profiles.
    let placeholder = Source {
        was_active: registry
            .sources
            .get(key)
            .map_or(registry.phase == RegistryPhase::Active, |source| {
                source.was_active
            }),
        aliases,
        session_aliases: registry
            .sources
            .get(key)
            .map(|source| source.session_aliases.clone())
            .unwrap_or_default(),
        transaction_id: uuid::Uuid::new_v4().to_string(),
        before_sha256: raw_hash(raw),
        before_json: raw.map(str::to_string),
        floor_before_sha256: "missing".into(),
        floor_before_json: None,
        after_sha256: String::new(),
        after_json: String::new(),
        rosterless_snapshots,
    };
    registry.sources.insert(key.into(), placeholder);
    let proven: Vec<_> = registry
        .roster
        .as_ref()
        .into_iter()
        .flat_map(|roster| &roster.members)
        .filter_map(|member| {
            let session_id = member.origin_session_id.as_ref()?;
            let owns = registry.sources[key]
                .aliases
                .values()
                .any(|id| id == &member.agent_member_id);
            (owns
                && seeds.iter().any(|seed| {
                    seed.origin_session_id.as_ref() == Some(session_id)
                        && seed.agent_id == member.agent_id
                }))
            .then(|| (session_id.clone(), member.agent_member_id.clone()))
        })
        .collect();
    for (session, id) in proven {
        let aliases = &mut registry
            .sources
            .get_mut(key)
            .expect("source inserted")
            .session_aliases;
        if aliases
            .get(&session)
            .is_some_and(|existing| existing != &id)
        {
            return Err(invalid("Source Session identity changed during Agent migration; preserve the recorded alias before retrying"));
        }
        aliases.insert(session, id);
    }
    let mut profiles = prefs.launch_profile_pool();
    for profile in &mut profiles {
        if let Some(id) = &mut profile.agent_member_id {
            *id = resolve(registry, key, id)?;
        }
    }
    prefs.set_launch_profile_pool(profiles);
    project_profiles(registry, key, &mut prefs);
    for profile in prefs.launch_profile_pool() {
        if let Some(id) = &profile.agent_member_id {
            registry
                .profiles
                .insert(id.clone(), AgentProfileCore::from_profile(&profile));
        }
    }
    project_identity(registry, key, &mut prefs)?;
    let before = registry.sources[key].clone();
    let source = source_from(registry, key, &prefs, raw, before.aliases)?;
    registry.sources.insert(key.into(), source);
    ensure_cores(registry);
    Ok(())
}

fn enroll(
    registry: &mut Registry,
    key: &str,
    observed: Option<&str>,
    seeds: &[AgentMember],
) -> io::Result<()> {
    let previous_roster = registry.roster.clone();
    let previous_profiles = registry.profiles.clone();
    let previous_settings_revision = settings_revision(registry);
    let prefs = decode(observed)?;
    let aliases = if prefs.agent_roster.is_some() {
        let _import_lock = crate::agent_registry_import::lock_for_registry()?;
        let mut imports = crate::agent_registry_import::load()?.unwrap_or_default();
        let imported = crate::agent_registry_import::stage_locked(
            &mut imports,
            key,
            observed.expect("roster source"),
            false,
        )?;
        imported.member_aliases
    } else {
        BTreeMap::new()
    };
    add_source(registry, key, observed, aliases, seeds)?;
    registry.revision = registry
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("Agent registry revision exhausted"))?;
    if registry.roster != previous_roster || registry.profiles != previous_profiles {
        if let Some(roster) = &mut registry.roster {
            roster.revision = previous_settings_revision
                .checked_add(1)
                .ok_or_else(|| invalid("Agent settings revision exhausted"))?;
        }
    }
    Ok(())
}

fn source_keys() -> io::Result<BTreeSet<String>> {
    let mut keys = BTreeSet::new();
    match fs::read_dir(gwt_core::paths::gwt_projects_dir()) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let key = entry.file_name().to_string_lossy().into_owned();
                if ProjectKey::parse(&key).is_ok() && prefs_path(&key)?.try_exists()? {
                    keys.insert(key);
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(imports) = crate::agent_registry_import::load()? {
        keys.extend(imports.sources.into_keys());
    }
    Ok(keys)
}

fn source_locks(keys: &BTreeSet<String>) -> io::Result<Vec<NamedFileLock>> {
    keys.iter()
        .map(|key| {
            let path = prefs_path(key)?;
            fs::create_dir_all(path.parent().expect("project state parent"))?;
            NamedFileLock::acquire(&path.with_extension("lock"), "Agent registry source")
        })
        .collect()
}

fn leased_seeds() -> io::Result<(
    BTreeMap<String, Vec<AgentMember>>,
    gwt_agent::SessionPathLeases,
)> {
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let before =
        crate::agent_names::observed_project_member_seeds(&sessions).map_err(io::Error::other)?;
    let ids: BTreeSet<_> = before
        .values()
        .flatten()
        .filter_map(|member| member.origin_session_id.as_deref())
        .collect();
    let leases = gwt_agent::acquire_session_path_leases(
        &sessions,
        &ids.iter().copied().collect::<Vec<_>>(),
    )?;
    let after =
        crate::agent_names::observed_project_member_seeds(&sessions).map_err(io::Error::other)?;
    if before != after {
        return Err(io::Error::new(io::ErrorKind::WouldBlock, "Current named Sessions changed before Agent migration; retry after their launch settles"));
    }
    Ok((after, leases))
}

fn activate_prepared(registry: &mut Registry) -> io::Result<()> {
    confirm_durable()?;
    // Refuse all unknown source/floor bytes before publishing any fence.
    for key in registry.sources.keys() {
        let path = prefs_path(key)?;
        assert_known(registry, key, raw(&path)?.as_deref())?;
        assert_known_floor(&registry.sources[key], &path)?;
    }
    if let Some(key) = registry.sources.keys().next() {
        // Preserve the existing machine-policy migration while every legacy
        // source still exists. Schema2 is deliberately excluded from discovery.
        crate::machine_agent_settings::migration_before_project_save(&prefs_path(key)?)?;
        // Initialize the existing global update preference even for an empty
        // first project, so modern autonomous mode is never mistaken for legacy ON.
        crate::update_settings::migrate()?;
    }
    for (key, source) in &registry.sources {
        let path = prefs_path(key)?;
        assert_known(registry, key, raw(&path)?.as_deref())?;
        publish(source, &path)?;
    }
    // Verify both fences while every source lock is still held.
    for (key, source) in &registry.sources {
        let path = prefs_path(key)?;
        for fenced in [&path, &path.with_extension("committed.json")] {
            if raw_hash(raw(fenced)?.as_deref()) != source.after_sha256 {
                return Err(invalid(format!("Agent source fence is incomplete: {key}")));
            }
        }
    }
    registry.phase = RegistryPhase::Active;
    for source in registry.sources.values_mut() {
        source.was_active = true;
    }
    registry.revision = registry
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("Agent registry revision exhausted"))?;
    save(registry)
}

/// Roster operations and owner/CLI mutation preflights establish shared ownership.
/// Ordinary preference queries remain read-only.
pub fn ensure_active_for_project(root: &Path) -> io::Result<()> {
    let path = crate::issue_monitor_prefs_path_for_repo_path(root);
    let Some(key) = source_key(&path) else {
        return Ok(());
    };
    ensure_active_for_source(&ProjectKey::parse(&key).map_err(|error| invalid(error.to_string()))?)
}

/// Called only at mutating entry points, before their source writer lock.
/// Ordinary public preference reads never materialize an enrollment.
pub(crate) fn preflight_write(path: &Path) -> io::Result<()> {
    let Some(key) = source_key(path) else {
        return Ok(());
    };
    let Some(registry) = load()? else {
        return Ok(());
    };
    if registry.phase == RegistryPhase::Active && registry.sources.contains_key(&key) {
        return Ok(());
    }
    if registry.phase == RegistryPhase::Active
        && !path.try_exists()?
        && !crate::agent_names::observed_project_member_seeds(&gwt_core::paths::gwt_sessions_dir())
            .map_err(io::Error::other)?
            .contains_key(&key)
    {
        return Ok(());
    }
    if gwt_agent::current_thread_holds_session_lease() {
        return Err(io::Error::new(io::ErrorKind::WouldBlock, format!(
            "Agent source {key} needs registry enrollment before this Session operation; release its Session lease and retry the same command"
        )));
    }
    ensure_active_for_source(&ProjectKey::parse(&key).map_err(|error| invalid(error.to_string()))?)
}

fn ensure_active_for_source(source: &ProjectKey) -> io::Result<()> {
    let key = source.to_string();
    let current_path = prefs_path(&key)?;
    let observed_registry = load()?;
    if observed_registry.as_ref().is_some_and(|registry| {
        registry.phase == RegistryPhase::Active && registry.sources.contains_key(&key)
    }) {
        fs::create_dir_all(current_path.parent().expect("project state parent"))?;
        let _source = NamedFileLock::acquire(
            &current_path.with_extension("lock"),
            "Agent registry enrollment",
        )?;
        let _registry = RegistryWrite::acquire(&current_path)?;
        return Ok(());
    }
    if gwt_agent::current_thread_holds_session_lease() {
        return Err(io::Error::new(io::ErrorKind::WouldBlock, format!(
            "Agent source {key} needs migration outside the current Session lease; retry after the Session operation settles"
        )));
    }
    let (seeds, session_leases) = leased_seeds()?;
    let mut keys = source_keys()?;
    keys.extend(seeds.keys().cloned());
    if let Some(registry) = &observed_registry {
        keys.extend(registry.sources.keys().cloned());
    }
    keys.insert(key);
    let sources = source_locks(&keys)?;
    let registry_lock = lock()?;
    if crate::agent_names::observed_project_member_seeds(&gwt_core::paths::gwt_sessions_dir())
        .map_err(io::Error::other)?
        != seeds
    {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Current named Sessions changed during Agent migration; retry the same operation",
        ));
    }
    if let Some(mut registry) = load()? {
        if registry.sources.keys().any(|key| !keys.contains(key)) {
            drop(registry_lock);
            drop(sources);
            drop(session_leases);
            return ensure_active_for_source(source);
        }
        if registry.phase == RegistryPhase::Prepared {
            activate_prepared(&mut registry)?;
        }
        let additions: Vec<_> = keys
            .iter()
            .filter(|key| !registry.sources.contains_key(*key))
            .cloned()
            .collect();
        if additions.is_empty() {
            return Ok(());
        }
        let mut requires_fence = false;
        for key in &additions {
            requires_fence |= prefs_path(key)?.try_exists()?
                || seeds.get(key).is_some_and(|seeds| !seeds.is_empty());
        }
        if requires_fence {
            // Previously Active source data remains authoritative; the new
            // source is only an intent until every legacy writer is fenced.
            for (key, entry) in &registry.sources {
                let path = prefs_path(key)?;
                assert_known(&registry, key, raw(&path)?.as_deref())?;
                assert_known_floor(entry, &path)?;
            }
            registry.phase = RegistryPhase::Prepared;
        }
        for key in additions {
            let path = prefs_path(&key)?;
            enroll(
                &mut registry,
                &key,
                raw(&path)?.as_deref(),
                seeds.get(&key).map_or(&[], Vec::as_slice),
            )?;
        }
        save(&registry)?;
        if requires_fence {
            activate_prepared(&mut registry)?;
        } else {
            for key in &keys {
                let path = prefs_path(key)?;
                publish(&registry.sources[key], &path)?;
            }
        }
        return Ok(());
    }
    if source_keys()?.iter().any(|key| !keys.contains(key)) {
        drop(registry_lock);
        drop(sources);
        drop(session_leases);
        return ensure_active_for_source(source);
    }
    let imports_lock = crate::agent_registry_import::lock_for_registry()?;
    let mut imports = crate::agent_registry_import::load()?.unwrap_or_default();
    let mut registry = Registry {
        schema_version: 1,
        phase: RegistryPhase::Prepared,
        revision: 1,
        roster: None,
        profiles: BTreeMap::new(),
        sources: BTreeMap::new(),
    };
    for key in &keys {
        let observed = raw(&prefs_path(key)?)?;
        let imported = imports.sources.get(key).cloned();
        let source_raw = imported
            .as_ref()
            .map(|source| source.preferences_json.clone())
            .or(observed);
        let aliases = if let Some(imported) = imported {
            imported.member_aliases
        } else if decode(source_raw.as_deref())?.agent_roster.is_some() {
            crate::agent_registry_import::stage_locked(
                &mut imports,
                key,
                source_raw.as_deref().expect("roster source"),
                false,
            )?
            .member_aliases
        } else {
            BTreeMap::new()
        };
        add_source(
            &mut registry,
            key,
            source_raw.as_deref(),
            aliases,
            seeds.get(key).map_or(&[], Vec::as_slice),
        )?;
    }
    if let Some(roster) = &mut registry.roster {
        roster.revision = 1;
    }
    save(&registry)?;
    drop(imports_lock);
    activate_prepared(&mut registry)
}

pub(crate) fn inactive_import_guard() -> io::Result<NamedFileLock> {
    let guard = lock()?;
    if load()?.is_some() {
        return Err(invalid("Agent registry is prepared or active; use agent.registry.status/reconcile instead of inactive imports"));
    }
    Ok(guard)
}

pub fn status(source_key: Option<&str>) -> io::Result<RegistryStatus> {
    if let Some(key) = source_key {
        ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?;
    }
    let Some(registry) = load()? else {
        return Ok(RegistryStatus {
            phase: RegistryPhase::Inactive,
            registry_revision: 0,
            projects: Vec::new(),
            assigned_members: 0,
            unknown_assignments: 0,
        });
    };
    let mut projects = Vec::new();
    for (key, source) in &registry.sources {
        if source_key.is_some_and(|requested| requested != key) {
            continue;
        }
        let path = prefs_path(key)?;
        let observed = raw_hash(raw(&path)?.as_deref());
        let floor_observed = raw_hash(raw(&path.with_extension("committed.json"))?.as_deref());
        let pending = observed == source.before_sha256 && observed != source.after_sha256;
        let conflict = observed != source.before_sha256 && observed != source.after_sha256;
        let floor_pending =
            floor_observed == source.floor_before_sha256 && floor_observed != source.after_sha256;
        let floor_conflict =
            floor_observed != source.floor_before_sha256 && floor_observed != source.after_sha256;
        let recovery_action = (conflict || floor_conflict).then_some(
            if registry.phase == RegistryPhase::Prepared
                && !source.was_active
                && observed != source.after_sha256
            {
                RegistryReconcileAction::RefreshPreparedSource
            } else {
                RegistryReconcileAction::QuarantineReplayMirror
            },
        );
        projects.push(RegistryProjectStatus {
            source_key: key.clone(),
            transaction_id: source.transaction_id.clone(),
            before_sha256: source.before_sha256.clone(),
            after_sha256: source.after_sha256.clone(),
            observed_sha256: observed,
            floor_before_sha256: source.floor_before_sha256.clone(),
            floor_observed_sha256: floor_observed,
            floor_pending,
            floor_conflict,
            pending,
            conflict,
            recovery_action,
        });
    }
    let assignments: Vec<_> = registry
        .roster
        .as_ref()
        .into_iter()
        .flat_map(|roster| &roster.members)
        .filter_map(|member| member.assignment.as_ref())
        .collect();
    Ok(RegistryStatus {
        phase: registry.phase,
        registry_revision: registry.revision,
        projects,
        assigned_members: assignments.len(),
        unknown_assignments: assignments
            .iter()
            .filter(|assignment| {
                assignment.host_id.is_empty()
                    || assignment.host_id != crate::process::current_hostname()
                    || assignment.host_started_at.is_none()
                    || assignment.session_id.as_ref().is_none_or(|id| {
                        gwt_agent::Session::load(
                            &gwt_core::paths::gwt_sessions_dir().join(format!("{id}.toml")),
                        )
                        .is_err()
                    })
            })
            .count(),
    })
}

pub fn reconcile(request: &RegistryReconcileRequest) -> io::Result<RegistryStatus> {
    let key = ProjectKey::parse(&request.source_key).map_err(|error| invalid(error.to_string()))?;
    let snapshot = load()?.ok_or_else(|| invalid("Agent registry is inactive"))?;
    let keys = if snapshot.phase == RegistryPhase::Prepared {
        snapshot.sources.keys().cloned().collect()
    } else {
        BTreeSet::from([key.to_string()])
    };
    // Refresh imports current, physically observed Sessions, so acquire their
    // leases before any source/registry locks and retain them through all fences.
    let fresh = matches!(
        request.action,
        RegistryReconcileAction::RefreshPreparedSource
    )
    .then(leased_seeds)
    .transpose()?;
    let _sources = source_locks(&keys)?;
    let _registry = lock()?;
    if let Some((seeds, _)) = &fresh {
        if crate::agent_names::observed_project_member_seeds(&gwt_core::paths::gwt_sessions_dir())
            .map_err(io::Error::other)?
            != *seeds
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Current named Sessions changed during Agent recovery; retry the same operation",
            ));
        }
    }
    let mut registry = load()?.ok_or_else(|| invalid("Agent registry disappeared"))?;
    let source = registry
        .sources
        .get(key.as_str())
        .ok_or_else(|| invalid("Agent source is not enrolled"))?
        .clone();
    let path = prefs_path(key.as_str())?;
    let observed = raw(&path)?;
    let floor_path = path.with_extension("committed.json");
    let floor = raw(&floor_path)?;
    let floor_hash = raw_hash(floor.as_deref());
    let floor_unknown =
        floor_hash != source.floor_before_sha256 && floor_hash != source.after_sha256;
    if registry.phase != request.expected_phase
        || registry.revision != request.expected_registry_revision
        || source.transaction_id != request.expected_transaction_id
        || raw_hash(observed.as_deref()) != request.expected_source_sha256
    {
        return Err(invalid(
            "Agent recovery observation changed; read agent.registry.status and retry",
        ));
    }
    if request
        .expected_floor_sha256
        .as_ref()
        .is_some_and(|expected| expected != &floor_hash)
        || floor_unknown && request.expected_floor_sha256.is_none()
    {
        return Err(invalid(format!("Agent recovery floor observation changed or is missing: observed={floor_hash}; read agent.registry.status and supply expected_floor_sha256")));
    }
    confirm_durable()?;
    match request.action {
        RegistryReconcileAction::RefreshPreparedSource => {
            if registry.phase != RegistryPhase::Prepared
                || source.was_active
                || decode(observed.as_deref())?.schema_version != 1
            {
                return Err(invalid(
                    "refresh_prepared_source requires a new, never-Active prepared schema1 source",
                ));
            }
            let observed = observed
                .as_deref()
                .ok_or_else(|| invalid("Cannot refresh a missing Agent source"))?;
            let seeds = fresh
                .as_ref()
                .and_then(|(seeds, _)| seeds.get(key.as_str()))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let refreshed = decode(Some(observed))?;
            if let Some(roster) = &refreshed.agent_roster {
                // A fresh schema1 roster cannot discard an already recorded
                // current live identity or replace its exact claim. Refuse
                // before changing either migration ledger or canonical bytes.
                for seed in seeds {
                    let Some(global) = source.aliases.get(&seed.agent_member_id) else {
                        return Err(io::Error::new(io::ErrorKind::WouldBlock,
                            "Refreshed Agent roster does not retain a current source identity; restore its recorded member and exact assignment before retrying"));
                    };
                    let current = registry
                        .roster
                        .as_ref()
                        .and_then(|roster| {
                            roster
                                .members
                                .iter()
                                .find(|member| &member.agent_member_id == global)
                        })
                        .ok_or_else(|| {
                            invalid("Current Agent source proof has no recorded member")
                        })?;
                    let retained = roster.members.iter().any(|member| {
                        if source.aliases.get(&member.agent_member_id) != Some(global)
                            || member.origin_session_id != current.origin_session_id
                        {
                            return false;
                        }
                        let mut assignment = member.assignment.clone();
                        if let Some(assignment) = &mut assignment {
                            assignment.project_key = Some(key.to_string());
                        }
                        assignment == current.assignment
                            && member.agent_id.as_deref().is_none_or(|tool| {
                                gwt_agent::resolve_agent_id(tool).is_some_and(|tool| {
                                    current.agent_id.as_deref() == Some(tool.command())
                                })
                            })
                    });
                    if !retained {
                        return Err(io::Error::new(io::ErrorKind::WouldBlock,
                            "Refreshed Agent roster would drop a current source identity or assignment; restore its recorded member and exact assignment before retrying"));
                    }
                }
            }
            if floor_unknown {
                if let Some(floor) = &floor {
                    quarantine(&floor_path, floor)?;
                }
            }
            let _imports = crate::agent_registry_import::lock_for_registry()?;
            let mut imports = crate::agent_registry_import::load()?.unwrap_or_default();
            let mut aliases = if !source.rosterless_snapshots.is_empty() {
                // These identities were committed directly with rosterless
                // live provenance. Preserve that lineage in the same registry;
                // an inactive import would allocate a second UUID for them.
                let mut aliases = source.aliases.clone();
                if let Some(roster) = &refreshed.agent_roster {
                    roster.validate().map_err(invalid)?;
                    for member in &roster.members {
                        aliases
                            .entry(member.agent_member_id.clone())
                            .or_insert_with(|| uuid::Uuid::new_v4().to_string());
                    }
                }
                aliases
            } else if refreshed.agent_roster.is_some() {
                crate::agent_registry_import::stage_locked(
                    &mut imports,
                    key.as_str(),
                    observed,
                    true,
                )?
                .member_aliases
            } else {
                source.aliases.clone()
            };
            for (old, global) in &source.aliases {
                if aliases.get(old).is_some_and(|id| id != global) {
                    return Err(invalid("Refreshed Agent source changed an immutable identity alias; retain the recorded source identity before retrying"));
                }
                aliases.insert(old.clone(), global.clone());
            }
            let retired: BTreeSet<_> = source.aliases.values().cloned().collect();
            if let Some(roster) = &mut registry.roster {
                roster
                    .members
                    .retain(|member| !retired.contains(&member.agent_member_id));
            }
            registry.profiles.retain(|id, _| !retired.contains(id));
            add_source(&mut registry, key.as_str(), Some(observed), aliases, seeds)?;
            registry.revision = registry
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("Agent registry revision exhausted"))?;
            save(&registry)?;
            drop(_imports);
            activate_prepared(&mut registry)?;
        }
        RegistryReconcileAction::QuarantineReplayMirror => {
            if registry.phase != RegistryPhase::Active
                && !(registry.phase == RegistryPhase::Prepared
                    && (source.was_active || raw_hash(observed.as_deref()) == source.after_sha256))
            {
                return Err(invalid(
                    "quarantine_replay_mirror requires Active or a Prepared source with its verified canonical fence",
                ));
            }
            for (file, contents, unknown) in [
                (
                    &path,
                    &observed,
                    raw_hash(observed.as_deref()) != source.before_sha256
                        && raw_hash(observed.as_deref()) != source.after_sha256,
                ),
                (&floor_path, &floor, floor_unknown),
            ] {
                if unknown {
                    if let Some(contents) = contents {
                        quarantine(file, contents)?;
                    }
                }
            }
            publish_verified(&source, &path)?;
            if registry.phase == RegistryPhase::Prepared {
                activate_prepared(&mut registry)?;
            }
        }
    }
    drop(_registry);
    status(Some(key.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};

    fn source() -> (PathBuf, PathBuf, String) {
        let root = gwt_core::paths::gwt_home().join("repo");
        fs::create_dir_all(&root).unwrap();
        let path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut prefs = serde_json::to_value(IssueMonitorPrefs::default()).unwrap();
        prefs["agent_roster"] = serde_json::json!({"schema_version":1,"revision":1,"members":[{
            "agent_member_id":"a4263a32-14e9-4bb0-ab78-ec3d4816e811", "agent_id":"codex",
            "name":"Cedar", "enabled":true, "created_at":"2026-10-01T00:00:00Z"
        }]});
        let raw = serde_json::to_string_pretty(&prefs).unwrap();
        fs::write(&path, &raw).unwrap();
        fs::write(path.with_extension("committed.json"), &raw).unwrap();
        (root, path, raw)
    }

    fn recovery_request(status: &RegistryStatus, floor: &str) -> RegistryReconcileRequest {
        let row = &status.projects[0];
        serde_json::from_value(serde_json::json!({
            "expected_phase": status.phase,
            "expected_registry_revision": status.registry_revision,
            "source_key": row.source_key,
            "expected_transaction_id": row.transaction_id,
            "expected_source_sha256": row.observed_sha256,
            "expected_floor_sha256": floor,
            "action": "quarantine_replay_mirror"
        }))
        .unwrap()
    }

    #[test]
    fn project_only_commits_do_not_invalidate_global_configuration_observations() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, a_path, _) = source();
        let b_root = home.path().join("project-b");
        fs::create_dir_all(&b_root).unwrap();
        let b_path = crate::issue_monitor_prefs_path_for_repo_path(&b_root);
        fs::create_dir_all(b_path.parent().unwrap()).unwrap();
        fs::write(
            &b_path,
            serde_json::to_vec(&IssueMonitorPrefs::default()).unwrap(),
        )
        .unwrap();
        ensure_active_for_project(&root).unwrap();
        let before = crate::load_issue_monitor_prefs(&a_path).unwrap();
        let durable_generation = status(None).unwrap().registry_revision;
        crate::try_mutate_issue_monitor_prefs(&b_path, |prefs| {
            prefs.merged_issues.push(77);
            Ok(())
        })
        .unwrap();
        let after = crate::load_issue_monitor_prefs(&a_path).unwrap();
        assert_eq!(
            after.config_token(),
            before.config_token(),
            "another project's runtime history does not change saved global configuration"
        );
        assert_eq!(
            after.agent_roster.as_ref().unwrap().revision,
            before.agent_roster.as_ref().unwrap().revision
        );
        assert!(
            status(None).unwrap().registry_revision > durable_generation,
            "the durable commit generation still advances for exact replay and recovery CAS"
        );
        let id = after.agent_roster.as_ref().unwrap().members[0]
            .agent_member_id
            .clone();
        crate::agent_names::mutate_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir(),
            crate::agent_names::AgentRosterAction::Rename {
                member_id: id,
                name: "New Cedar".into(),
            },
            None,
        )
        .unwrap();
        assert_ne!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .config_token(),
            before.config_token(),
            "an actual global edit must invalidate the observation"
        );
    }

    #[test]
    fn selecting_a_global_person_with_sparse_patch_keeps_omitted_saved_core() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, a_path, _) = source();
        let mut a: serde_json::Value = serde_json::from_slice(&fs::read(&a_path).unwrap()).unwrap();
        a["launch_profiles"] = serde_json::json!([{
            "agent_id":"codex", "agent_member_id":"a4263a32-14e9-4bb0-ab78-ec3d4816e811",
            "model":"saved-global-model", "reasoning":"high", "version":"saved-version", "fast_mode":true
        }]);
        fs::write(&a_path, serde_json::to_vec(&a).unwrap()).unwrap();
        let b_root = home.path().join("project-b");
        fs::create_dir_all(&b_root).unwrap();
        let b_path = crate::issue_monitor_prefs_path_for_repo_path(&b_root);
        fs::create_dir_all(b_path.parent().unwrap()).unwrap();
        fs::write(
            &b_path,
            serde_json::to_vec(&IssueMonitorPrefs::default()).unwrap(),
        )
        .unwrap();
        ensure_active_for_project(&root).unwrap();
        let b = crate::load_issue_monitor_prefs(&b_path).unwrap();
        let id = &b.agent_roster.as_ref().unwrap().members[0].agent_member_id;
        let omitted =
            serde_json::from_value(serde_json::json!({"agent_id":"codex", "agent_member_id":id}))
                .unwrap();
        let request =
            crate::issue_monitor::IssueMonitorProfilesSetRequest::capture(&b, vec![omitted])
                .unwrap();
        let (selected, _) = request.resolve(&b).unwrap();
        assert_eq!(
            selected[0].model.as_deref(),
            Some("saved-global-model"),
            "choosing an unselected global person does not clear its omitted tuning"
        );
        assert_eq!(selected[0].reasoning.as_deref(), Some("high"));
        assert_eq!(selected[0].version.as_deref(), Some("saved-version"));
        assert!(selected[0].fast_mode);
        assert!(
            !selected[0].skip_permissions,
            "execution permissions remain this project's defaults"
        );
        let explicit = serde_json::from_value(serde_json::json!({
            "agent_id":"codex", "agent_member_id":id, "model":null, "reasoning":null,
            "version":null, "fast_mode":false
        }))
        .unwrap();
        let request =
            crate::issue_monitor::IssueMonitorProfilesSetRequest::capture(&b, vec![explicit])
                .unwrap();
        let (cleared, _) = request.resolve(&b).unwrap();
        assert_eq!(cleared[0].model, None);
        assert_eq!(cleared[0].reasoning, None);
        assert_eq!(cleared[0].version, None);
        assert!(!cleared[0].fast_mode);
    }

    #[test]
    fn legacy_provider_pool_keeps_each_persons_original_tool_and_core() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, path, _) = source();
        let mut prefs: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        prefs["agent_roster"]["members"][0]
            .as_object_mut()
            .unwrap()
            .remove("agent_id");
        prefs["agent_roster"]["members"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "agent_member_id":"b4263a32-14e9-4bb0-ab78-ec3d4816e811", "name":"Aspen",
                "enabled":true, "created_at":"2026-10-01T00:00:00Z"
            }));
        prefs["max_active_agents"] = serde_json::json!(2);
        prefs["launch_profiles"] = serde_json::json!([
            {"agent_id":"codex", "model":"codex-model", "reasoning":"high"},
            {"agent_id":"claude", "model":"claude-model", "fast_mode":true}
        ]);
        fs::write(&path, serde_json::to_vec(&prefs).unwrap()).unwrap();
        ensure_active_for_project(&root).unwrap();
        let projected = crate::load_issue_monitor_prefs(&path).unwrap();
        let members = &projected.agent_roster.as_ref().unwrap().members;
        assert_eq!(
            members
                .iter()
                .map(|member| member.agent_id.as_deref())
                .collect::<Vec<_>>(),
            [Some("codex"), Some("claude")],
            "the existing provider-pool migration does not turn everyone into its first tool"
        );
        let pool = projected.launch_profile_pool();
        assert_eq!(
            pool.len(),
            2,
            "migration keeps candidate cardinality and order"
        );
        for (profile, member) in pool.iter().zip(members) {
            assert_eq!(
                profile.agent_member_id.as_deref(),
                Some(member.agent_member_id.as_str())
            );
        }
        assert_eq!(pool[0].model.as_deref(), Some("codex-model"));
        assert_eq!(pool[1].model.as_deref(), Some("claude-model"));
        assert!(pool[1].fast_mode);
    }

    #[test]
    fn legacy_unbound_manual_core_and_rosterless_defaults_survive_activation() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, path, _) = source();
        let mut prefs: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        prefs["launch_profiles"] = serde_json::json!([{
            "agent_id":"codex", "model":"legacy-model", "reasoning":"high",
            "version":"legacy-version", "fast_mode":true, "skip_permissions":true
        }]);
        fs::write(&path, serde_json::to_vec_pretty(&prefs).unwrap()).unwrap();
        ensure_active_for_project(&root).unwrap();
        let observed = crate::load_issue_monitor_prefs(&path).unwrap();
        let profile = &observed.launch_profile_pool()[0];
        assert_eq!(
            profile.model.as_deref(),
            Some("legacy-model"),
            "binding a source person must precede extracting its manual core"
        );
        assert_eq!(profile.reasoning.as_deref(), Some("high"));
        assert_eq!(profile.version.as_deref(), Some("legacy-version"));
        assert!(profile.fast_mode && profile.skip_permissions);

        let empty_home = tempfile::tempdir().unwrap();
        let _empty_home = ScopedGwtHome::set(empty_home.path());
        let empty_root = empty_home.path().join("rosterless");
        fs::create_dir_all(&empty_root).unwrap();
        let empty_path = crate::issue_monitor_prefs_path_for_repo_path(&empty_root);
        fs::create_dir_all(empty_path.parent().unwrap()).unwrap();
        let mut prefs = serde_json::to_value(IssueMonitorPrefs::default()).unwrap();
        prefs["launch_profiles"] = serde_json::json!([{"agent_id":"codex", "model":"first-person-model", "reasoning":"high", "fast_mode":true}]);
        fs::write(&empty_path, serde_json::to_vec(&prefs).unwrap()).unwrap();
        ensure_active_for_project(&empty_root).unwrap();
        let unbound = crate::load_issue_monitor_prefs(&empty_path).unwrap();
        assert_eq!(
            unbound.launch_profile_pool()[0].model.as_deref(),
            Some("first-person-model")
        );
        crate::agent_names::read_project_roster(&empty_root, &gwt_core::paths::gwt_sessions_dir())
            .unwrap();
        let bound = crate::load_issue_monitor_prefs(&empty_path).unwrap();
        assert!(bound.launch_profile_pool()[0].agent_member_id.is_some());
        assert_eq!(
            bound.launch_profile_pool()[0].model.as_deref(),
            Some("first-person-model")
        );
        assert!(bound.launch_profile_pool()[0].fast_mode);
    }

    #[test]
    fn registry_commit_parent_sync_failure_needs_confirmation_before_noop_ack() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, project_path, _) = source();
        ensure_active_for_project(&root).unwrap();
        let registry_path = super::path();
        let marker = registry_path.with_extension("parent-sync-fail-once");
        let mut failure = None;
        assert!(
            crate::try_mutate_issue_monitor_prefs(&project_path, |prefs| {
                // Inject after acquisition so the first failure is the real commit rename.
                fs::write(&marker, b"fail once")?;
                failure = Some(ScopedEnvVar::set(
                    "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
                    &registry_path,
                ));
                prefs.agent_roster.as_mut().unwrap().members[0].name = "Committed Cedar".into();
                Ok(())
            })
            .is_err()
        );
        let committed = fs::read(&registry_path).unwrap();
        fs::write(&marker, b"fail confirmation once").unwrap();
        assert!(
            crate::try_mutate_issue_monitor_prefs(&project_path, |_| Ok(())).is_err(),
            "a retry cannot ACK an unchanged logical value before confirming registry durability"
        );
        assert_eq!(fs::read(&registry_path).unwrap(), committed);
        let (replayed, ()) =
            crate::try_mutate_issue_monitor_prefs(&project_path, |_| Ok(())).unwrap();
        assert_eq!(
            replayed.agent_roster.unwrap().members[0].name,
            "Committed Cedar"
        );
        assert_eq!(fs::read(&registry_path).unwrap(), committed);
        drop(failure);
    }

    #[test]
    fn recovery_floor_conflicts_require_exact_status_cas_and_preserve_quarantine_bytes() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, path, original) = source();
        ensure_active_for_project(&root).unwrap();
        let registry_before = fs::read(super::path()).unwrap();
        let canonical_before = fs::read(&path).unwrap();
        let floor = path.with_extension("committed.json");
        let unknown = "unknown recovery floor\n";
        fs::write(&floor, unknown).unwrap();
        let observed = status(None).unwrap();
        let status_json = serde_json::to_value(&observed).unwrap();
        assert_eq!(
            status_json["projects"][0]["floor_conflict"], true,
            "status must expose the unknown recovery floor instead of silently replacing it"
        );
        assert!(crate::load_issue_monitor_prefs(&path).is_err());
        assert!(crate::agent_names::read_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir()
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), canonical_before);
        assert_eq!(fs::read_to_string(&floor).unwrap(), unknown);
        let stale = recovery_request(&observed, "stale-floor-observation");
        assert!(reconcile(&stale).is_err());
        assert_eq!(fs::read(super::path()).unwrap(), registry_before);
        assert_eq!(fs::read_to_string(&floor).unwrap(), unknown);
        let exact = recovery_request(&observed, &fingerprint(unknown));
        let restored = reconcile(&exact).unwrap();
        assert_eq!(restored.phase, RegistryPhase::Active);
        assert_eq!(fs::read(&floor).unwrap(), canonical_before);
        let quarantines: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("registry-quarantine")
            })
            .collect();
        assert!(quarantines
            .iter()
            .any(|path| fs::read_to_string(path).unwrap() == unknown));
        // A known before generation is legitimate replay, not an unknown conflict.
        fs::write(&path, &original).unwrap();
        fs::write(&floor, original).unwrap();
        crate::agent_names::read_project_roster(&root, &gwt_core::paths::gwt_sessions_dir())
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), canonical_before);
        assert_eq!(fs::read(&floor).unwrap(), canonical_before);
    }

    #[test]
    fn prepared_fenced_source_has_explicit_floor_recovery_before_activation() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, path, _) = source();
        let floor = path.with_extension("committed.json");
        let fail_once = floor.with_extension("parent-sync-fail-once");
        fs::write(&fail_once, b"fail once").unwrap();
        let failure =
            ScopedEnvVar::set("GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE", &floor);
        assert!(ensure_active_for_project(&root).is_err());
        drop(failure);
        let before = status(None).unwrap();
        assert_eq!(before.phase, RegistryPhase::Prepared);
        let canonical: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(canonical["schema_version"], 2);
        let unknown = "unknown floor after the canonical fence\n";
        fs::write(&floor, unknown).unwrap();
        let observed = status(None).unwrap();
        let status_json = serde_json::to_value(&observed).unwrap();
        assert_eq!(status_json["projects"][0]["floor_conflict"], true);
        assert!(ensure_active_for_project(&root).is_err());
        assert_eq!(status(None).unwrap().phase, RegistryPhase::Prepared);
        assert!(reconcile(&recovery_request(&observed, "stale")).is_err());
        let restored = reconcile(&recovery_request(&observed, &fingerprint(unknown))).unwrap();
        assert_eq!(restored.phase, RegistryPhase::Active);
        assert_eq!(fs::read(&path).unwrap(), fs::read(&floor).unwrap());
        let quarantine = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("registry-quarantine")
            })
            .expect("the explicitly replaced floor remains quarantined");
        assert_eq!(fs::read_to_string(quarantine).unwrap(), unknown);
    }
    #[test]
    fn late_legacy_enrollment_fences_before_active_and_cannot_refresh_an_old_active_source() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_root, a_path, a_original) = source();
        ensure_active_for_project(&a_root).unwrap();
        let a_committed = fs::read(&a_path).unwrap();
        let b_root = home.path().join("late-project-b");
        fs::create_dir_all(&b_root).unwrap();
        let b_path = crate::issue_monitor_prefs_path_for_repo_path(&b_root);
        fs::create_dir_all(b_path.parent().unwrap()).unwrap();
        let source_bytes = serde_json::to_vec(&IssueMonitorPrefs::default()).unwrap();
        fs::write(&b_path, &source_bytes).unwrap();
        let b_floor = b_path.with_extension("committed.json");
        fs::write(&b_floor, source_bytes).unwrap();
        fs::write(
            b_floor.with_extension("parent-sync-fail-once"),
            b"fail once",
        )
        .unwrap();
        let failure = ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            &b_floor,
        );
        assert!(ensure_active_for_project(&b_root).is_err());
        drop(failure);
        assert_eq!(
            status(None).unwrap().phase,
            RegistryPhase::Prepared,
            "a late legacy source cannot grant common admission before both schema fences finish"
        );
        let mut drift: serde_json::Value = serde_json::from_str(&a_original).unwrap();
        drift["enabled"] = serde_json::json!(true);
        let drift = serde_json::to_string(&drift).unwrap();
        fs::write(&a_path, &drift).unwrap();
        let key = source_key(&a_path).unwrap();
        let observed = status(Some(&key)).unwrap();
        assert_eq!(
            observed.projects[0].recovery_action,
            Some(RegistryReconcileAction::QuarantineReplayMirror)
        );
        let floor_hash = raw_hash(
            raw(&a_path.with_extension("committed.json"))
                .unwrap()
                .as_deref(),
        );
        let mut request = recovery_request(&observed, &floor_hash);
        request.action = RegistryReconcileAction::RefreshPreparedSource;
        let before = fs::read(super::path()).unwrap();
        assert!(
            reconcile(&request).is_err(),
            "Prepared does not turn an old Active source back into an import authority"
        );
        assert_eq!(fs::read(super::path()).unwrap(), before);
        assert_eq!(fs::read_to_string(&a_path).unwrap(), drift);
        request.action = RegistryReconcileAction::QuarantineReplayMirror;
        assert_eq!(reconcile(&request).unwrap().phase, RegistryPhase::Active);
        assert_eq!(fs::read(&a_path).unwrap(), a_committed);
        assert_eq!(fs::read(&b_path).unwrap(), fs::read(&b_floor).unwrap());
        let quarantine = fs::read_dir(a_path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("registry-quarantine")
            })
            .unwrap();
        assert_eq!(fs::read_to_string(quarantine).unwrap(), drift);
    }

    #[test]
    fn ordinary_cli_writer_preflights_late_source_before_taking_its_prefs_lock() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_root, a_path, _) = source();
        ensure_active_for_project(&a_root).unwrap();
        let a_token = crate::load_issue_monitor_prefs(&a_path)
            .unwrap()
            .config_token();
        let b_root = home.path().join("late-project-b");
        fs::create_dir_all(&b_root).unwrap();
        let b_path = crate::issue_monitor_prefs_path_for_repo_path(&b_root);
        fs::create_dir_all(b_path.parent().unwrap()).unwrap();
        let original = serde_json::to_vec(&IssueMonitorPrefs::default()).unwrap();
        fs::write(&b_path, &original).unwrap();
        let before = fs::read(super::path()).unwrap();
        let mut called = false;
        gwt_agent::with_session_path_lease(
            &gwt_core::paths::gwt_sessions_dir(),
            "late-enrollment-guard",
            |_| {
                let result = crate::try_mutate_issue_monitor_prefs(&b_path, |_| {
                    called = true;
                    Ok(())
                });
                assert!(
                    result.is_err(),
                    "unknown legacy enrollment must wait until the held Session lease is released"
                );
                Ok(())
            },
        )
        .unwrap();
        assert!(!called);
        assert_eq!(fs::read(super::path()).unwrap(), before);
        assert_eq!(fs::read(&b_path).unwrap(), original);
        let (saved, ()) = crate::try_mutate_issue_monitor_prefs(&b_path, |prefs| {
            prefs.enabled = true;
            Ok(())
        })
        .unwrap();
        assert!(
            saved.enabled,
            "ordinary CLI writes enroll automatically without a GUI-only detour"
        );
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&b_path).unwrap()).unwrap();
        assert_eq!(raw["schema_version"], 2);
        assert_eq!(
            fs::read(&b_path).unwrap(),
            fs::read(b_path.with_extension("committed.json")).unwrap()
        );
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .config_token(),
            a_token,
            "enrolling a rosterless project does not edit global people or their saved core"
        );
    }

    // Insert these tests into agent_registry.rs::tests. They use only the existing
    // source fixture and existing durable-writer parent-sync fault injection.

    #[test]
    fn prepared_schema1_drift_requires_exact_refresh_before_all_sources_activate() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_root, a_path, a_raw) = source();
        let b_root = home.path().join("project-b");
        fs::create_dir_all(&b_root).unwrap();
        let b_path = crate::issue_monitor_prefs_path_for_repo_path(&b_root);
        fs::create_dir_all(b_path.parent().unwrap()).unwrap();
        let mut b: serde_json::Value = serde_json::from_str(&a_raw).unwrap();
        b["agent_roster"]["members"][0]["agent_member_id"] =
            serde_json::json!("b4263a32-14e9-4bb0-ab78-ec3d4816e811");
        b["agent_roster"]["members"][0]["name"] = serde_json::json!("Birch");
        let b_raw = serde_json::to_string_pretty(&b).unwrap();
        fs::write(&b_path, &b_raw).unwrap();
        fs::write(b_path.with_extension("committed.json"), &b_raw).unwrap();
        let (fenced, unfenced) = if source_key(&a_path) < source_key(&b_path) {
            (&a_path, &b_path)
        } else {
            (&b_path, &a_path)
        };
        // Stop after the first canonical rename; the next source remains schema1.
        let marker = fenced.with_extension("parent-sync-fail-once");
        fs::write(&marker, b"fail canonical sync once").unwrap();
        let failure =
            ScopedEnvVar::set("GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE", fenced);
        assert!(ensure_active_for_project(&a_root).is_err());
        drop(failure);
        assert_eq!(status(None).unwrap().phase, RegistryPhase::Prepared);
        let key = source_key(unfenced).unwrap();
        let aliases = load().unwrap().unwrap().sources[&key].aliases.clone();
        let unfenced_before = fs::read_to_string(unfenced).unwrap();
        let mut drift: serde_json::Value = serde_json::from_str(&unfenced_before).unwrap();
        assert_eq!(drift["schema_version"], 1);
        drift["agent_roster"]["revision"] = serde_json::json!(2);
        drift["agent_roster"]["members"][0]["name"] = serde_json::json!("Drifted Cedar");
        let drift = serde_json::to_string_pretty(&drift).unwrap();
        fs::write(unfenced, &drift).unwrap();
        let registry_before = fs::read(super::path()).unwrap();
        let fenced_before = fs::read(fenced).unwrap();
        assert!(ensure_active_for_project(&a_root).is_err());
        assert_eq!(fs::read(super::path()).unwrap(), registry_before);
        assert_eq!(fs::read(fenced).unwrap(), fenced_before);
        assert_eq!(fs::read_to_string(unfenced).unwrap(), drift);
        let observed = status(Some(&key)).unwrap();
        let row = &observed.projects[0];
        assert!(row.conflict);
        assert_eq!(
            row.recovery_action,
            Some(RegistryReconcileAction::RefreshPreparedSource)
        );
        let request = RegistryReconcileRequest {
            expected_phase: observed.phase,
            expected_registry_revision: observed.registry_revision,
            source_key: key.clone(),
            expected_transaction_id: row.transaction_id.clone(),
            expected_source_sha256: row.observed_sha256.clone(),
            expected_floor_sha256: Some(row.floor_observed_sha256.clone()),
            action: RegistryReconcileAction::RefreshPreparedSource,
        };
        let mut stale = request.clone();
        stale.expected_transaction_id.push_str("-stale");
        assert!(reconcile(&stale).is_err());
        assert_eq!(fs::read(super::path()).unwrap(), registry_before);
        assert_eq!(fs::read_to_string(unfenced).unwrap(), drift);
        assert_eq!(reconcile(&request).unwrap().phase, RegistryPhase::Active);
        let active = load().unwrap().unwrap();
        assert_eq!(active.sources[&key].aliases, aliases);
        let imported = crate::agent_registry_import::load().unwrap().unwrap();
        assert!(imported.sources[&key]
            .previous_snapshots
            .iter()
            .any(|snapshot| snapshot.preferences_json == unfenced_before));
        assert!(crate::load_issue_monitor_prefs(unfenced)
            .unwrap()
            .agent_roster
            .unwrap()
            .members
            .iter()
            .any(|member| member.name == "Drifted Cedar"));
        for path in [&a_path, &b_path] {
            for mirror in [path.clone(), path.with_extension("committed.json")] {
                let raw: serde_json::Value =
                    serde_json::from_slice(&fs::read(mirror).unwrap()).unwrap();
                assert_eq!(raw["schema_version"], 2);
            }
        }
    }

    #[test]
    fn active_unknown_canonical_requires_fresh_cas_and_quarantines_exact_bytes() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, path, _) = source();
        ensure_active_for_project(&root).unwrap();
        let registry_before = fs::read(super::path()).unwrap();
        let canonical_before = fs::read(&path).unwrap();
        let floor_path = path.with_extension("committed.json");
        let floor_before = fs::read(&floor_path).unwrap();
        let unknown = "unrecognized execution mirror\n";
        fs::write(&path, unknown).unwrap();
        let observed = status(None).unwrap();
        let row = &observed.projects[0];
        assert!(row.conflict);
        assert_eq!(
            row.recovery_action,
            Some(RegistryReconcileAction::QuarantineReplayMirror)
        );
        assert!(!serde_json::to_string(&observed)
            .unwrap()
            .contains("unrecognized execution mirror"));
        assert!(crate::load_issue_monitor_prefs(&path).is_err());
        assert!(crate::agent_names::read_project_roster(
            &root,
            &gwt_core::paths::gwt_sessions_dir()
        )
        .is_err());
        let request = recovery_request(&observed, &row.floor_observed_sha256);
        let mut stale = request.clone();
        stale.expected_registry_revision -= 1;
        assert!(reconcile(&stale).is_err());
        assert_eq!(fs::read(super::path()).unwrap(), registry_before);
        assert_eq!(fs::read_to_string(&path).unwrap(), unknown);
        assert_eq!(fs::read(&floor_path).unwrap(), floor_before);
        let changed_unknown = "changed execution mirror after status\n";
        fs::write(&path, changed_unknown).unwrap();
        assert!(
            reconcile(&request).is_err(),
            "an inspected source hash cannot authorize replacing newer bytes"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), changed_unknown);
        assert_eq!(fs::read(super::path()).unwrap(), registry_before);
        let fresh = status(None).unwrap();
        let exact = recovery_request(&fresh, &fresh.projects[0].floor_observed_sha256);
        assert_eq!(reconcile(&exact).unwrap().phase, RegistryPhase::Active);
        assert_eq!(fs::read(super::path()).unwrap(), registry_before);
        assert_eq!(fs::read(&path).unwrap(), canonical_before);
        assert_eq!(fs::read(&floor_path).unwrap(), floor_before);
        assert!(fs::read_dir(path.parent().unwrap()).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("registry-quarantine")
                && fs::read_to_string(path).unwrap() == changed_unknown
        }));
        assert!(crate::load_issue_monitor_prefs(&path).is_ok());
    }

    #[test]
    fn active_collision_import_keeps_same_names_and_source_qualified_old_ids_distinct() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_root, a_path, a_raw) = source();
        let b_root = home.path().join("project-b");
        fs::create_dir_all(&b_root).unwrap();
        let b_path = crate::issue_monitor_prefs_path_for_repo_path(&b_root);
        fs::create_dir_all(b_path.parent().unwrap()).unwrap();
        let old_id = "a4263a32-14e9-4bb0-ab78-ec3d4816e811";
        let mut originals = Vec::new();
        for (path, model) in [(&a_path, "model-a"), (&b_path, "model-b")] {
            let mut raw: serde_json::Value = serde_json::from_str(&a_raw).unwrap();
            raw["launch_profiles"] = serde_json::json!([{
                "agent_id":"codex", "agent_member_id":old_id, "model":model
            }]);
            let raw = serde_json::to_string_pretty(&raw).unwrap();
            fs::write(path, &raw).unwrap();
            fs::write(path.with_extension("committed.json"), &raw).unwrap();
            originals.push((source_key(path).unwrap(), raw));
        }
        ensure_active_for_project(&a_root).unwrap();
        let a_id = resolve_member_id(&a_root, old_id).unwrap();
        let b_id = resolve_member_id(&b_root, old_id).unwrap();
        assert_ne!(
            a_id, b_id,
            "same old UUIDs from different sources are different people"
        );
        assert_ne!(a_id, old_id);
        assert_ne!(b_id, old_id);
        let a = crate::load_issue_monitor_prefs(&a_path).unwrap();
        let b = crate::load_issue_monitor_prefs(&b_path).unwrap();
        assert_eq!(a.agent_roster, b.agent_roster);
        let members = &a.agent_roster.as_ref().unwrap().members;
        assert_eq!(members.len(), 2);
        assert!(members.iter().all(|member| member.name == "Cedar"));
        let a_profile = &a.launch_profile_pool()[0];
        let b_profile = &b.launch_profile_pool()[0];
        assert_eq!(a_profile.agent_member_id.as_ref(), Some(&a_id));
        assert_eq!(b_profile.agent_member_id.as_ref(), Some(&b_id));
        assert_eq!(a_profile.model.as_deref(), Some("model-a"));
        assert_eq!(b_profile.model.as_deref(), Some("model-b"));
        let imports = crate::agent_registry_import::load().unwrap().unwrap();
        for (key, original) in originals {
            assert_eq!(imports.sources[&key].preferences_json, original);
        }
        let committed = fs::read(super::path()).unwrap();
        ensure_active_for_project(&b_root).unwrap();
        assert_eq!(resolve_member_id(&a_root, old_id).unwrap(), a_id);
        assert_eq!(resolve_member_id(&b_root, old_id).unwrap(), b_id);
        assert_eq!(fs::read(super::path()).unwrap(), committed);
    }

    #[test]
    fn lock_free_reader_retries_when_two_commits_follow_its_source_read() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, path, _) = source();
        ensure_active_for_project(&root).unwrap();
        let before = crate::load_issue_monitor_prefs(&path).unwrap();
        let before_revision = status(None).unwrap().registry_revision;
        let committed = std::rc::Rc::new(std::cell::Cell::new(0));
        let committed_by_observer = committed.clone();
        let writer_path = path.clone();
        READ_PROJECT_AFTER_SOURCE.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some(Box::new(move || {
                for (index, name) in ["Cedar after one commit", "Cedar after two commits"]
                    .into_iter()
                    .enumerate()
                {
                    crate::try_mutate_issue_monitor_prefs(&writer_path, |prefs| {
                        prefs.agent_roster.as_mut().unwrap().members[0].name = name.into();
                        prefs.merged_issues.push(index as u64 + 100);
                        Ok(())
                    })
                    .unwrap();
                    committed_by_observer.set(committed_by_observer.get() + 1);
                }
            }));
        });
        let after = crate::load_issue_monitor_prefs(&path).unwrap();
        assert_eq!(committed.get(), 2);
        assert_eq!(status(None).unwrap().registry_revision, before_revision + 2);
        assert_eq!(
            after.agent_roster.as_ref().unwrap().members[0].name,
            "Cedar after two commits",
            "a reader retries its whole snapshot after crossing multiple commit generations",
        );
        assert_ne!(after.config_token(), before.config_token());
        READ_PROJECT_AFTER_SOURCE.with(|slot| assert!(slot.borrow().is_none()));
    }
    #[test]
    fn schema2_cutover_preserves_the_unmigrated_machine_manual_policy() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, source_path, _) = source();
        let mut old: serde_json::Value =
            serde_json::from_slice(&fs::read(&source_path).unwrap()).unwrap();
        old.as_object_mut().unwrap().remove("agent_roster");
        old["max_active_agents"] = serde_json::json!(5);
        old["max_active_agents_mode"] = serde_json::json!("manual");
        old["max_active_last_change"] = serde_json::json!({
            "changed_at":"2026-10-01T00:00:00Z", "source":"json_operation", "max_active_agents_override":5
        });
        fs::write(&source_path, serde_json::to_vec_pretty(&old).unwrap()).unwrap();
        assert!(crate::machine_agent_settings::load().unwrap().is_none());
        ensure_active_for_project(&root).unwrap();
        let policy = crate::machine_agent_settings::ensure_migrated().unwrap();
        assert_eq!(policy.mode, crate::issue_monitor::IssueMonitorMaxActiveMode::Manual,
            "the shared ownership fence must preserve legacy machine policy before schema2 excludes legacy discovery");
        assert_eq!(policy.manual_limit, Some(5));
        assert_eq!(
            policy
                .last_change
                .as_ref()
                .unwrap()
                .max_active_agents_override,
            Some(5)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&source_path).unwrap()).unwrap()
                ["schema_version"],
            2
        );
    }
    #[test]
    fn sparse_selection_fences_the_saved_core_outside_its_project_pool() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, a_path, _) = source();
        let mut a: serde_json::Value = serde_json::from_slice(&fs::read(&a_path).unwrap()).unwrap();
        a["launch_profiles"] = serde_json::json!([{
            "agent_id":"codex", "agent_member_id":"a4263a32-14e9-4bb0-ab78-ec3d4816e811", "model":"observed-core"
        }]);
        fs::write(&a_path, serde_json::to_vec(&a).unwrap()).unwrap();
        let b_root = home.path().join("project-b");
        fs::create_dir_all(&b_root).unwrap();
        let b_path = crate::issue_monitor_prefs_path_for_repo_path(&b_root);
        fs::create_dir_all(b_path.parent().unwrap()).unwrap();
        fs::write(
            &b_path,
            serde_json::to_vec(&IssueMonitorPrefs::default()).unwrap(),
        )
        .unwrap();
        ensure_active_for_project(&root).unwrap();
        let observed = crate::load_issue_monitor_prefs(&b_path).unwrap();
        assert!(observed.launch_profile_pool().is_empty());
        let id = &observed.agent_roster.as_ref().unwrap().members[0].agent_member_id;
        let patch =
            serde_json::from_value(serde_json::json!({"agent_id":"codex","agent_member_id":id}))
                .unwrap();
        let request =
            crate::issue_monitor::IssueMonitorProfilesSetRequest::capture(&observed, vec![patch])
                .unwrap();
        crate::try_mutate_issue_monitor_prefs(&b_path, |prefs| {
            prefs.merged_issues.push(77);
            Ok(())
        })
        .unwrap();
        assert!(
            request
                .resolve(&crate::load_issue_monitor_prefs(&b_path).unwrap())
                .is_ok(),
            "project runtime changes do not invalidate a saved-core observation"
        );
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            let mut profiles = prefs.launch_profile_pool();
            profiles[0].model = Some("new-global-core".into());
            prefs.set_launch_profile_pool(profiles);
            Ok(())
        })
        .unwrap();
        let latest = crate::load_issue_monitor_prefs(&b_path).unwrap();
        assert_eq!(latest.launch_profile_pool(), observed.launch_profile_pool());
        assert!(request.resolve(&latest).is_err(),
            "a daemon cannot ACK a new inherited core while the CLI reconstructs its response from the old core");
    }
    #[test]
    fn formal_provider_only_write_binds_before_committing_its_requested_core() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, a_path, _) = source();
        let mut a: serde_json::Value = serde_json::from_slice(&fs::read(&a_path).unwrap()).unwrap();
        a["launch_profiles"] = serde_json::json!([{
            "agent_id":"codex", "agent_member_id":"a4263a32-14e9-4bb0-ab78-ec3d4816e811",
            "model":"old-global-core", "skip_permissions":false
        }]);
        fs::write(&a_path, serde_json::to_vec(&a).unwrap()).unwrap();
        let b_root = home.path().join("project-b");
        fs::create_dir_all(&b_root).unwrap();
        let b_path = crate::issue_monitor_prefs_path_for_repo_path(&b_root);
        fs::create_dir_all(b_path.parent().unwrap()).unwrap();
        fs::write(
            &b_path,
            serde_json::to_vec(&IssueMonitorPrefs::default()).unwrap(),
        )
        .unwrap();
        ensure_active_for_project(&root).unwrap();
        let before_a = crate::load_issue_monitor_prefs(&a_path).unwrap();
        let a_bytes = fs::read(&a_path).unwrap();
        let person = before_a.launch_profile_pool()[0]
            .agent_member_id
            .clone()
            .unwrap();
        assert!(crate::load_issue_monitor_prefs(&b_path)
            .unwrap()
            .launch_profile_pool()
            .is_empty());
        let (saved, ()) = crate::try_mutate_issue_monitor_prefs(&b_path, |prefs| {
            let profile = serde_json::from_value(serde_json::json!({
                "agent_id":"codex", "model":"new-global-core", "skip_permissions":true
            }))
            .unwrap();
            prefs.set_launch_profile_pool(vec![profile]);
            Ok(())
        })
        .unwrap();
        let profile = &saved.launch_profile_pool()[0];
        assert_eq!(profile.model.as_deref(), Some("new-global-core"),
            "a successful canonical writer must not discard its requested model while binding the person");
        assert_eq!(profile.agent_member_id.as_ref(), Some(&person));
        assert!(profile.skip_permissions);
        let b = crate::load_issue_monitor_prefs(&b_path).unwrap();
        assert_eq!(b.launch_profile_pool(), saved.launch_profile_pool());
        let after_a = crate::load_issue_monitor_prefs(&a_path).unwrap();
        assert_eq!(
            after_a.launch_profile_pool()[0].model.as_deref(),
            Some("new-global-core")
        );
        assert!(
            !after_a.launch_profile_pool()[0].skip_permissions,
            "another project's execution permission is never part of the common saved core"
        );
        assert_eq!(fs::read(&a_path).unwrap(), a_bytes);
    }
}
