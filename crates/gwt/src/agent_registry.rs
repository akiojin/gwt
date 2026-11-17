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
    agent_names::{AgentAssignmentRole, AgentMember, AgentRoster, AgentRosterAction},
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
    #[serde(default)]
    pub codex_speed: gwt_agent::CodexSpeed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_target: Option<gwt_agent::LaunchRuntimeTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_permissions: Option<bool>,
}

impl AgentProfileCore {
    fn from_profile(profile: &IssueMonitorLaunchProfile) -> Self {
        Self {
            agent_id: profile.agent_id.clone(),
            model: profile.model.clone(),
            reasoning: profile.reasoning.clone(),
            version: profile.version.clone(),
            fast_mode: profile.fast_mode,
            codex_speed: profile.codex_speed,
            runtime_target: None,
            skip_permissions: None,
        }
    }

    fn apply(&self, profile: &mut IssueMonitorLaunchProfile) {
        profile.agent_id.clone_from(&self.agent_id);
        profile.model.clone_from(&self.model);
        profile.reasoning.clone_from(&self.reasoning);
        profile.version.clone_from(&self.version);
        profile.fast_mode = self.fast_mode;
        profile.codex_speed = self.codex_speed;
        if let Some(runtime_target) = self.runtime_target {
            profile.runtime_target = runtime_target;
        }
        if let Some(skip_permissions) = self.skip_permissions {
            profile.skip_permissions = skip_permissions;
        }
    }

    fn retaining_overrides(mut self, previous: Option<&Self>) -> Self {
        if let Some(previous) = previous {
            self.runtime_target = previous.runtime_target;
            self.skip_permissions = previous.skip_permissions;
        }
        self
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
    #[serde(default)]
    project_assignments:
        BTreeMap<String, BTreeMap<String, crate::agent_names::AgentMemberAssignment>>,
    #[serde(default)]
    pm_projects: BTreeMap<String, PmProjectIdentity>,
    #[serde(skip)]
    store: Store,
}

/// Where this in-memory registry lives on disk.
///
/// Schema 3 keeps every project's mirror inside `agent-registry.json`.
/// Split storage (schema 4) keeps only the shared core there and one record
/// file per project, so a project's transaction reads and writes its own
/// record and never another project's mirror.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Store {
    split: bool,
    /// Superseded snapshots were not loaded and must not be written back.
    partial: bool,
    /// Enrolled projects whose record this read did not load.
    unloaded: BTreeMap<String, CoreSource>,
}

/// The shared part of one enrolled project in split storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoreSource {
    was_active: bool,
    aliases: BTreeMap<String, String>,
    session_aliases: BTreeMap<String, String>,
    /// The record transaction the latest core write committed together with
    /// itself. A pending record is the committed generation only while this
    /// names it; otherwise the core write never happened and it is ignored.
    record_transaction_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Core {
    schema_version: u32,
    phase: RegistryPhase,
    revision: u64,
    roster: Option<AgentRoster>,
    profiles: BTreeMap<String, AgentProfileCore>,
    sources: BTreeMap<String, CoreSource>,
    project_assignments:
        BTreeMap<String, BTreeMap<String, crate::agent_names::AgentMemberAssignment>>,
    pm_projects: BTreeMap<String, PmProjectIdentity>,
}

/// One project's committed mirror generation in split storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRecord {
    transaction_id: String,
    before_sha256: String,
    floor_before_sha256: String,
    after_sha256: String,
    after_json: String,
}

