//! Revisioned PM planning state (SPEC #3431 T-109/T-110).
//!
//! This store owns planning data, never PM registration or Monitor execution
//! state. Protected operations and semantic policy are separate adapters; a
//! caller of this storage primitive must establish its own PM authority.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use gwt_core::operation_deadline::NamedFileLock;
use serde::{Deserialize, Serialize};

pub const PLANNING_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Goal {
    pub id: String,
    pub outcome: String,
    pub metric: String,
    pub target: String,
    pub horizon: String,
    pub owner: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoadmapHorizon {
    Now,
    Next,
    #[default]
    Later,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoadmapItem {
    pub id: String,
    pub goal_id: Option<String>,
    pub category: Option<String>,
    pub horizon: RoadmapHorizon,
    pub milestone_id: Option<String>,
    pub issues: Vec<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Milestone {
    pub id: String,
    pub target_date: Option<String>,
    pub exit_criteria: Vec<String>,
    pub health: Option<String>,
    pub confidence: Option<String>,
    pub issues: Vec<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Risk {
    pub id: String,
    pub severity: String,
    pub probability: String,
    pub impact: String,
    pub assumptions: Vec<String>,
    pub owner: String,
    pub review_date: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Decision {
    pub id: String,
    pub assumptions: Vec<String>,
    pub options: Vec<String>,
    pub rationale: String,
    pub owner: String,
    pub review_date: Option<String>,
    pub source_revision: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecurringItem {
    pub id: String,
    pub cadence: String,
    pub owner: String,
    pub next_run: Option<String>,
    pub miss_policy: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlanningPolicy {
    pub id: String,
    pub version: u64,
    pub wip_limit: Option<u32>,
    pub sla_seconds: Option<u64>,
    pub starvation_after_seconds: Option<u64>,
    pub allow_preemption: bool,
    pub token_budget: Option<u64>,
    pub cost_budget: Option<String>,
    pub timezone: Option<String>,
    pub quiet_hours: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Forecast {
    pub id: String,
    pub source_revision: u64,
    pub sample_size: u64,
    pub estimate: Option<String>,
    pub confidence: Option<String>,
    pub missing_inputs: Vec<String>,
}

/// A bounded planning-only upsert, never an execution or registration action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PlanningAction {
    Goal(Goal),
    Milestone(Milestone),
    Risk(Risk),
    Decision(Decision),
    Policy(PlanningPolicy),
    Recurring(RecurringItem),
    Link(RoadmapItem),
}

impl PlanningAction {
    fn target(&self) -> (&'static str, &str) {
        match self {
            Self::Goal(value) => ("goal", &value.id),
            Self::Milestone(value) => ("milestone", &value.id),
            Self::Risk(value) => ("risk", &value.id),
            Self::Decision(value) => ("decision", &value.id),
            Self::Policy(value) => ("policy", &value.id),
            Self::Recurring(value) => ("recurring", &value.id),
            Self::Link(value) => ("link", &value.id),
        }
    }

    fn upsert(&self, state: &mut PmPlanningState) {
        fn upsert<T>(items: &mut Vec<T>, value: T, id: impl Fn(&T) -> &str) {
            if let Some(item) = items.iter_mut().find(|item| id(item) == id(&value)) {
                *item = value;
            } else {
                items.push(value);
            }
        }
        match self {
            Self::Goal(value) => upsert(&mut state.goals, value.clone(), |item| &item.id),
            Self::Milestone(value) => upsert(&mut state.milestones, value.clone(), |item| &item.id),
            Self::Risk(value) => upsert(&mut state.risks, value.clone(), |item| &item.id),
            Self::Decision(value) => upsert(&mut state.decisions, value.clone(), |item| &item.id),
            Self::Policy(value) => upsert(&mut state.policies, value.clone(), |item| &item.id),
            Self::Recurring(value) => {
                upsert(&mut state.recurring_items, value.clone(), |item| &item.id)
            }
            Self::Link(value) => upsert(&mut state.roadmap_items, value.clone(), |item| &item.id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningApplyRequest {
    pub action: PlanningAction,
    pub expected_revision: u64,
    pub reason: String,
    pub idempotency_key: String,
}

impl PlanningApplyRequest {
    pub fn validate(&self) -> io::Result<()> {
        if self.reason.trim().is_empty()
            || self.idempotency_key.trim().is_empty()
            || self.action.target().1.trim().is_empty()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "planning reason, idempotency_key and action value.id must be nonempty",
            ));
        }
        let action = serde_json::to_string(&self.action).map_err(io::Error::other)?;
        if gwt_core::error_ledger::redact_credentials(&action) != action {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "planning records must not contain credentials",
            ));
        }
        Ok(())
    }
}

/// Receipts contain identity and request hashes, never a raw action or key.
/// Retention is intentionally append-only until the later audit-policy slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningReceipt {
    pub actor: String,
    pub requester: String,
    pub action_kind: String,
    pub target_id: String,
    pub expected_revision: u64,
    pub resulting_revision: u64,
    pub reason: String,
    pub policy_versions: BTreeMap<String, u64>,
    pub risk_class: String,
    pub outcome: String,
    pub request_digest: String,
    pub idempotency_key_digest: String,
}

/// Typed decision data. Missing fields retain safe empty defaults; unknown
/// fields are refused so an older writer cannot silently discard newer data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PmPlanningState {
    pub schema_version: u32,
    pub revision: u64,
    pub goals: Vec<Goal>,
    pub roadmap_items: Vec<RoadmapItem>,
    pub milestones: Vec<Milestone>,
    pub risks: Vec<Risk>,
    pub decisions: Vec<Decision>,
    pub recurring_items: Vec<RecurringItem>,
    pub policies: Vec<PlanningPolicy>,
    pub latest_forecasts: Vec<Forecast>,
    pub audit_cursor: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub audit_receipts: Vec<PlanningReceipt>,
}

impl Default for PmPlanningState {
    fn default() -> Self {
        Self {
            schema_version: PLANNING_SCHEMA_VERSION,
            revision: 0,
            goals: Vec::new(),
            roadmap_items: Vec::new(),
            milestones: Vec::new(),
            risks: Vec::new(),
            decisions: Vec::new(),
            recurring_items: Vec::new(),
            policies: Vec::new(),
            latest_forecasts: Vec::new(),
            audit_cursor: None,
            audit_receipts: Vec::new(),
        }
    }
}

pub fn planning_state_path_for_repo(repo: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo).join("project-state/pm-planning.json")
}

/// Read a snapshot without creating directories, acquiring a writer lock, or
/// rewriting legacy data. Atomic publication keeps snapshots complete.
pub fn load_planning_state(path: &Path) -> io::Result<PmPlanningState> {
    read_state(path, false)
}

fn read_state(path: &Path, preserve_corrupt: bool) -> io::Result<PmPlanningState> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(PmPlanningState::default());
        }
        Err(error) => return Err(error),
    };
    let mut state: PmPlanningState = serde_json::from_slice(&bytes).map_err(|error| {
        let malformed = matches!(
            error.classify(),
            serde_json::error::Category::Syntax | serde_json::error::Category::Eof
        );
        let diagnosis = if preserve_corrupt && malformed {
            match preserve_corrupt_copy(path, &bytes) {
                Ok(backup) => format!("{error}; original retained, backup: {}", backup.display()),
                Err(backup_error) => {
                    format!("{error}; original retained, backup failed: {backup_error}")
                }
            }
        } else {
            error.to_string()
        };
        io::Error::new(io::ErrorKind::InvalidData, diagnosis)
    })?;
    if state.schema_version > PLANNING_SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported PM planning schema {}", state.schema_version),
        ));
    }
    // Schema zero and unversioned documents have the same field meanings;
    // normalization becomes durable only on a successful CAS transaction.
    state.schema_version = PLANNING_SCHEMA_VERSION;
    Ok(state)
}

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn create_parent_durably(path: &Path) -> io::Result<()> {
    let parent = parent_directory(path);
    fs::create_dir_all(parent)?;
    // A synced file inside an unsynced new directory can still disappear on
    // crash. Even an existing directory may have just been created by a peer
    // that has not synced its parent yet; every writer establishes durability.
    let mut ancestors: Vec<_> = parent
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .collect();
    if parent.is_relative() && parent != Path::new(".") {
        ancestors.push(Path::new("."));
    }
    for directory in ancestors.into_iter().rev() {
        sync_directory(directory)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    // Windows cannot open a directory as a std::fs::File. The payload is
    // still synced before tempfile's atomic replacement, like PM prefs.
    Ok(())
}

fn preserve_corrupt_copy(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let prefix = format!(
        "{}.corrupt-",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("pm-planning.json")
    );
    let mut backup = tempfile::Builder::new()
        .prefix(&prefix)
        .tempfile_in(parent_directory(path))?;
    backup.write_all(bytes)?;
    backup.as_file().sync_all()?;
    let (_, retained_path) = backup.keep().map_err(|error| error.error)?;
    sync_directory(parent_directory(path))?;
    Ok(retained_path)
}

fn publish_state(path: &Path, state: &PmPlanningState) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent_directory(path))?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    gwt_core::operation_deadline::ensure_remaining("PM planning durable rename")?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_directory(parent_directory(path))
}

