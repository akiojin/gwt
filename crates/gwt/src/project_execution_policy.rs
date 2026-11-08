//! Host observations for Project window policy. A socket or backend context is
//! never an actual-open grant; only the GUI's accepted one-use ready receipt
//! publishes this observation. Readers pin its process birth before admission.
use chrono::{DateTime, Utc};
use gwt_core::project_execution_policy::{self as store, Choice, EffectState};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActualViewReceipt {
    pub receipt_id: String,
    pub accepted_at: DateTime<Utc>,
    pub admitted_policy_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedCloseIntent {
    pub request: store::ChoiceRequest,
    pub view_set_revision: u64,
    #[serde(default)]
    pub policy_corruption_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeObservation {
    pub schema_version: u32,
    pub project_key: String,
    pub project_root: PathBuf,
    pub generation: u64,
    pub host_pid: u32,
    pub host_started_at: u64,
    pub view_set_revision: u64,
    pub views: BTreeMap<String, ActualViewReceipt>,
    pub last_open_receipt: Option<ActualViewReceipt>,
    #[serde(default)]
    pub close_intents: BTreeMap<String, AcceptedCloseIntent>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationInvalidation {
    pub host_pid: u32,
    pub host_started_at: u64,
    pub generation: u64,
    pub view_set_revision: u64,
    pub last_open_receipt_id: Option<String>,
}

type ObservationRevocations =
    std::collections::HashMap<(String, u32, u64), ObservationInvalidation>;
fn observation_revocations() -> &'static std::sync::Mutex<ObservationRevocations> {
    static REVOKED: std::sync::OnceLock<std::sync::Mutex<ObservationRevocations>> =
        std::sync::OnceLock::new();
    REVOKED.get_or_init(Default::default)
}

/// Feature-local revocation, shared by the physical owner and its Monitor.
/// This can only remove an actual-view prerequisite, never grant one.
pub fn invalidate_observation(repo: &Path, invalidation: ObservationInvalidation) {
    let key = (
        store::project_key(repo),
        invalidation.host_pid,
        invalidation.host_started_at,
    );
    let mut revoked = observation_revocations()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if revoked.get(&key).is_none_or(|previous| {
        (previous.generation, previous.view_set_revision)
            <= (invalidation.generation, invalidation.view_set_revision)
    }) {
        revoked.insert(key, invalidation);
    }
}

pub(crate) fn observation_is_revoked(observed: &RuntimeObservation) -> bool {
    observation_revocations()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&(
            observed.project_key.clone(),
            observed.host_pid,
            observed.host_started_at,
        ))
        .is_some_and(|revoked| {
            observed.generation < revoked.generation
                || (observed.generation == revoked.generation
                    && observed.view_set_revision <= revoked.view_set_revision)
                || observed
                    .last_open_receipt
                    .as_ref()
                    .map(|receipt| &receipt.receipt_id)
                    == revoked.last_open_receipt_id.as_ref()
        })
}

pub fn observation_path(repo: &Path) -> PathBuf {
    store::policy_path(repo).with_file_name("project-window-runtime.json")
}

fn observation_lock_is_contended(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

pub(crate) fn lock_observation(repo: &Path) -> io::Result<std::fs::File> {
    use fs2::FileExt;
    let path = observation_path(repo);
    std::fs::create_dir_all(path.parent().expect("observation parent"))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(error) if observation_lock_is_contended(&error) => {
                if std::time::Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Project observation is busy; retry after reconciliation",
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(lock)
}

pub fn write_observation(repo: &Path, observation: &RuntimeObservation) -> io::Result<()> {
    // Never unlink without the lock: a competing writer may already have
    // published a newer view set. Readers take this same bounded lock, so a
    // missing/unreadable lock also withholds an old disk observation.
    let lock = lock_observation(repo);
    if lock.is_ok() {
        let existing = std::fs::read(observation_path(repo))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RuntimeObservation>(&bytes).ok());
        if existing.is_some_and(|existing| {
            existing.schema_version == 1
                && existing.project_key == observation.project_key
                && (existing.host_pid, existing.host_started_at)
                    != (observation.host_pid, observation.host_started_at)
                && !existing.views.is_empty()
                && crate::process::is_process_alive(existing.host_pid)
                && crate::process::host_process_start_time(existing.host_pid)
                    .is_none_or(|birth| birth == existing.host_started_at)
        }) {
            // This host has not written anything. In particular, do not enter
            // the failed-write cleanup and remove another host's live views.
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Another live GUI host owns this Project view; use that instance",
            ));
        }
    }
    let result = (|| {
        let _lock = lock
            .as_ref()
            .map_err(|error| io::Error::new(error.kind(), error.to_string()))?;
        gwt_core::atomic_file::write_atomic(
            &observation_path(repo),
            &serde_json::to_vec(observation)?,
        )
    })();
    if result.is_err() {
        invalidate_observation(
            repo,
            ObservationInvalidation {
                host_pid: observation.host_pid,
                host_started_at: observation.host_started_at,
                generation: observation.generation,
                view_set_revision: observation.view_set_revision,
                last_open_receipt_id: observation
                    .last_open_receipt
                    .as_ref()
                    .map(|receipt| receipt.receipt_id.clone()),
            },
        );
        if lock.is_ok() {
            if let Err(error) = std::fs::remove_file(observation_path(repo)) {
                if error.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(%error,"stale Project observation could not be removed; its in-process grant is revoked");
                }
            }
        }
    }
    result
}