impl SourceRecord {
    fn of(source: &Source) -> Self {
        Self {
            transaction_id: source.transaction_id.clone(),
            before_sha256: source.before_sha256.clone(),
            floor_before_sha256: source.floor_before_sha256.clone(),
            after_sha256: source.after_sha256.clone(),
            after_json: source.after_json.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordFile {
    schema_version: u32,
    source_key: String,
    /// Committed by its own durable write (a transaction that left the core unchanged).
    record: Option<SourceRecord>,
    /// Written ahead of a core write; committed only by that core write.
    pending: Option<SourceRecord>,
}

impl RecordFile {
    fn decode(key: &str, raw: &str) -> io::Result<Self> {
        let file: Self = serde_json::from_str(raw).map_err(io::Error::other)?;
        if file.schema_version != 1 || file.source_key != key {
            return Err(invalid(format!(
                "Agent source record for {key} belongs to another project or schema"
            )));
        }
        Ok(file)
    }

    fn committed(self, core_transaction_id: Option<&str>) -> Option<SourceRecord> {
        match self.pending {
            Some(pending) if Some(pending.transaction_id.as_str()) == core_transaction_id => {
                Some(pending)
            }
            _ => self.record,
        }
    }
}

const SPLIT_SCHEMA_VERSION: u32 = 4;

impl Registry {
    fn enrolled(&self, key: &str) -> bool {
        self.sources.contains_key(key) || self.store.unloaded.contains_key(key)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PmProjectIdentity {
    #[serde(default)]
    member_id: Option<String>,
    #[serde(default)]
    legacy_sessions: BTreeMap<String, PmLegacyMemberProof>,
    /// Issue #7542: PM assignments set aside by the NEW-launch quarantine
    /// fallback, keyed by member id. The record is the only place the
    /// displaced assignment survives; restoring it is an explicit operation
    /// (`restore_quarantined_pm_assignment`), never an automatic merge.
    #[serde(default)]
    quarantined_assignments: BTreeMap<String, PmQuarantinedAssignment>,
}

/// Issue #7542: a PM assignment parked by the quarantine fallback. The
/// assignment is preserved verbatim — reservation token, bound Session id,
/// reserving host — so the displaced conversation can later be restored
/// exactly; the bound Session's file is never touched by quarantining.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PmQuarantinedAssignment {
    assignment: crate::agent_names::AgentMemberAssignment,
    reason: String,
    quarantined_at_unix: u64,
}

fn project_roster(registry: &Registry, key: &str) -> Option<AgentRoster> {
    let mut roster = registry.roster.clone()?;
    let selected = registry
        .pm_projects
        .get(key)
        .and_then(|project| project.member_id.as_deref());
    for member in &mut roster.members {
        member.assignment = registry
            .project_assignments
            .get(key)
            .and_then(|assignments| assignments.get(&member.agent_member_id))
            .cloned();
        member.pm_project_key =
            (selected == Some(member.agent_member_id.as_str())).then(|| key.into());
    }
    Some(roster)
}

fn project_member(registry: &Registry, key: &str, id: &str) -> Option<AgentMember> {
    project_roster(registry, key)?
        .members
        .into_iter()
        .find(|member| member.agent_member_id == id)
}

fn selected_pm_member(registry: &Registry, key: &str) -> Option<AgentMember> {
    let id = registry.pm_projects.get(key)?.member_id.as_deref()?;
    project_member(registry, key, id)
}

fn project_assignment<'a>(
    registry: &'a Registry,
    key: &str,
    id: &str,
) -> Option<&'a crate::agent_names::AgentMemberAssignment> {
    registry.project_assignments.get(key)?.get(id)
}

fn set_project_assignment(
    registry: &mut Registry,
    key: &str,
    id: &str,
    assignment: Option<crate::agent_names::AgentMemberAssignment>,
) {
    if let Some(assignment) = assignment {
        registry
            .project_assignments
            .entry(key.into())
            .or_default()
            .insert(id.into(), assignment);
    } else if let Some(assignments) = registry.project_assignments.get_mut(key) {
        assignments.remove(id);
        if assignments.is_empty() {
            registry.project_assignments.remove(key);
        }
    }
}

fn member_has_pm_duty(registry: &Registry, id: &str) -> bool {
    registry
        .pm_projects
        .values()
        .any(|project| project.member_id.as_deref() == Some(id))
        || registry.project_assignments.values().any(|assignments| {
            assignments
                .get(id)
                .is_some_and(|assignment| assignment.role == AgentAssignmentRole::Pm)
        })
}

/// Move legacy runtime fields without changing source snapshots or identity aliases.
fn extract_legacy_runtime(registry: &mut Registry) -> io::Result<()> {
    if let Some(roster) = &mut registry.roster {
        for member in &mut roster.members {
            if let Some(key) = member.pm_project_key.take() {
                let selected = &mut registry.pm_projects.entry(key).or_default().member_id;
                if selected
                    .as_ref()
                    .is_some_and(|id| id != &member.agent_member_id)
                {
                    return Err(invalid(
                        "Conflicting project PM selection during Agent migration",
                    ));
                }
                *selected = Some(member.agent_member_id.clone());
            }
            if let Some(assignment) = member.assignment.take() {
                let key = assignment
                    .project_key
                    .clone()
                    .ok_or_else(|| invalid("Shared Agent assignment has no source project"))?;
                let assignments = registry.project_assignments.entry(key).or_default();
                if assignments
                    .get(&member.agent_member_id)
                    .is_some_and(|current| current != &assignment)
                {
                    return Err(invalid(
                        "Conflicting project Agent assignment during migration",
                    ));
                }
                assignments.insert(member.agent_member_id.clone(), assignment);
            }
        }
    }
    Ok(())
}

fn store_project_roster(
    registry: &mut Registry,
    key: &str,
    roster: Option<AgentRoster>,
) -> io::Result<()> {
    let Some(mut roster) = roster else {
        registry.roster = None;
        return Ok(());
    };
    let mut assignments = BTreeMap::new();
    let mut selected = None;
    for member in &mut roster.members {
        if (!member.enabled || !member.pm_eligible)
            && member_has_pm_duty(registry, &member.agent_member_id)
        {
            return Err(invalid(
                "Choose another PM before resting this member or removing PM eligibility",
            ));
        }
        if let Some(assignment) = member.assignment.take() {
            if assignment.project_key.as_deref() != Some(key) {
                return Err(invalid("Agent assignment belongs to another project"));
            }
            assignments.insert(member.agent_member_id.clone(), assignment);
        }
        if let Some(project) = member.pm_project_key.take() {
            if project != key || selected.is_some() {
                return Err(invalid("Ambiguous project PM selection"));
            }
            selected = Some(member.agent_member_id.clone());
        }
    }
    registry.roster = Some(roster);
    if assignments.is_empty() {
        registry.project_assignments.remove(key);
    } else {
        registry.project_assignments.insert(key.into(), assignments);
    }
    if selected.is_some() || registry.pm_projects.contains_key(key) {
        registry
            .pm_projects
            .entry(key.into())
            .or_default()
            .member_id = selected;
    }
    Ok(())
}

fn validate_assignment_budget(previous: &Registry, next: &Registry) -> io::Result<()> {
    let all = |registry: &Registry| {
        registry
            .project_assignments
            .values()
            .flat_map(|assignments| assignments.values())
            .cloned()
            .collect::<Vec<_>>()
    };
    let after = all(next);
    let implementation_count = |assignments: &[crate::agent_names::AgentMemberAssignment]| {
        assignments
            .iter()
            .filter(|assignment| assignment.role == AgentAssignmentRole::Implementation)
            .count()
    };
    let new_slot = |role| {
        next.project_assignments.iter().any(|(key, assignments)| {
            assignments.iter().any(|(id, assignment)| {
                assignment.role == role
                    && project_assignment(previous, key, id).is_none_or(|old| {
                        old.role != assignment.role
                            || old.reservation_id != assignment.reservation_id
                            || old.host_id != assignment.host_id
                            || old.host_pid != assignment.host_pid
                            || old.host_started_at != assignment.host_started_at
                            || old.session_id.is_some()
                                && assignment.session_id.is_some()
                                && old.session_id != assignment.session_id
                    })
            })
        })
    };
    let new_worker = new_slot(AgentAssignmentRole::Implementation);
    let new_pm = new_slot(AgentAssignmentRole::Pm);
    if !new_worker && !new_pm {
        return Ok(());
    }
    let policy = crate::machine_agent_settings::load()?;
    let census = crate::agent_capacity::registry_assignment_census()?;
    let mut occupied = implementation_count(&after);
    let mut pm_count = after
        .iter()
        .filter(|assignment| assignment.role == AgentAssignmentRole::Pm)
        .count();
    if let Some((_, inventory)) = &census {
        let hostname = crate::process::current_hostname();
        for observed in &inventory.sessions {
            // A cached census can outlive its child. Keep unknown observations
            // and surviving descendant groups charged until the next sample.
            if observed.session.child_pid > 0
                && observed.session.child_started_at > 0
                && crate::process::host_process_liveness(observed.session.child_pid) == Some(false)
                && !crate::process::is_process_group_alive(observed.session.child_pid)
            {
                continue;
            }
            let host_started_at =
                crate::process::host_process_start_time(observed.session.host_pid);
            let project_key = observed.repo_hash.clone().or_else(|| {
                observed.project_root.exists().then(|| {
                    gwt_core::paths::project_scope_hash(&observed.project_root).to_string()
                })
            });
            if after.iter().any(|assignment| {
                assignment.session_id.as_deref() == Some(observed.session.session_id.as_str())
                    && assignment.host_pid == observed.session.host_pid
                    && assignment.host_id == hostname
                    && assignment
                        .host_started_at
                        .is_some_and(|started| started > 0 && host_started_at == Some(started))
                    && assignment.project_key.is_some()
                    && assignment.project_key == project_key
            }) {
                continue;
            }
            if observed.role == crate::session_inventory::MachineSessionRole::Pm {
                pm_count += 1;
            } else {
                occupied += 1;
            }
        }
    }
    let measured_budget = census.as_ref().and_then(|(budget, _)| *budget);
    let total = occupied.saturating_add(pm_count.max(1));
    let registered_limit = next.roster.as_ref().map_or(0, AgentRoster::enabled_count);
    let over_budget = match policy {
        Some(policy) if policy.mode == crate::issue_monitor::IssueMonitorMaxActiveMode::Manual => {
            (new_worker && policy.manual_limit.is_some_and(|limit| occupied > limit))
                || (new_pm && total > measured_budget.unwrap_or(registered_limit))
        }
        Some(policy) if policy.last_change.is_some() => total > measured_budget.ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock,
            "Machine Agent capacity observation is unavailable; wait for the next fresh measurement"))?,
        // First-project migration materializes default Auto before its sampler
        // runs. Preserve the former global roster bound until a measurement
        // exists; an explicit Auto choice above still waits for that evidence.
        _ => measured_budget.map_or_else(
            || occupied.saturating_add(pm_count) > registered_limit,
            |budget| total > budget,
        ),
    };
    if over_budget {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Machine Agent capacity is occupied; wait for an exact slot to be released",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PmLegacyMemberProof {
    member_id: String,
    session_identity_sha256: String,
    registration_identity_sha256: String,
    imported_at: String,
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

/// Test-only observation of the I/O one registry transaction performs.
#[cfg(test)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RegistryIo {
    pub(crate) durability_confirmations: usize,
    pub(crate) registry_writes: usize,
    pub(crate) mirror_publishes: usize,
    pub(crate) record_writes: Vec<String>,
    pub(crate) validated_payloads: Vec<String>,
    /// Whole-mirror decodes: the cost that grows with a project's record.
    pub(crate) mirror_decodes: usize,
}

#[cfg(test)]
thread_local! {
    static REGISTRY_IO: std::cell::RefCell<RegistryIo> = std::cell::RefCell::new(RegistryIo::default());
}

#[cfg(test)]
pub(crate) fn take_registry_io_for_test() -> RegistryIo {
    REGISTRY_IO.with(|io| std::mem::take(&mut *io.borrow_mut()))
}

#[cfg(test)]
fn record_io(observe: impl FnOnce(&mut RegistryIo)) {
    REGISTRY_IO.with(|io| observe(&mut io.borrow_mut()));
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub fn path() -> PathBuf {
    gwt_core::paths::gwt_home().join("machine-state/agent-registry.json")
}

fn records_dir() -> PathBuf {
    gwt_core::paths::gwt_home().join("machine-state/agent-registry-sources")
}

fn record_path(key: &str) -> PathBuf {
    records_dir().join(format!("{key}.json"))
}

fn snapshots_path(key: &str) -> PathBuf {
    records_dir().join(format!("{key}.snapshots.json"))
}

/// The complete schema 3 registry as it was immediately before the split.
pub fn schema3_backup_path() -> PathBuf {
    gwt_core::paths::gwt_home().join("machine-state/agent-registry.schema3-backup.json")
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

/// Exclusive ownership of the shared core and of every project's record.
///
/// Lock order, the same on every path: project prefs lock(s), then the core
/// lock, then the registry lock. Only [`lock`] and [`RegistryWrite::acquire`]
/// take the last two. The core lock orders everything that reads the shared
/// core to write from it. The registry lock is the one an
/// older binary takes for a whole transaction: a project transaction holds
/// it shared, so several projects write their own record at once while this
/// guard, and any older binary, still excludes all of them (Issue #7486).
pub(crate) struct RegistryLock {
    _registry: NamedFileLock,
    _core: NamedFileLock,
}

fn lock_path() -> io::Result<PathBuf> {
    let path = path();
    gwt_config::home::assert_test_path(&path);
    fs::create_dir_all(path.parent().expect("machine state parent"))?;
    Ok(path.with_extension("lock"))
}

fn core_lock() -> io::Result<NamedFileLock> {
    NamedFileLock::acquire(
        &lock_path()?.with_extension("core.lock"),
        "shared Agent registry core",
    )
}

fn lock() -> io::Result<RegistryLock> {
    let core = core_lock()?;
    Ok(RegistryLock {
        _registry: NamedFileLock::acquire(&lock_path()?, "shared Agent registry")?,
        _core: core,
    })
}

/// The committed mirrors one registry read or write proves byte-for-byte.
///
/// Every project shares the roster, profiles and assignments, so those are
/// always validated. A project's own transaction proves only the committed
/// mirror it serves: another project's mirror, and the superseded snapshots
/// kept beside each mirror, are carried unchanged and stay closed to that
/// project and to every complete read.
#[derive(Clone, Copy)]
enum Mirrors<'a> {
    All,
    Only(&'a str),
    None,
}

impl Mirrors<'_> {
    fn covers(self, key: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only(only) => only == key,
            Self::None => false,
        }
    }

    fn covers_superseded(self) -> bool {
        matches!(self, Self::All)
    }
}

fn load() -> io::Result<Option<Registry>> {
    load_proving(Mirrors::All)
}

fn load_proving(mirrors: Mirrors<'_>) -> io::Result<Option<Registry>> {
    raw(&path())?
        .map(|core| decode_registry(&core, mirrors, &|key| raw(&record_path(key))))
        .transpose()
}

/// The decoded mirrors a validation proved, keyed by project.
///
/// Decoding a mirror costs time in proportion to its project's record (Issue
/// #7550), so the reader that asked for the proof takes its result instead of
/// decoding the same bytes again.
type Proven = BTreeMap<String, IssueMonitorPrefs>;

#[derive(Deserialize)]
struct StorageSchema {
    schema_version: u32,
}

fn decode_registry(
    raw: &str,
    mirrors: Mirrors<'_>,
    record: &dyn Fn(&str) -> io::Result<Option<String>>,
) -> io::Result<Registry> {
    decode_registry_proven(raw, mirrors, record).map(|(registry, _)| registry)
}

fn decode_registry_proven(
    raw: &str,
    mirrors: Mirrors<'_>,
    record: &dyn Fn(&str) -> io::Result<Option<String>>,
) -> io::Result<(Registry, Proven)> {
    let schema: StorageSchema = serde_json::from_str(raw).map_err(io::Error::other)?;
    if schema.schema_version >= SPLIT_SCHEMA_VERSION {
        return decode_split(raw, mirrors, record);
    }
    let mut registry: Registry = serde_json::from_str(raw).map_err(io::Error::other)?;
    let mut proven = validate_decoding(&registry, mirrors)?;
    if registry.schema_version < 3 {
        extract_legacy_runtime(&mut registry)?;
        registry.schema_version = 3;
        if let Some(roster) = &mut registry.roster {
            roster.schema_version = 3;
        }
        proven = validate_decoding(&registry, mirrors)?;
    }
    Ok((registry, proven))
}

fn decode_split(
    raw: &str,
    mirrors: Mirrors<'_>,
    record: &dyn Fn(&str) -> io::Result<Option<String>>,
) -> io::Result<(Registry, Proven)> {
    let core: Core = serde_json::from_str(raw).map_err(io::Error::other)?;
    if core.schema_version != SPLIT_SCHEMA_VERSION {
        return Err(invalid("Unsupported shared Agent registry schema or phase"));
    }
    let mut registry = Registry {
        schema_version: 3,
        phase: core.phase,
        revision: core.revision,
        roster: core.roster,
        profiles: core.profiles,
        sources: BTreeMap::new(),
        project_assignments: core.project_assignments,
        pm_projects: core.pm_projects,
        store: Store {
            split: true,
            partial: !mirrors.covers_superseded(),
            unloaded: BTreeMap::new(),
        },
    };
    for (key, shared) in core.sources {
        if !mirrors.covers(&key) {
            registry.store.unloaded.insert(key, shared);
            continue;
        }
        let committed = record(&key)?
            .map(|raw| RecordFile::decode(&key, &raw))
            .transpose()?
            .and_then(|file| file.committed(Some(&shared.record_transaction_id)))
            .ok_or_else(|| {
                invalid(format!(
                    "Agent source record for {key} has no committed generation; inspect agent.registry.status"
                ))
            })?;
        let rosterless_snapshots = if mirrors.covers_superseded() {
            self::raw(&snapshots_path(&key))?
                .map(|raw| serde_json::from_str(&raw).map_err(io::Error::other))
                .transpose()?
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        registry.sources.insert(
            key,
            Source {
                was_active: shared.was_active,
                aliases: shared.aliases,
                session_aliases: shared.session_aliases,
                transaction_id: committed.transaction_id,
                before_sha256: committed.before_sha256,
                before_json: None,
                floor_before_sha256: committed.floor_before_sha256,
                floor_before_json: None,
                after_sha256: committed.after_sha256,
                after_json: committed.after_json,
                rosterless_snapshots,
            },
        );
    }
    let proven = validate_decoding(&registry, mirrors)?;
    Ok((registry, proven))
}

fn save(registry: &Registry) -> io::Result<()> {
    save_proving(registry, Mirrors::All)
}

fn save_proving(registry: &Registry, mirrors: Mirrors<'_>) -> io::Result<()> {
    let mut registry = registry.clone();
    registry.schema_version = 3;
    if let Some(roster) = &mut registry.roster {
        roster.schema_version = 3;
    }
    validate_proving(&registry, mirrors)?;
    #[cfg(test)]
    if std::env::var_os("GWT_TEST_AGENT_REGISTRY_SPLIT").is_some() {
        registry.store.split = true;
    }
    // Raised before the write and lowered only after it is durable, so a
    // later transaction confirms durability exactly when this one could not.
    fs::write(unconfirmed_path(), b"")?;
    if registry.store.split {
        save_split(&registry)?;
    } else {
        let content = serde_json::to_vec_pretty(&registry).map_err(io::Error::other)?;
        #[cfg(test)]
        record_io(|io| io.registry_writes += 1);
        crate::issue_monitor::durable_atomic_write(&path(), &content)?;
    }
    clear_unconfirmed()
}

fn record_content(
    key: &str,
    record: Option<SourceRecord>,
    pending: Option<SourceRecord>,
) -> io::Result<Vec<u8>> {
    serde_json::to_vec_pretty(&RecordFile {
        schema_version: 1,
        source_key: key.into(),
        record,
        pending,
    })
    .map_err(io::Error::other)
}

#[cfg(test)]
thread_local! {
    static BEFORE_RECORD_WRITE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

fn write_record(key: &str, content: &[u8]) -> io::Result<()> {
    #[cfg(test)]
    record_io(|io| io.record_writes.push(key.into()));
    #[cfg(test)]
    if let Some(observer) = BEFORE_RECORD_WRITE.with(|slot| slot.borrow_mut().take()) {
        observer();
    }
    crate::issue_monitor::durable_atomic_write(&record_path(key), content)
}

/// Split storage has two commit points and exactly one applies per write.
///
/// When the shared core is unchanged, each changed record is committed by
/// its own durable write. When the core changes, every changed record is
/// first written as `pending` beside its previous generation, and the core
/// write that names those transactions commits all of them at once: a crash
/// before it leaves every project on its previous generation.
fn save_split(registry: &Registry) -> io::Result<()> {
    let disk = raw(&path())?;
    let disk_core = disk
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Core>(raw).ok())
        .filter(|core| core.schema_version == SPLIT_SCHEMA_VERSION);
    let committed_transaction = |key: &str| {
        disk_core
            .as_ref()
            .and_then(|core| core.sources.get(key))
            .map(|source| source.record_transaction_id.clone())
    };
    let mut core = Core {
        schema_version: SPLIT_SCHEMA_VERSION,
        phase: registry.phase,
        revision: registry.revision,
        roster: registry.roster.clone(),
        profiles: registry.profiles.clone(),
        sources: registry.store.unloaded.clone(),
        project_assignments: registry.project_assignments.clone(),
        pm_projects: registry.pm_projects.clone(),
    };
    for (key, source) in &registry.sources {
        core.sources.insert(
            key.clone(),
            CoreSource {
                was_active: source.was_active,
                aliases: source.aliases.clone(),
                session_aliases: source.session_aliases.clone(),
                record_transaction_id: committed_transaction(key)
                    .unwrap_or_else(|| source.transaction_id.clone()),
            },
        );
    }
    let encode = |core: &Core| serde_json::to_vec_pretty(core).map_err(io::Error::other);
    let core_changed = disk.as_deref().map(str::as_bytes) != Some(encode(&core)?.as_slice());
    fs::create_dir_all(records_dir())?;
    for (key, source) in &registry.sources {
        let next = SourceRecord::of(source);
        if !core_changed {
            let content = record_content(key, Some(next), None)?;
            if fs::read(record_path(key)).ok().as_deref() != Some(content.as_slice()) {
                write_record(key, &content)?;
            }
            continue;
        }
        let previous = raw(&record_path(key))?
            .and_then(|raw| RecordFile::decode(key, &raw).ok())
            .and_then(|file| file.committed(committed_transaction(key).as_deref()));
        if previous.as_ref() == Some(&next) {
            continue;
        }
        core.sources
            .get_mut(key)
            .expect("loaded source")
            .record_transaction_id
            .clone_from(&next.transaction_id);
        write_record(key, &record_content(key, previous, Some(next))?)?;
    }
    if !registry.store.partial {
        for (key, source) in &registry.sources {
            let path = snapshots_path(key);
            if source.rosterless_snapshots.is_empty() && !path.try_exists()? {
                continue;
            }
            let content = serde_json::to_vec_pretty(&source.rosterless_snapshots)
                .map_err(io::Error::other)?;
            if fs::read(&path).ok().as_deref() != Some(content.as_slice()) {
                crate::issue_monitor::durable_atomic_write(&path, &content)?;
            }
        }
    }
    if core_changed {
        #[cfg(test)]
        record_io(|io| io.registry_writes += 1);
        crate::issue_monitor::durable_atomic_write(&path(), &encode(&core)?)?;
    }
    Ok(())
}

/// Move a schema 3 registry to split storage. Only the desktop app calls
/// this, once at startup: a binary older than the split fails closed on the
/// new files, so helper CLIs keep whichever storage they find.
///
/// The untouched schema 3 file is kept at [`schema3_backup_path`] first.
pub fn migrate_to_split_storage() -> io::Result<bool> {
    if !path().try_exists()? {
        return Ok(false);
    }
    let _guard = lock()?;
    let Some(original) = raw(&path())? else {
        return Ok(false);
    };
    let mut registry = decode_registry(&original, Mirrors::All, &|key| raw(&record_path(key)))?;
    if registry.store.split {
        return Ok(false);
    }
    confirm_durable()?;
    crate::issue_monitor::durable_atomic_write(&schema3_backup_path(), original.as_bytes())?;
    registry.store.split = true;
    save(&registry)?;
    Ok(true)
}

/// Restore the current split registry for binaries that only support schema 3.
/// Stop the desktop app and all Agents first; the next modern app start would
/// migrate it back to split storage. Records and the migration backup stay intact.
pub fn restore_schema3() -> io::Result<RegistryStatus> {
    let snapshot = load()?.ok_or_else(|| invalid("Agent registry is inactive"))?;
    let keys = snapshot.sources.keys().cloned().collect();
    let _sources = source_locks(&keys)?;
    let _registry = lock()?;
    let mut registry = load()?.ok_or_else(|| invalid("Agent registry disappeared"))?;
    if registry.sources.keys().any(|key| !keys.contains(key)) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Agent registry enrollment changed; retry agent.registry.restore_schema3",
        ));
    }
    if !registry.store.split {
        if unconfirmed_path().try_exists()? {
            confirm_durable()?;
        }
        return status(None);
    }
    if registry.phase != RegistryPhase::Active {
        return Err(invalid(
            "Agent registry is prepared; finish agent.registry.reconcile before restoring schema 3",
        ));
    }
    if registry
        .project_assignments
        .values()
        .any(|assignments| !assignments.is_empty())
    {
        return Err(invalid("Agent assignments remain; stop all Agents and release their reservations before restoring schema 3 (inspect agent.registry.status)"));
    }
    let inventory =
        crate::session_inventory::observe_machine_sessions(&gwt_core::paths::gwt_sessions_dir());
    if !inventory.sessions.is_empty() || !inventory.uncertainties.is_empty() {
        let live = inventory
            .sessions
            .iter()
            .map(|entry| entry.session.session_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let uncertain = inventory
            .uncertainties
            .iter()
            .map(|entry| entry.reason.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(invalid(format!("Agent Sessions are live or their absence is unproven: live=[{live}], uncertain=[{uncertain}]; stop all Agents before restoring schema 3")));
    }
    for (key, source) in &mut registry.sources {
        let path = prefs_path(key)?;
        let mirror = raw(&path)?;
        let floor = raw(&path.with_extension("committed.json"))?;
        let mirror_hash = raw_hash(mirror.as_deref());
        if mirror_hash != source.before_sha256 && mirror_hash != source.after_sha256 {
            return Err(invalid(format!("Agent mirror conflict for {key}; inspect agent.registry.status and reconcile before restoring schema 3")));
        }
        assert_known_floor_hash(source, &path, &raw_hash(floor.as_deref()))?;
        // Split records retain only superseded hashes. Use the known on-disk
        // mirrors as schema 3's recovery baselines, including pending publishes.
        source.before_sha256 = mirror_hash;
        source.before_json = mirror;
        source.floor_before_sha256 = raw_hash(floor.as_deref());
        source.floor_before_json = floor;
    }
    registry.store = Store::default();
    validate_proving(&registry, Mirrors::All)?;
    let content = serde_json::to_vec_pretty(&registry).map_err(io::Error::other)?;
    confirm_durable()?;
    fs::write(unconfirmed_path(), b"")?;
    // One commit point: before rename schema 4 remains authoritative; after
    // rename both old and new readers see the complete schema 3 generation.
    crate::issue_monitor::durable_atomic_write(&path(), &content)?;
    clear_unconfirmed()?;
    status(None)
}

/// Present while the latest registry write is not known to be durable.
fn unconfirmed_path() -> PathBuf {
    path().with_extension("unconfirmed")
}

fn clear_unconfirmed() -> io::Result<()> {
    clear_marker(&unconfirmed_path())
}

fn clear_marker(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// The same marker for a record its project writes without the core lock,
/// where the shared marker would be lowered by another project's write.
fn record_unconfirmed_path(key: &str) -> PathBuf {
    records_dir().join(format!("{key}.unconfirmed"))
}

/// Commit one project's record when the shared core is unchanged.
fn save_record(registry: &Registry, key: &str) -> io::Result<()> {
    validate_proving(registry, Mirrors::None)?;
    let marker = record_unconfirmed_path(key);
    fs::write(&marker, b"")?;
    let record = SourceRecord::of(&registry.sources[key]);
    write_record(key, &record_content(key, Some(record), None)?)?;
    clear_marker(&marker)
}

fn confirm_durable() -> io::Result<()> {
    #[cfg(test)]
    record_io(|io| io.durability_confirmations += 1);
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
    match fs::read_dir(records_dir()) {
        Ok(records) => {
            for record in records {
                // Another project may rename its temporary file away meanwhile.
                match fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(record?.path())
                {
                    Ok(record) => record.sync_all()?,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            #[cfg(unix)]
            fs::File::open(records_dir())?.sync_all()?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    clear_unconfirmed()
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
                    codex_speed: gwt_agent::CodexSpeed::Off,
                    runtime_target: None,
                    skip_permissions: None,
                });
        }
    }
}

fn settings_revision(registry: &Registry) -> u64 {
    registry.roster.as_ref().map_or(0, |roster| roster.revision)
}

fn validate_proving(registry: &Registry, mirrors: Mirrors<'_>) -> io::Result<()> {
    validate_decoding(registry, mirrors).map(drop)
}

fn validate_decoding(registry: &Registry, mirrors: Mirrors<'_>) -> io::Result<Proven> {
    let mut proven = Proven::new();
    if !matches!(registry.schema_version, 1..=3)
        || !matches!(
            registry.phase,
            RegistryPhase::Prepared | RegistryPhase::Active
        )
    {
        return Err(invalid("Unsupported shared Agent registry schema or phase"));
    }
    let mut imported = BTreeSet::new();
    for (key, project) in &registry.pm_projects {
        ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?;
        for (session, proof) in &project.legacy_sessions {
            if proof.session_identity_sha256.is_empty()
                || proof.registration_identity_sha256.is_empty()
                || !imported.insert(proof.member_id.as_str())
                || !registry.roster.as_ref().is_some_and(|roster| {
                    roster.members.iter().any(|member| {
                        member.agent_member_id == proof.member_id
                            && member.origin_session_id.as_deref() == Some(session)
                    })
                })
            {
                return Err(invalid("PM legacy proof has no exact registered origin"));
            }
        }
    }
    for (key, source) in &registry.sources {
        ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?;
        if source.transaction_id.is_empty() {
            return Err(invalid(format!("Agent registry commit mismatch for {key}")));
        }
        if mirrors.covers(key) {
            #[cfg(test)]
            record_io(|io| io.validated_payloads.push(key.clone()));
            // Split storage keeps the superseded fingerprints, not their bytes.
            if fingerprint(&source.after_json) != source.after_sha256
                || mirrors.covers_superseded()
                    && !registry.store.split
                    && (raw_hash(source.before_json.as_deref()) != source.before_sha256
                        || raw_hash(source.floor_before_json.as_deref())
                            != source.floor_before_sha256)
            {
                return Err(invalid(format!("Agent registry commit mismatch for {key}")));
            }
            for observation in source
                .rosterless_snapshots
                .iter()
                .filter(|_| mirrors.covers_superseded())
            {
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
            proven.insert(key.clone(), prefs);
        }
    }
    let shared = registry
        .sources
        .iter()
        .map(|(key, source)| {
            (
                key,
                source.was_active,
                &source.aliases,
                &source.session_aliases,
            )
        })
        .chain(registry.store.unloaded.iter().map(|(key, source)| {
            (
                key,
                source.was_active,
                &source.aliases,
                &source.session_aliases,
            )
        }));
    for (key, was_active, aliases, session_aliases) in shared {
        ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?;
        if registry.phase == RegistryPhase::Active && !was_active {
            return Err(invalid("Active Agent registry has an unfenced enrollment"));
        }
        for id in aliases.values() {
            uuid::Uuid::parse_str(id).map_err(|error| invalid(error.to_string()))?;
            if !imported.insert(id.as_str()) {
                return Err(invalid(
                    "Agent registry source aliases share a global identity",
                ));
            }
        }
        for (session_id, id) in session_aliases {
            if !aliases.values().any(|alias| alias == id)
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
        if !matches!(
            (registry.schema_version, roster.schema_version),
            (1, 2) | (2, 3) | (3, 3)
        ) {
            return Err(invalid(
                "Unsupported shared Agent registry/roster schema pair",
            ));
        }
        let mut names: BTreeMap<String, Vec<&AgentMember>> = BTreeMap::new();
        for member in &roster.members {
            uuid::Uuid::parse_str(&member.agent_member_id)
                .map_err(|error| invalid(error.to_string()))?;
            names
                .entry(member.name.trim().to_lowercase())
                .or_default()
                .push(member);
            if registry.schema_version == 3
                && (member.assignment.is_some() || member.pm_project_key.is_some())
            {
                return Err(invalid(
                    "Project runtime fields cannot be stored in the common Agent roster",
                ));
            }
            if let Some(assignment) = &member.assignment {
                let Some(key) = assignment.project_key.as_deref() else {
                    return Err(invalid("Shared Agent assignment has no source project"));
                };
                if !registry.enrolled(key)
                    && !(assignment.role == AgentAssignmentRole::Pm
                        && registry.pm_projects.contains_key(key))
                {
                    return Err(invalid(
                        "Shared Agent assignment has an unknown source project",
                    ));
                }
            }
        }
        for (key, assignments) in &registry.project_assignments {
            ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?;
            for (id, assignment) in assignments {
                if !roster
                    .members
                    .iter()
                    .any(|member| &member.agent_member_id == id)
                    || assignment.project_key.as_deref() != Some(key)
                    || (!registry.enrolled(key)
                        && !(assignment.role == AgentAssignmentRole::Pm
                            && registry.pm_projects.contains_key(key)))
                {
                    return Err(invalid(
                        "Shared Agent assignment has no exact registered member and source project",
                    ));
                }
            }
        }
        for project in registry.pm_projects.values() {
            if project.member_id.as_ref().is_some_and(|id| {
                !roster.members.iter().any(|member| {
                    &member.agent_member_id == id && member.enabled && member.pm_eligible
                })
            }) {
                return Err(invalid("Configured PM must remain enabled and PM-eligible"));
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
    Ok(proven)
}

fn decode(raw: Option<&str>) -> io::Result<IssueMonitorPrefs> {
    #[cfg(test)]
    if raw.is_some() {
        record_io(|io| io.mirror_decodes += 1);
    }
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
    load_proving(Mirrors::None)
        .map(|registry| registry.is_some_and(|registry| registry.phase == RegistryPhase::Active))
}

/// Shared registration and PM selection do not mutate project execution mirrors.
pub(crate) fn mutate_pm_selection(
    root: &Path,
    action: AgentRosterAction,
    expected_revision: Option<u64>,
) -> io::Result<IssueMonitorPrefs> {
    let key = gwt_core::paths::project_scope_hash(root).to_string();
    let adding = matches!(action, AgentRosterAction::Add { .. });
    let source_path = prefs_path(&key)?;
    let _source_guard = if adding && source_path.exists() {
        Some(NamedFileLock::acquire(
            &source_path.with_extension("lock"),
            "Agent registry source",
        )?)
    } else {
        None
    };
    let _guard = lock()?;
    let mut registry =
        load()?.ok_or_else(|| invalid("Load the Agent roster before changing members"))?;
    if registry.phase != RegistryPhase::Active {
        return Err(invalid(
            "Agent registry is prepared; inspect agent.registry.status and reconcile",
        ));
    }
    confirm_durable()?;
    let roster = project_roster(&registry, &key)
        .ok_or_else(|| invalid("Load the Agent roster before changing members"))?;
    if !matches!(action, AgentRosterAction::Read)
        && (if adding && registry.sources.contains_key(&key) {
            expected_revision.is_some_and(|expected| expected != roster.revision)
        } else {
            expected_revision != Some(roster.revision)
        })
    {
        return Err(invalid("Agent roster changed; reload before retrying"));
    }
    let mut default_tool = "codex".to_string();
    if adding {
        if let Some(source) = registry.sources.get(&key) {
            assert_known(&registry, &key, raw(&source_path)?.as_deref())?;
            assert_known_floor(source, &source_path)?;
            default_tool = decode(Some(&source.after_json))?
                .launch_profile_pool()
                .first()
                .map(|profile| profile.agent_id.clone())
                .unwrap_or(default_tool);
        }
    }
    let registration_tuning = if let AgentRosterAction::Add {
        agent_id,
        model,
        reasoning,
        ..
    } = &action
    {
        let normalize = |value: &Option<String>| {
            value
                .as_ref()
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
        };
        let model = normalize(model);
        let reasoning = normalize(reasoning);
        gwt_agent::validate_native_tuning(
            agent_id.as_deref().unwrap_or(&default_tool),
            model.as_deref(),
            reasoning.as_deref(),
        )
        .map_err(invalid)?;
        Some((model, reasoning))
    } else {
        None
    };
    let member_id: &str = match &action {
        AgentRosterAction::SetPmEligibility { member_id, .. }
        | AgentRosterAction::SelectPm { member_id } => member_id,
        _ => "",
    };
    if !member_id.is_empty() {
        let member = roster
            .members
            .iter()
            .find(|member| member.agent_member_id == member_id)
            .ok_or_else(|| invalid("Agent member no longer exists"))?;
        match &action {
            AgentRosterAction::SetPmEligibility {
                eligible: false, ..
            } if member_has_pm_duty(&registry, member_id) => {
                return Err(invalid("Choose another PM before removing PM eligibility"));
            }
            AgentRosterAction::SelectPm { .. } => {
                if !member.enabled || !member.pm_eligible {
                    return Err(invalid("Choose an enabled, PM-eligible Agent"));
                }
                if member
                    .pm_project_key
                    .as_deref()
                    .is_some_and(|project| project != key)
                {
                    return Err(invalid(format!(
                        "Agent {} is configured as PM for project {:?}; open that project first",
                        member.name, member.pm_project_key
                    )));
                }
                if member.assignment.as_ref().is_some_and(|assignment| {
                    assignment.role != AgentAssignmentRole::Pm
                        || assignment.project_key.as_deref() != Some(&key)
                }) {
                    return Err(invalid(format!(
                        "Agent {} retains a busy or unknown assignment for project {:?}",
                        member.name,
                        member
                            .assignment
                            .as_ref()
                            .and_then(|assignment| assignment.project_key.as_ref())
                    )));
                }
                for (source_key, source) in registry
                    .sources
                    .iter()
                    .filter(|(source_key, _)| *source_key == &key)
                {
                    let source_path = prefs_path(source_key)?;
                    assert_known(&registry, source_key, raw(&source_path)?.as_deref())?;
                    assert_known_floor(source, &source_path)?;
                    let prefs = project(&registry, source_key, Some(&source.after_json))?;
                    let mut selected = false;
                    for profile in decode(Some(&source.after_json))?.launch_profile_pool() {
                        if let Some(id) = profile.agent_member_id {
                            selected |= resolve(&registry, source_key, &id)? == member_id;
                        }
                    }
                    let pinned = prefs
                        .pending_launch_deliveries
                        .iter()
                        .any(|delivery| delivery.agent_member_id.as_deref() == Some(member_id))
                        || prefs
                            .agent_issue_schedule
                            .reservations
                            .iter()
                            .any(|entry| entry.agent_member_id == member_id)
                        || prefs.agent_switch_intents.values().any(|intent| {
                            intent.has_admission_pin() && intent.request.member_id == member_id
                        });
                    if selected || pinned {
                        return Err(invalid(format!("Agent {} is an implementation candidate or pinned in project {source_key}; remove its candidate or wait for its launch before selecting PM", member.name)));
                    }
                }
            }
            _ => {}
        }
    }
    let before = project_roster(&registry, &key);
    let mut roster = before.clone().expect("checked roster");
    match action {
        AgentRosterAction::SetPmEligibility {
            member_id,
            eligible,
        } => {
            roster
                .members
                .iter_mut()
                .find(|member| member.agent_member_id == member_id)
                .expect("checked member")
                .pm_eligible = eligible;
        }
        AgentRosterAction::SelectPm { member_id } => {
            for member in &mut roster.members {
                if member.agent_member_id == member_id {
                    member.pm_project_key = Some(key.clone());
                } else if member.pm_project_key.as_deref() == Some(&key) {
                    member.pm_project_key = None;
                }
            }
            registry.pm_projects.entry(key.clone()).or_default();
        }
        action => crate::agent_names::apply_roster_action(
            &mut roster,
            &history_path(),
            &default_tool,
            action,
        )?,
    }
    store_project_roster(&mut registry, &key, Some(roster))?;
    if project_roster(&registry, &key) != before {
        let roster = registry.roster.as_mut().expect("checked roster");
        roster.schema_version = 3;
        roster.revision = roster
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("Agent roster revision exhausted"))?;
        registry.schema_version = 3;
        registry.revision = registry
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("Agent registry revision exhausted"))?;
        ensure_cores(&mut registry);
        if let Some((model, reasoning)) = registration_tuning {
            let member = registry.roster.as_ref().unwrap().members.last().unwrap();
            let core = registry.profiles.get_mut(&member.agent_member_id).unwrap();
            core.model = model;
            core.reasoning = reasoning;
        }
        save(&registry)?;
    }
    let source = registry
        .sources
        .get(&key)
        .map(|source| source.after_json.as_str());
    project(&registry, &key, source)
}

fn normalized_pm_path(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn pm_session_identity(key: &ProjectKey, session: &gwt_agent::Session) -> io::Result<String> {
    let expected = normalized_pm_path(&gwt_core::paths::gwt_project_dir(key).join("pm/worktree"));
    if normalized_pm_path(&session.worktree_path) != expected
        || session
            .repo_hash
            .as_deref()
            .is_some_and(|observed| observed != key.as_str())
        || session
            .project_state_root
            .as_ref()
            .is_some_and(|root| root.exists() && gwt_core::paths::project_scope_hash(root) != *key)
    {
        return Err(invalid(
            "PM Session belongs to a different project or canonical worktree",
        ));
    }
    Ok(fingerprint(
        &serde_json::to_string(&(
            &session.id,
            key.as_str(),
            expected,
            session.agent_id.command(),
            session.created_at,
        ))
        .map_err(io::Error::other)?,
    ))
}

fn legacy_pm_reservation(key: &str, session: &str, proof: &str) -> String {
    format!(
        "pm-legacy:{}",
        fingerprint(&format!("{key}\n{session}\n{proof}"))
    )
}

fn pm_host_matches(assignment: &crate::agent_names::AgentMemberAssignment) -> bool {
    assignment.host_id == crate::process::current_hostname()
        && assignment.host_pid == std::process::id()
        && assignment.host_started_at.is_some_and(|started| {
            started > 0
                && Some(started) == crate::process::host_process_start_time(std::process::id())
        })
}

/// Issue #7472: whether a configured PM's existing assignment is a leftover
/// that a new launch may reconcile. Only an unspawned reservation (no bound
/// Session) whose reserving host incarnation is provably gone qualifies; the
/// liveness proof reuses the roster discipline (`assignment_is_busy`), so a
/// live holder, a bound Session, another host's reservation, and this
/// process's own in-flight attempt all stay busy and keep blocking.
///
/// Issue #7475 extends the same reconciliation to one Session-bound class:
/// a legacy reservation imported by the registry migration (see
/// `pm_legacy_assignment_is_dead`). Issue #7542 adds the second
/// Session-bound class: a modern reservation whose bound Session is
/// provably finished under the full multi-signal proof (see
/// `pm_bound_assignment_is_dead`), and the hostless re-import of a finished
/// current-format Session, which records no incarnation for that proof to
/// read (see `pm_hostless_import_is_dead`). Every other Session-bound
/// assignment keeps blocking so a resumable conversation is never displaced.
fn pm_leftover_assignment_is_dead(
    root: &Path,
    sessions: &Path,
    key: &str,
    registry: &Registry,
    member_id: &str,
    assignment: &crate::agent_names::AgentMemberAssignment,
) -> bool {
    if assignment.role != AgentAssignmentRole::Pm
        || assignment.project_key.as_deref() != Some(key)
        || pm_host_matches(assignment)
    {
        return false;
    }
    match &assignment.session_id {
        None => !crate::agent_names::assignment_is_busy(root, assignment, sessions),
        Some(session_id) => {
            (pm_legacy_assignment_is_dead(
                registry, sessions, key, member_id, session_id, assignment,
            ) && !crate::agent_names::assignment_is_busy(root, assignment, sessions))
                || pm_bound_assignment_is_dead(root, sessions, assignment)
                || (pm_hostless_import_is_dead(sessions, key, member_id, session_id, assignment)
                    && !crate::agent_names::assignment_is_busy(root, assignment, sessions))
        }
    }
}

/// Issue #7542: whether a Session-bound assignment is exactly the hostless
/// reservation `ensure_current_pm_identity` imports for a registered
/// current-format PM Session, bound to a Session that is provably finished.
/// That import happens when a new gwtd generation finds the registered PM
/// without an assignment and without a live runtime; it records neither a
/// reserving host nor a legacy proof, so no cancel, reap, adoption, or #7475
/// path can ever release it. The token must equal the one recomputed from the
/// bound Session's own identity and the Session must name this member, so a
/// hand-written `pm-legacy:` string never qualifies.
fn pm_hostless_import_is_dead(
    sessions: &Path,
    key: &str,
    member_id: &str,
    session_id: &str,
    assignment: &crate::agent_names::AgentMemberAssignment,
) -> bool {
    if assignment.host_pid != 0 || !assignment.host_id.is_empty() {
        return false;
    }
    let Ok(project) = ProjectKey::parse(key) else {
        return false;
    };
    let Ok(session) = gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml"))) else {
        return false;
    };
    session.id == session_id
        && session.agent_member_id.as_deref() == Some(member_id)
        && matches!(
            session.status,
            gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
        )
        && pm_session_identity(&project, &session).is_ok_and(|identity| {
            assignment.reservation_id == legacy_pm_reservation(key, session_id, &identity)
        })
}

/// Issue #7542: whether the host incarnation that reserved `assignment` is
/// provably gone. The recorded (pid, start time) pair *is* the incarnation:
/// it is gone when the pid is dead, and when a different process now owns
/// the pid (its start time differs, so the recorded incarnation cannot
/// still exist). A live process whose start time cannot be read is not
/// proof of anything, so the check fails closed and PID-reuse ambiguity
/// never releases a reservation. Another host's incarnation can never be
/// proven gone from here.
fn pm_reserving_incarnation_is_gone(
    assignment: &crate::agent_names::AgentMemberAssignment,
) -> bool {
    if assignment.host_id != crate::process::current_hostname() {
        return false;
    }
    let Some(started) = assignment.host_started_at.filter(|started| *started > 0) else {
        return false;
    };
    if assignment.host_pid == std::process::id() {
        // This process is alive. A record naming it belongs to the current
        // incarnation only with this process's exact start time, which
        // `pm_host_matches` has already fenced; anything else is not a
        // provable death, so fail closed.
        return false;
    }
    match crate::process::host_process_start_time(assignment.host_pid) {
        Some(current) => current != started,
        None => !crate::process::is_host_process_alive(assignment.host_pid),
    }
}

/// Issue #7542: whether a Session-bound PM assignment under a modern
/// (non-legacy) reservation token is provably dead. Every signal must
/// agree: the bound Session loads with an exact identity, no launch
/// handshake is in flight for it, the reserving host incarnation is
/// provably gone, and the roster liveness discipline
/// (`assignment_is_busy`) reports it not busy — which itself requires the
/// Session inventory to show neither a live process nor an uncertainty,
/// and, for a Session still marked Running, the exact runtime proof
/// (execution identity, runtime incarnation, exited child tree). Anything
/// unprovable keeps blocking.
fn pm_bound_assignment_is_dead(
    root: &Path,
    sessions: &Path,
    assignment: &crate::agent_names::AgentMemberAssignment,
) -> bool {
    let Some(session_id) = assignment.session_id.as_deref() else {
        return false;
    };
    let Ok(session) = gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml"))) else {
        return false;
    };
    if session.id != session_id {
        return false;
    }
    if !matches!(
        std::fs::symlink_metadata(gwt_agent::active_launch_handshake_path(
            sessions, session_id
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    ) {
        return false;
    }
    pm_reserving_incarnation_is_gone(assignment)
        && !crate::agent_names::assignment_is_busy(root, assignment, sessions)
}

/// Issue #7542: whether a Session-bound PM assignment may be quarantined
/// so a NEW launch can proceed even though the assignment is not provably
/// dead. The bar is deliberately narrower than "unknown": the reserving
/// host incarnation must be provably gone and the bound Session must load
/// with an exact identity, so the displaced conversation stays restorable
/// from the quarantine record. A missing or corrupt Session file, another
/// host's reservation, and any doubt about the incarnation keep blocking
/// (fail-closed); the unbound case never reaches here because the
/// reconciliation above already owns it.
fn pm_assignment_is_quarantinable(
    sessions: &Path,
    assignment: &crate::agent_names::AgentMemberAssignment,
) -> bool {
    if assignment.role != AgentAssignmentRole::Pm || pm_host_matches(assignment) {
        return false;
    }
    let Some(session_id) = assignment.session_id.as_deref() else {
        return false;
    };
    let Ok(session) = gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml"))) else {
        return false;
    };
    session.id == session_id && pm_reserving_incarnation_is_gone(assignment)
}

/// Issue #7542: a stored PM registration fences NEW launches so a live
/// conversation is resumed, not displaced. The fence lifts only when the
/// registration names the very Session the configured PM's assignment is
/// bound to and that assignment is itself reconcilable — provably dead or
/// quarantinable — so both durable records describe the same wedged
/// launch. A registration naming any other Session keeps fencing.
fn pm_registration_is_reconcilable(
    root: &Path,
    sessions: &Path,
    registration: &crate::pm_registry::PmRegistration,
) -> bool {
    let key = gwt_core::paths::project_scope_hash(root);
    let Ok(Some(registry)) = load() else {
        return false;
    };
    let Some(member) = selected_pm_member(&registry, key.as_str()) else {
        return false;
    };
    let Some(assignment) = member.assignment.as_ref() else {
        return false;
    };
    assignment.session_id.as_deref() == Some(registration.session_id.as_str())
        && (pm_leftover_assignment_is_dead(
            root,
            sessions,
            key.as_str(),
            &registry,
            &member.agent_member_id,
            assignment,
        ) || pm_assignment_is_quarantinable(sessions, assignment))
}

/// Issue #7475: whether a Session-bound assignment is exactly the legacy
/// reservation the registry migration imported for this member, bound to a
/// Session that is provably finished. The stored legacy proof must name
/// this member for the bound Session and the reservation token must equal
/// the token recomputed from that proof, so a hand-written `pm-legacy:`
/// string or another member's proof never qualifies. A legacy reservation
/// records no reserving host (`host_pid` 0), so no release path can ever
/// match it; without this proof the assignment wedges every later launch.
/// The bound Session must load with an exact identity and a finished
/// status; a missing Session file stays unprovable here and in
/// `assignment_is_busy`, so it keeps blocking.
fn pm_legacy_assignment_is_dead(
    registry: &Registry,
    sessions: &Path,
    key: &str,
    member_id: &str,
    session_id: &str,
    assignment: &crate::agent_names::AgentMemberAssignment,
) -> bool {
    let Some(proof) = registry
        .pm_projects
        .get(key)
        .and_then(|project| project.legacy_sessions.get(session_id))
    else {
        return false;
    };
    if proof.member_id != member_id
        || assignment.reservation_id
            != legacy_pm_reservation(key, session_id, &proof.session_identity_sha256)
    {
        return false;
    }
    let Ok(session) = gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml"))) else {
        return false;
    };
    session.id == session_id
        && matches!(
            session.status,
            gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
        )
}

fn commit_pm_identity(registry: &mut Registry) -> io::Result<()> {
    let roster = registry
        .roster
        .as_mut()
        .ok_or_else(|| invalid("PM member roster is unavailable"))?;
    roster.schema_version = 3;
    roster.revision = roster
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("Agent roster revision exhausted"))?;
    registry.schema_version = 3;
    registry.revision = registry
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("Agent registry revision exhausted"))?;
    ensure_cores(registry);
    save(registry)
}

pub(crate) fn pm_assignment(
    root: &Path,
    member_id: &str,
    reservation: &str,
) -> io::Result<Option<crate::agent_names::AgentMemberAssignment>> {
    let key = gwt_core::paths::project_scope_hash(root);
    let Some(registry) = load()? else {
        return Ok(None);
    };
    let Some(assignment) = project_assignment(&registry, key.as_str(), member_id)
        .filter(|assignment| assignment.role == AgentAssignmentRole::Pm)
    else {
        return Ok(None);
    };
    if assignment.project_key.as_deref() != Some(key.as_str()) {
        return Err(invalid("PM reservation belongs to another exact project"));
    }
    if assignment.reservation_id != reservation {
        return Err(invalid(
            "Exact PM reservation changed; retain the current assignment",
        ));
    }
    Ok(Some(assignment.clone()))
}

pub struct PreparedPmMember {
    pub member: AgentMember,
    pub profile: crate::pm_registry::PmLaunchProfile,
    pub prior_assignment: Option<crate::agent_names::AgentMemberAssignment>,
}

pub(crate) struct PmLaunchDraft<'a> {
    pub member_id: Option<&'a str>,
    pub profile_revision: &'a str,
    pub agent_id: &'a str,
    pub model: Option<&'a str>,
    pub reasoning: Option<&'a str>,
    pub codex_speed: gwt_agent::CodexSpeed,
    pub fast_mode: bool,
}

/// Trusted canonical NEW only. Captures selection and PM tuning together.
pub fn reserve_new_pm_member(
    root: &Path,
    sessions: &Path,
    worktree: &Path,
    reservation: &str,
) -> io::Result<PreparedPmMember> {
    reserve_new_pm_member_inner(root, sessions, worktree, reservation, None)
}

pub(crate) fn reserve_new_pm_member_from_draft(
    root: &Path,
    sessions: &Path,
    worktree: &Path,
    reservation: &str,
    draft: &PmLaunchDraft<'_>,
) -> io::Result<PreparedPmMember> {
    reserve_new_pm_member_inner(root, sessions, worktree, reservation, Some(draft))
}

fn reserve_new_pm_member_inner(
    root: &Path,
    sessions: &Path,
    worktree: &Path,
    reservation: &str,
    draft: Option<&PmLaunchDraft<'_>>,
) -> io::Result<PreparedPmMember> {
    let key = gwt_core::paths::project_scope_hash(root);
    if normalized_pm_path(worktree)
        != normalized_pm_path(&crate::pm_registry::pm_worktree_path_for_repo_path(root))
    {
        return Err(invalid(
            "PM launch requires this project's canonical PM worktree",
        ));
    }
    ensure_current_pm_identity(root, sessions)?;
    ensure_active_for_owner(root)?;
    let started = crate::process::host_process_start_time(std::process::id())
        .filter(|started| *started > 0)
        .ok_or_else(|| {
            invalid("PM reserving host identity is unknown; retry from its project owner")
        })?;
    let pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(root);
    // Issue #7542: a stored registration normally fences NEW launches so a
    // live conversation is resumed, not displaced. When the registration
    // names the very Session the configured PM's wedged assignment is bound
    // to and that assignment is reconcilable (provably dead or
    // quarantinable), both records describe the same dead launch: clear the
    // registration first, then let the assignment handling below reconcile
    // or quarantine the assignment in the same launch. Clearing first keeps
    // a later failure retryable — the wedged assignment still fences, the
    // registration no longer double-fences.
    if let Some(registration) = crate::pm_registry::load_pm_prefs(&pm_path)?.registration {
        if !pm_registration_is_reconcilable(root, sessions, &registration) {
            return Err(invalid(
                "Resume the registered PM conversation before starting a new PM",
            ));
        }
        let wedged_session = registration.session_id.clone();
        crate::pm_registry::mutate_pm_prefs(&pm_path, |prefs| {
            if prefs
                .registration
                .as_ref()
                .is_some_and(|current| current.session_id == wedged_session)
            {
                prefs.registration = None;
            }
        })?;
    }
    crate::pm_registry::with_pm_prefs_snapshot(&pm_path, |prefs| {
        if prefs.registration.is_some() {
            return Err(invalid(
                "Resume the registered PM conversation before starting a new PM",
            ));
        }
        let profile = prefs.settings.launch_profile_with_agent_defaults(root);
        let revision = crate::pm_registry::pm_profile_revision(prefs);
        let expected_assignment = crate::agent_names::AgentMemberAssignment {
            role: AgentAssignmentRole::Pm,
            pm_profile_revision: Some(revision.clone()),
            project_key: Some(key.to_string()),
            reservation_id: reservation.into(),
            session_id: None,
            issue_number: None,
            spec_number: None,
            delivery_id: None,
            host_id: crate::process::current_hostname(),
            host_pid: std::process::id(),
            host_started_at: Some(started),
        };
        let _guard = lock()?;
        let mut registry =
            load()?.ok_or_else(|| invalid("PM registry activation is unavailable"))?;
        if registry.phase != RegistryPhase::Active {
            return Err(invalid(
                "Agent registry is prepared; inspect agent.registry.status and reconcile",
            ));
        }
        confirm_durable()?;
        let previous = registry.clone();
        if let Some(draft) = draft {
            let selected = selected_pm_member(&registry, key.as_str());
            let bootstrap_replay = draft.member_id.is_none()
                && selected
                    .as_ref()
                    .is_some_and(|member| member.assignment.as_ref() == Some(&expected_assignment));
            if selected
                .as_ref()
                .map(|member| member.agent_member_id.as_str())
                != draft.member_id
                && !bootstrap_replay
                || revision != draft.profile_revision
                || profile.agent_id != draft.agent_id
                || profile.model.as_deref() != draft.model
                || profile.reasoning.as_deref() != draft.reasoning
                || (if profile.agent_id == "codex" {
                    profile.codex_speed.unwrap_or_default()
                } else {
                    gwt_agent::CodexSpeed::Off
                }) != draft.codex_speed
                || (profile.fast_mode.unwrap_or(false)
                    && gwt_agent::resolve_agent_id(&profile.agent_id)
                        .is_some_and(|agent| agent.supports_fast_mode()))
                    != draft.fast_mode
            {
                return Err(invalid(
                    "PM selection or profile changed during preparation; refresh and retry NEW",
                ));
            }
        }
        if selected_pm_member(&registry, key.as_str()).is_none() {
            let roster = registry.roster.get_or_insert_with(|| AgentRoster {
                schema_version: 3,
                revision: 0,
                members: Vec::new(),
            });
            roster
                .add(&history_path(), Some(profile.agent_id.clone()), None)
                .map_err(io::Error::other)?;
            let member = roster.members.last_mut().expect("bootstrap member");
            member.pm_eligible = true;
            registry
                .pm_projects
                .entry(key.to_string())
                .or_default()
                .member_id = Some(member.agent_member_id.clone());
        }
        registry.pm_projects.entry(key.to_string()).or_default();
        let mut member = selected_pm_member(&registry, key.as_str()).expect("configured member");
        if !member.enabled || !member.pm_eligible {
            return Err(invalid(
                "Configured PM must be enabled and PM-eligible; choose another PM in Agents",
            ));
        }
        if let Some(assignment) = &member.assignment {
            if assignment == &expected_assignment {
                return Ok(PreparedPmMember {
                    member: member.clone(),
                    profile,
                    prior_assignment: None,
                });
            }
            if !pm_leftover_assignment_is_dead(
                root,
                sessions,
                key.as_str(),
                &registry,
                &member.agent_member_id,
                assignment,
            ) {
                if !pm_assignment_is_quarantinable(sessions, assignment) {
                    return Err(invalid(format!("Configured PM {} retains a live or unknown reservation; resume its conversation or choose another PM", member.name)));
                }
                // Issue #7542 quarantine fallback: the reserving host
                // incarnation is provably gone and the bound Session loads
                // exactly, but the Session is not provably finished, so the
                // assignment must not be deleted. Park it verbatim in the
                // project's quarantine record — the Session file stays
                // untouched and `restore_quarantined_pm_assignment` can
                // bring the assignment back exactly — and fall through to
                // the fresh reservation below, so no durable state can
                // wedge every future PM launch behind a dead generation.
                let record = PmQuarantinedAssignment {
                    assignment: assignment.clone(),
                    reason: format!(
                        "reserving host incarnation is gone; bound Session {} was not provably finished, so the assignment was quarantined instead of deleted",
                        assignment.session_id.as_deref().unwrap_or_default()
                    ),
                    quarantined_at_unix: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|elapsed| elapsed.as_secs())
                        .unwrap_or_default(),
                };
                registry
                    .pm_projects
                    .entry(key.to_string())
                    .or_default()
                    .quarantined_assignments
                    .insert(member.agent_member_id.clone(), record);
            }
            // Issue #7472: the reserving gwtd incarnation is gone and no
            // Session was ever bound, so no owner remains that could cancel
            // or reap this reservation. Reconcile it as part of this launch
            // instead of wedging every future PM launch on it; the fresh
            // assignment is written and committed below.
            //
            // Issue #7475: the same holds for a legacy reservation imported
            // by the registry migration whose bound Session is provably
            // finished — it records no reserving host, so no cancel, reap,
            // or adoption path can ever release it.
            //
            // Issue #7542: the same holds for a modern Session-bound
            // reservation whose bound Session is provably finished under
            // the multi-signal proof in `pm_bound_assignment_is_dead`, and
            // for a quarantinable assignment, which is parked above rather
            // than deleted.
        }
        member.assignment = Some(expected_assignment);
        set_project_assignment(
            &mut registry,
            key.as_str(),
            &member.agent_member_id,
            member.assignment.clone(),
        );
        validate_assignment_budget(&previous, &registry)?;
        commit_pm_identity(&mut registry)?;
        Ok(PreparedPmMember {
            member,
            profile,
            prior_assignment: None,
        })
    })
}