/// Serialize a CAS read-modify-write transaction on a stable sibling lock.
/// Stale revisions, failed mutations, unsupported data, and revision overflow
/// never publish. Schema and revision belong to the store, not the mutation.
pub fn mutate_planning_state<T>(
    path: &Path,
    expected_revision: u64,
    mutation: impl FnOnce(&mut PmPlanningState) -> io::Result<T>,
) -> io::Result<(PmPlanningState, T)> {
    with_planning_lock(path, || {
        let mut state = read_state(path, true)?;
        if state.revision != expected_revision {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "stale PM planning revision: expected {expected_revision}, current {}",
                    state.revision
                ),
            ));
        }
        let next_revision = state.revision.checked_add(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "PM planning revision overflow")
        })?;
        let value = mutation(&mut state)?;
        if state.revision != expected_revision || state.schema_version != PLANNING_SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "PM planning schema and revision are store-managed",
            ));
        }
        state.revision = next_revision;
        publish_state(path, &state)?;
        Ok((state, value))
    })
}

fn with_planning_lock<T>(path: &Path, action: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    create_parent_durably(path)?;
    let lock = NamedFileLock::acquire(&path.with_extension("lock"), "pm_planning")?;
    let result = action();
    let unlocked = lock.unlock();
    match (result, unlocked) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
    }
}