/// A dead or reused GUI process cannot leave behind an actual-open grant.
pub fn observation(repo: &Path) -> Result<Option<RuntimeObservation>, String> {
    if !observation_path(repo)
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Ok(None);
    }
    let _lock = lock_observation(repo).map_err(|error| error.to_string())?;
    observation_unlocked(repo)
}

/// Caller must hold `lock_observation` through the read and any publication.
pub(crate) fn observation_unlocked(repo: &Path) -> Result<Option<RuntimeObservation>, String> {
    let bytes = match std::fs::read(observation_path(repo)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let value: RuntimeObservation =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let key = store::project_key(repo);
    if value.schema_version != 1 || value.project_key != key {
        return Err("Project observation identity or schema mismatch".into());
    }
    if !crate::process::is_process_alive(value.host_pid)
        || crate::process::host_process_start_time(value.host_pid) != Some(value.host_started_at)
    {
        return Ok(None);
    }
    Ok(Some(value))
}

/// Revoke only this host's saved generation when a fresh birth lookup fails.
/// A newer writer or another host must retain its observation.
pub fn revoke_saved_observation(
    repo: &Path,
    host_pid: u32,
    generation: u64,
    view_set_revision: u64,
) -> io::Result<Option<ObservationInvalidation>> {
    let _lock = lock_observation(repo)?;
    let bytes = match std::fs::read(observation_path(repo)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let observed: RuntimeObservation = serde_json::from_slice(&bytes)?;
    if observed.schema_version != 1
        || observed.project_key != store::project_key(repo)
        || observed.host_pid != host_pid
        || observed.generation != generation
        || observed.view_set_revision > view_set_revision
    {
        return Ok(None);
    }
    let invalidation = ObservationInvalidation {
        host_pid,
        host_started_at: observed.host_started_at,
        generation,
        view_set_revision,
        last_open_receipt_id: observed.last_open_receipt.map(|receipt| receipt.receipt_id),
    };
    invalidate_observation(repo, invalidation.clone());
    match std::fs::remove_file(observation_path(repo)) {
        Ok(()) => Ok(Some(invalidation)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Some(invalidation)),
        Err(error) => Err(error),
    }
}

pub fn actual_open(repo: &Path) -> Result<bool, String> {
    let policy = store::load(repo).map_err(|error| error.to_string())?;
    let Some(observed) = observation(repo)? else {
        return Ok(false);
    };
    Ok(observation_is_open(&policy, &observed))
}

fn observation_is_open(policy: &store::Policy, observed: &RuntimeObservation) -> bool {
    if observation_is_revoked(observed) {
        return false;
    }
    observed.views.values().any(|receipt| {
        policy.choice != Some(Choice::Stop)
            || (policy
                .last_operation()
                .is_some_and(|operation| operation.effect_state == EffectState::Confirmed)
                && receipt.admitted_policy_revision == policy.revision)
    })
}

pub fn session_lineage(repo: &Path, session: &gwt_agent::Session) -> Vec<String> {
    let mut lineage = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut current = Some(session.clone());
    let key = store::project_key(repo);
    while let Some(session) = current {
        let state_root = session
            .project_state_root
            .as_deref()
            .unwrap_or(&session.worktree_path);
        if store::project_key(state_root) != key || !seen.insert(session.id.clone()) {
            break;
        }
        lineage.push(format!("session:{}", session.id));
        lineage.extend(
            session
                .linked_issue_number
                .map(|issue| format!("issue:{issue}")),
        );
        current = session
            .restore_source_session_id
            .as_ref()
            .and_then(|source| {
                gwt_agent::Session::load(
                    &gwt_core::paths::gwt_sessions_dir().join(format!("{source}.toml")),
                )
                .ok()
            });
    }
    if let Ok(Some(works)) = gwt_core::workspace_projection::load_workspace_work_items(repo) {
        for work in works.work_items {
            if work
                .agents
                .iter()
                .any(|agent| seen.contains(&agent.session_id))
            {
                lineage.push(format!("work:{}", work.id));
            }
        }
    }
    lineage.sort();
    lineage.dedup();
    lineage
}

/// Derive PM lineage from this project's canonical worktree and saved profile.
/// A caller-supplied role flag never supplies the Project prerequisite alone.
pub fn agent_lineage(repo: &Path, config: &gwt_agent::LaunchConfig) -> Vec<String> {
    let mut lineage = config
        .linked_issue_number
        .map(|issue| format!("issue:{issue}"))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(source) = config.predecessor_session_id.as_ref() {
        if let Ok(session) = gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir().join(format!("{source}.toml")),
        ) {
            lineage.extend(session_lineage(repo, &session));
        }
    }
    let canonical_pm = crate::pm_registry::pm_worktree_path_for_repo_path(repo);
    let is_same_pm_worktree = config.working_dir.as_deref().is_some_and(|worktree| {
        dunce::canonicalize(worktree)
            .ok()
            .zip(dunce::canonicalize(&canonical_pm).ok())
            .is_some_and(|(actual, expected)| actual == expected)
            && crate::pm_registry::is_canonical_pm_worktree(worktree)
    });
    if config.linked_issue_number.is_none()
        && config.suppress_execution_control
        && is_same_pm_worktree
        && config.runtime_target == gwt_agent::LaunchRuntimeTarget::Host
    {
        if let Ok(prefs) = crate::pm_registry::load_pm_prefs(
            &crate::pm_registry::pm_prefs_path_for_repo_path(repo),
        ) {
            let profile = prefs.settings.launch_profile_or_default();
            if gwt_agent::resolve_agent_id(&profile.agent_id).as_ref() == Some(&config.agent_id)
                && config.model == profile.model
                && config.reasoning_level == profile.reasoning
            {
                lineage.push("pm".into());
            }
        }
    }
    lineage
}

/// This predicate only supplies the Project policy prerequisite. Existing
/// singleton, claim, execution, provider and permission gates still run.
pub fn admission(repo: &Path, lineage: &[String]) -> Result<(), String> {
    if actual_open(repo).unwrap_or(false) {
        return Ok(());
    }
    admission_without_actual_view(repo, lineage)
}

/// Restricts admission after a failed observation; this never creates an open grant.
pub fn admission_without_actual_view(repo: &Path, lineage: &[String]) -> Result<(), String> {
    let policy = store::load(repo)
        .map_err(|error| format!("policy_unreadable: {error}; use project.execution.status"))?;
    if policy.choice == Some(Choice::Continue) {
        if lineage
            .iter()
            .any(|item| policy.authorized_work_lineage.contains(item))
        {
            return Ok(());
        }
        let derived = load_derived_owners(repo)?;
        let instructions =
            gwt_core::user_instruction::load(repo).map_err(|error| error.to_string())?;
        if lineage.iter().any(|item| {
            policy.authorized_work_lineage.contains(item)
                || item
                    .strip_prefix("work:")
                    .is_some_and(|work| work_has_continued_lineage(repo, work, &policy))
                || item
                    .strip_prefix("issue:")
                    .and_then(|number| number.parse::<u64>().ok())
                    .is_some_and(|issue| {
                        issue_has_continued_lineage(
                            issue,
                            &policy,
                            &derived,
                            &instructions,
                            &mut std::collections::BTreeSet::new(),
                        )
                    })
        }) {
            return Ok(());
        }
    }
    Err(match policy.choice {
        Some(Choice::Stop) => "user_stop: Project execution is stopped; open the Project or use project.execution.choose",
        Some(Choice::Continue) => "lineage_not_admitted: this owner was not included in the Continue choice; use project.execution.status",
        None => "selection_pending: render the Project view or resolve its choice with project.execution.choose",
    }.into())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DerivedOwnerReceipt {
    child_issue: u64,
    parent_issue: u64,
    issuer_session_id: String,
    source_creation_receipt: String,
}

fn derived_owners_path(repo: &Path) -> PathBuf {
    store::policy_path(repo).with_file_name("project-window-derived-owners.json")
}

fn load_derived_owners(repo: &Path) -> Result<Vec<DerivedOwnerReceipt>, String> {
    match std::fs::read(derived_owners_path(repo)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.to_string()),
    }
}

fn issue_has_continued_lineage(
    issue: u64,
    policy: &store::Policy,
    derived: &[DerivedOwnerReceipt],
    instructions: &[gwt_core::user_instruction::UserInstruction],
    visited: &mut std::collections::BTreeSet<u64>,
) -> bool {
    if !visited.insert(issue) {
        return false;
    }
    if policy
        .authorized_work_lineage
        .contains(&format!("issue:{issue}"))
    {
        return true;
    }
    if instructions.iter().any(|instruction| {
        instruction.authority.is_some()
            && instruction.superseded_by.is_none()
            && instruction.kind != gwt_core::user_instruction::UserInstructionKind::Question
            && instruction.issue_numbers.contains(&issue)
    }) {
        return true;
    }
    derived
        .iter()
        .filter(|receipt| {
            receipt.child_issue == issue
                && receipt.child_issue != receipt.parent_issue
                && !receipt.issuer_session_id.is_empty()
                && receipt
                    .source_creation_receipt
                    .starts_with(&format!("issue.create:{issue}:"))
        })
        .any(|receipt| {
            issue_has_continued_lineage(
                receipt.parent_issue,
                policy,
                derived,
                instructions,
                visited,
            )
        })
}

/// Called only after the existing PM owner-check and canonical Issue create
/// have succeeded. This internal receipt, never reporter-authored Issue text,
/// binds a necessary child to the continued approved parent.
pub fn record_derived_owner(
    repo: &Path,
    child_issue: u64,
    parent_issue: u64,
    issuer_session_id: &str,
    source_creation_receipt: &str,
) -> Result<(), String> {
    use fs2::FileExt;
    if child_issue == 0
        || parent_issue == 0
        || child_issue == parent_issue
        || issuer_session_id.is_empty()
        || !source_creation_receipt.starts_with(&format!("issue.create:{child_issue}:"))
    {
        return Err("invalid canonical derived-owner receipt".into());
    }
    let policy = store::load(repo).map_err(|error| error.to_string())?;
    if policy.choice != Some(Choice::Continue) {
        return Ok(());
    }
    admission(repo, &[format!("issue:{parent_issue}")])?;
    let registered =
        crate::pm_registry::load_pm_prefs(&crate::pm_registry::pm_prefs_path_for_repo_path(repo))
            .ok()
            .and_then(|prefs| prefs.registration)
            .is_some_and(|registration| registration.session_id == issuer_session_id)
            || crate::pm_registry::pm_repository_key(repo).is_some_and(|key| {
                crate::pm_registry::pm_registrations_for_repository(&key)
                    .iter()
                    .any(|registration| registration.registration.session_id == issuer_session_id)
            });
    if !registered {
        return Err("derived owner requires the registered PM".into());
    }
    let path = derived_owners_path(repo);
    std::fs::create_dir_all(path.parent().expect("derived-owner parent"))
        .map_err(|error| error.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|error| error.to_string())?;
    lock.lock_exclusive().map_err(|error| error.to_string())?;
    let mut receipts = load_derived_owners(repo)?;
    let receipt = DerivedOwnerReceipt {
        child_issue,
        parent_issue,
        issuer_session_id: issuer_session_id.into(),
        source_creation_receipt: source_creation_receipt.into(),
    };
    if let Some(existing) = receipts
        .iter()
        .find(|existing| existing.child_issue == child_issue)
    {
        return if existing == &receipt {
            Ok(())
        } else {
            Err("child already has a different canonical lineage receipt".into())
        };
    }
    receipts.push(receipt);
    gwt_core::atomic_file::write_atomic(
        &path,
        &serde_json::to_vec(&receipts).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn work_has_continued_lineage(repo: &Path, work_id: &str, policy: &store::Policy) -> bool {
    let Ok(Some(works)) = gwt_core::workspace_projection::load_workspace_work_items(repo) else {
        return false;
    };
    let mut pending = vec![work_id];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if policy
            .authorized_work_lineage
            .contains(&format!("work:{id}"))
        {
            return true;
        }
        let Some(work) = works.work_items.iter().find(|work| work.id == id) else {
            continue;
        };
        if work.agents.iter().any(|agent| {
            policy
                .authorized_work_lineage
                .contains(&format!("session:{}", agent.session_id))
        }) {
            return true;
        }
        pending.extend(
            work.events
                .iter()
                .filter(|event| event.is_successor_lineage_anchor())
                .filter_map(|event| event.related_work_item_id.as_deref()),
        );
    }
    false
}

/// Monitor queue intent is durable unfinished work even between PTY generations.
pub fn has_queued_continue_work(repo: &Path) -> bool {
    let Ok(policy) = store::load(repo) else {
        return false;
    };
    if policy.choice != Some(Choice::Continue) {
        return false;
    }
    crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(repo))
        .ok()
        .is_some_and(|prefs| {
            let monitor =
                crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
            monitor
                .local_terminal_queue_numbers()
                .into_iter()
                .chain(monitor.active_issue_numbers())
                .any(|issue| admission(repo, &[format!("issue:{issue}")]).is_ok())
        })
}

pub fn session_work_is_unfinished(session: &gwt_agent::Session) -> bool {
    match crate::cli::execution_state::load(&session.worktree_path) {
        Ok(Some(record)) if crate::cli::execution_state::integrity_ok(&record) => {
            record.status == crate::cli::execution_state::ExecutionControlStatus::Active
        }
        _ => true,
    }
}

pub fn policy_corruption_digest(repo: &Path) -> Option<String> {
    if store::load(repo).is_ok() {
        return None;
    }
    std::fs::read(store::policy_path(repo))
        .ok()
        .map(|bytes| gwt_core::pm_authorization::digest(&bytes))
}

pub fn local_status(
    repo: &Path,
    operation_id: Option<&str>,
) -> crate::protocol::ProjectExecutionStatusView {
    let observed = observation(repo);
    let mut observation_error = observed.as_ref().err().cloned();
    let observed = observed.ok().flatten();
    if observed.as_ref().is_some_and(observation_is_revoked) {
        observation_error = Some("runtime_observation_revoked".into());
    }
    let policy = store::load(repo);
    let policy_error = policy.as_ref().err().map(ToString::to_string);
    let corruption_digest = policy_error
        .as_ref()
        .and_then(|_| policy_corruption_digest(repo));
    let policy = policy.ok();
    let operation = policy
        .as_ref()
        .and_then(|policy| match operation_id {
            Some(id) => policy.operation(id),
            None => policy.last_operation(),
        })
        .cloned();
    // One status projection reuses its freshly read policy and observation.
    let open = policy
        .as_ref()
        .zip(observed.as_ref())
        .is_some_and(|(policy, observed)| observation_is_open(policy, observed));
    let choice = policy.as_ref().and_then(|value| value.choice);
    let latest_operation = policy.as_ref().and_then(|policy| policy.last_operation());
    let (state, reason) = if let Some(error) = policy_error {
        {
            tracing::warn!(%error, "Project policy cannot be read");
            ("unknown", "policy_unreadable".into())
        }
    } else if let Some(error) = observation_error {
        {
            tracing::warn!(%error, "Project runtime observation cannot be read");
            ("unknown", "runtime_observation_unreadable".into())
        }
    } else if choice == Some(Choice::Stop)
        && !open
        && latest_operation.is_some_and(|receipt| receipt.effect_state == EffectState::Confirmed)
        && observed
            .as_ref()
            .and_then(|observation| observation.last_open_receipt.as_ref())
            .is_some_and(|receipt| {
                policy
                    .as_ref()
                    .is_some_and(|policy| receipt.admitted_policy_revision == policy.revision)
            })
    {
        ("selection_pending", "view_disconnected".into())
    } else if choice == Some(Choice::Stop) && !open {
        match latest_operation.map(|receipt| receipt.effect_state) {
            Some(EffectState::Confirmed) => ("stopped", "user_stop".into()),
            Some(EffectState::Pending) => ("stopping", "drain_pending".into()),
            _ => ("unknown", "drain_unconfirmed".into()),
        }
    } else if open {
        ("waiting", "window_open".into())
    } else if choice == Some(Choice::Continue) {
        ("unknown", "runtime_observation_unavailable".into())
    } else {
        ("selection_pending", "actual_view_required".into())
    };
    crate::protocol::ProjectExecutionStatusView {
        project_key: policy.as_ref().map_or_else(
            || store::project_key(repo),
            |policy| policy.project_key.clone(),
        ),
        generation: observed.as_ref().map_or(0, |value| value.generation),
        revision: policy.as_ref().map_or(0, |value| value.revision),
        choice,
        effective_state: state.into(),
        start_reason: reason,
        view_state: if open {
            "open"
        } else if observed.is_some() {
            "selection_pending"
        } else {
            "unknown"
        }
        .into(),
        lineage_summary: policy
            .as_ref()
            .map_or_else(Vec::new, |value| value.authorized_work_lineage.clone()),
        operation_outcome: operation.as_ref().map(Into::into),
        observed_at: Utc::now().to_rfc3339(),
        degraded: state == "unknown",
        recovery_operation: (state == "unknown" || state == "selection_pending")
            .then(|| "project.execution.choose".into()),
        view_set_revision: observed.as_ref().map_or(0, |value| value.view_set_revision),
        policy_corruption_digest: corruption_digest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedGwtHome;

    #[test]
    fn issue6682_policy_observation_platform_contention_is_retryable() {
        assert!(observation_lock_is_contended(&fs2::lock_contended_error()));
        assert!(observation_lock_is_contended(&io::Error::from(
            io::ErrorKind::WouldBlock
        )));
        assert!(!observation_lock_is_contended(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        #[cfg(windows)]
        assert!(observation_lock_is_contended(
            &io::Error::from_raw_os_error(33)
        ));
    }

    fn continued(repo: &Path, lineage: Vec<String>) {
        let receipt = store::choose(
            repo,
            &store::ChoiceRequest {
                operation_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 0,
                generation: 1,
                choice: Choice::Continue,
                consent_source_id: "fixture-consent".into(),
                actor: "browser:fixture".into(),
                authorized_work_lineage: lineage,
            },
        )
        .unwrap();
        store::record_effect(repo, &receipt.operation_id, EffectState::Confirmed, None).unwrap();
    }

    #[test]
    fn issue6682_policy_continue_admits_derived_child_and_rejects_another_owner_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        continued(&repo, vec!["issue:100".into()]);
        let path = gwt_core::paths::gwt_project_dir_for_repo_path(&repo)
            .join("project-state/project-window-derived-owners.json");
        std::fs::write(
            path,
            serde_json::to_vec(&serde_json::json!([{
                "child_issue":101,"parent_issue":100,"issuer_session_id":"registered-pm",
                "source_creation_receipt":"issue.create:101:2026-11-03T00:00:00Z"
            }]))
            .unwrap(),
        )
        .unwrap();
        assert!(
            admission(&repo, &["issue:101".into()]).is_ok(),
            "a required typed child keeps its admitted parent lineage after restart"
        );
        assert!(
            admission(&repo, &["issue:102".into()]).is_err(),
            "another owner is not covered by the Continue choice"
        );
        std::fs::write(derived_owners_path(&repo), b"unreadable lineage").unwrap();
        assert!(
            admission(&repo, &["issue:100".into()]).is_ok(),
            "direct Continue membership does not depend on auxiliary receipts"
        );
        assert!(admission(&repo, &["issue:101".into()]).is_err());
    }

    #[test]
    fn issue6682_policy_continue_admits_later_captured_human_request_only() {
        use gwt_core::user_instruction::{self, UserInstructionInput, UserInstructionKind};
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        continued(&repo, vec!["issue:100".into()]);
        user_instruction::record_from_prompt(
            &repo,
            UserInstructionInput {
                original: "Please implement Issue 103".into(),
                summary: "Issue 103".into(),
                answer: "Implement the scoped issue".into(),
                actions: vec!["implement".into()],
                issue_numbers: vec![103],
                kind: UserInstructionKind::OneOff,
                supersedes: Vec::new(),
                authorizations: Vec::new(),
            },
            "registered-pm",
            "human-turn",
        )
        .unwrap();
        user_instruction::record(
            &repo,
            UserInstructionInput {
                original: "Agent suggestion for Issue 104".into(),
                summary: "Issue 104".into(),
                answer: "Unadopted suggestion".into(),
                actions: vec!["implement".into()],
                issue_numbers: vec![104],
                kind: UserInstructionKind::OneOff,
                supersedes: Vec::new(),
                authorizations: Vec::new(),
            },
        )
        .unwrap();
        assert!(admission(&repo, &["issue:103".into()]).is_ok());
        assert!(admission(&repo, &["issue:104".into()]).is_err());
    }

    #[test]
    fn issue6682_policy_old_operation_lookup_does_not_change_current_stop_state() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        continued(&repo, vec!["issue:100".into()]);
        let old = store::load(&repo)
            .unwrap()
            .last_operation()
            .unwrap()
            .operation_id
            .clone();
        store::choose(
            &repo,
            &store::ChoiceRequest {
                operation_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: 1,
                generation: 1,
                choice: Choice::Stop,
                consent_source_id: "stop-consent".into(),
                actor: "browser:fixture".into(),
                authorized_work_lineage: Vec::new(),
            },
        )
        .unwrap();
        let status = local_status(&repo, Some(&old));
        assert_eq!(status.effective_state, "stopping");
        assert_eq!(status.operation_outcome.unwrap().choice, Choice::Continue);
    }
    fn accepted_observation(repo: &Path) -> RuntimeObservation {
        let receipt = ActualViewReceipt {
            receipt_id: uuid::Uuid::new_v4().to_string(),
            accepted_at: Utc::now(),
            admitted_policy_revision: 0,
        };
        RuntimeObservation {
            schema_version: 1,
            project_key: gwt_core::paths::resolve_project_scope(repo)
                .hash
                .to_string(),
            project_root: repo.into(),
            generation: 1,
            host_pid: std::process::id(),
            host_started_at: crate::process::host_process_start_time(std::process::id()).unwrap(),
            view_set_revision: 1,
            views: BTreeMap::from([("actual-view".into(), receipt.clone())]),
            last_open_receipt: Some(receipt),
            close_intents: BTreeMap::new(),
            observed_at: Utc::now(),
        }
    }

    #[test]
    fn issue6682_policy_observation_lock_failure_preserves_disk_but_denies_admission() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let observed = accepted_observation(&repo);
        write_observation(&repo, &observed).unwrap();
        let path = observation_path(&repo);
        let bytes = std::fs::read(&path).unwrap();
        let lock_path = path.with_extension("lock");
        std::fs::remove_file(&lock_path).unwrap();
        std::fs::create_dir(&lock_path).unwrap();
        assert!(write_observation(&repo, &observed).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(observation(&repo).is_err());
        assert!(admission(&repo, &["issue:999".into()]).is_err());
    }

    #[test]
    fn issue6682_policy_second_host_cannot_replace_live_views() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let observed = accepted_observation(&repo);
        write_observation(&repo, &observed).unwrap();
        let mut competing = observed.clone();
        competing.host_pid = u32::MAX;
        competing.views.clear();
        assert!(write_observation(&repo, &competing).is_err());
        assert_eq!(
            observation(&repo).unwrap().unwrap().host_pid,
            observed.host_pid
        );
        assert!(actual_open(&repo).unwrap());
    }

    #[test]
    fn issue6682_policy_observation_revocation_is_shared_but_preserves_valid_continue() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut observed = accepted_observation(&repo);
        write_observation(&repo, &observed).unwrap();
        assert!(admission(&repo, &["issue:999".into()]).is_ok());
        invalidate_observation(
            &repo,
            ObservationInvalidation {
                host_pid: observed.host_pid,
                host_started_at: observed.host_started_at,
                generation: 1,
                view_set_revision: 1,
                last_open_receipt_id: observed
                    .last_open_receipt
                    .as_ref()
                    .map(|receipt| receipt.receipt_id.clone()),
            },
        );
        assert!(
            observation_path(&repo).exists(),
            "even an unremovable stale file cannot retain its grant"
        );
        assert!(admission(&repo, &["issue:999".into()]).is_err());
        continued(&repo, vec!["issue:100".into()]);
        assert!(admission(&repo, &["issue:100".into()]).is_ok());
        assert!(admission(&repo, &["issue:999".into()]).is_err());
        observed.view_set_revision = 2;
        let receipt = ActualViewReceipt {
            receipt_id: uuid::Uuid::new_v4().to_string(),
            accepted_at: Utc::now(),
            admitted_policy_revision: 1,
        };
        observed.views = BTreeMap::from([("fresh-view".into(), receipt.clone())]);
        observed.last_open_receipt = Some(receipt);
        write_observation(&repo, &observed).unwrap();
        assert!(
            admission(&repo, &["issue:999".into()]).is_ok(),
            "only a new accepted receipt restores normal open admission"
        );
        assert!(revoke_saved_observation(&repo, observed.host_pid, 1, 1)
            .unwrap()
            .is_none());
        assert!(actual_open(&repo).unwrap(), "a newer view set is retained");
        let revoked = revoke_saved_observation(&repo, observed.host_pid, 1, 2)
            .unwrap()
            .unwrap();
        assert_eq!(revoked.host_started_at, observed.host_started_at);
        assert!(observation(&repo).unwrap().is_none());
        assert!(admission(&repo, &["issue:999".into()]).is_err());
        assert!(admission(&repo, &["issue:100".into()]).is_ok());
    }

    #[test]
    fn issue6682_policy_pm_lineage_requires_this_canonical_worktree_and_profile() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let worktree = crate::pm_registry::pm_worktree_path_for_repo_path(&repo);
        std::fs::create_dir_all(&worktree).unwrap();
        let profile = crate::pm_registry::PmLaunchProfile::default_profile();
        let agent = gwt_agent::resolve_agent_id(&profile.agent_id).unwrap();
        let mut config = gwt_agent::AgentLaunchBuilder::new(agent)
            .working_dir(worktree)
            .build();
        config.suppress_execution_control = true;
        continued(&repo, vec!["pm".into()]);
        assert!(
            admission(&repo, &agent_lineage(&repo, &config)).is_ok(),
            "a Continued PM can restart through the final owner gate without a view"
        );
        config.working_dir = Some(repo.clone());
        assert!(
            admission(&repo, &agent_lineage(&repo, &config)).is_err(),
            "a caller role flag alone does not impersonate the PM"
        );
    }

    #[test]
    fn issue6682_policy_manual_restore_and_work_successor_keep_their_original_lineage() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let source = gwt_agent::Session::new(
            &repo,
            "manual",
            gwt_agent::AgentId::Custom("fixture".into()),
        );
        source.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        continued(
            &repo,
            vec![format!("session:{}", source.id), "work:parent".into()],
        );
        let mut restored = gwt_agent::Session::new(&repo, "manual", source.agent_id.clone());
        restored.restore_source_session_id = Some(source.id.clone());
        assert!(admission_without_actual_view(&repo, &session_lineage(&repo, &restored)).is_ok());
        assert!(session_work_is_unfinished(&restored));
        let successor = gwt_core::workspace_projection::successor_work_id("parent");
        assert!(
            admission(&repo, &[format!("work:{successor}")]).is_err(),
            "a guessed name has no durable structural proof"
        );
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            &successor,
            Utc::now(),
        );
        event.related_work_item_id = Some("parent".into());
        gwt_core::workspace_projection::record_workspace_work_event(&repo, event).unwrap();
        assert!(admission(&repo, &[format!("work:{successor}")]).is_ok());
        let next = gwt_core::workspace_projection::successor_work_id(&successor);
        let mut event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            &next,
            Utc::now(),
        );
        event.related_work_item_id = Some(successor);
        gwt_core::workspace_projection::record_workspace_work_event(&repo, event).unwrap();
        assert!(admission(&repo, &[format!("work:{next}")]).is_ok());
        let foreign = temp.path().join("foreign");
        std::fs::create_dir_all(&foreign).unwrap();
        restored.project_state_root = None;
        restored.worktree_path = foreign;
        assert!(admission_without_actual_view(&repo, &session_lineage(&repo, &restored)).is_err());
    }
    #[test]
    fn issue6682_policy_reopened_view_disconnect_does_not_reuse_an_old_stop_choice() {
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let request = store::ChoiceRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            expected_revision: 0,
            generation: 1,
            choice: Choice::Stop,
            consent_source_id: "explicit-stop".into(),
            actor: "browser:fixture".into(),
            authorized_work_lineage: Vec::new(),
        };
        store::choose(&repo, &request).unwrap();
        store::record_effect(&repo, &request.operation_id, EffectState::Confirmed, None).unwrap();
        assert_eq!(local_status(&repo, None).effective_state, "stopped");
        let mut observed = accepted_observation(&repo);
        let mut receipt = observed.last_open_receipt.take().unwrap();
        receipt.admitted_policy_revision = 1;
        observed.last_open_receipt = Some(receipt.clone());
        observed.views = BTreeMap::from([("fresh-after-stop".into(), receipt)]);
        write_observation(&repo, &observed).unwrap();
        assert!(actual_open(&repo).unwrap());
        observed.views.clear();
        observed.view_set_revision += 1;
        write_observation(&repo, &observed).unwrap();
        assert_eq!(
            local_status(&repo, None).effective_state,
            "selection_pending",
            "a raw detach after a new open is not a second Stop consent"
        );
        assert_eq!(
            store::load(&repo).unwrap().revision,
            1,
            "observation never rewrites durable user choice"
        );
    }
}