/// Issue #7542: restore a PM assignment previously parked by the
/// quarantine fallback in `reserve_new_pm_member_inner`. The parked
/// assignment returns verbatim — same reservation token, same bound
/// Session — and the quarantine record is removed in the same commit, so
/// the member's conversation becomes resumable exactly as it was.
///
/// Restoration is allowed only while the member's current assignment
/// cannot belong to a live conversation: no assignment at all, or this
/// host incarnation's own unspawned reservation (the fresh NEW reservation
/// the quarantine made room for, before any Session was bound to it).
/// Anything else is an explicit error, never a silent overwrite.
// The operator surface that calls this lands with the GUI affordance in
// the follow-up slice (Issue #7543); the registry path itself is complete
// and covered by tests here.
#[allow(dead_code)]
pub(crate) fn restore_quarantined_pm_assignment(
    root: &Path,
    member_id: &str,
) -> io::Result<crate::agent_names::AgentMemberAssignment> {
    let key = gwt_core::paths::project_scope_hash(root);
    let _guard = lock()?;
    let mut registry = load()?.ok_or_else(|| invalid("PM registry activation is unavailable"))?;
    if registry.phase != RegistryPhase::Active {
        return Err(invalid(
            "Agent registry is prepared; inspect agent.registry.status and reconcile",
        ));
    }
    confirm_durable()?;
    let record = registry
        .pm_projects
        .get(key.as_str())
        .and_then(|project| project.quarantined_assignments.get(member_id))
        .cloned()
        .ok_or_else(|| invalid("No quarantined PM assignment is recorded for this member"))?;
    let replaceable = match project_assignment(&registry, key.as_str(), member_id) {
        None => true,
        Some(current) => current.session_id.is_none() && pm_host_matches(current),
    };
    if !replaceable {
        return Err(invalid(
            "The current PM assignment belongs to a live or unknown conversation; stop it before restoring the quarantined assignment",
        ));
    }
    registry
        .pm_projects
        .get_mut(key.as_str())
        .expect("quarantine record exists")
        .quarantined_assignments
        .remove(member_id);
    set_project_assignment(
        &mut registry,
        key.as_str(),
        member_id,
        Some(record.assignment.clone()),
    );
    commit_pm_identity(&mut registry)?;
    Ok(record.assignment)
}

pub(crate) fn cancel_unspawned_pm_member(
    root: &Path,
    sessions: &Path,
    member_id: &str,
    reservation: &str,
    candidate: Option<&gwt_agent::Session>,
    prior: Option<(
        &gwt_agent::Session,
        Option<&crate::agent_names::AgentMemberAssignment>,
    )>,
) -> io::Result<bool> {
    let key = gwt_core::paths::project_scope_hash(root);
    let Some(observed) = load()?
        .and_then(|registry| project_assignment(&registry, key.as_str(), member_id).cloned())
        .filter(|assignment| {
            assignment.role == AgentAssignmentRole::Pm && assignment.reservation_id == reservation
        })
    else {
        return Ok(false);
    };
    if observed.project_key.as_deref() != Some(key.as_str()) || !pm_host_matches(&observed) {
        return Err(invalid(
            "Only the exact reserving PM host may cancel this attempt",
        ));
    }
    let mut ids = Vec::new();
    if candidate.is_some_and(|candidate| {
        prior.is_some_and(|(source, _)| source.id == candidate.id)
            || candidate.restore_source_session_id.is_some()
                && !prior.is_some_and(|(source, _)| {
                    candidate.restore_source_session_id.as_deref() == Some(source.id.as_str())
                })
    }) {
        return Err(invalid(
            "PM cancellation has no captured original-conversation receipt",
        ));
    }
    if let Some(candidate) = candidate {
        ids.push(candidate.id.as_str());
    }
    if let Some((source, _)) = prior {
        ids.push(source.id.as_str());
    }
    let _leases = gwt_agent::acquire_session_path_leases(sessions, &ids)?;
    let mut runtime_writers = Vec::new();
    let mut runtime_publications = Vec::new();
    for id in &ids {
        let runtime = gwt_agent::runtime_state_path(sessions, id);
        runtime_writers.push(gwt_agent::session::runtime_writer_lease(&runtime)?);
        runtime_publications.push(gwt_agent::session::runtime_publication_lease(&runtime)?);
        if gwt_agent::active_launch_handshake_path(sessions, id).try_exists()? {
            return Err(invalid(
                "PM cancellation retains an active or unknown launch attempt",
            ));
        }
    }
    if let Some(candidate) = candidate {
        pm_session_identity(&key, candidate)?;
        if candidate.agent_member_id.as_deref() != Some(member_id)
            || candidate.status != gwt_agent::AgentStatus::Stopped
            || candidate.execution_binding.is_some()
            || prior.is_some_and(|(source, _)| source.id == candidate.id)
        {
            return Err(invalid(
                "PM cancellation has no exact stopped successor candidate",
            ));
        }
        let current = gwt_agent::Session::load(&sessions.join(format!("{}.toml", candidate.id)))?;
        if serde_json::to_value(&current).map_err(io::Error::other)?
            != serde_json::to_value(candidate).map_err(io::Error::other)?
            || gwt_agent::runtime_state_path(sessions, &candidate.id).try_exists()?
        {
            return Err(invalid(
                "PM candidate or provisional runtime changed; retain its reservation",
            ));
        }
        let observation = crate::session_inventory::observe_session(candidate, sessions);
        if !observation.sessions.is_empty() || !observation.uncertainties.is_empty() {
            return Err(invalid(
                "PM candidate retains published or uncertain runtime evidence",
            ));
        }
    }
    if let Some((source, _)) = prior {
        let current = gwt_agent::Session::load(&sessions.join(format!("{}.toml", source.id)))?;
        if serde_json::to_value(&current).map_err(io::Error::other)?
            != serde_json::to_value(source).map_err(io::Error::other)?
        {
            return Err(invalid(
                "Original PM conversation changed before reservation restoration",
            ));
        }
        let observation = crate::session_inventory::observe_session(source, sessions);
        if !observation.sessions.is_empty() || !observation.uncertainties.is_empty() {
            return Err(invalid(
                "Original PM conversation retains live or unknown runtime evidence",
            ));
        }
    }
    let allowed_session = observed.session_id.as_deref().is_none_or(|id| {
        candidate.is_some_and(|candidate| candidate.id == id)
            || prior.is_some_and(|(source, _)| source.id == id)
    });
    if !allowed_session || candidate.is_none() && prior.is_none() && observed.session_id.is_some() {
        return Err(invalid(
            "PM cancellation has no positive proof for its bound Session",
        ));
    }
    let pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(root);
    crate::pm_registry::with_pm_prefs_snapshot(&pm_path, |prefs| {
        let _guard = lock()?;
        let mut registry = load()?.ok_or_else(|| invalid("PM registry is unavailable"))?;
        if registry.phase != RegistryPhase::Active {
            return Err(invalid(
                "Agent registry is prepared; inspect agent.registry.status and reconcile",
            ));
        }
        confirm_durable()?;
        if let Some((source, previous)) = prior {
            pm_session_identity(&key, source)?;
            if !prefs
                .registration
                .as_ref()
                .is_some_and(|registration| registration.session_id == source.id)
                || (SessionMemberSnapshot {
                    registry: Some(registry.clone()),
                })
                .resolve_session(source)?
                .as_deref()
                    != Some(member_id)
                || observed.pm_profile_revision.as_deref()
                    != Some(pm_session_profile_revision(source).as_str())
                || previous.is_some_and(|assignment| {
                    assignment.role != AgentAssignmentRole::Pm
                        || assignment.project_key.as_deref() != Some(key.as_str())
                        || assignment.session_id.as_deref() != Some(source.id.as_str())
                })
            {
                return Err(invalid(
                    "PM restore proof changed before prior reservation restoration",
                ));
            }
        }
        if project_assignment(&registry, key.as_str(), member_id) != Some(&observed) {
            return Ok(false);
        }
        set_project_assignment(
            &mut registry,
            key.as_str(),
            member_id,
            prior.and_then(|(_, previous)| previous.cloned()),
        );
        commit_pm_identity(&mut registry)?;
        Ok(true)
    })
}

pub(crate) fn reserve_resume_pm_member(
    root: &Path,
    sessions: &Path,
    source: &gwt_agent::Session,
    reservation: &str,
    worktree_reserved: bool,
) -> io::Result<PreparedPmMember> {
    if worktree_reserved {
        return Err(invalid(
            "The original PM worktree is already reserved by its owner",
        ));
    }
    ensure_current_pm_identity(root, sessions)?;
    let key = gwt_core::paths::project_scope_hash(root);
    pm_session_identity(&key, source)?;
    let started = crate::process::host_process_start_time(std::process::id())
        .filter(|started| *started > 0)
        .ok_or_else(|| invalid("PM reserving host identity is unknown"))?;
    gwt_agent::with_session_path_lease(sessions, &source.id, |state| {
        let current = match state {
            gwt_agent::SessionPathState::Present(current) => current,
            gwt_agent::SessionPathState::Missing => {
                return Err(invalid("Original PM conversation is unavailable"))
            }
            gwt_agent::SessionPathState::Error(error) => return Err(error),
        };
        if serde_json::to_value(&current).map_err(io::Error::other)?
            != serde_json::to_value(source).map_err(io::Error::other)?
        {
            return Err(invalid(
                "Original PM conversation changed; refresh and retry Resume",
            ));
        }
        let runtime = gwt_agent::runtime_state_path(sessions, &source.id);
        let _writer = gwt_agent::session::runtime_writer_lease(&runtime)?;
        let _publication = gwt_agent::session::runtime_publication_lease(&runtime)?;
        let observation = crate::session_inventory::observe_session(source, sessions);
        if !observation.sessions.is_empty() || !observation.uncertainties.is_empty() {
            return Err(invalid(
                "Registered PM conversation retains live or unknown runtime evidence",
            ));
        }
        if gwt_agent::active_launch_handshake_path(sessions, &source.id).try_exists()? {
            return Err(invalid(
                "Registered PM retains an active or unknown launch attempt",
            ));
        }
        let pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(root);
        crate::pm_registry::with_pm_prefs_snapshot(&pm_path, |prefs| {
            if !prefs.registration.as_ref().is_some_and(|registration| {
                registration.session_id == source.id
                    && registration.agent_id == source.agent_id.command()
                    && normalized_pm_path(Path::new(&registration.worktree_path))
                        == normalized_pm_path(&source.worktree_path)
            }) {
                return Err(invalid(
                    "Resume must retain the project's exact registered PM conversation",
                ));
            }
            let _guard = lock()?;
            let mut registry = load()?.ok_or_else(|| invalid("PM registry is unavailable"))?;
            if registry.phase != RegistryPhase::Active {
                return Err(invalid(
                    "Agent registry is prepared; inspect agent.registry.status and reconcile",
                ));
            }
            confirm_durable()?;
            let previous = registry.clone();
            let id = (SessionMemberSnapshot {
                registry: Some(registry.clone()),
            })
            .resolve_session(source)?
            .ok_or_else(|| invalid("Original PM member proof is unavailable"))?;
            let mut member = project_member(&registry, key.as_str(), &id)
                .ok_or_else(|| invalid("Original PM member is unavailable"))?;
            if member
                .pm_project_key
                .as_deref()
                .is_some_and(|project| project != key.as_str())
            {
                return Err(invalid(
                    "Original PM member is configured for a different project",
                ));
            }
            let prior_assignment = member.assignment.clone();
            let revision = pm_session_profile_revision(source);
            if let Some(assignment) = &prior_assignment {
                if assignment.role != AgentAssignmentRole::Pm
                    || assignment.project_key.as_deref() != Some(key.as_str())
                    || assignment.session_id.as_deref() != Some(source.id.as_str())
                    || !assignment.host_id.is_empty()
                        && assignment.host_id != crate::process::current_hostname()
                {
                    return Err(invalid(
                        "Original PM member retains a foreign or conflicting assignment",
                    ));
                }
                if assignment.host_pid > 0
                    && crate::process::is_host_process_alive(assignment.host_pid)
                {
                    let observed_start =
                        crate::process::host_process_start_time(assignment.host_pid);
                    if assignment.host_started_at.is_none() || observed_start.is_none() {
                        return Err(invalid("Original PM reserving host identity is unknown"));
                    }
                    if observed_start == assignment.host_started_at && !pm_host_matches(assignment)
                    {
                        return Err(invalid("The original PM host is still alive; resume it through its exact owner"));
                    }
                }
                if assignment.reservation_id == reservation
                    && assignment.pm_profile_revision.as_deref() == Some(revision.as_str())
                    && pm_host_matches(assignment)
                {
                    return Ok(PreparedPmMember {
                        member: member.clone(),
                        profile: crate::pm_registry::PmLaunchProfile {
                            agent_id: source.agent_id.command().into(),
                            model: source.model.clone(),
                            reasoning: source.reasoning_level.clone(),
                            version: source.tool_version_selector.clone(),
                            codex_speed: Some(source.codex_speed()),
                            fast_mode: Some(source.fast_mode_enabled()),
                        },
                        prior_assignment,
                    });
                }
            }
            member.assignment = Some(crate::agent_names::AgentMemberAssignment {
                role: AgentAssignmentRole::Pm,
                pm_profile_revision: Some(revision),
                project_key: Some(key.to_string()),
                reservation_id: reservation.into(),
                session_id: Some(source.id.clone()),
                issue_number: None,
                spec_number: None,
                delivery_id: None,
                host_id: crate::process::current_hostname(),
                host_pid: std::process::id(),
                host_started_at: Some(started),
            });
            set_project_assignment(
                &mut registry,
                key.as_str(),
                &member.agent_member_id,
                member.assignment.clone(),
            );
            validate_assignment_budget(&previous, &registry)?;
            commit_pm_identity(&mut registry)?;
            Ok(PreparedPmMember {
                member,
                profile: crate::pm_registry::PmLaunchProfile {
                    agent_id: source.agent_id.command().into(),
                    model: source.model.clone(),
                    reasoning: source.reasoning_level.clone(),
                    version: source.tool_version_selector.clone(),
                    codex_speed: Some(source.codex_speed()),
                    fast_mode: Some(source.fast_mode_enabled()),
                },
                prior_assignment,
            })
        })
    })
}

fn pm_session_profile_revision(session: &gwt_agent::Session) -> String {
    format!(
        "pm-session-profile-v1:{}",
        fingerprint(
            &serde_json::to_string(&(
                session.agent_id.command(),
                &session.model,
                &session.reasoning_level,
                &session.tool_version_selector,
                &session.tool_version,
            ))
            .expect("serializable PM Session profile")
        )
    )
}

pub(crate) fn bind_pm_member_session(
    root: &Path,
    member_id: &str,
    reservation: &str,
    session_id: &str,
) -> io::Result<()> {
    bind_prepared_pm_member_session(root, member_id, reservation, session_id, None)
}

pub(crate) fn bind_prepared_pm_member_session(
    root: &Path,
    member_id: &str,
    reservation: &str,
    session_id: &str,
    profile_revision: Option<&str>,
) -> io::Result<()> {
    let key = gwt_core::paths::project_scope_hash(root);
    let expected = pm_assignment(root, member_id, reservation)?
        .ok_or_else(|| invalid("Exact PM reservation changed; refresh Agents before retrying"))?;
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let mut ids = vec![session_id];
    if let Some(source) = expected
        .session_id
        .as_deref()
        .filter(|source| *source != session_id)
    {
        ids.push(source);
    }
    let _leases = gwt_agent::acquire_session_path_leases(&sessions, &ids)?;
    let candidate = gwt_agent::Session::load(&sessions.join(format!("{session_id}.toml")))?;
    pm_session_identity(&key, &candidate)?;
    if candidate.id != session_id || candidate.agent_member_id.as_deref() != Some(member_id) {
        return Err(invalid(
            "PM candidate has a different exact member or Session",
        ));
    }
    let pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(root);
    crate::pm_registry::with_pm_prefs_snapshot(&pm_path, |prefs| {
        let _guard = lock()?;
        let mut registry = load()?.ok_or_else(|| invalid("PM registry is unavailable"))?;
        if registry.phase != RegistryPhase::Active {
            return Err(invalid(
                "Agent registry is prepared; inspect agent.registry.status and reconcile",
            ));
        }
        confirm_durable()?;
        let member = project_member(&registry, key.as_str(), member_id)
            .ok_or_else(|| invalid("PM member is unavailable"))?;
        if member.assignment.as_ref() != Some(&expected)
            || !pm_host_matches(&expected)
            || profile_revision
                .is_some_and(|revision| expected.pm_profile_revision.as_deref() != Some(revision))
        {
            return Err(invalid(
                "Exact PM reservation, captured profile or reserving host changed",
            ));
        }
        if expected.session_id.as_deref() == Some(session_id) {
            return Ok(());
        }
        if let Some(source_id) = candidate.restore_source_session_id.as_deref() {
            if source_id == session_id || expected.session_id.as_deref() != Some(source_id) {
                return Err(invalid("PM restore has a different original conversation"));
            }
            let source = gwt_agent::Session::load(&sessions.join(format!("{source_id}.toml")))?;
            pm_session_identity(&key, &source)?;
            if !prefs.registration.as_ref().is_some_and(|registration| {
                registration.session_id == source_id
                    && registration.agent_id == source.agent_id.command()
                    && normalized_pm_path(Path::new(&registration.worktree_path))
                        == normalized_pm_path(&source.worktree_path)
            }) || (SessionMemberSnapshot {
                registry: Some(registry.clone()),
            })
            .resolve_session(&source)?
            .as_deref()
                != Some(member_id)
                || expected.pm_profile_revision.as_deref()
                    != Some(pm_session_profile_revision(&source).as_str())
                || candidate.agent_id != source.agent_id
                || candidate.model != source.model
                || candidate.reasoning_level != source.reasoning_level
            {
                return Err(invalid(
                    "Registered PM conversation or recorded profile changed before restore bind",
                ));
            }
            let observation = crate::session_inventory::observe_session(&source, &sessions);
            if !observation.sessions.is_empty() || !observation.uncertainties.is_empty() {
                return Err(invalid(
                    "Original PM conversation retains live or unknown runtime evidence",
                ));
            }
        } else {
            let profile = prefs.settings.launch_profile_or_default();
            if expected.session_id.is_some()
                || prefs.registration.is_some()
                || !member.enabled
                || !member.pm_eligible
                || member.pm_project_key.as_deref() != Some(key.as_str())
                || expected.pm_profile_revision.as_deref()
                    != Some(crate::pm_registry::pm_profile_revision(prefs).as_str())
                || candidate.agent_id.command() != profile.agent_id
                || candidate.model != profile.model
                || candidate.reasoning_level != profile.reasoning
                || candidate.agent_name.as_deref() != Some(member.name.as_str())
            {
                return Err(invalid(
                    "PM selection or profile changed before bind; refresh Agents and retry NEW",
                ));
            }
        }
        registry
            .project_assignments
            .get_mut(key.as_str())
            .and_then(|assignments| assignments.get_mut(member_id))
            .expect("exact assignment")
            .session_id = Some(session_id.into());
        commit_pm_identity(&mut registry)
    })
}

pub(crate) fn reaped_pm_assignment(
    root: &Path,
    captured: &gwt_agent::Session,
    reservation: Option<&str>,
) -> io::Result<
    Option<(
        ProjectKey,
        String,
        crate::agent_names::AgentMemberAssignment,
    )>,
> {
    let Some(registry) = load()? else {
        return Ok(None);
    };
    if registry.phase != RegistryPhase::Active {
        return Err(invalid(
            "Agent registry is prepared; inspect agent.registry.status and reconcile",
        ));
    }
    let Some(_) = registry.roster.as_ref() else {
        return Ok(None);
    };
    let (key, id, expected_token) = if let Some(id) = captured.agent_member_id.as_ref() {
        let assignments = registry
            .project_assignments
            .values()
            .filter_map(|assignments| assignments.get(id))
            .filter(|assignment| assignment.session_id.as_deref() == Some(captured.id.as_str()))
            .collect::<Vec<_>>();
        if assignments.len() > 1 {
            return Err(invalid("PM Session has ambiguous project assignments"));
        }
        let Some(assignment) = assignments.first().copied() else {
            return Ok(None);
        };
        if assignment.role != AgentAssignmentRole::Pm {
            return Err(invalid("Canonical PM Session retains a non-PM assignment"));
        }
        let key = ProjectKey::parse(
            assignment
                .project_key
                .as_deref()
                .ok_or_else(|| invalid("PM assignment has no project"))?,
        )
        .map_err(|error| invalid(error.to_string()))?;
        (
            key,
            id.clone(),
            reservation
                .ok_or_else(|| invalid("Exact PM reservation token is unavailable"))?
                .to_string(),
        )
    } else {
        let mut proofs = Vec::new();
        for (key, project) in &registry.pm_projects {
            let Some(proof) = project.legacy_sessions.get(&captured.id) else {
                continue;
            };
            let key = ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?;
            if pm_session_identity(&key, captured)? != proof.session_identity_sha256 {
                return Err(invalid(
                    "Legacy PM identity proof changed; retain its assignment",
                ));
            }
            proofs.push((
                key.clone(),
                proof.member_id.clone(),
                legacy_pm_reservation(key.as_str(), &captured.id, &proof.session_identity_sha256),
            ));
        }
        if proofs.len() > 1 {
            return Err(invalid(
                "Legacy PM identity proof is ambiguous; retain its assignments",
            ));
        }
        let Some(proof) = proofs.pop() else {
            return Ok(None);
        };
        if reservation.is_some_and(|reservation| reservation != proof.2) {
            return Err(invalid(
                "Legacy PM reservation token changed; retain its assignment",
            ));
        }
        proof
    };
    pm_session_identity(&key, captured)?;
    if root.exists() && gwt_core::paths::project_scope_hash(root) != key {
        return Err(invalid("PM reservation belongs to another exact project"));
    }
    let Some(assignment) = project_assignment(&registry, key.as_str(), &id) else {
        return Ok(None);
    };
    if assignment.role != AgentAssignmentRole::Pm
        || assignment.project_key.as_deref() != Some(key.as_str())
        || assignment.reservation_id != expected_token
        || assignment.session_id.as_deref() != Some(captured.id.as_str())
        || !pm_host_matches(assignment)
    {
        return Err(invalid("PM reservation has a changed Session, token or reserving host; retain the current assignment"));
    }
    Ok(Some((key, id, assignment.clone())))
}

pub(crate) fn release_reaped_pm_assignment(
    key: &ProjectKey,
    member_id: &str,
    expected: &crate::agent_names::AgentMemberAssignment,
    captured: &gwt_agent::Session,
) -> io::Result<()> {
    let pm_path = gwt_core::paths::gwt_project_dir(key).join("project-state/pm.json");
    crate::pm_registry::with_pm_prefs_snapshot(&pm_path, |_| {
        let _guard = lock()?;
        let mut registry = load()?.ok_or_else(|| invalid("PM registry is unavailable"))?;
        if registry.phase != RegistryPhase::Active {
            return Err(invalid(
                "Agent registry is prepared; inspect agent.registry.status and reconcile",
            ));
        }
        confirm_durable()?;
        let identity = pm_session_identity(key, captured)?;
        if captured.agent_member_id.is_none()
            && !registry
                .pm_projects
                .get(key.as_str())
                .and_then(|project| project.legacy_sessions.get(&captured.id))
                .is_some_and(|proof| {
                    proof.member_id == member_id
                        && proof.session_identity_sha256 == identity
                        && expected.reservation_id
                            == legacy_pm_reservation(key.as_str(), &captured.id, &identity)
                })
        {
            return Err(invalid(
                "Legacy PM proof changed before exact owner reap; retain its assignment",
            ));
        }
        let Some(assignment) = project_assignment(&registry, key.as_str(), member_id) else {
            return Ok(());
        };
        if assignment != expected
            || expected.role != AgentAssignmentRole::Pm
            || expected.project_key.as_deref() != Some(key.as_str())
            || !pm_host_matches(expected)
        {
            return Err(invalid(
                "PM reservation changed during exact owner reap; retain the current assignment",
            ));
        }
        set_project_assignment(&mut registry, key.as_str(), member_id, None);
        commit_pm_identity(&mut registry)
    })
}