/// The caller must prove registered PM authority before invoking this store.
/// Data and replay evidence are published together under the existing lock.
pub fn apply_planning_action(
    path: &Path,
    actor: &str,
    request: &PlanningApplyRequest,
) -> io::Result<PlanningReceipt> {
    request.validate()?;
    if actor.trim().is_empty() || gwt_core::error_ledger::redact_credentials(actor) != actor {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid planning actor",
        ));
    }
    let request_digest =
        gwt_core::pm_authorization::digest(&serde_json::to_vec(request).map_err(io::Error::other)?);
    let key_digest = gwt_core::pm_authorization::digest(request.idempotency_key.as_bytes());
    with_planning_lock(path, || {
        let mut state = read_state(path, true)?;
        // Retry precedes CAS: later valid writes do not invalidate the receipt.
        if let Some(receipt) = state
            .audit_receipts
            .iter()
            .find(|receipt| receipt.idempotency_key_digest == key_digest)
        {
            if receipt.actor == actor && receipt.request_digest == request_digest {
                return Ok(receipt.clone());
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "planning idempotency key already used by a different actor or request",
            ));
        }
        if state.revision != request.expected_revision {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "stale PM planning revision: expected {}, current {}",
                    request.expected_revision, state.revision
                ),
            ));
        }
        let next_revision = state.revision.checked_add(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "PM planning revision overflow")
        })?;
        request.action.upsert(&mut state);
        let (kind, id) = request.action.target();
        let receipt = PlanningReceipt {
            actor: actor.into(),
            requester: actor.into(),
            action_kind: kind.into(),
            target_id: id.into(),
            expected_revision: request.expected_revision,
            resulting_revision: next_revision,
            reason: gwt_core::error_ledger::redact_credentials(&request.reason),
            policy_versions: state
                .policies
                .iter()
                .map(|policy| (policy.id.clone(), policy.version))
                .collect(),
            risk_class: "planning_only".into(),
            outcome: "applied".into(),
            request_digest,
            idempotency_key_digest: key_digest,
        };
        state.revision = next_revision;
        state.audit_cursor = Some(format!("plan-{next_revision}"));
        state.audit_receipts.push(receipt.clone());
        publish_state(path, &state)?;
        Ok(receipt)
    })
}