/// Import only the current, registered PM under Session -> PM prefs -> registry.
/// This path never enrolls a nonexistent Monitor source or edits Session bytes.
pub(crate) fn ensure_current_pm_identity(root: &Path, sessions: &Path) -> io::Result<bool> {
    let key = gwt_core::paths::project_scope_hash(root);
    let pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(root);
    let observed = crate::pm_registry::load_pm_prefs(&pm_path)?;
    let Some(registration) = observed.registration else {
        return Ok(load()?.is_some_and(|registry| {
            registry.phase == RegistryPhase::Active
                && registry
                    .pm_projects
                    .get(key.as_str())
                    .is_some_and(|project| {
                        project.member_id.is_some() || !project.legacy_sessions.is_empty()
                    })
        }));
    };
    ensure_active_for_owner(root)?;
    gwt_agent::with_session_path_lease(sessions, &registration.session_id, |state| {
        let session = match state {
            gwt_agent::SessionPathState::Present(session) => session,
            gwt_agent::SessionPathState::Error(error) => return Err(error),
            gwt_agent::SessionPathState::Missing => return Err(invalid(
                "Registered PM Session is unavailable; open or resume the PM, then refresh Agents",
            )),
        };
        crate::pm_registry::with_pm_prefs_snapshot(&pm_path, |prefs| {
            if prefs.registration.as_ref() != Some(&registration) {
                return Err(invalid(
                    "PM registration changed; refresh Agents before retrying",
                ));
            }
            let identity = pm_session_identity(&key, &session)?;
            if session.id != registration.session_id
                || session.agent_id.command() != registration.agent_id
                || normalized_pm_path(Path::new(&registration.worktree_path))
                    != normalized_pm_path(&session.worktree_path)
            {
                return Err(invalid("Registered PM identity is conflicting; open its exact project and refresh Agents"));
            }
            let _guard = lock()?;
            let mut registry =
                load()?.ok_or_else(|| invalid("PM registry activation is unavailable"))?;
            if registry.phase != RegistryPhase::Active {
                return Err(invalid(
                    "Agent registry is prepared; inspect agent.registry.status and reconcile",
                ));
            }
            confirm_durable()?;
            let before_roster = registry.roster.clone();
            let before_projects = registry.pm_projects.clone();
            let before_assignments = registry.project_assignments.clone();
            if let Some(proof) = registry
                .pm_projects
                .get(key.as_str())
                .and_then(|project| project.legacy_sessions.get(&session.id))
            {
                if proof.session_identity_sha256 != identity {
                    return Err(invalid("Registered legacy PM proof changed; retain its assignment and refresh Agents"));
                }
                return Ok(true);
            }
            let observation = crate::session_inventory::observe_session(&session, sessions);
            let hosts: BTreeSet<_> = observation
                .sessions
                .iter()
                .map(|entry| entry.host_pid)
                .collect();
            if hosts.len() > 1 || !observation.uncertainties.is_empty() {
                return Err(invalid("Registered PM runtime is unknown or ambiguous; open its exact PM conversation and refresh Agents"));
            }
            let host_pid = hosts.into_iter().next().unwrap_or(0);
            let configured = selected_pm_member(&registry, key.as_str()).is_some();
            let member_id = if let Some(id) = session.agent_member_id.as_ref() {
                let member = project_member(&registry, key.as_str(), id).ok_or_else(|| {
                    invalid("Registered PM member is unavailable; refresh Agents")
                })?;
                if member
                    .pm_project_key
                    .as_deref()
                    .is_some_and(|project| project != key.as_str())
                    || member.assignment.as_ref().is_some_and(|assignment| {
                        assignment.role != AgentAssignmentRole::Pm
                            || assignment.session_id.as_deref() != Some(&session.id)
                            || assignment.project_key.as_deref() != Some(key.as_str())
                    })
                {
                    return Err(invalid(
                        "Registered PM member retains a conflicting project assignment",
                    ));
                }
                registry
                    .roster
                    .as_mut()
                    .expect("registered roster")
                    .members
                    .iter_mut()
                    .find(|member| &member.agent_member_id == id)
                    .expect("registered member")
                    .pm_eligible = true;
                if !configured {
                    registry
                        .pm_projects
                        .entry(key.to_string())
                        .or_default()
                        .member_id = Some(id.clone());
                }
                id.clone()
            } else {
                let name = session
                    .agent_name
                    .as_ref()
                    .filter(|name| !name.trim().is_empty())
                    .ok_or_else(|| {
                        invalid("Registered PM name is unknown; open the PM and refresh Agents")
                    })?
                    .clone();
                crate::agent_names::remember(
                    &history_path(),
                    &std::collections::HashSet::from([name.clone()]),
                )
                .map_err(io::Error::other)?;
                let id = uuid::Uuid::new_v4().to_string();
                registry
                    .roster
                    .get_or_insert_with(|| AgentRoster {
                        schema_version: 3,
                        revision: 0,
                        members: Vec::new(),
                    })
                    .members
                    .push(AgentMember {
                        agent_member_id: id.clone(),
                        agent_id: Some(session.agent_id.command().into()),
                        name,
                        enabled: true,
                        pm_eligible: true,
                        pm_project_key: None,
                        created_at: session.created_at.to_rfc3339(),
                        origin_session_id: Some(session.id.clone()),
                        last_issue_number: None,
                        last_spec_number: None,
                        last_owner_project_key: None,
                        assignment: None,
                    });
                if !configured {
                    registry
                        .pm_projects
                        .entry(key.to_string())
                        .or_default()
                        .member_id = Some(id.clone());
                }
                let registration_identity = fingerprint(
                    &serde_json::to_string(&(
                        &registration.session_id,
                        normalized_pm_path(Path::new(&registration.worktree_path)),
                        &registration.agent_id,
                        &registration.created_at,
                    ))
                    .map_err(io::Error::other)?,
                );
                registry
                    .pm_projects
                    .entry(key.to_string())
                    .or_default()
                    .legacy_sessions
                    .insert(
                        session.id.clone(),
                        PmLegacyMemberProof {
                            member_id: id.clone(),
                            session_identity_sha256: identity.clone(),
                            registration_identity_sha256: registration_identity,
                            imported_at: gwt_core::timestamp::UtcTimestamp::now().to_rfc3339(),
                        },
                    );
                id
            };
            registry.pm_projects.entry(key.to_string()).or_default();
            if project_assignment(&registry, key.as_str(), &member_id).is_none() {
                set_project_assignment(
                    &mut registry,
                    key.as_str(),
                    &member_id,
                    Some(crate::agent_names::AgentMemberAssignment {
                        role: AgentAssignmentRole::Pm,
                        pm_profile_revision: None,
                        project_key: Some(key.to_string()),
                        reservation_id: legacy_pm_reservation(key.as_str(), &session.id, &identity),
                        session_id: Some(session.id.clone()),
                        issue_number: None,
                        spec_number: None,
                        delivery_id: None,
                        host_id: if host_pid == 0 {
                            String::new()
                        } else {
                            crate::process::current_hostname()
                        },
                        host_pid,
                        host_started_at: (host_pid > 0)
                            .then(|| crate::process::host_process_start_time(host_pid))
                            .flatten(),
                    }),
                );
            }
            if registry.roster == before_roster
                && registry.pm_projects == before_projects
                && registry.project_assignments == before_assignments
            {
                return Ok(true);
            }
            let roster = registry.roster.as_mut().expect("imported roster");
            roster.schema_version = 3;
            roster.revision = roster
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("Agent roster revision exhausted"))?;
            registry.schema_version = 3;
            registry.revision = registry
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("Agent registry revision exhausted"))?;
            ensure_cores(&mut registry);
            save(&registry)?;
            Ok(true)
        })
    })
}

/// The existing shared configuration revision, without acquiring writer locks.
/// Project runtime commits advance the durable generation independently.
pub(crate) fn active_settings_revision(prefs_path: &Path) -> io::Result<Option<u64>> {
    if source_key(prefs_path).is_none() {
        return Ok(None);
    }
    let Some(registry) = load_proving(Mirrors::None)? else {
        return Ok(None);
    };
    if registry.phase != RegistryPhase::Active {
        return Err(invalid(
            "Agent registry is prepared; inspect agent.registry.status and reconcile",
        ));
    }
    Ok(Some(settings_revision(&registry)))
}

pub fn resolve_member_id_in_source(source: &ProjectKey, observed_id: &str) -> io::Result<String> {
    SessionMemberSnapshot::load()?.resolve_member_id(source, observed_id)
}

/// Exact core observation; names, pool routing and unrelated members are excluded.
pub fn implementation_core_revision(member_id: &str, core: &AgentProfileCore) -> String {
    let bytes = serde_json::to_vec(&(member_id, core)).expect("serializable implementation core");
    format!(
        "implementation-core-v1:{}",
        hex::encode(Sha256::digest(bytes))
    )
}

pub struct MemberTuningOutcome {
    pub snapshot: Option<SessionMemberSnapshot>,
    pub error: Option<String>,
}

/// A target-only transaction. Activation, replay and Session cleanup are separate operations.
pub fn set_member_tuning(request: &crate::agent_names::MemberTuning) -> MemberTuningOutcome {
    let mut snapshot = None;
    let result = (|| -> io::Result<()> {
        loop {
            let observed =
                load()?.ok_or_else(|| invalid("Load the Agent roster before editing tuning"))?;
            let mut keys = source_keys()?;
            keys.extend(observed.sources.keys().cloned());
            let _sources = source_locks(&keys)?;
            let _registry = lock()?;
            let mut current =
                load()?.ok_or_else(|| invalid("Agent registry is unavailable; reload"))?;
            let mut fresh_keys = source_keys()?;
            fresh_keys.extend(current.sources.keys().cloned());
            if fresh_keys != keys {
                continue;
            }
            if keys.iter().any(|key| !current.sources.contains_key(key)) {
                return Err(invalid("Agent source enrollment is required. Refresh the Agent roster before editing tuning."));
            }
            if current.phase != RegistryPhase::Active {
                return Err(invalid(
                    "Agent registry needs recovery; inspect agent.registry.status and reconcile",
                ));
            }
            confirm_durable()?;
            for (key, source) in &current.sources {
                let path = prefs_path(key)?;
                let observed = raw(&path)?;
                assert_known(&current, key, observed.as_deref())?;
                assert_known_floor(source, &path)?;
                if raw_hash(observed.as_deref()) != source.after_sha256
                    || raw_hash(raw(&path.with_extension("committed.json"))?.as_deref())
                        != source.after_sha256
                {
                    return Err(invalid("Agent mirrors need confirmation. Refresh the Agent roster or inspect agent.registry.status and reconcile before editing tuning."));
                }
            }
            snapshot = Some(SessionMemberSnapshot {
                registry: Some(current.clone()),
            });
            let member = current
                .roster
                .as_ref()
                .and_then(|roster| {
                    roster
                        .members
                        .iter()
                        .find(|member| member.agent_member_id == request.member_id)
                })
                .ok_or_else(|| {
                    invalid("Agent member no longer exists; reload registered members")
                })?;
            let core = current
                .profiles
                .get(&request.member_id)
                .ok_or_else(|| invalid("Saved implementation core is unavailable; reload"))?;
            if member.agent_id.as_deref() != Some(core.agent_id.as_str()) {
                return Err(invalid("The member's saved implementation tool is inconsistent; inspect agent.registry.status"));
            }
            if implementation_core_revision(&request.member_id, core)
                != request.expected_core_revision
            {
                return Err(invalid("This implementation profile changed elsewhere. Reload, review your draft and retry."));
            }
            let normalize = |value: &Option<String>| {
                value
                    .as_ref()
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty())
            };
            let mut next = core.clone();
            let agent =
                gwt_agent::resolve_agent_id(request.agent_id.as_deref().unwrap_or(&core.agent_id))
                    .ok_or_else(|| invalid("Unknown Agent tool"))?;
            let custom = if let gwt_agent::AgentId::Custom(id) = &agent {
                Some(
                    crate::issue_monitor::custom_coding_agent_definition(id).ok_or_else(|| {
                        invalid("Choose a known coding tool when editing an Agent")
                    })?,
                )
            } else {
                None
            };
            let model = normalize(&request.model);
            let reasoning = normalize(&request.reasoning);
            gwt_agent::validate_native_tuning(
                agent.command(),
                model.as_deref(),
                reasoning.as_deref(),
            )
            .map_err(invalid)?;
            if next.agent_id != agent.command() {
                next.agent_id = agent.command().into();
                next.model = None;
                next.reasoning = None;
                next.version = None;
                next.fast_mode = false;
                next.codex_speed = gwt_agent::CodexSpeed::Off;
            } else {
                next.model = model;
                next.reasoning = reasoning;
            }
            if let Some(runtime) = request.runtime_target {
                next.runtime_target = runtime;
            }
            if let Some(skip) = request.skip_permissions {
                next.skip_permissions = skip;
            }
            if next.skip_permissions == Some(true)
                && !gwt_agent::provider_skip_mapping(&agent, custom.as_ref()).supported
            {
                return Err(invalid("This tool does not support skipping permissions. Choose normal permissions or inherit the project setting."));
            }
            if *core == next {
                return Ok(());
            }
            current
                .profiles
                .insert(request.member_id.clone(), next.clone());
            current
                .roster
                .as_mut()
                .expect("checked roster")
                .members
                .iter_mut()
                .find(|member| member.agent_member_id == request.member_id)
                .expect("checked member")
                .agent_id = Some(next.agent_id);
            current.revision = current
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("Agent registry revision exhausted"))?;
            let roster = current.roster.as_mut().expect("checked member roster");
            roster.revision = roster
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("Agent roster revision exhausted"))?;
            if let Err(error) = save(&current) {
                // Rename may have committed. Never replay or ACK an uncertain write.
                snapshot = None;
                if confirm_durable().is_ok() {
                    if let Ok(Some(readback)) = load() {
                        snapshot = Some(SessionMemberSnapshot {
                            registry: Some(readback),
                        });
                    }
                }
                return Err(io::Error::other(format!(
                    "Profile write outcome is unknown; reload before retrying: {error}"
                )));
            }
            snapshot = Some(SessionMemberSnapshot {
                registry: Some(current),
            });
            return Ok(());
        }
    })();
    MemberTuningOutcome {
        snapshot,
        error: result.err().map(|error| error.to_string()),
    }
}

/// Keep the selected-member proof locked through the caller's PM durable save.
pub(crate) fn with_configured_pm_guard<T>(
    key: &ProjectKey,
    member_id: &str,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let _guard = lock()?;
    let registry =
        load()?.ok_or_else(|| invalid("Load registered members before editing the PM profile"))?;
    if registry.phase != RegistryPhase::Active {
        return Err(invalid(
            "Agent registry needs recovery; inspect agent.registry.status and reconcile",
        ));
    }
    confirm_durable()?;
    let member = selected_pm_member(&registry, key.as_str());
    if member
        .as_ref()
        .map(|member| member.agent_member_id.as_str())
        != Some(member_id)
    {
        return Err(invalid(
            "The configured Project Manager changed. Reload and review this draft.",
        ));
    }
    operation()
}

/// Coherent read-only PM settings and member identity under the writer's lock order.
pub fn pm_profile_status_snapshot(
    root: &Path,
) -> io::Result<(crate::pm_registry::PmPrefs, SessionMemberSnapshot)> {
    crate::pm_registry::with_pm_prefs_snapshot(
        &crate::pm_registry::pm_prefs_path_for_repo_path(root),
        |prefs| {
            let _guard = lock()?;
            let registry = load()?;
            if let Some(registry) = &registry {
                if registry.phase != RegistryPhase::Active {
                    return Err(invalid("Agent registry needs recovery; inspect agent.registry.status and reconcile"));
                }
                confirm_durable()?;
            }
            Ok((prefs.clone(), SessionMemberSnapshot { registry }))
        },
    )
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

    pub fn project_snapshot(&self, key: &ProjectKey) -> io::Result<IssueMonitorPrefs> {
        let registry = self
            .registry
            .as_ref()
            .filter(|registry| registry.phase == RegistryPhase::Active)
            .ok_or_else(|| invalid("Agent registry snapshot is unavailable; reload"))?;
        project(
            registry,
            key.as_str(),
            registry
                .sources
                .get(key.as_str())
                .map(|source| source.after_json.as_str()),
        )
    }

    pub fn implementation_core(&self, member_id: &str) -> Option<(AgentProfileCore, String)> {
        let registry = self
            .registry
            .as_ref()
            .filter(|registry| registry.phase == RegistryPhase::Active)?;
        let core = registry.profiles.get(member_id)?;
        Some((core.clone(), implementation_core_revision(member_id, core)))
    }

    pub fn configured_pm_member(&self, source: &ProjectKey) -> io::Result<Option<AgentMember>> {
        let Some(registry) = &self.registry else {
            return Ok(None);
        };
        if registry.phase != RegistryPhase::Active {
            return Err(invalid(
                "Agent registry is prepared; inspect agent.registry.status and reconcile",
            ));
        }
        Ok(registry
            .pm_projects
            .get(source.as_str())
            .and_then(|project| project.member_id.as_deref())
            .and_then(|id| project_member(registry, source.as_str(), id)))
    }

    pub fn assignments_for_member(
        &self,
        member_id: &str,
    ) -> io::Result<Vec<crate::agent_names::AgentMemberAssignment>> {
        let Some(registry) = &self.registry else {
            return Ok(Vec::new());
        };
        if registry.phase != RegistryPhase::Active {
            return Err(invalid(
                "Agent registry is prepared; inspect agent.registry.status and reconcile",
            ));
        }
        Ok(registry
            .project_assignments
            .values()
            .filter_map(|assignments| assignments.get(member_id).cloned())
            .collect())
    }

    pub(crate) fn member_in_project(
        &self,
        source: &ProjectKey,
        member_id: &str,
    ) -> io::Result<Option<AgentMember>> {
        let Some(registry) = &self.registry else {
            return Ok(None);
        };
        if registry.phase != RegistryPhase::Active {
            return Err(invalid(
                "Agent registry is prepared; inspect agent.registry.status and reconcile",
            ));
        }
        Ok(project_member(registry, source.as_str(), member_id))
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
        if session.agent_member_id.is_none() {
            if let Some(registry) = self
                .registry
                .as_ref()
                .filter(|registry| registry.phase == RegistryPhase::Active)
            {
                let mut matches = BTreeSet::new();
                for (key, project) in &registry.pm_projects {
                    let Some(proof) = project.legacy_sessions.get(&session.id) else {
                        continue;
                    };
                    let key = ProjectKey::parse(key).map_err(|error| invalid(error.to_string()))?;
                    if pm_session_identity(&key, session)? != proof.session_identity_sha256 {
                        return Err(invalid(
                            "Legacy PM Session has a conflicting exact identity proof",
                        ));
                    }
                    matches.insert(proof.member_id.clone());
                }
                if matches.len() > 1 {
                    return Err(invalid(
                        "Legacy PM Session has ambiguous project-qualified identity proofs",
                    ));
                }
                if let Some(id) = matches.into_iter().next() {
                    return Ok(Some(id));
                }
            }
        }
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
    let local_roster = project_roster(registry, key);
    let mut profiles = prefs.launch_profile_pool();
    for profile in &mut profiles {
        if let Some(id) = profile.agent_member_id.as_deref() {
            if let Ok(global) = resolve(registry, key, id) {
                profile.agent_member_id = Some(global);
            }
        } else if let Some(roster) = &local_roster {
            let source = registry.sources.get(key);
            let matching = |member: &&AgentMember| {
                member.implementation_eligible()
                    && member.agent_id.as_ref() == Some(&profile.agent_id)
            };
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
                    member.enabled
                        && member.implementation_eligible()
                        && member.agent_id.as_ref() == Some(&profile.agent_id)
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
    project_decoded(registry, key, decode(raw)?)
}

/// Project a mirror that was already decoded from its committed bytes.
fn project_decoded(
    registry: &Registry,
    key: &str,
    mut prefs: IssueMonitorPrefs,
) -> io::Result<IssueMonitorPrefs> {
    project_identity(registry, key, &mut prefs)?;
    prefs.agent_roster = project_roster(registry, key);
    if let Some(roster) = &prefs.agent_roster {
        prefs.max_active_agents = roster.enabled_count();
    }
    prefs.schema_version = 2;
    prefs.agent_registry_revision = Some(settings_revision(registry));
    prefs.agent_registry_project_key = Some(key.into());
    prefs.agent_registry_profiles = registry.profiles.clone();
    Ok(prefs)
}

/// Encode the mirror a project commits, keeping what this version cannot read.
///
/// `decoded` is `original` as [`decode`] reads it. A caller that already holds
/// that decode passes it, so the committed bytes are not parsed a second time.
fn mirror_json(
    prefs: &IssueMonitorPrefs,
    original: Option<&str>,
    decoded: Option<&IssueMonitorPrefs>,
) -> io::Result<String> {
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
        profile.codex_speed = gwt_agent::CodexSpeed::Off;
    }
    mirror.set_launch_profile_pool(profiles);
    let fields_of =
        |prefs: &IssueMonitorPrefs| -> io::Result<serde_json::Map<String, serde_json::Value>> {
            match serde_json::to_value(prefs).map_err(io::Error::other)? {
                serde_json::Value::Object(object) => Ok(object),
                _ => Err(invalid("Project preferences must be an object")),
            }
        };
    let mut object = serde_json::Map::new();
    if let Some(original) = original {
        // Only the top level is read: a field this version knows is replaced
        // whole below, and one it does not know is carried as it was written.
        let fields: BTreeMap<String, &serde_json::value::RawValue> = serde_json::from_str(original)
            .map_err(|error| {
                if error.is_data() {
                    invalid("Project preferences must be an object")
                } else {
                    io::Error::other(error)
                }
            })?;
        let mut known = fields_of(&IssueMonitorPrefs::default())?;
        known.extend(match decoded {
            Some(decoded) => fields_of(decoded)?,
            None => fields_of(&decode(Some(original))?)?,
        });
        for (field, value) in fields {
            if field != "agent_roster" && !known.contains_key(&field) {
                object.insert(
                    field,
                    serde_json::from_str(value.get()).map_err(io::Error::other)?,
                );
            }
        }
    }
    object.extend(fields_of(&mirror)?);
    serde_json::to_string_pretty(&serde_json::Value::Object(object)).map_err(io::Error::other)
}

fn source_from(
    registry: &Registry,
    key: &str,
    prefs: &IssueMonitorPrefs,
    original: Option<&str>,
    aliases: BTreeMap<String, String>,
) -> io::Result<Source> {
    let after = mirror_json(prefs, original, None)?;
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
    assert_known_floor_hash(source, path, &observed)
}

fn assert_known_floor_hash(source: &Source, path: &Path, observed: &str) -> io::Result<()> {
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
    #[cfg(test)]
    record_io(|io| io.mirror_publishes += 1);
    // The schema1 recovery writer must not be able to restore an old schema1
    // floor after the canonical schema2 file is damaged.
    crate::issue_monitor::durable_atomic_write_siblings(
        path,
        &path.with_extension("committed.json"),
        source.after_json.as_bytes(),
    )
}

/// Where the mirror is kept as it was before retention first dropped entries.
pub(crate) fn pre_retention_mirror_path(path: &Path) -> PathBuf {
    path.with_extension("pre-retention.json")
}

/// Keep one generation: the mirror the first retention pass is about to shrink.
fn keep_pre_retention_mirror(path: &Path, committed: &str) -> io::Result<()> {
    let kept = pre_retention_mirror_path(path);
    if kept.try_exists()? {
        return Ok(());
    }
    crate::issue_monitor::durable_atomic_write(&kept, committed.as_bytes())
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
        let generation = || -> io::Result<Option<(String, Option<String>)>> {
            let Some(core) = raw(&self::path())? else {
                return Ok(None);
            };
            Ok(Some((core, raw(&record_path(&key))?)))
        };
        let Some(before) = generation()? else {
            return Ok(None);
        };
        let observed = raw(path)?;
        let floor = raw(&path.with_extension("committed.json"))?;
        #[cfg(test)]
        if let Some(observer) = READ_PROJECT_AFTER_SOURCE.with(|slot| slot.borrow_mut().take()) {
            observer();
        }
        let Some(after) = generation()? else {
            continue;
        };
        // Identical bytes are one generation: decode and prove it once.
        if before != after {
            std::thread::yield_now();
            continue;
        }
        let (core, record) = after;
        let (after, mut proven) =
            decode_registry_proven(&core, Mirrors::Only(&key), &|_| Ok(record.clone()))?;
        if after.phase != RegistryPhase::Active {
            return Err(invalid(
                "Agent registry is prepared; inspect agent.registry.status and reconcile",
            ));
        }
        if let Some(source) = after.sources.get(&key) {
            let committed = Some(source.after_json.as_str());
            if observed.as_deref() != committed {
                assert_known(&after, &key, observed.as_deref())?;
            }
            if floor.as_deref() != committed {
                let floor_hash = raw_hash(floor.as_deref());
                if floor_hash != source.floor_before_sha256 && floor_hash != source.after_sha256 {
                    return Err(invalid(format!("Agent recovery floor conflict for {key}: observed={floor_hash}; read agent.registry.status and explicitly reconcile with expected_floor_sha256")));
                }
            }
            return match proven.remove(&key) {
                Some(committed) => project_decoded(&after, &key, committed),
                None => project(&after, &key, Some(&source.after_json)),
            }
            .map(Some);
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
            codex_speed: gwt_agent::CodexSpeed::Off,
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
///
/// The core lock is held until the transaction is known to leave the shared
/// core unchanged; the registry lock is shared unless enrollment needs it whole.
pub(crate) struct RegistryWrite {
    _lock: Option<NamedFileLock>,
    core_lock: Option<NamedFileLock>,
    registry: Option<Registry>,
    key: Option<String>,
    before: Option<IssueMonitorPrefs>,
    /// The committed mirror as decoded, so it is decoded once per generation.
    committed: Option<IssueMonitorPrefs>,
    /// Both mirror files hold the committed generation, proven under these locks.
    mirrors_current: bool,
}

impl RegistryWrite {
    pub(crate) fn acquire(path: &Path) -> io::Result<Self> {
        let Some(key) = source_key(path) else {
            return Ok(Self {
                _lock: None,
                core_lock: None,
                registry: None,
                key: None,
                before: None,
                committed: None,
                mirrors_current: false,
            });
        };
        let core = core_lock()?;
        let mut guard = NamedFileLock::acquire_shared(&lock_path()?, "shared Agent registry")?;
        let loaded = raw(&self::path())?
            .map(|core| {
                decode_registry_proven(&core, Mirrors::Only(&key), &|key| raw(&record_path(key)))
            })
            .transpose()?;
        let (mut registry, mut decoded) = match loaded {
            Some((registry, mut proven)) => (Some(registry), proven.remove(&key)),
            None => (None, None),
        };
        if let Some(current) = &mut registry {
            if current.phase != RegistryPhase::Active {
                return Err(invalid(
                    "Agent registry is prepared; inspect agent.registry.status and reconcile",
                ));
            }
            // Every registry write lowers this marker once it is durable.
            let record_marker = record_unconfirmed_path(&key);
            if unconfirmed_path().try_exists()? || record_marker.try_exists()? {
                confirm_durable()?;
                clear_marker(&record_marker)?;
            }
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
                // Enrollment rewrites shared identity: prove every mirror first,
                // with no other project's record write open. The core lock
                // held throughout keeps the shared core as it was just read.
                drop(guard);
                guard = NamedFileLock::acquire(&lock_path()?, "shared Agent registry")?;
                *current = load()?.ok_or_else(|| invalid("Agent registry disappeared"))?;
                enroll(current, &key, None, &[])?;
                save(current)?;
                decoded = None;
            }
            let source = &current.sources[&key];
            if decoded.is_none() {
                decoded = Some(decode(Some(&source.after_json))?);
            }
            let committed = Some(source.after_json.as_str());
            let observed = raw(path)?;
            let floor = raw(&path.with_extension("committed.json"))?;
            // Equal bytes are the committed generation; only a difference is hashed.
            let mirror_current = observed.as_deref() == committed;
            let floor_current = floor.as_deref() == committed;
            if !mirror_current {
                assert_known(current, &key, observed.as_deref())?;
            }
            if !floor_current {
                assert_known_floor_hash(source, path, &raw_hash(floor.as_deref()))?;
            }
            if !mirror_current || !floor_current {
                publish_verified(source, path)?;
            }
        }
        Ok(Self {
            _lock: Some(guard),
            core_lock: Some(core),
            mirrors_current: registry.is_some(),
            registry,
            key: Some(key),
            before: None,
            committed: decoded,
        })
    }

    /// The committed generation as its readers see it.
    fn project_committed(&self, registry: &Registry, key: &str) -> io::Result<IssueMonitorPrefs> {
        match &self.committed {
            Some(committed) => project_decoded(registry, key, committed.clone()),
            None => project(registry, key, Some(&registry.sources[key].after_json)),
        }
    }

    pub(crate) fn load(&mut self, path: &Path) -> io::Result<Option<IssueMonitorPrefs>> {
        let (Some(registry), Some(key)) = (&self.registry, &self.key) else {
            return Ok(None);
        };
        let prefs = self.project_committed(registry, key)?;
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
        // `before` is this same committed generation whenever it was loaded.
        let latest = match &self.before {
            Some(before) => before.clone(),
            None => self.project_committed(registry, key)?,
        };
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
                store_project_roster(&mut next, key, prefs.agent_roster.clone())?;
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
                    let core = AgentProfileCore::from_profile(&profile)
                        .retaining_overrides(next.profiles.get(id));
                    next.profiles.insert(id.clone(), core);
                }
            }
        }
        ensure_cores(&mut next);
        let global_changed = next.roster != registry.roster
            || next.profiles != registry.profiles
            || next.project_assignments != registry.project_assignments
            || next.pm_projects != registry.pm_projects;
        if registry.store.split && !global_changed {
            // Only this project's record can change from here on, and the
            // prefs lock already orders its writers: let the other projects
            // read the shared core before the mirror is encoded and written.
            self.core_lock = None;
        }
        candidate.agent_roster = project_roster(&next, key);
        if let Some(roster) = &candidate.agent_roster {
            candidate.max_active_agents = roster.enabled_count();
        }
        project_profiles(&next, key, &mut candidate);
        crate::agent_names::validate_registered_profiles(
            &candidate,
            &candidate.launch_profile_pool(),
        )
        .map_err(invalid)?;
        validate_assignment_budget(registry, &next)?;
        // Issue #7550: every writer saves through here, so none can keep or
        // bring back what the retention rule drops.
        candidate.enforce_retention();
        let source = &registry.sources[key];
        let after_json = mirror_json(
            &candidate,
            Some(&source.after_json),
            self.committed.as_ref(),
        )?;
        if next.roster == registry.roster
            && next.profiles == registry.profiles
            && next.project_assignments == registry.project_assignments
            && next.pm_projects == registry.pm_projects
            && after_json == source.after_json
        {
            return Ok(Some(latest));
        }
        // In split storage the revision belongs to the shared core, which a
        // transaction that changes only this project's mirror never writes.
        if global_changed || !registry.store.split {
            next.revision = next
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("Agent registry revision exhausted"))?;
        }
        if let Some(roster) = &mut next.roster {
            roster.schema_version = 3;
            if global_changed {
                roster.revision = settings_revision(registry)
                    .checked_add(1)
                    .ok_or_else(|| invalid("Agent settings revision exhausted"))?;
            }
        }
        let committed = if self.mirrors_current {
            // Both files hold `source.after_json`: it is the superseded
            // canonical and floor snapshot without reading or hashing it again.
            Source {
                transaction_id: uuid::Uuid::new_v4().to_string(),
                before_sha256: source.after_sha256.clone(),
                before_json: Some(source.after_json.clone()),
                floor_before_sha256: source.after_sha256.clone(),
                floor_before_json: Some(source.after_json.clone()),
                after_sha256: fingerprint(&after_json),
                after_json,
                was_active: source.was_active,
                aliases: source.aliases.clone(),
                session_aliases: source.session_aliases.clone(),
                rosterless_snapshots: source.rosterless_snapshots.clone(),
            }
        } else {
            source_from(
                &next,
                key,
                &candidate,
                Some(&source.after_json),
                source.aliases.clone(),
            )?
        };
        next.sources.insert(key.clone(), committed);
        if self
            .committed
            .clone()
            .is_some_and(|mut committed| committed.enforce_retention())
        {
            keep_pre_retention_mirror(path, &source.after_json)?;
        }
        // Decode the new mirror before it becomes the committed generation.
        let proven = decode(Some(&next.sources[key].after_json))?;
        let saved = project_decoded(&next, key, proven.clone())?;
        // The sole logical commit point for an Active transaction. Its mirror
        // was serialized, fingerprinted and decoded just above.
        if registry.store.split && !global_changed {
            save_record(&next, key)?;
        } else if self.core_lock.is_some() {
            save_proving(&next, Mirrors::None)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Agent registry changed; reload the exact project before retrying",
            ));
        }
        self.registry = Some(next.clone());
        self.committed = Some(proven);
        // `acquire` proved both mirrors under the locks this transaction still holds.
        self.mirrors_current = false;
        publish_verified(&next.sources[key], path)?;
        self.mirrors_current = true;
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
        roster.schema_version = 3;
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
                    schema_version: 3,
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
    extract_legacy_runtime(registry)?;
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
            let core = AgentProfileCore::from_profile(&profile)
                .retaining_overrides(registry.profiles.get(id));
            registry.profiles.insert(id.clone(), core);
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

/// Starting a physical owner does not opt a PM-only project into the Monitor.
/// Real Monitor sources and observed implementation seeds retain their fences.
pub(crate) fn ensure_active_for_owner(root: &Path) -> io::Result<()> {
    let key = gwt_core::paths::project_scope_hash(root);
    let current = crate::issue_monitor_prefs_path_for_repo_path(root);
    if current.try_exists()?
        || current.with_extension("committed.json").try_exists()?
        || load()?.is_some_and(|registry| registry.sources.contains_key(key.as_str()))
        || crate::agent_registry_import::load()?
            .is_some_and(|imports| imports.sources.contains_key(key.as_str()))
    {
        ensure_active_for_project(root)
    } else {
        ensure_active(None)
    }
}

/// Called only at mutating entry points, before their source writer lock.
/// Ordinary public preference reads never materialize an enrollment.
pub(crate) fn preflight_write(path: &Path) -> io::Result<()> {
    let Some(key) = source_key(path) else {
        return Ok(());
    };
    // Enrollment is shared state; this project's mirror is proven under its lock.
    let Some(registry) = load_proving(Mirrors::None)? else {
        return Ok(());
    };
    if registry.phase == RegistryPhase::Active && registry.enrolled(&key) {
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
    ensure_active(Some(source))
}

fn ensure_active(source: Option<&ProjectKey>) -> io::Result<()> {
    let key = source.map(ToString::to_string);
    let current_path = key.as_deref().map(prefs_path).transpose()?;
    let observed_registry = load()?;
    if let Some(current_path) = current_path.as_ref().filter(|_| {
        observed_registry.as_ref().is_some_and(|registry| {
            registry.phase == RegistryPhase::Active
                && key
                    .as_ref()
                    .is_some_and(|key| registry.sources.contains_key(key))
        })
    }) {
        fs::create_dir_all(current_path.parent().expect("project state parent"))?;
        let _source = NamedFileLock::acquire(
            &current_path.with_extension("lock"),
            "Agent registry enrollment",
        )?;
        let _registry = RegistryWrite::acquire(current_path)?;
        return Ok(());
    }
    if gwt_agent::current_thread_holds_session_lease() {
        return Err(io::Error::new(io::ErrorKind::WouldBlock, format!(
            "Agent source {key:?} needs migration outside the current Session lease; retry after the Session operation settles"
        )));
    }
    let (seeds, session_leases) = leased_seeds()?;
    let mut keys = source_keys()?;
    keys.extend(seeds.keys().cloned());
    if let Some(registry) = &observed_registry {
        keys.extend(registry.sources.keys().cloned());
    }
    keys.extend(key);
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
            return ensure_active(source);
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
        return ensure_active(source);
    }
    let imports_lock = crate::agent_registry_import::lock_for_registry()?;
    let mut imports = crate::agent_registry_import::load()?.unwrap_or_default();
    let mut registry = Registry {
        schema_version: 3,
        phase: RegistryPhase::Prepared,
        revision: 1,
        roster: None,
        profiles: BTreeMap::new(),
        sources: BTreeMap::new(),
        pm_projects: BTreeMap::new(),
        project_assignments: BTreeMap::new(),
        store: Store::default(),
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

pub(crate) fn inactive_import_guard() -> io::Result<RegistryLock> {
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
        .project_assignments
        .values()
        .flat_map(|assignments| assignments.values())
        .collect();
    Ok(RegistryStatus {
        phase: registry.phase,
        registry_revision: registry.revision,
        projects,
        assigned_members: registry
            .project_assignments
            .values()
            .flat_map(|assignments| assignments.keys())
            .collect::<BTreeSet<_>>()
            .len(),
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
                    let current =
                        project_member(&registry, key.as_str(), global).ok_or_else(|| {
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
            if let Some(assignments) = registry.project_assignments.get_mut(key.as_str()) {
                assignments.retain(|id, _| !retired.contains(id));
            }
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

/// Fill a project's prefs to the record size Issue #7550 measured (1.3 MB).
#[cfg(test)]
pub(crate) fn fill_heavy_prefs_for_test(prefs: &mut IssueMonitorPrefs) {
    prefs.closure_records = (0..2_237u64)
        .map(|issue| crate::issue_monitor::IssueClosureRecord {
            issue_number: 10_000 + issue,
            generation: 3,
            state: Default::default(),
            evidence: Default::default(),
            issue_updated_at: Some("2026-11-17T12:16:00Z".into()),
            reopened_after_close: false,
        })
        .collect();
    prefs.claim_diagnostics = (0..377u64)
        .map(|issue| {
            let diagnostics = crate::issue_monitor::IssueMonitorClaimDiagnostics {
                claim_id: Some(format!(
                    "gwt-queue:{issue}:b47249f5-bb49-4252-a319-5eebc718ab61"
                )),
                owner: Some("ccs-win-125:operator:50212".into()),
                expires_at: Some("2026-11-17T12:47:49.661945900+00:00".into()),
                last_attempt_at: Some("2026-11-17T12:32:49.661938400+00:00".into()),
                last_success_at: Some("2026-11-17T12:32:49.661938400+00:00".into()),
                ..Default::default()
            };
            (20_000 + issue, diagnostics)
        })
        .collect();
    prefs.claim_identities = (0..471u64)
        .map(|issue| crate::issue_monitor::IssueMonitorClaimIdentity {
            issue_number: 20_000 + issue,
            claim_id: format!("gwt-queue:{issue}:b47249f5-bb49-4252-a319-5eebc718ab61"),
            owner: "ccs-win-125:operator:50212".into(),
        })
        .collect();
    prefs.merged_issues = (0..52_000).map(|index| 1_000_000 + index).collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};

    fn pm_only_fixture() -> (PathBuf, gwt_agent::Session, crate::pm_registry::PmPrefs) {
        let root = gwt_core::paths::gwt_home().join("pm-only-project");
        fs::create_dir_all(&root).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let key = gwt_core::paths::project_scope_hash(&root);
        let worktree = crate::pm_registry::pm_worktree_path_for_repo_path(&root);
        fs::create_dir_all(&worktree).unwrap();
        let mut session =
            gwt_agent::Session::new(&worktree, "gwt/pm", gwt_agent::AgentId::ClaudeCode);
        session.repo_hash = Some(key.to_string());
        session.project_state_root = Some(root.clone());
        session.agent_name = Some("Legacy PM".into());
        session.model = Some("historical-model".into());
        session.agent_session_id = Some(uuid::Uuid::new_v4().to_string());
        session.save(&sessions).unwrap();
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        let prefs = crate::pm_registry::PmPrefs {
            registration: Some(crate::pm_registry::PmRegistration {
                session_id: session.id.clone(),
                agent_id: "claude".into(),
                worktree_path: worktree.to_string_lossy().into_owned(),
                created_at: Some(session.created_at.to_rfc3339()),
                consecutive_crashes: 0,
                next_not_before: None,
            }),
            ..Default::default()
        };
        crate::pm_registry::save_pm_prefs(&prefs_path, &prefs).unwrap();
        (root, session, prefs)
    }

    fn measured_machine_budget(budget: usize) {
        let observed_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let path = gwt_core::paths::gwt_home().join("machine-state/agent-capacity.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            serde_json::to_vec(&serde_json::json!({
                "observed_at": observed_at,
                "expires_at": observed_at + 30,
                "machine_budget": budget,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn pm_only_identity_import_preserves_legacy_bytes_without_a_monitor_source() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, _) = pm_only_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let key = gwt_core::paths::project_scope_hash(&root);
        let worktree = session.worktree_path.clone();
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        let session_path = sessions.join(format!("{}.toml", session.id));
        let session_bytes = fs::read(&session_path).unwrap();
        let pm_bytes = fs::read(&prefs_path).unwrap();
        let roster = crate::agent_names::read_project_roster(&root, &sessions).unwrap();
        let monitor_path = crate::issue_monitor_prefs_path_for_repo_path(&root);
        assert!(
            !monitor_path.exists(),
            "PM identity import must not invent a Monitor source"
        );
        assert!(!monitor_path.with_extension("committed.json").exists());
        assert_eq!(roster.schema_version, 3);
        assert_eq!(roster.members.len(), 1);
        let member = &roster.members[0];
        assert_eq!(
            member.origin_session_id.as_deref(),
            Some(session.id.as_str())
        );
        assert_eq!(member.pm_project_key.as_deref(), Some(key.as_str()));
        assert!(member.pm_eligible);
        assert_eq!(roster.implementation_enabled_count(), 0);
        assert_eq!(
            SessionMemberSnapshot::load()
                .unwrap()
                .resolve_session(&session)
                .unwrap(),
            Some(member.agent_member_id.clone())
        );
        assert_eq!(
            crate::agent_names::read_project_roster(&root, &sessions).unwrap(),
            roster
        );
        assert_eq!(fs::read(&session_path).unwrap(), session_bytes);
        assert_eq!(fs::read(&prefs_path).unwrap(), pm_bytes);
        // A prepared NEW may not bind after its independently saved PM profile changes.
        let mut registry = load().unwrap().unwrap();
        let member_id = member.agent_member_id.clone();
        set_project_assignment(&mut registry, key.as_str(), &member_id, None);
        save(&registry).unwrap();
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| prefs.registration = None)
            .unwrap();
        let prepared =
            reserve_new_pm_member(&root, &sessions, &worktree, "pm-new-attempt").unwrap();
        assert_eq!(prepared.member.agent_member_id, member_id);
        assert_eq!(
            prepared.member.assignment.as_ref().unwrap().role,
            AgentAssignmentRole::Pm
        );
        let mut candidate =
            gwt_agent::Session::new(&worktree, "gwt/pm", gwt_agent::AgentId::ClaudeCode);
        candidate.repo_hash = Some(key.to_string());
        candidate.project_state_root = Some(root.clone());
        candidate.agent_member_id = Some(member_id.clone());
        candidate.agent_name = Some(member.name.clone());
        candidate.save(&sessions).unwrap();
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            let mut profile = prefs.settings.launch_profile_or_default();
            profile.model = Some("next-new-model".into());
            prefs.settings.launch_profile = Some(profile);
        })
        .unwrap();
        let before = fs::read(path()).unwrap();
        assert!(
            crate::agent_names::bind_project_member_session(
                &root,
                &member_id,
                "pm-new-attempt",
                &candidate.id
            )
            .is_err(),
            "a stale captured PM profile must be rejected before provider handoff"
        );
        assert_eq!(fs::read(path()).unwrap(), before);
        assert!(!monitor_path.exists());
        assert_eq!(fs::read(&session_path).unwrap(), session_bytes);
        candidate.update_status(gwt_agent::AgentStatus::Stopped);
        candidate.save(&sessions).unwrap();
        assert!(cancel_unspawned_pm_member(
            &root,
            &sessions,
            &member_id,
            "pm-new-attempt",
            Some(&candidate),
            None
        )
        .unwrap());
        let fresh = reserve_new_pm_member(&root, &sessions, &worktree, "fresh-pm-attempt").unwrap();
        assert_eq!(fresh.member.agent_member_id, member_id);
        assert_eq!(fresh.profile.model.as_deref(), Some("next-new-model"));
        let before = fs::read(path()).unwrap();
        assert!(!cancel_unspawned_pm_member(
            &root,
            &sessions,
            &member_id,
            "pm-new-attempt",
            Some(&candidate),
            None
        )
        .unwrap());
        assert_eq!(
            fs::read(path()).unwrap(),
            before,
            "an old cancellation must retain the fresh reservation"
        );
        assert!(cancel_unspawned_pm_member(
            &root,
            &sessions,
            &member_id,
            "fresh-pm-attempt",
            None,
            None
        )
        .unwrap());
        assert_eq!(fs::read(&session_path).unwrap(), session_bytes);
        // A provider-only saved row is not an explicit person pin.
        let (source_root, source_path, _) = source();
        ensure_active_for_project(&source_root).unwrap();
        let source_key = source_key(&source_path).unwrap();
        let registry = load().unwrap().unwrap();
        let target = registry.sources[&source_key]
            .aliases
            .values()
            .next()
            .unwrap()
            .clone();
        let eligible = crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::SetPmEligibility {
                member_id: target.clone(),
                eligible: true,
            },
            Some(registry.roster.as_ref().unwrap().revision),
        )
        .unwrap();
        let write_candidate = |registry: &mut Registry, profile: IssueMonitorLaunchProfile| {
            let mut prefs = decode(Some(&registry.sources[&source_key].after_json)).unwrap();
            prefs.set_launch_profile_pool(vec![profile]);
            let before = registry.sources[&source_key].clone();
            let next = source_from(
                registry,
                &source_key,
                &prefs,
                Some(&before.after_json),
                before.aliases.clone(),
            )
            .unwrap();
            registry.sources.insert(source_key.clone(), next);
            registry.revision += 1;
            save(registry).unwrap();
            publish(&registry.sources[&source_key], &source_path).unwrap();
        };
        let provider_only: IssueMonitorLaunchProfile =
            serde_json::from_value(serde_json::json!({"agent_id":"codex"})).unwrap();
        write_candidate(&mut load().unwrap().unwrap(), provider_only.clone());
        let selected = crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::SelectPm { member_id: target },
            Some(eligible.revision),
        )
        .expect("provider-only input must not become an inferred person pin");
        let added = crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Add {
                model: Some("pm-only-registration-model".into()),
                reasoning: Some("high".into()),
                agent_id: Some("codex".into()),
                name: Some("Pinned candidate".into()),
                pm_eligible: true,
            },
            Some(selected.revision),
        )
        .unwrap();
        let pinned = added.members.last().unwrap().agent_member_id.clone();
        let core = SessionMemberSnapshot::load()
            .unwrap()
            .implementation_core(&pinned)
            .unwrap()
            .0;
        assert_eq!(core.model.as_deref(), Some("pm-only-registration-model"));
        assert_eq!(core.reasoning.as_deref(), Some("high"));
        assert!(!monitor_path.exists());
        let mut explicit = provider_only;
        explicit.agent_member_id = Some(pinned.clone());
        write_candidate(&mut load().unwrap().unwrap(), explicit);
        let before = fs::read(path()).unwrap();
        let source_before = fs::read(&source_path).unwrap();
        crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::SelectPm {
                member_id: pinned.clone(),
            },
            Some(added.revision),
        )
        .expect("a candidate in another project does not exclude this project's PM");
        assert_ne!(fs::read(path()).unwrap(), before);
        assert_eq!(
            selected_pm_member(&load().unwrap().unwrap(), key.as_str())
                .unwrap()
                .agent_member_id,
            pinned
        );
        assert_eq!(fs::read(&source_path).unwrap(), source_before);
        assert!(!monitor_path.exists());
    }

    #[test]
    fn pm_fresh_owner_draft_reuses_only_its_exact_unpublished_bootstrap_reservation() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let root = home.path().join("fresh-pm");
        fs::create_dir_all(&root).unwrap();
        let worktree = crate::pm_registry::pm_worktree_path_for_repo_path(&root);
        fs::create_dir_all(&worktree).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        fs::create_dir_all(&sessions).unwrap();
        let prefs = crate::pm_registry::load_pm_prefs(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&root),
        )
        .unwrap();
        let revision = crate::pm_registry::pm_profile_revision(&prefs);
        let draft = PmLaunchDraft {
            member_id: None,
            profile_revision: &revision,
            agent_id: "claude",
            model: None,
            reasoning: None,
            codex_speed: gwt_agent::CodexSpeed::Off,
            fast_mode: false,
        };
        for stale in [
            PmLaunchDraft {
                codex_speed: gwt_agent::CodexSpeed::Fast,
                ..draft
            },
            PmLaunchDraft {
                fast_mode: true,
                ..draft
            },
        ] {
            assert!(
                reserve_new_pm_member_from_draft(
                    &root,
                    &sessions,
                    &worktree,
                    "stale-speed",
                    &stale
                )
                .is_err(),
                "PM speed and Fast Mode must be fenced before reserving a member"
            );
        }
        let first = reserve_new_pm_member_from_draft(
            &root,
            &sessions,
            &worktree,
            "bootstrap-attempt",
            &draft,
        )
        .unwrap();
        let before = fs::read(path()).unwrap();
        let replay = reserve_new_pm_member_from_draft(
            &root,
            &sessions,
            &worktree,
            "bootstrap-attempt",
            &draft,
        )
        .expect("the same exact unpublished bootstrap attempt is idempotent");
        assert_eq!(replay.member, first.member);
        assert_eq!(fs::read(path()).unwrap(), before);
        let registry = load().unwrap().unwrap();
        let roster = project_roster(
            &registry,
            gwt_core::paths::project_scope_hash(&root).as_str(),
        )
        .unwrap();
        assert_eq!(roster.members.len(), 1);
        assert_eq!(roster.implementation_enabled_count(), 0);
        assert!(gwt_agent::session_ledger::load_sessions_shared(&sessions)
            .unwrap()
            .is_empty());
        assert!(!crate::issue_monitor_prefs_path_for_repo_path(&root).exists());
        assert!(reserve_new_pm_member_from_draft(
            &root,
            &sessions,
            &worktree,
            "other-attempt",
            &draft
        )
        .is_err());
        assert_eq!(fs::read(path()).unwrap(), before);
        let added = crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Add {
                model: None,
                reasoning: None,
                agent_id: Some("codex".into()),
                name: Some("Successor".into()),
                pm_eligible: true,
            },
            Some(roster.revision),
        )
        .unwrap();
        let successor = added.members.last().unwrap().agent_member_id.clone();
        crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::SelectPm {
                member_id: successor,
            },
            Some(added.revision),
        )
        .unwrap();
        let changed = fs::read(path()).unwrap();
        assert!(reserve_new_pm_member_from_draft(
            &root,
            &sessions,
            &worktree,
            "bootstrap-attempt",
            &draft
        )
        .is_err());
        assert_eq!(fs::read(path()).unwrap(), changed);
        assert_eq!(
            project_assignment(
                &load().unwrap().unwrap(),
                gwt_core::paths::project_scope_hash(&root).as_str(),
                &first.member.agent_member_id
            )
            .cloned(),
            first.member.assignment
        );
        let defaults_root = home.path().join("pm-with-agent-defaults");
        fs::create_dir_all(&defaults_root).unwrap();
        crate::try_mutate_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(&defaults_root),
            |prefs| {
                prefs.set_launch_profile_pool(vec![serde_json::from_value(serde_json::json!({
                    "agent_id":"codex", "model":"gpt-6-astra", "reasoning":"high", "codex_speed":"ultrafast"
                })).unwrap()]);
                Ok(())
            },
        ).unwrap();
        let pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(&defaults_root);
        let mut prefs = crate::pm_registry::load_pm_prefs(&pm_path).unwrap();
        prefs.settings.launch_profile = Some(crate::pm_registry::PmLaunchProfile {
            agent_id: "codex".into(),
            ..Default::default()
        });
        crate::pm_registry::save_pm_prefs(&pm_path, &prefs).unwrap();
        let revision = crate::pm_registry::pm_profile_revision(&prefs);
        let configured = SessionMemberSnapshot::load()
            .unwrap()
            .configured_pm_member(&gwt_core::paths::project_scope_hash(&defaults_root))
            .unwrap();
        let effective = prefs
            .settings
            .launch_profile_with_agent_defaults(&defaults_root);
        assert_eq!(effective.model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(
            effective.codex_speed,
            Some(gwt_agent::CodexSpeed::Ultrafast)
        );
        measured_machine_budget(2);
        let inherited = reserve_new_pm_member_from_draft(
            &defaults_root,
            &sessions,
            &crate::pm_registry::pm_worktree_path_for_repo_path(&defaults_root),
            "inherited-defaults",
            &PmLaunchDraft {
                member_id: configured
                    .as_ref()
                    .map(|member| member.agent_member_id.as_str()),
                profile_revision: &revision,
                agent_id: "codex",
                model: Some("gpt-6-astra"),
                reasoning: Some("high"),
                codex_speed: gwt_agent::CodexSpeed::Ultrafast,
                fast_mode: false,
            },
        )
        .expect("owner admission must resolve the same inherited defaults as its NEW draft");
        assert_eq!(inherited.profile.model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(
            inherited.profile.codex_speed,
            Some(gwt_agent::CodexSpeed::Ultrafast)
        );
    }

    #[test]
    fn reaped_sampled_child_releases_capacity_before_the_census_expires() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let root = home.path().join("project");
        fs::create_dir_all(&root).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        crate::agent_names::read_project_roster(&root, &sessions).unwrap();
        crate::machine_agent_settings::set(
            Some(1),
            crate::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
            "2026-11-10T00:00:00Z",
        )
        .unwrap();
        measured_machine_budget(2);
        let census_path = gwt_core::paths::gwt_home().join("machine-state/agent-capacity.json");
        let mut census: serde_json::Value =
            serde_json::from_slice(&fs::read(&census_path).unwrap()).unwrap();
        let pid = std::process::id();
        let birth = crate::process::host_process_start_time(pid).unwrap();
        census["inventory"] = serde_json::json!({
            "sessions": [{
                "project_root": root, "repo_hash": null,
                "role": "implementation", "monitor_owned": false, "resident_memory_bytes": null,
                "session": {
                    "session_id": "sampled-child", "issue_number": null,
                    "execution_binding_missing": false, "agent_id": "codex",
                    "worktree_path": root, "worktree_exists": true,
                    "host_pid": pid, "child_pid": pid, "child_started_at": birth,
                    "started_at": "2026-11-10T00:00:00Z", "launch_origin": "unknown",
                    "launch_entrypoint": null, "restore_source_session_id": null
                }
            }],
            "uncertainties": []
        });
        let reserve = || {
            crate::agent_names::reserve_project_member(
                &root,
                &sessions,
                "replacement",
                None,
                None,
                None,
                None,
            )
        };
        fs::write(&census_path, serde_json::to_vec(&census).unwrap()).unwrap();
        assert!(reserve().unwrap_err().contains("Machine Agent capacity"));
        let absent_pid = i32::MAX as u32;
        assert_eq!(
            crate::process::host_process_liveness(absent_pid),
            Some(false)
        );
        assert!(!crate::process::is_process_group_alive(absent_pid));
        census["inventory"]["sessions"][0]["session"]["child_pid"] = absent_pid.into();
        census["inventory"]["sessions"][0]["session"]["child_started_at"] = 0.into();
        fs::write(&census_path, serde_json::to_vec(&census).unwrap()).unwrap();
        assert!(reserve().unwrap_err().contains("Machine Agent capacity"));

        census["inventory"]["sessions"][0]["session"]["child_started_at"] = birth.into();
        fs::write(&census_path, serde_json::to_vec(&census).unwrap()).unwrap();
        reserve()
            .expect("a reaped sampled child must free its slot without waiting for census TTL");
    }

    #[test]
    fn shared_pm_reservations_obey_total_budget_without_charging_manual_worker_limit() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let a = home.path().join("a");
        let b = home.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let first = reserve_new_pm_member(
            &a,
            &sessions,
            &crate::pm_registry::pm_worktree_path_for_repo_path(&a),
            "first-pm",
        )
        .unwrap();
        let roster = crate::agent_names::read_project_roster(&b, &sessions).unwrap();
        let error = crate::agent_names::reserve_project_member(
            &b,
            &sessions,
            "worker-after-pm",
            None,
            None,
            None,
            None,
        )
        .expect_err("unmeasured default Auto must count the already reserved PM against its total");
        assert!(error.contains("Machine Agent capacity"));
        crate::agent_names::mutate_project_roster(
            &b,
            &sessions,
            AgentRosterAction::SelectPm {
                member_id: first.member.agent_member_id.clone(),
            },
            Some(roster.revision),
        )
        .unwrap();
        let prefs =
            crate::pm_registry::load_pm_prefs(&crate::pm_registry::pm_prefs_path_for_repo_path(&b))
                .unwrap();
        let revision = crate::pm_registry::pm_profile_revision(&prefs);
        let draft = PmLaunchDraft {
            member_id: Some(&first.member.agent_member_id),
            profile_revision: &revision,
            agent_id: "claude",
            model: None,
            reasoning: None,
            codex_speed: gwt_agent::CodexSpeed::Off,
            fast_mode: false,
        };
        let reserve_second = || {
            reserve_new_pm_member_from_draft(
                &b,
                &sessions,
                &crate::pm_registry::pm_worktree_path_for_repo_path(&b),
                "second-pm",
                &draft,
            )
        };
        let before = fs::read(path()).unwrap();
        crate::machine_agent_settings::set(
            None,
            crate::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
            "2026-11-10T00:00:00Z",
        )
        .unwrap();
        assert_eq!(
            reserve_second()
                .err()
                .expect("NEW PM must wait for an explicit Auto measurement")
                .kind(),
            io::ErrorKind::WouldBlock
        );
        measured_machine_budget(1);
        assert_eq!(
            reserve_second()
                .err()
                .expect("NEW PM must reject a fully occupied measured budget")
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(fs::read(path()).unwrap(), before);
        crate::machine_agent_settings::set(
            Some(1),
            crate::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
            "2026-11-10T00:01:00Z",
        )
        .unwrap();
        assert_eq!(
            reserve_second()
                .err()
                .expect("Manual worker limit must not bypass the PM physical budget")
                .kind(),
            io::ErrorKind::WouldBlock
        );
        measured_machine_budget(2);
        let second =
            reserve_second().expect("two PM slots fit despite the worker-only limit of one");
        assert_eq!(second.member.agent_member_id, first.member.agent_member_id);
        assert_eq!(
            SessionMemberSnapshot::load()
                .unwrap()
                .assignments_for_member(&first.member.agent_member_id)
                .unwrap()
                .len(),
            2
        );
        measured_machine_budget(0);
        assert_eq!(
            reserve_second().unwrap().member,
            second.member,
            "same-slot replay does not consume another slot"
        );
    }

    #[test]
    fn pm_restore_retains_original_member_profile_and_prior_absence_on_cancel() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, source, mut prefs) = pm_only_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        let original = crate::agent_names::read_project_roster(&root, &sessions).unwrap();
        let old_id = original.members[0].agent_member_id.clone();
        let added = crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::Add {
                model: None,
                reasoning: None,
                agent_id: Some("codex".into()),
                name: Some("Successor".into()),
                pm_eligible: true,
            },
            Some(original.revision),
        )
        .unwrap();
        let successor = added.members.last().unwrap().agent_member_id.clone();
        crate::agent_names::mutate_project_roster(
            &root,
            &sessions,
            AgentRosterAction::SelectPm {
                member_id: successor.clone(),
            },
            Some(added.revision),
        )
        .unwrap();
        // A completed crash reap can retain registration/backoff with no assignment.
        let mut registry = load().unwrap().unwrap();
        set_project_assignment(
            &mut registry,
            gwt_core::paths::project_scope_hash(&root).as_str(),
            &old_id,
            None,
        );
        save(&registry).unwrap();
        prefs.settings.launch_profile = Some(crate::pm_registry::PmLaunchProfile {
            agent_id: "codex".into(),
            model: Some("configured-successor-model".into()),
            reasoning: None,
            version: None,
            codex_speed: None,
            fast_mode: None,
        });
        crate::pm_registry::save_pm_prefs(&pm_path, &prefs).unwrap();
        let old_bytes = fs::read(sessions.join(format!("{}.toml", source.id))).unwrap();
        let profile_bytes = fs::read(&pm_path).unwrap();
        crate::machine_agent_settings::set(
            None,
            crate::issue_monitor::IssueMonitorMaxActiveChangeSource::JsonOperation,
            "2026-11-10T00:00:00Z",
        )
        .unwrap();
        measured_machine_budget(0);
        let before = fs::read(path()).unwrap();
        assert_eq!(
            reserve_resume_pm_member(&root, &sessions, &source, "restore-pm-attempt", false)
                .err()
                .expect("PM Resume must reject a zero measured physical budget")
                .kind(),
            io::ErrorKind::WouldBlock,
        );
        assert_eq!(fs::read(path()).unwrap(), before);
        measured_machine_budget(1);
        let restored =
            reserve_resume_pm_member(&root, &sessions, &source, "restore-pm-attempt", false)
                .unwrap();
        assert_eq!(restored.member.agent_member_id, old_id);
        assert!(restored.prior_assignment.is_none());
        measured_machine_budget(0);
        assert_eq!(
            reserve_resume_pm_member(&root, &sessions, &source, "restore-pm-attempt", false)
                .unwrap()
                .member,
            restored.member,
            "same-slot PM Resume remains replayable after capacity changes",
        );
        assert_eq!(restored.profile.agent_id, "claude");
        assert_eq!(restored.profile.model, source.model);
        assert_eq!(
            restored
                .member
                .assignment
                .as_ref()
                .unwrap()
                .session_id
                .as_deref(),
            Some(source.id.as_str())
        );
        let mut config = crate::session_launch::launch_config_from_persisted_session(&source);
        config
            .env_vars
            .insert("GWT_AGENT_MEMBER_ID".into(), old_id.clone());
        config
            .env_vars
            .insert("GWT_AGENT_NAME".into(), restored.member.name.clone());
        let mut candidate =
            crate::session_launch::initialize_launch_session(&source.worktree_path, &root, &config);
        assert_ne!(candidate.id, source.id);
        assert_eq!(
            candidate.restore_source_session_id.as_deref(),
            Some(source.id.as_str())
        );
        candidate.save(&sessions).unwrap();
        crate::agent_names::bind_project_member_session(
            &root,
            &old_id,
            "restore-pm-attempt",
            &candidate.id,
        )
        .unwrap();
        candidate.update_status(gwt_agent::AgentStatus::Stopped);
        candidate.save(&sessions).unwrap();
        assert!(cancel_unspawned_pm_member(
            &root,
            &sessions,
            &old_id,
            "restore-pm-attempt",
            Some(&candidate),
            Some((&source, None))
        )
        .unwrap());
        let after = load().unwrap().unwrap();
        assert!(project_assignment(
            &after,
            gwt_core::paths::project_scope_hash(&root).as_str(),
            &old_id
        )
        .is_none());
        assert_eq!(
            selected_pm_member(&after, gwt_core::paths::project_scope_hash(&root).as_str())
                .unwrap()
                .agent_member_id,
            successor
        );
        assert_eq!(
            fs::read(sessions.join(format!("{}.toml", source.id))).unwrap(),
            old_bytes
        );
        assert_eq!(fs::read(&pm_path).unwrap(), profile_bytes);
        let before = fs::read(path()).unwrap();
        let runtime = gwt_agent::runtime_state_path(&sessions, &source.id);
        fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        fs::write(runtime, "unknown runtime").unwrap();
        assert!(
            reserve_resume_pm_member(&root, &sessions, &source, "unknown-restore", false).is_err()
        );
        assert_eq!(fs::read(path()).unwrap(), before);

        // An enrolled PM launch must not migrate unrelated projects again.
        fs::remove_file(gwt_agent::runtime_state_path(&sessions, &source.id)).unwrap();
        ensure_active_for_project(&root).unwrap();
        let unrelated = gwt_core::paths::gwt_home().join("unrelated-migration-project");
        fs::create_dir_all(&unrelated).unwrap();
        let unrelated_prefs = crate::issue_monitor_prefs_path_for_repo_path(&unrelated);
        fs::create_dir_all(unrelated_prefs.parent().unwrap()).unwrap();
        fs::write(&unrelated_prefs, b"{").unwrap();
        assert!(ensure_current_pm_identity(&root, &sessions).unwrap());
        let registry_bytes = fs::read(path()).unwrap();
        let refusal = reserve_new_pm_member(
            &root,
            &sessions,
            &source.worktree_path,
            "registered-pm-refusal",
        )
        .err()
        .expect("an existing PM must use its saved conversation");
        assert!(refusal
            .to_string()
            .contains("Resume the registered PM conversation before starting a new PM"));
        assert_eq!(fs::read(path()).unwrap(), registry_bytes);
        assert_eq!(fs::read(&unrelated_prefs).unwrap(), b"{");
        assert!(!unrelated_prefs.with_extension("committed.json").exists());
    }

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

    #[test]
    fn legacy_single_assignment_migrates_without_changing_its_project_or_aliases() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, prefs_path, _) = source();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let member = crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "legacy-slot",
            Some(42),
            None,
            None,
            None,
        )
        .unwrap();
        let key = gwt_core::paths::project_scope_hash(&root);
        let before = load().unwrap().unwrap();
        let mut legacy = serde_json::to_value(&before).unwrap();
        legacy["schema_version"] = serde_json::json!(2);
        legacy
            .as_object_mut()
            .unwrap()
            .remove("project_assignments");
        let row = legacy["roster"]["members"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["agent_member_id"] == member.agent_member_id)
            .unwrap();
        row["assignment"] = serde_json::to_value(&member.assignment).unwrap();
        let bytes = serde_json::to_vec(&legacy).unwrap();
        fs::write(path(), &bytes).unwrap();
        let snapshot = SessionMemberSnapshot::load().unwrap();
        assert_eq!(
            snapshot
                .assignments_for_member(&member.agent_member_id)
                .unwrap(),
            vec![member.assignment.clone().unwrap()]
        );
        assert_eq!(
            snapshot
                .project_snapshot(&key)
                .unwrap()
                .agent_roster
                .unwrap()
                .members[0]
                .assignment,
            member.assignment
        );
        assert_eq!(
            fs::read(path()).unwrap(),
            bytes,
            "read-only migration does not rewrite durable state"
        );
        crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
            prefs.priority_order = vec![42];
            Ok(())
        })
        .unwrap();
        let saved = load().unwrap().unwrap();
        assert_eq!(saved.schema_version, 3);
        assert!(saved
            .roster
            .as_ref()
            .unwrap()
            .members
            .iter()
            .all(|member| member.assignment.is_none() && member.pm_project_key.is_none()));
        assert_eq!(saved.project_assignments, before.project_assignments);
        assert_eq!(
            saved.sources[key.as_str()].aliases,
            before.sources[key.as_str()].aliases
        );
    }

    #[test]
    fn member_tuning_preserves_candidates_other_cores_and_immutable_refusals() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, mirror, _) = source();
        ensure_active_for_project(&root).unwrap();
        let mut registry = load().unwrap().unwrap();
        let target = registry.roster.as_ref().unwrap().members[0]
            .agent_member_id
            .clone();
        let mut other = registry.roster.as_ref().unwrap().members[0].clone();
        other.agent_member_id = uuid::Uuid::new_v4().to_string();
        other.name = "Another member".into();
        other.assignment = Some(serde_json::from_value(serde_json::json!({
            "reservation_id":"retained-dead","session_id":"dead-session","issue_number":null,"spec_number":null,
            "delivery_id":null,"project_key":gwt_core::paths::project_scope_hash(&root).to_string(),"host_id":"unreachable-host","host_pid":0,"host_started_at":null
        })).unwrap());
        let other_assignment = other.assignment.take();
        registry
            .roster
            .as_mut()
            .unwrap()
            .members
            .push(other.clone());
        set_project_assignment(
            &mut registry,
            gwt_core::paths::project_scope_hash(&root).as_str(),
            &other.agent_member_id,
            other_assignment,
        );
        let core = registry.profiles.get_mut(&target).unwrap();
        core.version = Some("kept-version".into());
        core.fast_mode = true;
        let request = crate::agent_names::MemberTuning {
            member_id: target.clone(),
            model: Some("  private-model  ".into()),
            reasoning: Some("high".into()),
            expected_core_revision: implementation_core_revision(&target, core),
            agent_id: None,
            runtime_target: None,
            skip_permissions: None,
        };
        registry.profiles.insert(
            other.agent_member_id.clone(),
            AgentProfileCore {
                agent_id: "codex".into(),
                model: Some("newer-other-core".into()),
                reasoning: None,
                version: None,
                fast_mode: false,
                codex_speed: gwt_agent::CodexSpeed::Off,
                runtime_target: None,
                skip_permissions: None,
            },
        );
        save(&registry).unwrap();
        crate::try_mutate_issue_monitor_prefs(&mirror, |prefs| {
            prefs.set_launch_profile_pool(vec![serde_json::from_value(serde_json::json!({
                "agent_id":"codex", "agent_member_id":other.agent_member_id,
                "model":"newer-other-core"
            }))
            .unwrap()]);
            Ok(())
        })
        .unwrap();
        let before = load().unwrap().unwrap();
        let old_core_json = serde_json::to_value(before.profiles.get(&target).unwrap()).unwrap();
        assert!(old_core_json.get("runtime_target").is_none());
        assert!(old_core_json.get("skip_permissions").is_none());
        let mirror_bytes = fs::read(&mirror).unwrap();
        let floor_bytes = fs::read(mirror.with_extension("committed.json")).unwrap();
        let outcome = set_member_tuning(&request);
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        let snapshot = outcome.snapshot.unwrap();
        let prefs = snapshot
            .project_snapshot(&gwt_core::paths::project_scope_hash(&root))
            .unwrap();
        assert_eq!(prefs.launch_profile_pool().len(), 1);
        assert_eq!(prefs.launch_profile_pool()[0].agent_member_id.as_deref(), Some(other.agent_member_id.as_str()), "a concurrently added candidate is retained and the unselected target is never inserted");
        let after = load().unwrap().unwrap();
        let mut expected = before.clone();
        expected.revision += 1;
        expected.roster.as_mut().unwrap().revision += 1;
        expected.profiles.get_mut(&target).unwrap().model = Some("private-model".into());
        expected.profiles.get_mut(&target).unwrap().reasoning = Some("high".into());
        assert_eq!(after, expected);
        assert_eq!(fs::read(&mirror).unwrap(), mirror_bytes);
        assert_eq!(
            fs::read(mirror.with_extension("committed.json")).unwrap(),
            floor_bytes
        );
        let extended: crate::agent_names::MemberTuning = serde_json::from_value(serde_json::json!({
            "member_id": target, "model": null, "reasoning": null, "agent_id": "claude",
            "runtime_target": "Docker", "skip_permissions": false,
            "expected_core_revision": implementation_core_revision(&target, after.profiles.get(&target).unwrap())
        })).expect("new execution settings use the same tuning request");
        assert!(set_member_tuning(&extended).error.is_none());
        let edited = load().unwrap().unwrap();
        let core = serde_json::to_value(edited.profiles.get(&target).unwrap()).unwrap();
        assert_eq!(core["agent_id"], "claude");
        assert_eq!(core["runtime_target"], "Docker");
        assert_eq!(core["skip_permissions"], false);
        assert_eq!(core["version"], serde_json::Value::Null);
        assert_eq!(core["fast_mode"], false);
        assert_eq!(edited.project_assignments, after.project_assignments);
        assert_eq!(
            edited.profiles.get(&other.agent_member_id),
            after.profiles.get(&other.agent_member_id)
        );
        assert_eq!(fs::read(&mirror).unwrap(), mirror_bytes);
        let edited_snapshot = SessionMemberSnapshot::load().unwrap();
        let projected = edited_snapshot
            .project_snapshot(&gwt_core::paths::project_scope_hash(&root))
            .unwrap();
        assert_eq!(projected.launch_profile_pool().len(), 1);
        let resolved = member_profile(&projected, &target).unwrap();
        assert_eq!(resolved.agent_id, "claude");
        assert_eq!(
            resolved.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert!(!resolved.skip_permissions);
        let retained: crate::agent_names::MemberTuning =
            serde_json::from_value(serde_json::json!({
                "member_id": target, "model": "changed-model", "reasoning": null,
                "expected_core_revision": edited_snapshot.implementation_core(&target).unwrap().1
            }))
            .unwrap();
        assert!(set_member_tuning(&retained).error.is_none());
        let retained_snapshot = SessionMemberSnapshot::load().unwrap();
        let retained_core = retained_snapshot.implementation_core(&target).unwrap().0;
        assert_eq!(
            retained_core.runtime_target,
            Some(gwt_agent::LaunchRuntimeTarget::Docker)
        );
        assert_eq!(retained_core.skip_permissions, Some(false));
        let carried = AgentProfileCore::from_profile(&IssueMonitorLaunchProfile::from(
            &gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode).build(),
        ))
        .retaining_overrides(Some(&retained_core));
        assert_eq!(carried.runtime_target, retained_core.runtime_target);
        assert_eq!(carried.skip_permissions, retained_core.skip_permissions);
        let reset: crate::agent_names::MemberTuning = serde_json::from_value(serde_json::json!({
            "member_id": target, "model": null, "reasoning": null,
            "runtime_target": null, "skip_permissions": null,
            "expected_core_revision": retained_snapshot.implementation_core(&target).unwrap().1
        }))
        .unwrap();
        assert!(set_member_tuning(&reset).error.is_none());
        let reset_core = SessionMemberSnapshot::load()
            .unwrap()
            .implementation_core(&target)
            .unwrap()
            .0;
        assert!(reset_core.runtime_target.is_none() && reset_core.skip_permissions.is_none());
        assert!(serde_json::to_value(&reset_core)
            .unwrap()
            .get("runtime_target")
            .is_none());
        let invalid: crate::agent_names::MemberTuning = serde_json::from_value(serde_json::json!({
            "member_id": target, "model": null, "reasoning": null, "agent_id": "openclaw",
            "skip_permissions": true, "expected_core_revision": implementation_core_revision(&target, &reset_core)
        })).unwrap();
        let before_invalid = fs::read(path()).unwrap();
        assert!(set_member_tuning(&invalid)
            .error
            .unwrap()
            .contains("permissions"));
        assert_eq!(fs::read(path()).unwrap(), before_invalid);
        // Keep the original invalid/stale regression based on the pre-extension state.
        save(&after).unwrap();
        let bytes = fs::read(path()).unwrap();
        let stale = set_member_tuning(&request);
        assert!(stale.error.unwrap().contains("changed elsewhere"));
        assert!(stale
            .snapshot
            .unwrap()
            .project_snapshot(&gwt_core::paths::project_scope_hash(&root))
            .unwrap()
            .agent_roster
            .unwrap()
            .members
            .iter()
            .any(|member| member.assignment == other.assignment));
        assert_eq!(fs::read(path()).unwrap(), bytes);
        let mut request = request;
        request.expected_core_revision = snapshot.implementation_core(&target).unwrap().1;
        assert!(set_member_tuning(&request).error.is_none());
        assert_eq!(
            fs::read(path()).unwrap(),
            bytes,
            "no-op has no revision or byte changes"
        );
        let late_root = home.path().join("late-project");
        fs::create_dir_all(&late_root).unwrap();
        let late_path = crate::issue_monitor_prefs_path_for_repo_path(&late_root);
        fs::create_dir_all(late_path.parent().unwrap()).unwrap();
        fs::write(
            &late_path,
            serde_json::to_vec(&IssueMonitorPrefs::default()).unwrap(),
        )
        .unwrap();
        assert!(set_member_tuning(&request)
            .error
            .unwrap()
            .contains("enrollment"));
        assert_eq!(fs::read(path()).unwrap(), bytes);
        assert!(!late_path.with_extension("committed.json").exists());
        ensure_active_for_project(&late_root).unwrap();
        request.expected_core_revision = SessionMemberSnapshot::load()
            .unwrap()
            .implementation_core(&target)
            .unwrap()
            .1;
        assert!(
            set_member_tuning(&request).error.is_none(),
            "normal prerequisite recovery admits the same legitimate target"
        );
        assert!(late_path.with_extension("committed.json").exists());
        let bytes = fs::read(path()).unwrap();
        request.reasoning = Some("ultracode".into());
        assert!(set_member_tuning(&request)
            .error
            .unwrap()
            .contains("unsupported"));
        assert_eq!(fs::read(path()).unwrap(), bytes);
        fs::write(mirror.with_extension("committed.json"), "unconfirmed floor").unwrap();
        assert!(set_member_tuning(&request).error.is_some());
        assert_eq!(fs::read(path()).unwrap(), bytes);
        assert_eq!(
            fs::read(mirror.with_extension("committed.json")).unwrap(),
            b"unconfirmed floor"
        );
        fs::write(mirror.with_extension("committed.json"), &floor_bytes).unwrap();
        fs::write(&mirror, b"unknown mirror").unwrap();
        assert!(set_member_tuning(&request).error.is_some());
        assert_eq!(fs::read(path()).unwrap(), bytes);
        assert_eq!(fs::read(&mirror).unwrap(), b"unknown mirror");
        let mut prepared = load().unwrap().unwrap();
        prepared.phase = RegistryPhase::Prepared;
        save(&prepared).unwrap();
        let prepared_bytes = fs::read(path()).unwrap();
        assert!(set_member_tuning(&request)
            .error
            .unwrap()
            .contains("recovery"));
        assert_eq!(fs::read(path()).unwrap(), prepared_bytes);
    }

    #[test]
    fn selected_pm_profile_cas_preserves_version_core_and_original_session() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, _) = pm_only_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        ensure_current_pm_identity(&root, &sessions).unwrap();
        let key = gwt_core::paths::project_scope_hash(&root);
        let member = SessionMemberSnapshot::load()
            .unwrap()
            .configured_pm_member(&key)
            .unwrap()
            .unwrap();
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.settings.launch_profile = Some(crate::pm_registry::PmLaunchProfile {
                agent_id: "claude".into(),
                model: Some("opusplan".into()),
                reasoning: None,
                version: Some("keep-version".into()),
                codex_speed: None,
                fast_mode: None,
            })
        })
        .unwrap();
        let before = crate::pm_registry::load_pm_prefs(&prefs_path).unwrap();
        let token = crate::pm_registry::pm_profile_revision(&before);
        let registry_bytes = fs::read(path()).unwrap();
        let session_bytes = fs::read(sessions.join(format!("{}.toml", session.id))).unwrap();
        let saved = crate::pm_registry::set_selected_pm_profile(
            &root,
            &member.agent_member_id,
            &token,
            crate::pm_registry::PmLaunchProfile {
                agent_id: "claude".into(),
                model: Some(" opusplan ".into()),
                reasoning: Some(" high ".into()),
                version: None,
                codex_speed: None,
                fast_mode: None,
            },
        )
        .unwrap();
        assert_eq!(
            saved
                .settings
                .launch_profile
                .as_ref()
                .unwrap()
                .version
                .as_deref(),
            Some("keep-version")
        );
        assert_eq!(
            saved
                .settings
                .launch_profile
                .as_ref()
                .unwrap()
                .reasoning
                .as_deref(),
            Some("high")
        );
        assert_eq!(saved.registration, before.registration);
        assert_eq!(fs::read(path()).unwrap(), registry_bytes);
        assert_eq!(
            fs::read(sessions.join(format!("{}.toml", session.id))).unwrap(),
            session_bytes
        );
        assert!(!crate::issue_monitor_prefs_path_for_repo_path(&root).exists());
        let core_snapshot = SessionMemberSnapshot::load().unwrap();
        let tuning = crate::agent_names::MemberTuning {
            member_id: member.agent_member_id.clone(),
            model: Some("future-implementation-model".into()),
            reasoning: None,
            expected_core_revision: core_snapshot
                .implementation_core(&member.agent_member_id)
                .unwrap()
                .1,
            agent_id: None,
            runtime_target: None,
            skip_permissions: None,
        };
        assert!(
            set_member_tuning(&tuning).error.is_none(),
            "PM-only registries need no Monitor source to tune a legitimate member"
        );
        assert!(!crate::issue_monitor_prefs_path_for_repo_path(&root).exists());
        let bytes = fs::read(&prefs_path).unwrap();
        assert!(crate::pm_registry::set_selected_pm_profile(
            &root,
            &member.agent_member_id,
            &token,
            crate::pm_registry::PmLaunchProfile::default_profile()
        )
        .is_err());
        assert!(crate::pm_registry::selected_pm_profile_snapshot(&root, "another-member").is_err());
        assert_eq!(fs::read(&prefs_path).unwrap(), bytes);
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
        use crate::project_runtime::installation::IssueMonitorPrefsStamp;

        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, a_path, _) = source();
        let absent_root = home.path().join("project-without-mirror");
        fs::create_dir_all(&absent_root).unwrap();
        let absent_path = crate::issue_monitor_prefs_path_for_repo_path(&absent_root);
        let before_active = IssueMonitorPrefsStamp::read(&absent_path).unwrap();
        assert!(before_active.is_none());
        assert!(IssueMonitorPrefsStamp::matches_current(
            before_active.as_ref(),
            &absent_path
        ));
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
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            let id = prefs.agent_roster.as_ref().unwrap().members[0]
                .agent_member_id
                .clone();
            prefs.set_launch_profile_pool(vec![serde_json::from_value(serde_json::json!({
                "agent_id": "codex", "agent_member_id": id, "model": "observed-core"
            }))
            .unwrap()]);
            Ok(())
        })
        .unwrap();
        let before = crate::load_issue_monitor_prefs(&a_path).unwrap();
        let stamp = IssueMonitorPrefsStamp::read(&a_path).unwrap();
        let mirror = fs::read(&a_path).unwrap();
        let absent_stamp = IssueMonitorPrefsStamp::read(&absent_path).unwrap();
        assert!(
            absent_stamp.is_some(),
            "Active shared settings must be retained without a project mirror"
        );
        assert!(IssueMonitorPrefsStamp::matches_current(
            absent_stamp.as_ref(),
            &absent_path
        ));
        assert!(
            !IssueMonitorPrefsStamp::matches_current(before_active.as_ref(), &absent_path),
            "registry activation must stale an absent legacy snapshot"
        );
        let durable_generation = status(None).unwrap().registry_revision;
        crate::try_mutate_issue_monitor_prefs(&b_path, |prefs| {
            prefs.merged_issues.push(77);
            prefs.pending_launch_deliveries.push(serde_json::from_value(serde_json::json!({
                "delivery_id": "launch:runtime-reservation", "issue_number": 78,
                "branch_name": "work/issue-78", "linked_issue_kind": crate::LinkedIssueKind::Issue,
                "claim_id": "claim-runtime", "materializer_id": "runtime-materializer",
                "materializer_pid": std::process::id(), "created_at": "2026-10-31T00:00:00Z"
            })).unwrap());
            prefs.last_scan_at = Some("2026-10-31T00:00:00Z".into());
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
        assert!(
            IssueMonitorPrefsStamp::matches_current(stamp.as_ref(), &a_path),
            "another project's runtime reservation must not stale launch settings"
        );
        assert!(
            IssueMonitorPrefsStamp::matches_current(absent_stamp.as_ref(), &absent_path),
            "runtime commits must preserve the shared-settings absence snapshot"
        );
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            let mut profiles = prefs.launch_profile_pool();
            profiles[0].model = Some("changed-core".into());
            prefs.set_launch_profile_pool(profiles);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .launch_profile_pool()[0]
                .model
                .as_deref(),
            Some("changed-core")
        );
        assert_eq!(
            fs::read(&a_path).unwrap(),
            mirror,
            "shared core edits leave the execution mirror unchanged"
        );
        assert!(
            !IssueMonitorPrefsStamp::matches_current(stamp.as_ref(), &a_path),
            "a shared saved-core change must stale prepared launch settings"
        );
        assert!(
            !IssueMonitorPrefsStamp::matches_current(absent_stamp.as_ref(), &absent_path),
            "a core edit must stale a missing-mirror snapshot"
        );
        let absent_stamp = IssueMonitorPrefsStamp::read(&absent_path).unwrap();
        let stamp = IssueMonitorPrefsStamp::read(&a_path).unwrap();
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
        assert_eq!(fs::read(&a_path).unwrap(), mirror);
        assert!(
            !IssueMonitorPrefsStamp::matches_current(stamp.as_ref(), &a_path),
            "a shared roster edit must stale prepared launch settings"
        );
        assert!(
            !IssueMonitorPrefsStamp::matches_current(absent_stamp.as_ref(), &absent_path),
            "a roster edit must stale a missing-mirror snapshot"
        );
        let absent_stamp = IssueMonitorPrefsStamp::read(&absent_path).unwrap();
        let settings_before_mirror = crate::load_issue_monitor_prefs(&a_path)
            .unwrap()
            .agent_registry_revision;
        assert!(!absent_path.exists());
        ensure_active_for_project(&absent_root).unwrap();
        assert!(absent_path.exists());
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .agent_registry_revision,
            settings_before_mirror,
            "enrolling the empty mirror must not change shared settings"
        );
        assert!(
            !IssueMonitorPrefsStamp::matches_current(absent_stamp.as_ref(), &absent_path),
            "a new project mirror must stale the captured absence"
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
        old["enabled"] = serde_json::json!(true);
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
        let before = crate::load_issue_monitor_prefs(&source_path).unwrap();
        let disabled = crate::issue_monitor::disable_issue_monitor_during_recovery(
            &source_path,
            Some(&before.config_token()),
            || true,
        )
        .expect("OFF recovery must commit through the active registry");
        assert!(!disabled.enabled);
        assert_eq!(
            crate::load_issue_monitor_prefs(&source_path).unwrap(),
            disabled
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
            profiles[0].codex_speed = gwt_agent::CodexSpeed::Ultrafast;
            prefs.set_launch_profile_pool(profiles);
            Ok(())
        })
        .unwrap();
        let latest = crate::load_issue_monitor_prefs(&b_path).unwrap();
        assert_eq!(latest.launch_profile_pool(), observed.launch_profile_pool());
        let core = SessionMemberSnapshot::load()
            .unwrap()
            .implementation_core(id)
            .unwrap()
            .0;
        assert_eq!(
            serde_json::to_value(core).unwrap()["codex_speed"],
            "ultrafast",
            "Codex speed belongs to the common saved core"
        );
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

    fn rewrite_pm_assignment(
        root: &Path,
        member_id: &str,
        rewrite: impl FnOnce(&mut crate::agent_names::AgentMemberAssignment),
    ) {
        let key = gwt_core::paths::project_scope_hash(root);
        let mut registry = load().unwrap().unwrap();
        let mut assignment = project_assignment(&registry, key.as_str(), member_id)
            .cloned()
            .expect("PM assignment");
        rewrite(&mut assignment);
        set_project_assignment(&mut registry, key.as_str(), member_id, Some(assignment));
        save(&registry).unwrap();
    }

    /// Issue #7472: a PM reservation left behind by a gwtd incarnation that
    /// no longer exists must not wedge every later PM launch. The reserving
    /// host can never cancel or reap it (every cleanup path requires the
    /// exact reserving host), so a fresh NEW launch reconciles it.
    #[test]
    fn reserve_new_pm_member_reconciles_reservation_from_a_dead_host_incarnation() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, _) = pm_only_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let worktree = session.worktree_path.clone();
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| prefs.registration = None)
            .unwrap();

        let first = reserve_new_pm_member(&root, &sessions, &worktree, "wedged-attempt").unwrap();
        let member_id = first.member.agent_member_id.clone();

        // Rewrite the durable assignment to the dead incarnation that
        // reserved it: a pid that cannot exist on any supported host.
        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.host_pid = 4_000_000_000;
            assignment.host_started_at = Some(1);
        });

        let prepared =
            reserve_new_pm_member(&root, &sessions, &worktree, "recovery-attempt").unwrap();
        assert_eq!(prepared.member.agent_member_id, member_id);
        let assignment = prepared.member.assignment.expect("new PM assignment");
        assert_eq!(assignment.reservation_id, "recovery-attempt");
        assert_eq!(assignment.host_pid, std::process::id());
        assert_eq!(assignment.session_id, None);

        // The cancelled-unspawned cleanup path still owns this process's own
        // reservation exactly as before.
        assert!(cancel_unspawned_pm_member(
            &root,
            &sessions,
            &member_id,
            "recovery-attempt",
            None,
            None
        )
        .unwrap());
        assert!(reserve_new_pm_member(&root, &sessions, &worktree, "after-cancel").is_ok());
    }

    /// Issue #7472: the reconciliation above must never weaken the guard.
    /// A live reservation, a Session-bound assignment, and another host's
    /// reservation all keep blocking a new PM launch.
    #[test]
    fn reserve_new_pm_member_retains_live_or_unprovable_reservations() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, _) = pm_only_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let worktree = session.worktree_path.clone();
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| prefs.registration = None)
            .unwrap();

        let first = reserve_new_pm_member(&root, &sessions, &worktree, "live-attempt").unwrap();
        let member_id = first.member.agent_member_id.clone();

        // This process's own live reservation blocks a second NEW launch.
        let error = reserve_new_pm_member(&root, &sessions, &worktree, "second-attempt")
            .err()
            .expect("a live PM reservation must block a new launch");
        assert!(
            error
                .to_string()
                .contains("retains a live or unknown reservation"),
            "{error}"
        );

        // A bound Session is never auto-reconciled, even from a dead host.
        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.host_pid = 4_000_000_000;
            assignment.host_started_at = Some(1);
            assignment.session_id = Some("bound-session".into());
        });
        let error = reserve_new_pm_member(&root, &sessions, &worktree, "third-attempt")
            .err()
            .expect("a Session-bound assignment must block a new launch");
        assert!(
            error
                .to_string()
                .contains("retains a live or unknown reservation"),
            "{error}"
        );

        // Another host's reservation cannot be proven dead from this host.
        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.host_id = "another-host".into();
            assignment.session_id = None;
        });
        let error = reserve_new_pm_member(&root, &sessions, &worktree, "fourth-attempt")
            .err()
            .expect("another host's reservation must block a new launch");
        assert!(
            error
                .to_string()
                .contains("retains a live or unknown reservation"),
            "{error}"
        );
    }

    /// Issue #7475 fixture: import the pm-only fixture's stopped PM Session
    /// through the registry migration path so the member holds exactly the
    /// legacy reservation the migration creates (bound Session, no reserving
    /// host), then drop the prefs registration — the state a later launch
    /// observes once the PM pane has been closed.
    fn legacy_pm_fixture() -> (PathBuf, gwt_agent::Session, String) {
        let (root, mut session, _) = pm_only_fixture();
        session.update_status(gwt_agent::AgentStatus::Stopped);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        session.save(&sessions).unwrap();
        crate::agent_names::read_project_roster(&root, &sessions).unwrap();
        let key = gwt_core::paths::project_scope_hash(&root);
        let registry = load().unwrap().unwrap();
        let member = selected_pm_member(&registry, key.as_str()).expect("imported PM member");
        let assignment = member.assignment.clone().expect("legacy assignment");
        assert!(
            assignment.reservation_id.starts_with("pm-legacy:"),
            "{assignment:?}"
        );
        assert_eq!(assignment.session_id.as_deref(), Some(session.id.as_str()));
        assert_eq!(assignment.host_pid, 0);
        let member_id = member.agent_member_id.clone();
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| prefs.registration = None)
            .unwrap();
        (root, session, member_id)
    }

    /// Issue #7475: a legacy reservation imported by the registry migration
    /// records no reserving host, so no cancel, reap, or adoption path can
    /// ever release it. When its bound Session is provably finished, a fresh
    /// NEW launch reconciles it instead of wedging every later PM launch.
    #[test]
    fn reserve_new_pm_member_reconciles_dead_legacy_reservation() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, member_id) = legacy_pm_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let worktree = session.worktree_path.clone();

        let prepared =
            reserve_new_pm_member(&root, &sessions, &worktree, "recovery-attempt").unwrap();
        assert_eq!(prepared.member.agent_member_id, member_id);
        let assignment = prepared.member.assignment.expect("new PM assignment");
        assert_eq!(assignment.reservation_id, "recovery-attempt");
        assert_eq!(assignment.session_id, None);
        assert_eq!(assignment.host_pid, std::process::id());
    }

    /// Issue #7475: the legacy reconciliation must never displace a Session
    /// that is not provably finished. A legacy reservation bound to a
    /// running Session keeps blocking a new launch.
    #[test]
    fn reserve_new_pm_member_retains_legacy_reservation_for_a_running_session() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, _) = legacy_pm_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let worktree = session.worktree_path.clone();

        let mut running =
            gwt_agent::Session::load(&sessions.join(format!("{}.toml", session.id))).unwrap();
        running.update_status(gwt_agent::AgentStatus::Running);
        running.save(&sessions).unwrap();

        let error = reserve_new_pm_member(&root, &sessions, &worktree, "blocked-attempt")
            .err()
            .expect("a legacy reservation bound to a running Session must block a new launch");
        assert!(
            error
                .to_string()
                .contains("retains a live or unknown reservation"),
            "{error}"
        );
    }

    /// Issue #7475: only the exact legacy reservation the migration created
    /// qualifies. The same stopped Session bound under a modern reservation
    /// token keeps blocking, preserving resume protection for non-legacy
    /// Session-bound assignments.
    #[test]
    fn reserve_new_pm_member_retains_nonlegacy_bound_session_assignment() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, member_id) = legacy_pm_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let worktree = session.worktree_path.clone();

        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.reservation_id = "modern-reservation".into();
        });
        let error = reserve_new_pm_member(&root, &sessions, &worktree, "blocked-attempt")
            .err()
            .expect("a non-legacy Session-bound assignment must block a new launch");
        assert!(
            error
                .to_string()
                .contains("retains a live or unknown reservation"),
            "{error}"
        );
    }

    /// Issue #7542 fixture: reserve a fresh PM assignment, then rewrite it
    /// into the wedged modern shape — bound to the fixture's Session (saved
    /// with `session_status`) and reserved by a host incarnation that no
    /// longer exists (a pid that cannot exist on any supported host). The
    /// prefs registration is cleared, matching the state after the PM pane
    /// of the dead generation was closed.
    fn wedged_modern_pm_fixture(
        session_status: gwt_agent::AgentStatus,
    ) -> (PathBuf, gwt_agent::Session, String, PathBuf) {
        let (root, mut session, _) = pm_only_fixture();
        session.update_status(session_status);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        session.save(&sessions).unwrap();
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| prefs.registration = None)
            .unwrap();
        let worktree = session.worktree_path.clone();
        let first = reserve_new_pm_member(&root, &sessions, &worktree, "wedged-attempt").unwrap();
        let member_id = first.member.agent_member_id.clone();
        let session_id = session.id.clone();
        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.host_pid = 4_000_000_000;
            assignment.host_started_at = Some(1);
            assignment.session_id = Some(session_id);
        });
        (root, session, member_id, worktree)
    }

    fn quarantined_assignment(root: &Path, member_id: &str) -> Option<PmQuarantinedAssignment> {
        let key = gwt_core::paths::project_scope_hash(root);
        let registry = load().unwrap().unwrap();
        registry
            .pm_projects
            .get(key.as_str())
            .and_then(|project| project.quarantined_assignments.get(member_id))
            .cloned()
    }

    /// Issue #7542 (R1): a modern Session-bound reservation whose bound
    /// Session is provably finished and whose reserving incarnation is gone
    /// is reconciled by a fresh NEW launch — the class that wedged PM
    /// startup after the 2026-11-17 daemon generation change. The Session
    /// file is never rewritten and nothing is quarantined: the assignment
    /// was provably dead, not merely unknown.
    #[test]
    fn reserve_new_pm_member_reconciles_modern_bound_assignment_for_a_stopped_session() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, member_id, worktree) =
            wedged_modern_pm_fixture(gwt_agent::AgentStatus::Stopped);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let session_path = sessions.join(format!("{}.toml", session.id));
        let session_bytes = fs::read(&session_path).unwrap();

        let prepared =
            reserve_new_pm_member(&root, &sessions, &worktree, "recovery-attempt").unwrap();
        assert_eq!(prepared.member.agent_member_id, member_id);
        let assignment = prepared.member.assignment.expect("new PM assignment");
        assert_eq!(assignment.reservation_id, "recovery-attempt");
        assert_eq!(assignment.session_id, None);
        assert!(quarantined_assignment(&root, &member_id).is_none());
        assert_eq!(fs::read(&session_path).unwrap(), session_bytes);
    }

    /// Issue #7542 (R2): a Session still marked Running is reconciled when
    /// its runtime sidecar carries the exact proof — execution identity,
    /// runtime incarnation, the reserving host's start time, and a child
    /// tree that has exited — and the reserving incarnation is gone.
    #[test]
    fn reserve_new_pm_member_reconciles_running_session_with_exact_runtime_proof() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, member_id, worktree) =
            wedged_modern_pm_fixture(gwt_agent::AgentStatus::Running);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        // The fixture Session carries no execution binding, so its exact
        // identity is the absent one; the runtime sidecar proves the rest:
        // the reserving host's start time, a runtime incarnation, and a
        // child tree that has exited.
        let mut stored =
            gwt_agent::Session::load(&sessions.join(format!("{}.toml", session.id))).unwrap();
        stored.execution_binding = None;
        stored.save(&sessions).unwrap();
        let mut runtime = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running);
        runtime.host_started_at = Some(1);
        runtime.runtime_incarnation = Some(1);
        runtime.child_pid = Some(4_000_000_001);
        runtime.child_started_at = Some(1);
        runtime
            .save(&gwt_agent::runtime_state_path_for_pid(
                &sessions,
                4_000_000_000,
                &session.id,
            ))
            .unwrap();

        let prepared =
            reserve_new_pm_member(&root, &sessions, &worktree, "recovery-attempt").unwrap();
        let assignment = prepared.member.assignment.expect("new PM assignment");
        assert_eq!(assignment.reservation_id, "recovery-attempt");
        assert_eq!(assignment.session_id, None);
        assert!(quarantined_assignment(&root, &member_id).is_none());
    }

    /// Issue #7542 (R3): when the Session inventory reports uncertainty and
    /// the reserving incarnation is this live process, nothing is
    /// reconciled or quarantined — the launch keeps blocking.
    #[test]
    fn reserve_new_pm_member_retains_assignment_when_inventory_is_uncertain_and_host_is_live() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, member_id, worktree) =
            wedged_modern_pm_fixture(gwt_agent::AgentStatus::Stopped);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        // An unreadable runtime sidecar makes the inventory uncertain.
        let runtime_path =
            gwt_agent::runtime_state_path_for_pid(&sessions, std::process::id(), &session.id);
        fs::create_dir_all(runtime_path.parent().unwrap()).unwrap();
        fs::write(&runtime_path, b"not a runtime state").unwrap();
        let started = crate::process::host_process_start_time(std::process::id()).unwrap();
        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.host_pid = std::process::id();
            assignment.host_started_at = Some(started);
        });
        let error = reserve_new_pm_member(&root, &sessions, &worktree, "blocked-attempt")
            .err()
            .expect("an uncertain inventory with a live incarnation must block");
        assert!(
            error
                .to_string()
                .contains("retains a live or unknown reservation"),
            "{error}"
        );
        assert!(quarantined_assignment(&root, &member_id).is_none());
    }

    /// Issue #7542 (R4): another host's Session-bound reservation can never
    /// be proven dead or quarantined from this host, even with a finished
    /// Session.
    #[test]
    fn reserve_new_pm_member_retains_bound_assignment_from_another_host() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, _session, member_id, worktree) =
            wedged_modern_pm_fixture(gwt_agent::AgentStatus::Stopped);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.host_id = "another-host".into();
        });
        let error = reserve_new_pm_member(&root, &sessions, &worktree, "blocked-attempt")
            .err()
            .expect("another host's reservation must block a new launch");
        assert!(
            error
                .to_string()
                .contains("retains a live or unknown reservation"),
            "{error}"
        );
        assert!(quarantined_assignment(&root, &member_id).is_none());
    }

    /// Issue #7542 (R5 + R6): a bound Session whose file is missing or
    /// corrupt is unprovable in both directions — it is neither reconciled
    /// nor quarantined, so the launch keeps blocking (fail-closed).
    #[test]
    fn reserve_new_pm_member_retains_bound_assignment_when_session_file_is_unprovable() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, member_id, worktree) =
            wedged_modern_pm_fixture(gwt_agent::AgentStatus::Stopped);
        let sessions = gwt_core::paths::gwt_sessions_dir();

        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.session_id = Some("missing-session".into());
        });
        let error = reserve_new_pm_member(&root, &sessions, &worktree, "blocked-attempt")
            .err()
            .expect("a missing Session file must block a new launch");
        assert!(
            error
                .to_string()
                .contains("retains a live or unknown reservation"),
            "{error}"
        );
        assert!(quarantined_assignment(&root, &member_id).is_none());

        let wedged_session = session.id.clone();
        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.session_id = Some(wedged_session);
        });
        fs::write(
            sessions.join(format!("{}.toml", session.id)),
            "not a session",
        )
        .unwrap();
        let error = reserve_new_pm_member(&root, &sessions, &worktree, "blocked-attempt")
            .err()
            .expect("a corrupt Session file must block a new launch");
        assert!(
            error
                .to_string()
                .contains("retains a live or unknown reservation"),
            "{error}"
        );
        assert!(quarantined_assignment(&root, &member_id).is_none());
    }

    /// Issue #7542 (R7): when the recorded pid now belongs to a different
    /// live process (its start time differs), the recorded incarnation is
    /// provably gone and the finished Session's assignment is reconciled.
    #[test]
    fn reserve_new_pm_member_reconciles_when_the_recorded_pid_was_reused() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, _session, member_id, worktree) =
            wedged_modern_pm_fixture(gwt_agent::AgentStatus::Stopped);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let mut sleeper = gwt_core::process::resolved_command(
            gwt_core::process::ProcessPlanRequest::new("sleep"),
        )
        .expect("sleep resolves")
        .arg("30")
        .spawn()
        .unwrap();
        let sleeper_started = crate::process::host_process_start_time(sleeper.id()).unwrap();
        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.host_pid = sleeper.id();
            assignment.host_started_at = Some(sleeper_started + 999);
        });
        let prepared =
            reserve_new_pm_member(&root, &sessions, &worktree, "recovery-attempt").unwrap();
        let assignment = prepared.member.assignment.expect("new PM assignment");
        assert_eq!(assignment.reservation_id, "recovery-attempt");
        assert!(quarantined_assignment(&root, &member_id).is_none());
        let _ = sleeper.kill();
        let _ = sleeper.wait();
    }

    /// Issue #7542 (R8): a Session that is not provably finished (still
    /// Running, and its unreadable runtime sidecar makes the inventory
    /// uncertain) is not deleted — it is quarantined. The NEW launch
    /// proceeds, the quarantine record preserves the displaced assignment
    /// verbatim, and the Session file stays untouched.
    #[test]
    fn reserve_new_pm_member_quarantines_unprovable_assignment_from_a_dead_incarnation() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, member_id, worktree) =
            wedged_modern_pm_fixture(gwt_agent::AgentStatus::Running);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let runtime_path =
            gwt_agent::runtime_state_path_for_pid(&sessions, 4_000_000_000, &session.id);
        fs::create_dir_all(runtime_path.parent().unwrap()).unwrap();
        fs::write(&runtime_path, b"not a runtime state").unwrap();
        let session_path = sessions.join(format!("{}.toml", session.id));
        let session_bytes = fs::read(&session_path).unwrap();

        let prepared =
            reserve_new_pm_member(&root, &sessions, &worktree, "recovery-attempt").unwrap();
        let assignment = prepared.member.assignment.expect("new PM assignment");
        assert_eq!(assignment.reservation_id, "recovery-attempt");
        assert_eq!(assignment.session_id, None);

        let record = quarantined_assignment(&root, &member_id)
            .expect("the displaced assignment is quarantined, not deleted");
        assert_eq!(record.assignment.reservation_id, "wedged-attempt");
        assert_eq!(
            record.assignment.session_id.as_deref(),
            Some(session.id.as_str())
        );
        assert!(!record.reason.is_empty());
        assert_eq!(fs::read(&session_path).unwrap(), session_bytes);
    }

    /// Issue #7542 (R9): when the prefs registration names the same wedged
    /// Session, the NEW launch clears exactly that registration as part of
    /// reconciling; a registration naming any other Session keeps fencing.
    #[test]
    fn reserve_new_pm_member_clears_registration_for_the_same_wedged_session() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, _member_id, worktree) =
            wedged_modern_pm_fixture(gwt_agent::AgentStatus::Stopped);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        let registration = crate::pm_registry::PmRegistration {
            session_id: session.id.clone(),
            agent_id: "claude".into(),
            worktree_path: worktree.to_string_lossy().into_owned(),
            created_at: None,
            consecutive_crashes: 0,
            next_not_before: None,
        };
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.registration = Some(registration.clone())
        })
        .unwrap();
        let prepared =
            reserve_new_pm_member(&root, &sessions, &worktree, "recovery-attempt").unwrap();
        assert_eq!(
            prepared
                .member
                .assignment
                .expect("new PM assignment")
                .reservation_id,
            "recovery-attempt"
        );
        assert!(
            crate::pm_registry::load_pm_prefs(&prefs_path)
                .unwrap()
                .registration
                .is_none(),
            "the registration naming the wedged Session is cleared"
        );

        // A registration naming a different Session still fences NEW: it is
        // never cleared by the recovery path, and the current assignment is
        // left exactly as the successful launch prepared it. (The guard
        // that fires is the identity import's, because that Session does
        // not exist on disk.)
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.registration = Some(crate::pm_registry::PmRegistration {
                session_id: "another-session".into(),
                ..registration.clone()
            })
        })
        .unwrap();
        let error = reserve_new_pm_member(&root, &sessions, &worktree, "blocked-attempt")
            .err()
            .expect("a registration for another Session must fence a new launch");
        assert!(
            error
                .to_string()
                .contains("Registered PM Session is unavailable"),
            "{error}"
        );
        assert_eq!(
            crate::pm_registry::load_pm_prefs(&prefs_path)
                .unwrap()
                .registration
                .map(|registration| registration.session_id),
            Some("another-session".to_string()),
            "another Session's registration is never cleared"
        );
        let key = gwt_core::paths::project_scope_hash(&root);
        let registry = load().unwrap().unwrap();
        let member = selected_pm_member(&registry, key.as_str()).expect("configured PM member");
        assert_eq!(
            member
                .assignment
                .as_ref()
                .map(|assignment| assignment.reservation_id.as_str()),
            Some("recovery-attempt"),
            "the fenced launch leaves the current assignment untouched"
        );
    }

    /// Issue #7542 (R10): a quarantined assignment can be restored verbatim
    /// while the current assignment is still this incarnation's unspawned
    /// reservation; once the current assignment is bound to a conversation,
    /// restoration is an explicit error instead of a silent overwrite.
    #[test]
    fn restore_quarantined_pm_assignment_round_trips_and_fences_live_assignments() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, member_id, worktree) =
            wedged_modern_pm_fixture(gwt_agent::AgentStatus::Running);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let runtime_path =
            gwt_agent::runtime_state_path_for_pid(&sessions, 4_000_000_000, &session.id);
        fs::create_dir_all(runtime_path.parent().unwrap()).unwrap();
        fs::write(&runtime_path, b"not a runtime state").unwrap();
        reserve_new_pm_member(&root, &sessions, &worktree, "recovery-attempt").unwrap();
        assert!(quarantined_assignment(&root, &member_id).is_some());

        let restored = restore_quarantined_pm_assignment(&root, &member_id).unwrap();
        assert_eq!(restored.reservation_id, "wedged-attempt");
        assert_eq!(restored.session_id.as_deref(), Some(session.id.as_str()));
        assert!(quarantined_assignment(&root, &member_id).is_none());
        let key = gwt_core::paths::project_scope_hash(&root);
        let registry = load().unwrap().unwrap();
        assert_eq!(
            project_assignment(&registry, key.as_str(), &member_id),
            Some(&restored)
        );
        // Nothing left to restore.
        assert!(restore_quarantined_pm_assignment(&root, &member_id).is_err());

        // Quarantine again, then bind the current assignment to a Session:
        // restoration must now refuse rather than displace it.
        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.host_pid = 4_000_000_000;
            assignment.host_started_at = Some(1);
            assignment.reservation_id = "wedged-again".into();
        });
        reserve_new_pm_member(&root, &sessions, &worktree, "second-recovery").unwrap();
        assert!(quarantined_assignment(&root, &member_id).is_some());
        rewrite_pm_assignment(&root, &member_id, |assignment| {
            assignment.session_id = Some("live-session".into());
        });
        let error = restore_quarantined_pm_assignment(&root, &member_id)
            .expect_err("restoration must not displace a bound assignment");
        assert!(
            error.to_string().contains("belongs to a live or unknown"),
            "{error}"
        );
        assert!(quarantined_assignment(&root, &member_id).is_some());
    }

    /// Issue #7542 (R11): registry files written before the quarantine
    /// record existed still decode — the field is an additive default in
    /// both the schema 3 registry and the schema 4 split core, which share
    /// this type.
    #[test]
    fn pm_project_identity_without_quarantine_field_still_decodes() {
        let legacy: PmProjectIdentity =
            serde_json::from_str(r#"{"member_id":"m1","legacy_sessions":{}}"#).unwrap();
        assert_eq!(legacy.member_id.as_deref(), Some("m1"));
        assert!(legacy.quarantined_assignments.is_empty());
    }

    /// Issue #7542 fixture: a current-format PM Session (it names its member)
    /// that is registered while its member holds no assignment — the state a
    /// new gwtd generation finds after the previous one died with the PM.
    /// Returns the root, the stopped Session, and its member id.
    fn orphaned_current_pm_fixture() -> (PathBuf, gwt_agent::Session, String) {
        let (root, mut session, prefs) = pm_only_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let worktree = session.worktree_path.clone();
        let key = gwt_core::paths::project_scope_hash(&root);
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |prefs| prefs.registration = None)
            .unwrap();
        let member_id = reserve_new_pm_member(&root, &sessions, &worktree, "first-attempt")
            .unwrap()
            .member
            .agent_member_id;
        let mut registry = load().unwrap().unwrap();
        set_project_assignment(&mut registry, key.as_str(), &member_id, None);
        save(&registry).unwrap();
        session.agent_member_id = Some(member_id.clone());
        session.update_status(gwt_agent::AgentStatus::Stopped);
        session.save(&sessions).unwrap();
        crate::pm_registry::mutate_pm_prefs(&prefs_path, |current| {
            current.registration = prefs.registration.clone()
        })
        .unwrap();
        (root, session, member_id)
    }

    /// Issue #7542: after a gwtd generation change the registered PM is
    /// re-imported with a hostless reservation that records no legacy proof,
    /// so no cancel, reap, adoption, or #7475 reconciliation can release it.
    /// Once that PM is deregistered, a NEW launch must reconcile it.
    #[test]
    fn reserve_new_pm_member_reconciles_hostless_import_of_a_finished_current_session() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, member_id) = orphaned_current_pm_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let worktree = session.worktree_path.clone();
        let key = gwt_core::paths::project_scope_hash(&root);

        assert!(ensure_current_pm_identity(&root, &sessions).unwrap());
        let registry = load().unwrap().unwrap();
        let imported = project_assignment(&registry, key.as_str(), &member_id)
            .cloned()
            .expect("imported assignment");
        assert!(imported.reservation_id.starts_with("pm-legacy:"));
        assert_eq!(imported.session_id.as_deref(), Some(session.id.as_str()));
        assert_eq!(imported.host_pid, 0);
        assert!(
            registry.pm_projects[key.as_str()]
                .legacy_sessions
                .is_empty(),
            "the import of a current-format Session records no legacy proof"
        );

        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        crate::pm_registry::deregister_pm(&prefs_path, &session.id).unwrap();

        let prepared =
            reserve_new_pm_member(&root, &sessions, &worktree, "recovery-attempt").unwrap();
        assert_eq!(prepared.member.agent_member_id, member_id);
        let assignment = prepared.member.assignment.expect("new PM assignment");
        assert_eq!(assignment.reservation_id, "recovery-attempt");
        assert_eq!(assignment.session_id, None);
        assert_eq!(assignment.host_pid, std::process::id());
    }

    /// Issue #7542: the hostless import records no reserving host, so the
    /// bound Session's finished status is the only liveness proof. A Session
    /// that is not provably finished keeps blocking.
    #[test]
    fn reserve_new_pm_member_retains_hostless_import_of_a_running_session() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (root, session, _) = orphaned_current_pm_fixture();
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let worktree = session.worktree_path.clone();

        assert!(ensure_current_pm_identity(&root, &sessions).unwrap());
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&root);
        crate::pm_registry::deregister_pm(&prefs_path, &session.id).unwrap();
        let mut running =
            gwt_agent::Session::load(&sessions.join(format!("{}.toml", session.id))).unwrap();
        running.update_status(gwt_agent::AgentStatus::Running);
        running.save(&sessions).unwrap();

        let error = reserve_new_pm_member(&root, &sessions, &worktree, "blocked-attempt")
            .err()
            .expect("a hostless import bound to a running Session must block a new launch");
        assert!(
            error
                .to_string()
                .contains("retains a live or unknown reservation"),
            "{error}"
        );
    }

    /// Two enrolled projects: `a` carries the roster, `b` is a plain mirror.
    fn two_active_sources() -> (PathBuf, String, PathBuf, String) {
        let (root, a_path, _) = source();
        let b_root = gwt_core::paths::gwt_home().join("project-b");
        fs::create_dir_all(&b_root).unwrap();
        let b_path = crate::issue_monitor_prefs_path_for_repo_path(&b_root);
        fs::create_dir_all(b_path.parent().unwrap()).unwrap();
        fs::write(
            &b_path,
            serde_json::to_vec(&IssueMonitorPrefs::default()).unwrap(),
        )
        .unwrap();
        ensure_active_for_project(&root).unwrap();
        let a_key = source_key(&a_path).unwrap();
        let b_key = source_key(&b_path).unwrap();
        assert!(load().unwrap().unwrap().sources.contains_key(&b_key));
        (a_path, a_key, b_path, b_key)
    }

    fn tamper_committed_mirror(key: &str) {
        let mut registry: serde_json::Value =
            serde_json::from_slice(&fs::read(super::path()).unwrap()).unwrap();
        registry["sources"][key]["after_json"] = serde_json::json!("{\"tampered\":true}");
        fs::write(super::path(), serde_json::to_vec(&registry).unwrap()).unwrap();
    }

    #[test]
    fn unchanged_prefs_transaction_syncs_nothing_and_checks_only_its_own_mirror() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, a_key, _, b_key) = two_active_sources();
        let registry = fs::read(super::path()).unwrap();
        take_registry_io_for_test();
        crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        let io = take_registry_io_for_test();
        assert_eq!(
            (
                io.durability_confirmations,
                io.registry_writes,
                io.mirror_publishes
            ),
            (0, 0, 0),
            "a transaction that changes nothing confirms and writes nothing: {io:?}"
        );
        assert!(io.validated_payloads.contains(&a_key), "{io:?}");
        assert!(
            !io.validated_payloads.contains(&b_key),
            "another project's mirror is not decoded for this project: {io:?}"
        );
        assert_eq!(fs::read(super::path()).unwrap(), registry);
    }

    #[test]
    fn prefs_commit_writes_the_registry_and_its_mirror_once_without_other_mirrors() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, b_path, b_key) = two_active_sources();
        let b_mirror = fs::read(&b_path).unwrap();
        take_registry_io_for_test();
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(7486);
            Ok(())
        })
        .unwrap();
        let io = take_registry_io_for_test();
        assert_eq!(
            (
                io.durability_confirmations,
                io.registry_writes,
                io.mirror_publishes
            ),
            (0, 1, 1),
            "one commit is one registry write and one mirror publish: {io:?}"
        );
        assert!(!io.validated_payloads.contains(&b_key), "{io:?}");
        assert_eq!(fs::read(&b_path).unwrap(), b_mirror);
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .merged_issues,
            [7486]
        );
        // The untouched project still passes the complete validation.
        assert!(load().unwrap().unwrap().sources.contains_key(&b_key));
    }

    #[test]
    fn interrupted_publish_replays_the_committed_generation_for_readers_and_writers() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, _, _) = two_active_sources();
        let floor_path = a_path.with_extension("committed.json");
        let (mirror_before, floor_before) =
            (fs::read(&a_path).unwrap(), fs::read(&floor_path).unwrap());
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(7486);
            Ok(())
        })
        .unwrap();
        let committed = fs::read(&a_path).unwrap();
        assert_ne!(committed, mirror_before);
        // A crash after the registry commit leaves both mirrors one generation behind.
        fs::write(&a_path, &mirror_before).unwrap();
        fs::write(&floor_path, &floor_before).unwrap();
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .merged_issues,
            [7486],
            "a reader observes the committed generation, not the stale mirror"
        );
        take_registry_io_for_test();
        let (replayed, ()) = crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        let io = take_registry_io_for_test();
        assert_eq!(replayed.merged_issues, [7486]);
        assert_eq!((io.registry_writes, io.mirror_publishes), (0, 1), "{io:?}");
        assert_eq!(fs::read(&a_path).unwrap(), committed);
        assert_eq!(fs::read(&floor_path).unwrap(), committed);
        // A half-published mirror (canonical current, floor stale) is completed too.
        fs::write(&floor_path, &floor_before).unwrap();
        crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        assert_eq!(fs::read(&floor_path).unwrap(), committed);
    }

    #[test]
    fn damaged_committed_mirror_fails_closed_for_its_own_project_only() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, b_path, b_key) = two_active_sources();
        tamper_committed_mirror(&b_key);
        let b_mirror = fs::read(&b_path).unwrap();
        assert!(crate::load_issue_monitor_prefs(&b_path).is_err());
        assert!(crate::try_mutate_issue_monitor_prefs(&b_path, |prefs| {
            prefs.merged_issues.push(1);
            Ok(())
        })
        .is_err());
        assert_eq!(fs::read(&b_path).unwrap(), b_mirror);
        assert!(load().is_err(), "the complete registry read stays closed");
        // The damage is carried byte-for-byte, never repaired, by another project's commit.
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(7486);
            Ok(())
        })
        .unwrap();
        assert!(crate::load_issue_monitor_prefs(&b_path).is_err());
        assert!(load().is_err());
        assert_eq!(fs::read(&b_path).unwrap(), b_mirror);
    }

    /// Two enrolled projects after the desktop app moved them to split storage.
    fn two_split_sources() -> (PathBuf, String, PathBuf, String) {
        let sources = two_active_sources();
        assert!(migrate_to_split_storage().unwrap());
        sources
    }

    fn storage_schema() -> u32 {
        serde_json::from_slice::<StorageSchema>(&fs::read(super::path()).unwrap())
            .unwrap()
            .schema_version
    }

    fn rename_first_member(path: &Path, name: &str) {
        crate::try_mutate_issue_monitor_prefs(path, |prefs| {
            prefs.agent_roster.as_mut().unwrap().members[0].name = name.into();
            Ok(())
        })
        .unwrap();
    }

    fn first_member_name(path: &Path) -> String {
        crate::load_issue_monitor_prefs(path)
            .unwrap()
            .agent_roster
            .unwrap()
            .members[0]
            .name
            .clone()
    }

    #[test]
    fn split_migration_keeps_the_schema3_file_and_every_committed_generation() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, a_key, b_path, b_key) = two_active_sources();
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(7486);
            Ok(())
        })
        .unwrap();
        let schema3 = fs::read(super::path()).unwrap();
        let before = (
            crate::load_issue_monitor_prefs(&a_path).unwrap(),
            crate::load_issue_monitor_prefs(&b_path).unwrap(),
            status(None).unwrap(),
        );
        let mirrors = (fs::read(&a_path).unwrap(), fs::read(&b_path).unwrap());
        let legacy_snapshots = || {
            load()
                .unwrap()
                .unwrap()
                .sources
                .into_iter()
                .map(|(key, source)| (key, source.rosterless_snapshots))
                .collect::<BTreeMap<_, _>>()
        };
        let snapshots = legacy_snapshots();
        assert!(!snapshots[&b_key].is_empty());

        assert!(migrate_to_split_storage().unwrap());
        assert_eq!(storage_schema(), SPLIT_SCHEMA_VERSION);
        assert_eq!(
            legacy_snapshots(),
            snapshots,
            "every observed legacy snapshot survives the split"
        );
        assert_eq!(fs::read(schema3_backup_path()).unwrap(), schema3);
        let core = fs::read_to_string(super::path()).unwrap();
        assert!(
            !core.contains("after_json") && !core.contains("merged_issues"),
            "the shared core carries no project mirror"
        );
        for key in [&a_key, &b_key] {
            assert!(record_path(key).exists());
        }
        let after = (
            crate::load_issue_monitor_prefs(&a_path).unwrap(),
            crate::load_issue_monitor_prefs(&b_path).unwrap(),
            status(None).unwrap(),
        );
        assert_eq!(after.0, before.0);
        assert_eq!(after.1, before.1);
        assert_eq!(after.2.phase, RegistryPhase::Active);
        assert_eq!(
            after
                .2
                .projects
                .iter()
                .map(|row| (&row.transaction_id, &row.after_sha256, row.conflict))
                .collect::<Vec<_>>(),
            before
                .2
                .projects
                .iter()
                .map(|row| (&row.transaction_id, &row.after_sha256, row.conflict))
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            (fs::read(&a_path).unwrap(), fs::read(&b_path).unwrap()),
            mirrors,
            "migration rewrites no project mirror"
        );
        assert!(
            !migrate_to_split_storage().unwrap(),
            "a second start has nothing to migrate"
        );
        assert_eq!(fs::read(schema3_backup_path()).unwrap(), schema3);

        // The documented way back: restore the backup and drop the records.
        fs::copy(schema3_backup_path(), super::path()).unwrap();
        fs::remove_dir_all(records_dir()).unwrap();
        assert_eq!(storage_schema(), 3);
        assert_eq!(crate::load_issue_monitor_prefs(&a_path).unwrap(), before.0);
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(1);
            Ok(())
        })
        .unwrap();
        assert_eq!(storage_schema(), 3);
    }

    #[test]
    fn restore_schema3_preserves_current_prefs_and_pending_mirrors() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, b_path, b_key) = two_split_sources();
        let backup = fs::read(schema3_backup_path()).unwrap();
        rename_first_member(&a_path, "Current Cedar");
        let pending_mirror = fs::read(&a_path).unwrap();
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(7531);
            Ok(())
        })
        .unwrap();
        crate::try_mutate_issue_monitor_prefs(&b_path, |prefs| {
            prefs.merged_issues.push(7486);
            Ok(())
        })
        .unwrap();
        // A committed record whose publish was interrupted must also round trip.
        fs::write(&a_path, &pending_mirror).unwrap();
        fs::write(a_path.with_extension("committed.json"), &pending_mirror).unwrap();
        let mut expected = (
            crate::load_issue_monitor_prefs(&a_path).unwrap(),
            crate::load_issue_monitor_prefs(&b_path).unwrap(),
            status(None).unwrap(),
        );
        for row in &mut expected.2.projects {
            row.before_sha256.clone_from(&row.observed_sha256);
            row.floor_before_sha256
                .clone_from(&row.floor_observed_sha256);
        }
        let snapshots = load().unwrap().unwrap().sources[&b_key]
            .rosterless_snapshots
            .clone();
        assert!(!snapshots.is_empty());
        restore_schema3().unwrap();
        assert_eq!(storage_schema(), 3);
        // This is the schema 3 decoder used before split storage existed.
        let legacy: Registry = serde_json::from_slice(&fs::read(super::path()).unwrap()).unwrap();
        validate_proving(&legacy, Mirrors::All).unwrap();
        assert!(!legacy.store.split);
        assert_eq!(legacy.sources[&b_key].rosterless_snapshots, snapshots);
        assert_eq!(fs::read(&a_path).unwrap(), pending_mirror);
        assert_eq!(fs::read(schema3_backup_path()).unwrap(), backup);
        fs::remove_dir_all(records_dir()).unwrap();
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path).unwrap(),
            expected.0
        );
        assert_eq!(
            crate::load_issue_monitor_prefs(&b_path).unwrap(),
            expected.1
        );
        assert_eq!(status(None).unwrap(), expected.2);
        let restored = fs::read(super::path()).unwrap();
        restore_schema3().unwrap();
        assert_eq!(
            fs::read(super::path()).unwrap(),
            restored,
            "retry is a no-op"
        );
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(1);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .merged_issues,
            [7531, 1]
        );
        assert_eq!(storage_schema(), 3);
    }

    #[test]
    fn restore_schema3_interrupted_before_commit_keeps_split_storage_authoritative() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, a_key, _, _) = two_split_sources();
        let core = fs::read(super::path()).unwrap();
        let record = fs::read(record_path(&a_key)).unwrap();
        let prefs = crate::load_issue_monitor_prefs(&a_path).unwrap();
        let failure = ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            super::path(),
        );
        fs::write(super::path().with_extension("parent-sync-fail-once"), b"").unwrap();
        assert!(restore_schema3().is_err());
        assert_eq!(fs::read(super::path()).unwrap(), core);
        assert_eq!(fs::read(record_path(&a_key)).unwrap(), record);
        assert_eq!(storage_schema(), SPLIT_SCHEMA_VERSION);
        assert!(
            serde_json::from_slice::<Registry>(&core).is_err(),
            "old readers fail closed"
        );
        assert_eq!(crate::load_issue_monitor_prefs(&a_path).unwrap(), prefs);
        drop(failure);
        restore_schema3().unwrap();
        assert_eq!(storage_schema(), 3);
        // A crash after the atomic rename can leave schema 3 visible without
        // a successful directory sync. Retrying must confirm it before ACK.
        fs::write(unconfirmed_path(), b"").unwrap();
        let failure = ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            super::path(),
        );
        fs::write(super::path().with_extension("parent-sync-fail-once"), b"").unwrap();
        assert!(restore_schema3().is_err());
        assert!(unconfirmed_path().exists());
        drop(failure);
        restore_schema3().unwrap();
        assert!(!unconfirmed_path().exists());
        assert_eq!(crate::load_issue_monitor_prefs(&a_path).unwrap(), prefs);
    }

    #[test]
    fn restore_schema3_refuses_missing_or_damaged_records_without_writes() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (_, _, _, b_key) = two_split_sources();
        let core = fs::read(super::path()).unwrap();
        let record = fs::read(record_path(&b_key)).unwrap();
        fs::remove_file(record_path(&b_key)).unwrap();
        assert!(restore_schema3().is_err());
        assert_eq!(fs::read(super::path()).unwrap(), core);
        fs::write(record_path(&b_key), b"{ torn").unwrap();
        assert!(restore_schema3().is_err());
        assert_eq!(fs::read(super::path()).unwrap(), core);
        fs::write(record_path(&b_key), record).unwrap();
        restore_schema3().unwrap();
    }

    #[test]
    fn restore_schema3_refuses_assignments_and_live_or_uncertain_sessions() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, _, _) = two_split_sources();
        let root = gwt_core::paths::gwt_home().join("repo");
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let member = crate::agent_names::reserve_project_member(
            &root,
            &sessions,
            "downgrade-slot",
            Some(7531),
            None,
            None,
            None,
        )
        .unwrap();
        let core = fs::read(super::path()).unwrap();
        assert!(restore_schema3()
            .unwrap_err()
            .to_string()
            .contains("assignment"));
        assert_eq!(fs::read(super::path()).unwrap(), core);
        crate::agent_names::release_project_member(
            &root,
            &sessions,
            &member.agent_member_id,
            "downgrade-slot",
        )
        .unwrap();
        let core = fs::read(super::path()).unwrap();
        let session = gwt_agent::Session::new(&root, "unnamed", gwt_agent::AgentId::Codex);
        session.save(&sessions).unwrap();
        let runtime_path =
            gwt_agent::runtime_state_path_for_pid(&sessions, 2_000_000_000, &session.id);
        let mut runtime = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running);
        runtime.child_pid = Some(std::process::id());
        runtime.child_started_at = crate::process::host_process_start_time(std::process::id());
        assert!(runtime.child_started_at.is_some());
        runtime.save(&runtime_path).unwrap();
        assert!(restore_schema3()
            .unwrap_err()
            .to_string()
            .contains("Session"));
        assert_eq!(fs::read(super::path()).unwrap(), core);
        runtime.child_started_at = None;
        runtime.save(&runtime_path).unwrap();
        assert!(restore_schema3()
            .unwrap_err()
            .to_string()
            .contains("child_identity_missing"));
        assert_eq!(fs::read(super::path()).unwrap(), core);
        fs::remove_file(runtime_path).unwrap();
        restore_schema3().unwrap();
        assert!(crate::load_issue_monitor_prefs(&a_path).is_ok());
    }

    #[test]
    fn interrupted_split_migration_leaves_the_schema3_registry_authoritative() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, a_key, _, _) = two_active_sources();
        let schema3 = fs::read(super::path()).unwrap();
        // Records and the backup were written; the core write never happened.
        assert!(migrate_to_split_storage().unwrap());
        fs::write(super::path(), &schema3).unwrap();
        assert_eq!(storage_schema(), 3);
        let stale_record = fs::read(record_path(&a_key)).unwrap();
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(7486);
            Ok(())
        })
        .unwrap();
        assert_eq!(storage_schema(), 3, "the schema 3 writer keeps working");
        assert_eq!(fs::read(record_path(&a_key)).unwrap(), stale_record);

        assert!(
            migrate_to_split_storage().unwrap(),
            "the next start migrates again"
        );
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .merged_issues,
            [7486],
            "a record left by the interrupted attempt never replaces a later commit"
        );
        assert!(load().is_ok());
    }

    #[test]
    fn split_prefs_transaction_reads_and_writes_only_its_own_record() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, a_key, b_path, b_key) = two_split_sources();
        let core = fs::read(super::path()).unwrap();
        let revision = status(None).unwrap().registry_revision;
        // Nothing of another project is needed: not even its record file.
        let b_record = fs::read(record_path(&b_key)).unwrap();
        fs::remove_file(record_path(&b_key)).unwrap();

        take_registry_io_for_test();
        crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        let io = take_registry_io_for_test();
        assert_eq!(
            (
                io.durability_confirmations,
                io.registry_writes,
                io.record_writes.len(),
                io.mirror_publishes
            ),
            (0, 0, 0, 0),
            "{io:?}"
        );

        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(7486);
            Ok(())
        })
        .unwrap();
        let io = take_registry_io_for_test();
        assert_eq!(
            (
                io.durability_confirmations,
                io.registry_writes,
                io.mirror_publishes
            ),
            (0, 0, 1),
            "a mirror-only commit never writes the shared core: {io:?}"
        );
        assert_eq!(io.record_writes, std::slice::from_ref(&a_key));
        assert!(!io.validated_payloads.contains(&b_key), "{io:?}");
        assert_eq!(fs::read(super::path()).unwrap(), core);
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .merged_issues,
            [7486]
        );

        // The project whose record is missing is closed; so is every complete read.
        assert!(crate::load_issue_monitor_prefs(&b_path).is_err());
        assert!(crate::try_mutate_issue_monitor_prefs(&b_path, |_| Ok(())).is_err());
        assert!(load().is_err());
        fs::write(record_path(&b_key), b_record).unwrap();
        assert!(crate::load_issue_monitor_prefs(&b_path).is_ok());
        assert_eq!(status(None).unwrap().registry_revision, revision);
    }

    #[test]
    fn split_core_change_commits_its_record_with_the_core_or_not_at_all() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, a_key, _, _) = two_split_sources();
        let floor_path = a_path.with_extension("committed.json");
        let before = (
            fs::read(super::path()).unwrap(),
            fs::read(&a_path).unwrap(),
            fs::read(&floor_path).unwrap(),
        );
        let revision = status(None).unwrap().registry_revision;

        take_registry_io_for_test();
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.agent_roster.as_mut().unwrap().members[0].name = "Committed Cedar".into();
            prefs.merged_issues.push(7486);
            Ok(())
        })
        .unwrap();
        let io = take_registry_io_for_test();
        assert_eq!(
            (io.registry_writes, io.record_writes.clone()),
            (1, vec![a_key.clone()]),
            "{io:?}"
        );
        assert_eq!(status(None).unwrap().registry_revision, revision + 1);
        let committed = (
            fs::read(super::path()).unwrap(),
            fs::read(&a_path).unwrap(),
            fs::read(record_path(&a_key)).unwrap(),
        );

        // A crash before the core write: the pending record is on disk, the
        // core and both mirrors are one generation behind.
        fs::write(super::path(), &before.0).unwrap();
        fs::write(&a_path, &before.1).unwrap();
        fs::write(&floor_path, &before.2).unwrap();
        assert_eq!(first_member_name(&a_path), "Cedar");
        assert!(crate::load_issue_monitor_prefs(&a_path)
            .unwrap()
            .merged_issues
            .is_empty());
        take_registry_io_for_test();
        let (prefs, ()) = crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        assert!(prefs.merged_issues.is_empty());
        assert_eq!(take_registry_io_for_test().mirror_publishes, 0);
        assert_eq!(fs::read(&a_path).unwrap(), before.1);
        // The abandoned pending record never surfaces through a later commit.
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(1);
            Ok(())
        })
        .unwrap();
        assert_eq!(first_member_name(&a_path), "Cedar");
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .merged_issues,
            [1]
        );
        assert!(load().is_ok());

        // A crash after the core write: readers and writers roll forward.
        fs::write(record_path(&a_key), &committed.2).unwrap();
        fs::write(super::path(), &committed.0).unwrap();
        fs::write(&a_path, &before.1).unwrap();
        fs::write(&floor_path, &before.2).unwrap();
        assert_eq!(first_member_name(&a_path), "Committed Cedar");
        let (prefs, ()) = crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        assert_eq!(prefs.merged_issues, [7486]);
        assert_eq!(fs::read(&a_path).unwrap(), committed.1);
        assert_eq!(fs::read(&floor_path).unwrap(), committed.1);
    }

    #[test]
    fn split_core_write_of_unknown_durability_is_confirmed_before_any_ack() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, _, _) = two_split_sources();
        let registry_path = super::path();
        let marker = registry_path.with_extension("parent-sync-fail-once");
        let mut failure = None;
        assert!(crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            fs::write(&marker, b"fail once")?;
            failure = Some(ScopedEnvVar::set(
                "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
                &registry_path,
            ));
            prefs.agent_roster.as_mut().unwrap().members[0].name = "Committed Cedar".into();
            Ok(())
        })
        .is_err());
        fs::write(&marker, b"fail confirmation once").unwrap();
        assert!(crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).is_err());
        take_registry_io_for_test();
        let (replayed, ()) = crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        assert_eq!(take_registry_io_for_test().durability_confirmations, 1);
        assert_eq!(
            replayed.agent_roster.unwrap().members[0].name,
            "Committed Cedar"
        );
        crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        assert_eq!(take_registry_io_for_test().durability_confirmations, 0);
        drop(failure);
    }

    #[test]
    fn split_records_fail_closed_when_damaged_or_swapped_between_projects() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, a_key, b_path, b_key) = two_split_sources();
        let a_record = fs::read(record_path(&a_key)).unwrap();
        let a_mirror = fs::read(&a_path).unwrap();
        let closed = |reason: &str| {
            assert!(
                crate::load_issue_monitor_prefs(&a_path).is_err(),
                "{reason}: read"
            );
            assert!(
                crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
                    prefs.merged_issues.push(1);
                    Ok(())
                })
                .is_err(),
                "{reason}: write"
            );
            assert!(load().is_err(), "{reason}: complete read");
            assert_eq!(fs::read(&a_path).unwrap(), a_mirror, "{reason}: mirror");
            assert!(
                crate::load_issue_monitor_prefs(&b_path).is_ok(),
                "{reason}: the other project stays open"
            );
        };

        fs::copy(record_path(&b_key), record_path(&a_key)).unwrap();
        closed("another project's record");

        let mut damaged: serde_json::Value = serde_json::from_slice(&a_record).unwrap();
        let slot = if damaged["pending"].is_null() {
            "record"
        } else {
            "pending"
        };
        damaged[slot]["after_json"] = serde_json::json!("{\"tampered\":true}");
        fs::write(record_path(&a_key), serde_json::to_vec(&damaged).unwrap()).unwrap();
        closed("mirror bytes that do not match their fingerprint");

        fs::write(record_path(&a_key), b"{ torn").unwrap();
        closed("torn record");

        fs::write(record_path(&a_key), &a_record).unwrap();
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(7486);
            Ok(())
        })
        .unwrap();
        assert!(load().is_ok());
    }

    #[test]
    fn only_the_desktop_app_start_changes_the_storage_an_older_binary_reads() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, b_path, _) = two_active_sources();
        // Exactly how a binary older than the split decodes the registry.
        let older_binary_reads = || {
            serde_json::from_slice::<serde_json::Value>(&fs::read(super::path()).unwrap())
                .ok()
                .filter(|registry| registry["schema_version"].as_u64() <= Some(3))
                .is_some_and(|registry| {
                    registry["sources"]
                        .as_object()
                        .unwrap()
                        .values()
                        .all(|source| source["after_json"].is_string())
                })
        };
        // A helper CLI of this version works on the schema 3 file in place:
        // commits, core changes and a late enrollment all leave it readable.
        rename_first_member(&a_path, "Renamed Cedar");
        crate::try_mutate_issue_monitor_prefs(&b_path, |prefs| {
            prefs.merged_issues.push(7486);
            Ok(())
        })
        .unwrap();
        let late = gwt_core::paths::gwt_home().join("late-project");
        fs::create_dir_all(&late).unwrap();
        crate::try_mutate_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(&late),
            |prefs| {
                prefs.merged_issues.push(1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(storage_schema(), 3);
        assert!(!records_dir().exists());
        assert!(!schema3_backup_path().exists());
        assert!(older_binary_reads());

        assert!(migrate_to_split_storage().unwrap());
        assert!(
            !older_binary_reads(),
            "an older binary fails closed instead of reading a partial registry"
        );
        assert!(
            serde_json::from_slice::<Registry>(&fs::read(super::path()).unwrap()).is_err(),
            "the schema 3 decoder rejects the split core"
        );
        // The same helper CLI keeps working on the storage it now finds.
        rename_first_member(&a_path, "Split Cedar");
        assert_eq!(first_member_name(&a_path), "Split Cedar");
        assert_eq!(storage_schema(), SPLIT_SCHEMA_VERSION);
    }

    #[test]
    fn split_storage_enrolls_projects_and_serves_them_concurrently() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, b_path, _) = two_split_sources();
        let late = gwt_core::paths::gwt_home().join("late-project");
        fs::create_dir_all(&late).unwrap();
        let late_path = crate::issue_monitor_prefs_path_for_repo_path(&late);
        crate::try_mutate_issue_monitor_prefs(&late_path, |prefs| {
            prefs.merged_issues.push(0);
            Ok(())
        })
        .unwrap();
        assert_eq!(storage_schema(), SPLIT_SCHEMA_VERSION);
        assert!(record_path(&source_key(&late_path).unwrap()).exists());

        // One writer per project, as one daemon per project, plus a shared-core writer.
        let paths = [a_path.clone(), b_path, late_path];
        let home = home.path();
        std::thread::scope(|scope| {
            for path in &paths {
                scope.spawn(move || {
                    let _home = ScopedGwtHome::set(home);
                    for issue in 1..=20 {
                        crate::try_mutate_issue_monitor_prefs(path, |prefs| {
                            prefs.merged_issues.push(issue);
                            Ok(())
                        })
                        .unwrap();
                        crate::load_issue_monitor_prefs(path).unwrap();
                    }
                });
            }
            scope.spawn(|| {
                let _home = ScopedGwtHome::set(home);
                for index in 0..10 {
                    rename_first_member(&a_path, &format!("Cedar {index}"));
                }
            });
        });
        for path in &paths {
            let prefs = crate::load_issue_monitor_prefs(path).unwrap();
            assert_eq!(
                prefs
                    .merged_issues
                    .iter()
                    .filter(|issue| **issue > 0)
                    .count(),
                20,
                "{}",
                path.display()
            );
            assert_eq!(
                fs::read(path).unwrap(),
                fs::read(path.with_extension("committed.json")).unwrap()
            );
        }
        assert_eq!(first_member_name(&a_path), "Cedar 9");
        let status = status(None).unwrap();
        assert_eq!(status.phase, RegistryPhase::Active);
        assert!(status
            .projects
            .iter()
            .all(|row| !row.conflict && !row.pending && !row.floor_conflict));
    }

    /// Prints the same `held_ms` the "Issues prefs lock released" record reports.
    /// `cargo test -p gwt --lib --release measure_prefs_lock_held_ms -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement, not an assertion"]
    fn measure_prefs_lock_held_ms() {
        measure_prefs_lock_held(false);
    }

    /// The same measurement after the desktop app moved to split storage.
    #[test]
    #[ignore = "measurement, not an assertion"]
    fn measure_split_prefs_lock_held_ms() {
        measure_prefs_lock_held(true);
    }

    /// Issue #7486 AC-4: the daemons one GUI start launches together must not
    /// queue behind each other's durable writes.
    #[test]
    fn a_record_write_excludes_shared_writers_but_not_another_projects_transaction() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, b_path, _) = two_split_sources();
        let home = home.path().to_path_buf();
        let other = b_path.clone();
        BEFORE_RECORD_WRITE.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                let within = |budget: u64| {
                    gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                        gwt_core::operation_deadline::now()
                            + std::time::Duration::from_millis(budget),
                    )
                };
                std::thread::scope(|scope| {
                    let transaction = scope.spawn(|| {
                        let _home = ScopedGwtHome::set(&home);
                        // Ample: a transaction that still queued would spend all of it.
                        let _deadline = within(5_000);
                        crate::try_mutate_issue_monitor_prefs(&other, |prefs| {
                            prefs.merged_issues.push(7);
                            Ok(())
                        })
                    });
                    transaction
                        .join()
                        .unwrap()
                        .expect("another project commits during this durable write");
                    // A roster change through another project's transaction
                    // commits its pending record and the core meanwhile.
                    scope
                        .spawn(|| {
                            let _home = ScopedGwtHome::set(&home);
                            let _deadline = within(5_000);
                            rename_first_member(&other, "Interleaved Cedar");
                        })
                        .join()
                        .unwrap();
                    let shared_writer = scope.spawn(|| {
                        let _home = ScopedGwtHome::set(&home);
                        let _deadline = within(100);
                        lock().map(drop)
                    });
                    assert!(
                        shared_writer.join().unwrap().is_err(),
                        "a writer of shared state waits for the open record write"
                    );
                    let older_binary = scope.spawn(|| {
                        let _home = ScopedGwtHome::set(&home);
                        let _deadline = within(100);
                        NamedFileLock::acquire(&lock_path().unwrap(), "shared Agent registry")
                            .map(drop)
                    });
                    assert!(
                        older_binary.join().unwrap().is_err(),
                        "a binary that knows only the registry lock waits as well"
                    );
                });
            }));
        });
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(1);
            Ok(())
        })
        .unwrap();
        assert!(BEFORE_RECORD_WRITE.with(|slot| slot.borrow().is_none()));
        for path in [&a_path, &b_path] {
            assert_eq!(first_member_name(path), "Interleaved Cedar");
        }
        for (path, issue) in [(&a_path, 1), (&b_path, 7)] {
            assert!(crate::load_issue_monitor_prefs(path)
                .unwrap()
                .merged_issues
                .contains(&issue));
            assert_eq!(
                fs::read(path).unwrap(),
                fs::read(path.with_extension("committed.json")).unwrap()
            );
        }
        let status = status(None).unwrap();
        assert!(status
            .projects
            .iter()
            .all(|row| !row.conflict && !row.pending && !row.floor_conflict));
    }

    /// Every path takes the core lock before the registry lock, and a binary
    /// that knows only the registry lock still excludes a project transaction.
    #[test]
    fn registry_locks_are_taken_core_first_and_exclude_an_older_binary() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, ..) = two_split_sources();
        let home = home.path();
        let blocked = |attempt: &(dyn Fn() -> io::Result<()> + Sync)| {
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        let _home = ScopedGwtHome::set(home);
                        let _deadline =
                            gwt_core::operation_deadline::ScopedOperationDeadline::enter(
                                gwt_core::operation_deadline::now()
                                    + std::time::Duration::from_millis(100),
                            );
                        attempt().unwrap_err().to_string()
                    })
                    .join()
                    .unwrap()
            })
        };
        let transaction = || crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).map(drop);
        let shared_writer = || lock().map(drop);
        {
            let _both = lock().unwrap();
            for error in [blocked(&transaction), blocked(&shared_writer)] {
                assert!(
                    error.contains("operation=shared Agent registry core;"),
                    "{error}"
                );
            }
        }
        {
            let _older_binary =
                NamedFileLock::acquire(&lock_path().unwrap(), "shared Agent registry").unwrap();
            for error in [blocked(&transaction), blocked(&shared_writer)] {
                assert!(
                    error.contains("operation=shared Agent registry;"),
                    "{error}"
                );
            }
        }
        transaction().unwrap();
    }

    /// The record write after the core lock is released is interrupted once
    /// the new record is visible but before its durability is confirmed.
    #[test]
    fn interrupted_record_write_stays_committed_and_leaves_other_projects_open() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, b_path, _) = two_split_sources();
        let record = record_path(&source_key(&a_path).unwrap());
        let marker = record_unconfirmed_path(&source_key(&a_path).unwrap());
        let mirror_before = fs::read(&a_path).unwrap();
        fs::write(record.with_extension("parent-sync-fail-once"), b"").unwrap();
        {
            let _fail = ScopedEnvVar::set(
                "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
                &record,
            );
            assert!(crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
                prefs.merged_issues.push(7486);
                Ok(())
            })
            .is_err());
        }
        assert!(marker.exists());
        assert_eq!(fs::read(&a_path).unwrap(), mirror_before);
        crate::try_mutate_issue_monitor_prefs(&b_path, |prefs| {
            prefs.merged_issues.push(7);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .merged_issues,
            [7486],
            "the visible record is the committed generation"
        );
        take_registry_io_for_test();
        let (replayed, ()) = crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        let io = take_registry_io_for_test();
        assert_eq!(replayed.merged_issues, [7486]);
        assert_eq!(
            (io.durability_confirmations, io.mirror_publishes),
            (1, 1),
            "{io:?}"
        );
        assert!(!marker.exists());
        assert_eq!(
            fs::read(&a_path).unwrap(),
            fs::read(a_path.with_extension("committed.json")).unwrap()
        );
        let status = status(None).unwrap();
        assert!(status
            .projects
            .iter()
            .all(|row| !row.conflict && !row.pending && !row.floor_conflict));
    }

    /// A record write that could not confirm its durability left its marker.
    #[test]
    fn an_unconfirmed_record_write_is_confirmed_by_the_next_transaction() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, ..) = two_split_sources();
        let marker = record_unconfirmed_path(&source_key(&a_path).unwrap());
        fs::write(&marker, b"").unwrap();
        take_registry_io_for_test();
        crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        assert_eq!(take_registry_io_for_test().durability_confirmations, 1);
        assert!(!marker.exists());
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(1);
            Ok(())
        })
        .unwrap();
        assert_eq!(take_registry_io_for_test().durability_confirmations, 0);
        assert!(!marker.exists());
    }

    /// One saving writer per project, all started together, as the daemons of
    /// one GUI start. Prints each transaction's whole elapsed time, which is
    /// what `ISSUE_MONITOR_PREFS_BUDGET` (250 ms) bounds.
    /// `cargo test -p gwt --lib --release measure_concurrent_projects_prefs_transaction_ms -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement, not an assertion"]
    fn measure_concurrent_projects_prefs_transaction_ms() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, b_path, _) = two_split_sources();
        let mut paths = vec![a_path, b_path];
        for index in 0..6 {
            let root = gwt_core::paths::gwt_home().join(format!("project-{index}"));
            fs::create_dir_all(&root).unwrap();
            paths.push(crate::issue_monitor_prefs_path_for_repo_path(&root));
        }
        let padding: Vec<u64> = (0..46_000).map(|index| 1_000_000 + index).collect();
        for path in &paths {
            crate::try_mutate_issue_monitor_prefs(path, |prefs| {
                prefs.merged_issues.clone_from(&padding);
                Ok(())
            })
            .unwrap();
        }
        let home = home.path();
        let start = std::sync::Barrier::new(paths.len());
        let mut samples: Vec<f64> = std::thread::scope(|scope| {
            let writers: Vec<_> = paths
                .iter()
                .map(|path| {
                    let start = &start;
                    scope.spawn(move || {
                        let _home = ScopedGwtHome::set(home);
                        start.wait();
                        (0..20)
                            .map(|issue| {
                                let began = std::time::Instant::now();
                                crate::try_mutate_issue_monitor_prefs(path, |prefs| {
                                    prefs.merged_issues.push(issue);
                                    Ok(())
                                })
                                .unwrap();
                                began.elapsed().as_secs_f64() * 1000.0
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            writers
                .into_iter()
                .flat_map(|writer| writer.join().unwrap())
                .collect()
        });
        samples.sort_by(f64::total_cmp);
        println!(
            "projects={} mirror={}B transaction(median,p90,max)=({:.1},{:.1},{:.1})ms",
            paths.len(),
            fs::metadata(&paths[0]).unwrap().len(),
            samples[samples.len() / 2],
            samples[samples.len() * 9 / 10],
            samples[samples.len() - 1],
        );
    }

    fn measure_prefs_lock_held(split: bool) {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, _, b_path, _) = two_active_sources();
        if split {
            assert!(migrate_to_split_storage().unwrap());
        }
        let padding: Vec<u64> = (0..46_000).map(|index| 1_000_000 + index).collect();
        for path in [&a_path, &b_path] {
            crate::try_mutate_issue_monitor_prefs(path, |prefs| {
                prefs.merged_issues.clone_from(&padding);
                Ok(())
            })
            .unwrap();
        }
        let median = |mut samples: Vec<f64>| {
            samples.sort_by(f64::total_cmp);
            (samples[samples.len() / 2], samples[samples.len() * 9 / 10])
        };
        let mut unchanged = Vec::new();
        let mut committed = Vec::new();
        for index in 0..40 {
            crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
            unchanged.push(crate::issue_monitor::prefs_lock_held_ms_for_test());
            crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
                prefs.merged_issues.push(index);
                Ok(())
            })
            .unwrap();
            committed.push(crate::issue_monitor::prefs_lock_held_ms_for_test());
        }
        println!(
            "registry={}B mirror={}B unchanged(median,p90)={:?}ms commit(median,p90)={:?}ms",
            fs::metadata(super::path()).unwrap().len(),
            fs::metadata(&a_path).unwrap().len(),
            median(unchanged),
            median(committed),
        );
    }

    /// Issue #7550: what a transaction costs grows with its project's record,
    /// so each committed generation is decoded once and the new one proven once.
    #[test]
    fn a_prefs_transaction_decodes_each_mirror_generation_once() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, ..) = two_split_sources();
        take_registry_io_for_test();

        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(7550);
            Ok(())
        })
        .unwrap();
        assert_eq!(take_registry_io_for_test().mirror_decodes, 2);

        crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
        assert_eq!(take_registry_io_for_test().mirror_decodes, 1);

        let saved = crate::load_issue_monitor_prefs(&a_path).unwrap();
        assert_eq!(take_registry_io_for_test().mirror_decodes, 1);
        assert_eq!(saved.merged_issues, vec![7550]);
    }

    /// Issue #7550: a writer that edits the prefs directly saves them under
    /// the same retention rule, and the mirror it shrinks first is kept once.
    #[test]
    fn every_registry_writer_saves_under_the_retention_rule_and_keeps_the_first_mirror() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, ..) = two_split_sources();
        let kept = pre_retention_mirror_path(&a_path);
        let closed = |issue_number| crate::issue_monitor::IssueClosureRecord {
            issue_number,
            generation: 1,
            state: crate::issue_monitor::IssueClosureState::Closed,
            evidence: Default::default(),
            issue_updated_at: None,
            reopened_after_close: false,
        };
        let identity = |issue_number| crate::issue_monitor::IssueMonitorClaimIdentity {
            issue_number,
            claim_id: format!("gwt-queue:{issue_number}:claim"),
            owner: "host:user:1".into(),
        };
        // What a version without the rule committed.
        let mut legacy = decode(Some(&fs::read_to_string(&a_path).unwrap())).unwrap();
        legacy.closure_records = vec![closed(1)];
        legacy.claim_identities = vec![identity(1), identity(2)];
        let legacy_json = mirror_json(&legacy, None, None).unwrap();
        let mut registry = load().unwrap().unwrap();
        let key = source_key(&a_path).unwrap();
        let source = registry.sources.get_mut(&key).unwrap();
        source.before_sha256 = source.after_sha256.clone();
        source.floor_before_sha256 = source.after_sha256.clone();
        source.after_sha256 = fingerprint(&legacy_json);
        source.after_json = legacy_json.clone();
        save(&registry).unwrap();
        publish_verified(&registry.sources[&key], &a_path).unwrap();
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .claim_identities,
            vec![identity(1), identity(2)],
            "reading never drops anything"
        );
        assert!(!kept.exists());

        let (saved, ()) = crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.merged_issues.push(7550);
            Ok(())
        })
        .unwrap();

        assert_eq!(saved.claim_identities, vec![identity(2)]);
        assert_eq!(fs::read_to_string(&kept).unwrap(), legacy_json);

        // A writer that puts a retired entry back saves without it, and the
        // kept mirror stays the first generation.
        let (saved, ()) = crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            prefs.claim_identities.insert(0, identity(1));
            Ok(())
        })
        .unwrap();
        assert_eq!(saved.claim_identities, vec![identity(2)]);
        assert_eq!(
            crate::load_issue_monitor_prefs(&a_path)
                .unwrap()
                .claim_identities,
            vec![identity(2)]
        );
        assert_eq!(fs::read_to_string(&kept).unwrap(), legacy_json);
    }

    /// The mirror encoder as it was before Issue #7550, kept as the oracle for
    /// the bytes a project commits.
    fn mirror_json_before_7550(
        prefs: &IssueMonitorPrefs,
        original: Option<&str>,
    ) -> io::Result<String> {
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
            profile.codex_speed = gwt_agent::CodexSpeed::Off;
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
        let mut known =
            serde_json::to_value(IssueMonitorPrefs::default()).map_err(io::Error::other)?;
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

    /// Issue #7550: reusing the committed decode changes how the mirror is
    /// encoded, never one byte of what is committed.
    #[test]
    fn the_mirror_a_project_commits_is_byte_identical_to_the_previous_encoder() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, ..) = two_split_sources();
        let (projected, ()) = crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            fill_heavy_prefs_for_test(prefs);
            Ok(())
        })
        .unwrap();
        let committed = fs::read_to_string(&a_path).unwrap();
        // What an older or newer version wrote: fields this version does not
        // know, a known field written empty, a roster, and a legacy field.
        let mut foreign: serde_json::Value = serde_json::from_str(&committed).unwrap();
        foreign["future_field"] = serde_json::json!({"nested": [3, 1, {"z": null, "a": "é\n"}]});
        foreign["another_future_field"] = serde_json::json!("text");
        foreign["requeue_audit"] = serde_json::json!([]);
        foreign["launch_usage_threshold_percent"] = serde_json::json!(80);
        foreign["agent_roster"] =
            serde_json::json!({"schema_version": 1, "revision": 1, "members": []});
        let foreign = serde_json::to_string(&foreign).unwrap();
        let mut changed = projected.clone();
        changed.merged_issues.push(7550);
        changed.closure_records.truncate(3);
        changed.claim_diagnostics.clear();

        let mut compared = 0;
        for original in [None, Some(committed.as_str()), Some(foreign.as_str())] {
            let decoded = original.map(|original| decode(Some(original)).unwrap());
            for prefs in [&projected, &changed, &IssueMonitorPrefs::default()] {
                let expected = mirror_json_before_7550(prefs, original).unwrap();
                assert_eq!(mirror_json(prefs, original, None).unwrap(), expected);
                assert_eq!(
                    mirror_json(prefs, original, decoded.as_ref()).unwrap(),
                    expected
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 9);
        assert!(mirror_json_before_7550(&changed, Some(&foreign))
            .unwrap()
            .contains("\"future_field\""));
        for malformed in ["[1]", "{"] {
            assert_eq!(
                mirror_json(&projected, Some(malformed), Some(&projected))
                    .unwrap_err()
                    .to_string(),
                mirror_json_before_7550(&projected, Some(malformed))
                    .unwrap_err()
                    .to_string()
            );
        }
    }

    /// One project's transactions on the 1.3 MB record Issue #7550 reported:
    /// the read before the lock, an unchanged transaction and a saving one.
    /// `cargo test -p gwt --lib --release measure_heavy_record_prefs_transaction_ms -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement, not an assertion"]
    fn measure_heavy_record_prefs_transaction_ms() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let (a_path, a_key, ..) = two_split_sources();
        crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
            fill_heavy_prefs_for_test(prefs);
            Ok(())
        })
        .unwrap();
        let ms = |began: std::time::Instant| began.elapsed().as_secs_f64() * 1000.0;
        let median = |mut samples: Vec<f64>| {
            samples.sort_by(f64::total_cmp);
            samples[samples.len() / 2]
        };
        let (mut read, mut unchanged, mut saved) = (Vec::new(), Vec::new(), Vec::new());
        for index in 0..40 {
            let began = std::time::Instant::now();
            crate::load_issue_monitor_prefs(&a_path).unwrap();
            read.push(ms(began));
            let began = std::time::Instant::now();
            crate::try_mutate_issue_monitor_prefs(&a_path, |_| Ok(())).unwrap();
            unchanged.push(ms(began));
            let began = std::time::Instant::now();
            crate::try_mutate_issue_monitor_prefs(&a_path, |prefs| {
                prefs.merged_issues.push(index);
                Ok(())
            })
            .unwrap();
            saved.push(ms(began));
        }
        println!(
            "mirror={}B record={}B read={:.1}ms unchanged={:.1}ms saved={:.1}ms (medians)",
            fs::metadata(&a_path).unwrap().len(),
            fs::metadata(record_path(&a_key)).unwrap().len(),
            median(read),
            median(unchanged),
            median(saved),
        );
    }
}
