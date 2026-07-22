//! Persistence and history maintenance for the workspace projection family:
//! load/save of `current.json` / `journal.jsonl` / `work_items.json` /
//! `events.jsonl`, legacy-path migration, work-event recording and backfill
//! reconciliation, rebuild-from-events, legacy synthesis, and the
//! stale-detection / classify / prune pipeline.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    coordination::{BoardEntry, BoardEntryKind},
    error::{GwtError, JsonDecodeKind, Result},
    paths::{
        gwt_project_dir_for_repo_path, gwt_repo_local_work_events_path,
        gwt_workspace_journal_path_for_repo_path, gwt_workspace_projection_path_for_repo_path,
        gwt_workspace_work_events_closed_path_for_repo_path,
        gwt_workspace_work_events_path_for_repo_path, gwt_workspace_work_items_path_for_repo_path,
        resolve_current_worktree_root,
    },
};

use super::identity::{canonical_work_branch_identity, canonical_worktree_identity};
use super::work_items::non_empty_clone;
use super::*;

/// SPEC-2359 US-41 (FR-151): why a saved Workspace projection is treated as
/// stale by [`workspace_projection_stale_reason`]. `WorktreeMissing` and
/// `PrClosed` come from inspecting `git_details` / `linked_prs`;
/// `TimeThreshold` comes from `updated_at` exceeding
/// [`WorkspaceRetentionConfig::archive_after_days`]; `Compound` is returned
/// when two or more of those conditions hold simultaneously, so callers can
/// surface the strongest evidence without losing information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaleReason {
    WorktreeMissing,
    PrClosed,
    TimeThreshold,
    Compound,
    EmptyProjection,
}

impl StaleReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorktreeMissing => "worktree_missing",
            Self::PrClosed => "pr_closed",
            Self::TimeThreshold => "time_threshold",
            Self::Compound => "compound",
            Self::EmptyProjection => "empty_projection",
        }
    }
}

/// SPEC-2359 US-41 (FR-152): retention thresholds for the Workspace
/// projection pruner. `archive_after_days` triggers the `Active → Archived`
/// transition; `delete_after_archive_days` triggers the
/// `Archived → physical delete` transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRetentionConfig {
    pub archive_after_days: u32,
    pub delete_after_archive_days: u32,
}

impl Default for WorkspaceRetentionConfig {
    fn default() -> Self {
        Self {
            archive_after_days: 30,
            delete_after_archive_days: 60,
        }
    }
}

fn legacy_workspace_projection_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_project_dir_for_repo_path(repo_path).join("workspace/current.json")
}

fn legacy_workspace_journal_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_project_dir_for_repo_path(repo_path).join("workspace/journal.jsonl")
}

fn legacy_workspace_work_items_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_project_dir_for_repo_path(repo_path).join("workspace/work_items.json")
}

fn legacy_workspace_work_events_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_project_dir_for_repo_path(repo_path).join("workspace/work_events.jsonl")
}

fn copy_legacy_workspace_file_if_needed(legacy_path: &Path, canonical_path: &Path) -> Result<()> {
    if canonical_path.exists() || legacy_path == canonical_path || !legacy_path.is_file() {
        return Ok(());
    }
    if let Some(parent) = canonical_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = fs::read(legacy_path)?;
    let file_name = canonical_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("legacy-workspace-file");
    let temp_path = canonical_path.with_file_name(format!(
        ".{file_name}.migration-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    {
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    match fs::hard_link(&temp_path, canonical_path) {
        Ok(()) => {
            fs::remove_file(&temp_path)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(&temp_path)?;
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            Err(error.into())
        }
    }
}

/// SPEC-2359 Phase W-12 Slice 5b (FR-355): the gitattributes line that joins
/// the append-only Work event log across branch divergence via git's union
/// merge driver. The glob matches the repo-local event log regardless of the
/// directory it is checked out under.
const WORK_EVENTS_GITATTRIBUTES_LINE: &str = "**/.gwt/work/events.jsonl merge=union";

/// SPEC-2359 Phase W-12 Slice 5b (FR-353/FR-358): resolve the repo-local Work
/// event log path (`<repo_root>/.gwt/work/events.jsonl`), running the one-time
/// migration from the home (untracked) sources and ensuring the union-merge
/// gitattribute exists. Returns the repo-local path so every record/read entry
/// point shares one resolution + migration.
///
/// Migration (FR-358) is guarded by existence: when the repo-local event log
/// is absent, the home Project State event log (and its older Workspace path)
/// are copied into it exactly once. Once the repo-local file exists, the home
/// sources are never read again. The copy is idempotent across restarts and
/// across linked worktrees because they all resolve to the same main worktree
/// root.
fn repo_local_work_events_path_with_migration(repo_path: &Path) -> Result<PathBuf> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    with_workspace_work_items_lock(&work_items_path, || {
        repo_local_work_events_path_with_migration_locked(repo_path)
    })
}

fn repo_local_work_events_path_with_migration_locked(repo_path: &Path) -> Result<PathBuf> {
    let events_path = gwt_repo_local_work_events_path(repo_path);
    if !events_path.exists() {
        // Primary migration source: the home Project State event log.
        copy_legacy_workspace_file_if_needed(
            &gwt_workspace_work_events_path_for_repo_path(repo_path),
            &events_path,
        )?;
        // Fallback migration source: the older home Workspace event log. Only
        // consulted when neither the repo-local nor the Project State file
        // exists, so the most recent home log always wins.
        copy_legacy_workspace_file_if_needed(
            &legacy_workspace_work_events_path_for_repo_path(repo_path),
            &events_path,
        )?;
    }
    ensure_work_events_gitattributes(repo_path)?;
    Ok(events_path)
}

/// SPEC-2359 Phase W-12 Slice 5b (FR-355): ensure the repo's `.gitattributes`
/// carries the union-merge line for the repo-local Work event log. The entry
/// is appended at most once (idempotent): an existing line — regardless of
/// surrounding whitespace — is left untouched. The file is created when
/// absent. Failures to write are swallowed for non-repository / read-only
/// roots so recording a Work event never fails on the gitattributes side.
fn ensure_work_events_gitattributes(repo_path: &Path) -> Result<()> {
    let root = resolve_current_worktree_root(repo_path);
    // Defensive: a bare repository has no checked-out `.gitattributes`, so it
    // cannot drive the merge driver. `resolve_current_worktree_root` returns
    // the working tree, so this guard is normally inert, but stays as a guard.
    if root.join("HEAD").is_file() && root.join("objects").is_dir() && !root.join(".git").exists() {
        return Ok(());
    }
    let attributes_path = root.join(".gitattributes");
    let existing = fs::read_to_string(&attributes_path).unwrap_or_default();
    if existing
        .lines()
        .any(|line| line.trim() == WORK_EVENTS_GITATTRIBUTES_LINE)
    {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(WORK_EVENTS_GITATTRIBUTES_LINE);
    next.push('\n');
    // Best-effort: a read-only or non-repo root must not fail event recording.
    let _ = fs::write(&attributes_path, next);
    Ok(())
}

fn migrate_legacy_workspace_projection(
    repo_path: &Path,
    canonical_path: &Path,
) -> Result<Option<WorkspaceProjection>> {
    let work_items_path = canonical_path.with_file_name("works.json");
    with_workspace_work_items_lock(&work_items_path, || {
        if let Some(projection) = load_workspace_projection_from_path(canonical_path)? {
            return Ok(Some(projection));
        }

        let legacy_path = legacy_workspace_projection_path_for_repo_path(repo_path);
        if legacy_path == canonical_path {
            return load_workspace_projection_from_path(canonical_path);
        }
        let Some(projection) = load_workspace_projection_from_path(&legacy_path)? else {
            return Ok(None);
        };
        save_workspace_projection_to_path_unlocked(canonical_path, &projection)?;
        Ok(Some(projection))
    })
}

fn migrate_legacy_workspace_work_items(
    repo_path: &Path,
    canonical_path: &Path,
) -> Result<Option<WorkItemsProjection>> {
    with_workspace_work_items_lock(canonical_path, || {
        if let Some(projection) = load_workspace_work_items_from_path(canonical_path)? {
            return Ok(Some(projection));
        }
        let legacy_path = legacy_workspace_work_items_path_for_repo_path(repo_path);
        if legacy_path == canonical_path {
            return load_workspace_work_items_from_path(canonical_path);
        }
        let Some(projection) = load_workspace_work_items_from_path(&legacy_path)? else {
            return Ok(None);
        };
        save_workspace_work_items_projection_to_path(canonical_path, &projection)?;
        Ok(Some(projection))
    })
}

pub fn load_workspace_projection(repo_path: &Path) -> Result<Option<WorkspaceProjection>> {
    let path = gwt_workspace_projection_path_for_repo_path(repo_path);
    if let Some(projection) = load_workspace_projection_from_path(&path)? {
        return Ok(Some(projection));
    }
    migrate_legacy_workspace_projection(repo_path, &path)
}

/// SPEC-2359 FR-094 / FR-097 / FR-098 / FR-099: resolve the currently
/// assigned Workspace id for a given session. Returns `Some(id)` when
/// the agent is assigned to a Workspace and `None` otherwise (Unassigned
/// agent, missing agent, or load failure). Callers use this for Board
/// audience auto-attach, reminder injection scoping, and the
/// duplicate-work coordination gate corpus.
pub fn resolve_workspace_id_for_session(repo_path: &Path, session_id: &str) -> Option<String> {
    try_resolve_workspace_id_for_session(repo_path, session_id)
        .ok()
        .flatten()
}

pub fn try_resolve_workspace_id_for_session(
    repo_path: &Path,
    session_id: &str,
) -> Result<Option<String>> {
    Ok(
        match try_resolve_workspace_assignment_for_session(repo_path, session_id)? {
            WorkspaceSessionAssignment::Assigned(workspace_id) => Some(workspace_id),
            WorkspaceSessionAssignment::Unassigned | WorkspaceSessionAssignment::Missing => None,
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceSessionAssignment {
    Missing,
    Unassigned,
    Assigned(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceTerminalEventOutcome {
    Emitted,
    AlreadyMatching,
    WrongTerminal,
    AmbiguousTerminal,
    AssignedWorkMissing(String),
    NoTarget,
}

pub fn try_resolve_workspace_assignment_for_session(
    repo_path: &Path,
    session_id: &str,
) -> Result<WorkspaceSessionAssignment> {
    let Some(projection) = load_workspace_projection(repo_path)? else {
        return Ok(WorkspaceSessionAssignment::Missing);
    };
    Ok(workspace_assignment_for_session(&projection, session_id))
}

fn workspace_assignment_for_session(
    projection: &WorkspaceProjection,
    session_id: &str,
) -> WorkspaceSessionAssignment {
    let latest = latest_workspace_agent_for_session(projection, session_id);
    let Some(agent) = latest else {
        return WorkspaceSessionAssignment::Missing;
    };
    if agent.is_unassigned() {
        return WorkspaceSessionAssignment::Unassigned;
    }
    agent
        .workspace_id
        .clone()
        .map(WorkspaceSessionAssignment::Assigned)
        .unwrap_or(WorkspaceSessionAssignment::Unassigned)
}

fn latest_workspace_agent_for_session<'a>(
    projection: &'a WorkspaceProjection,
    session_id: &str,
) -> Option<&'a WorkspaceAgentSummary> {
    projection.latest_agent_for_session(session_id)
}

/// SPEC-2359 FR-097: resolve the currently assigned Workspace id for a
/// mention target. `target_kind` is `BoardMentionTargetKind::Agent` or
/// `BoardMentionTargetKind::Session`; the target value is matched
/// against agent display_name / agent_id (agent) or session_id (session).
/// Returns `Some(id)` when the matched agent is assigned, `None`
/// otherwise.
pub fn resolve_workspace_id_for_mention(
    repo_path: &Path,
    target_kind: &str,
    target_value: &str,
) -> Option<String> {
    let projection = load_workspace_projection(repo_path).ok().flatten()?;
    let agent = match target_kind {
        "session" => projection.latest_agent_for_session(target_value),
        "agent" => projection
            .latest_agents()
            .filter(|agent| {
                agent.agent_id == target_value
                    || agent.display_name == target_value
                    || agent.display_name.eq_ignore_ascii_case(target_value)
            })
            .max_by(|left, right| left.updated_at.cmp(&right.updated_at)),
        _ => None,
    };
    agent
        .filter(|agent| !agent.is_unassigned())
        .and_then(|agent| agent.workspace_id.clone())
}

pub fn load_or_default_workspace_projection(repo_path: &Path) -> Result<WorkspaceProjection> {
    Ok(load_workspace_projection(repo_path)?
        .unwrap_or_else(|| WorkspaceProjection::default_for_project(repo_path)))
}

pub fn save_workspace_projection(repo_path: &Path, projection: &WorkspaceProjection) -> Result<()> {
    save_workspace_projection_to_path(
        &gwt_workspace_projection_path_for_repo_path(repo_path),
        projection,
    )
}

/// Mutate the current Workspace projection while holding the project lock from
/// before load through the atomic save. This is the canonical RMW path for
/// writers that do not also emit a Work event.
pub fn mutate_workspace_projection<T>(
    repo_path: &Path,
    update: impl FnOnce(&mut WorkspaceProjection) -> Result<T>,
) -> Result<T> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    mutate_workspace_projection_at(&current_path, repo_path, update)
}

pub fn mutate_existing_workspace_projection<T>(
    repo_path: &Path,
    update: impl FnOnce(&mut WorkspaceProjection) -> Result<T>,
) -> Result<Option<T>> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    mutate_existing_workspace_projection_at(repo_path, &current_path, false, update)
}

pub fn mutate_existing_workspace_projection_for_cleanup<T>(
    repo_path: &Path,
    update: impl FnOnce(&mut WorkspaceProjection) -> Result<T>,
) -> Result<Option<T>> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    mutate_existing_workspace_projection_at(repo_path, &current_path, true, update)
}

fn mutate_existing_workspace_projection_at<T>(
    repo_path: &Path,
    current_path: &Path,
    invalidate_legacy: bool,
    update: impl FnOnce(&mut WorkspaceProjection) -> Result<T>,
) -> Result<Option<T>> {
    let work_items_path = current_path.with_file_name("works.json");
    with_workspace_work_items_lock(&work_items_path, || {
        let Some(mut projection) = load_workspace_projection_from_path(current_path)? else {
            if invalidate_legacy {
                remove_legacy_workspace_projection(repo_path, current_path)?;
            }
            return Ok(None);
        };
        projection.project_root = repo_path.to_path_buf();
        let result = update(&mut projection)?;
        if invalidate_legacy {
            remove_legacy_workspace_projection(repo_path, current_path)?;
        }
        save_workspace_projection_to_path_unlocked(current_path, &projection)?;
        Ok(Some(result))
    })
}

fn remove_legacy_workspace_projection(repo_path: &Path, canonical_path: &Path) -> Result<()> {
    let legacy_path = legacy_workspace_projection_path_for_repo_path(repo_path);
    if legacy_path == canonical_path {
        return Ok(());
    }
    match fs::remove_file(legacy_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn mutate_workspace_projection_at<T>(
    current_path: &Path,
    project_root: &Path,
    update: impl FnOnce(&mut WorkspaceProjection) -> Result<T>,
) -> Result<T> {
    let work_items_path = current_path.with_file_name("works.json");
    with_workspace_work_items_lock(&work_items_path, || {
        let mut projection =
            load_or_default_workspace_projection_from_path(current_path, project_root)?;
        projection.project_root = project_root.to_path_buf();
        let result = update(&mut projection)?;
        save_workspace_projection_to_path_unlocked(current_path, &projection)?;
        Ok(result)
    })
}

/// Update assignment/current state and its Work events under one project lock.
/// The closure sees one consistent snapshot of both projections and returns
/// the events that belong to the same state transition.
pub fn transact_workspace_state<T>(
    repo_path: &Path,
    update: impl FnOnce(
        &mut WorkspaceProjection,
        &WorkItemsProjection,
        bool,
    ) -> Result<(T, Vec<WorkEvent>)>,
) -> Result<T> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    let events_path = repo_local_work_events_path_with_migration(repo_path)?;
    transact_workspace_state_at(
        &current_path,
        &work_items_path,
        &events_path,
        repo_path,
        update,
    )
}

const WORKSPACE_STATE_TRANSACTION_VERSION: u32 = 2;
const MIN_WORKSPACE_STATE_TRANSACTION_VERSION: u32 = 1;
const WORKSPACE_STATE_TRANSACTION_COORDINATOR_DIR: &str =
    ".gwt-pending-workspace-state-transactions";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingWorkspaceStateTransaction {
    version: u32,
    #[serde(default)]
    transaction_id: Option<String>,
    current_path: PathBuf,
    work_items_path: PathBuf,
    #[serde(default)]
    current_precondition: Option<String>,
    #[serde(default)]
    work_items_precondition: Option<String>,
    projection: WorkspaceProjection,
    #[serde(default)]
    work_items: Option<WorkItemsProjection>,
    #[serde(default)]
    events_path: Option<PathBuf>,
    #[serde(default)]
    events: Vec<WorkEvent>,
    #[serde(default)]
    journal_path: Option<PathBuf>,
    #[serde(default)]
    journal_entries: Vec<WorkspaceJournalEntry>,
}

#[derive(Debug, Deserialize)]
struct PendingWorkspaceStateTransactionRouting {
    current_path: PathBuf,
    work_items_path: PathBuf,
}

fn pending_workspace_state_transaction_path(current_path: &Path) -> PathBuf {
    current_path.with_file_name("pending-state-transaction.json")
}

fn pending_workspace_state_transaction_path_for_work_items(work_items_path: &Path) -> PathBuf {
    work_items_path.with_file_name("pending-state-transaction.json")
}

fn pending_workspace_state_transaction_paths(
    transaction: &PendingWorkspaceStateTransaction,
) -> Vec<PathBuf> {
    let mut paths = vec![
        pending_workspace_state_transaction_path(&transaction.current_path),
        pending_workspace_state_transaction_path_for_work_items(&transaction.work_items_path),
    ];
    if let Some(path) = pending_workspace_state_transaction_coordinator_path(transaction) {
        paths.push(path);
    }
    if let Some(path) = legacy_pending_workspace_state_transaction_coordinator_path(transaction)
        .filter(|path| path.exists())
    {
        paths.push(path);
    }
    paths.sort();
    paths.dedup();
    paths
}

fn pending_workspace_state_transaction_coordinator_path(
    transaction: &PendingWorkspaceStateTransaction,
) -> Option<PathBuf> {
    let transaction_id = transaction.transaction_id.as_deref()?;
    Some(
        crate::paths::gwt_home()
            .join(WORKSPACE_STATE_TRANSACTION_COORDINATOR_DIR)
            .join(format!("{transaction_id}.json")),
    )
}

fn legacy_pending_workspace_state_transaction_coordinator_path(
    transaction: &PendingWorkspaceStateTransaction,
) -> Option<PathBuf> {
    let transaction_id = transaction.transaction_id.as_deref()?;
    let current_parent = transaction.current_path.parent()?;
    let work_items_parent = transaction.work_items_path.parent()?;
    let common_parent = common_path_ancestor(current_parent, work_items_parent)?;
    Some(
        common_parent
            .join(WORKSPACE_STATE_TRANSACTION_COORDINATOR_DIR)
            .join(format!("{transaction_id}.json")),
    )
}

fn common_path_ancestor(left: &Path, right: &Path) -> Option<PathBuf> {
    let mut common = PathBuf::new();
    for (left_component, right_component) in left.components().zip(right.components()) {
        if left_component != right_component {
            break;
        }
        common.push(left_component.as_os_str());
    }
    (!common.as_os_str().is_empty()).then_some(common)
}

fn discover_pending_workspace_state_transaction_coordinators(
    lock_targets: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    let mut coordinator_dirs = HashSet::new();
    let global_coordinator_dir =
        crate::paths::gwt_home().join(WORKSPACE_STATE_TRANSACTION_COORDINATOR_DIR);
    coordinator_dirs.insert(global_coordinator_dir.clone());
    let mut coordinator_paths = Vec::new();
    for target in lock_targets {
        let Some(parent) = target.parent() else {
            continue;
        };
        for ancestor in parent.ancestors() {
            let coordinator_dir = ancestor.join(WORKSPACE_STATE_TRANSACTION_COORDINATOR_DIR);
            coordinator_dirs.insert(coordinator_dir);
        }
    }
    for coordinator_dir in coordinator_dirs {
        if !coordinator_dir.is_dir() {
            continue;
        }
        let entries = match fs::read_dir(&coordinator_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            match serde_json::from_slice::<PendingWorkspaceStateTransactionRouting>(&bytes) {
                Ok(routing) => {
                    let current_lock_target = routing.current_path.with_file_name("works.json");
                    if lock_targets.iter().any(|target| {
                        target == &current_lock_target || target == &routing.work_items_path
                    }) {
                        coordinator_paths.push(path);
                    }
                }
                Err(_) if coordinator_dir != global_coordinator_dir => {
                    coordinator_paths.push(path);
                }
                Err(_) => {}
            }
        }
    }
    coordinator_paths.sort();
    coordinator_paths.dedup();
    Ok(coordinator_paths)
}

fn workspace_state_file_fingerprint(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};

    match fs::read(path) {
        Ok(bytes) => {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            Ok(format!("sha256:{:x}", hasher.finalize()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("missing".to_string()),
        Err(error) => Err(error.into()),
    }
}

pub fn transact_workspace_state_at<T>(
    current_path: &Path,
    work_items_path: &Path,
    events_path: &Path,
    project_root: &Path,
    update: impl FnOnce(
        &mut WorkspaceProjection,
        &WorkItemsProjection,
        bool,
    ) -> Result<(T, Vec<WorkEvent>)>,
) -> Result<T> {
    with_workspace_current_and_work_items_lock(current_path, work_items_path, || {
        let current_precondition = workspace_state_file_fingerprint(current_path)?;
        let work_items_precondition = workspace_state_file_fingerprint(work_items_path)?;
        let mut projection =
            load_or_default_workspace_projection_from_path(current_path, project_root)?;
        projection.project_root = project_root.to_path_buf();
        let journal_path = current_path.with_file_name("journal.jsonl");
        let persisted_work_items = load_workspace_work_items_from_path(work_items_path)?;
        let synthesized = persisted_work_items.is_none();
        let mut work_items = match persisted_work_items {
            Some(work_items) => work_items,
            None => load_or_synthesize_workspace_work_items_from_paths(
                work_items_path,
                current_path,
                &journal_path,
                project_root,
            )?,
        };
        let recovered_close_events = recover_unprojected_workspace_work_events_locked(
            &mut work_items,
            &work_items_path.with_file_name("work-events-closed.jsonl"),
        )?;
        let projection_before = projection.clone();
        let board_refs_before = projection_before
            .board_refs
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let (result, events) = update(&mut projection, &work_items, !synthesized)?;
        validate_workspace_state_transaction_mutations(
            &projection_before,
            &projection,
            &board_refs_before,
            &events,
        )?;

        let mut next_work_items = work_items;
        for event in &events {
            if next_work_items.apply_event(event.clone())
                == WorkEventApplyOutcome::RejectedSessionConflict
            {
                return Err(GwtError::Other(format!(
                    "workspace state transaction rejected Session conflict for Work {}",
                    event.work_item_id
                )));
            }
        }

        let transaction = PendingWorkspaceStateTransaction {
            version: WORKSPACE_STATE_TRANSACTION_VERSION,
            transaction_id: Some(Uuid::new_v4().to_string()),
            current_path: current_path.to_path_buf(),
            work_items_path: work_items_path.to_path_buf(),
            current_precondition: Some(current_precondition),
            work_items_precondition: Some(work_items_precondition),
            projection,
            work_items: (recovered_close_events || !events.is_empty()).then_some(next_work_items),
            events_path: (!events.is_empty()).then(|| events_path.to_path_buf()),
            events,
            journal_path: None,
            journal_entries: Vec::new(),
        };
        persist_workspace_state_transaction_locked(current_path, &transaction)?;
        Ok(result)
    })
}

/// Enforce the project transaction's Work principal before any durable file is
/// written. A cross-Work event must carry a Session that the same locked
/// projection assigns to that Work; otherwise a generic update could mutate a
/// foreign Work without passing through a Session-bound resolver. Likewise,
/// every newly-added current-projection Board ref must be backed by an event
/// for that same current Work in this transaction.
fn validate_workspace_state_transaction_mutations(
    projection_before: &WorkspaceProjection,
    projection_after: &WorkspaceProjection,
    board_refs_before: &HashSet<String>,
    events: &[WorkEvent],
) -> Result<()> {
    for event in events {
        if event.work_item_id == projection_before.id {
            continue;
        }
        let Some(session_id) = event
            .agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Err(GwtError::Other(
                "workspace state transaction rejected an unauthorized cross-Work event".to_string(),
            ));
        };
        let authority = if workspace_event_establishes_session_attachment(event.kind) {
            projection_after
        } else {
            projection_before
        };
        let assigned_to_target =
            authority
                .latest_agent_for_session(session_id)
                .is_some_and(|agent| {
                    agent.affiliation_status == WorkspaceAgentAffiliationStatus::Assigned
                        && agent.workspace_id.as_deref() == Some(event.work_item_id.as_str())
                });
        if !assigned_to_target {
            return Err(GwtError::Other(
                "workspace state transaction rejected an unauthorized cross-Work event".to_string(),
            ));
        }
    }

    for board_ref in projection_after
        .board_refs
        .iter()
        .filter(|board_ref| !board_refs_before.contains(*board_ref))
    {
        let has_current_work_event = events.iter().any(|event| {
            event.work_item_id == projection_before.id
                && event.board_entry_id.as_deref() == Some(board_ref.as_str())
        });
        if !has_current_work_event {
            return Err(GwtError::Other(
                "workspace state transaction rejected a Board ref without a matching Work event"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn workspace_event_establishes_session_attachment(kind: WorkEventKind) -> bool {
    matches!(
        kind,
        WorkEventKind::Start | WorkEventKind::Claim | WorkEventKind::Resume | WorkEventKind::Split
    )
}

pub fn update_workspace_projection_with_journal(
    repo_path: &Path,
    update: WorkspaceProjectionUpdate,
) -> Result<WorkspaceJournalEntry> {
    update_workspace_projection_with_journal_for_work_event_root(repo_path, repo_path, update)
}

pub fn update_workspace_projection_with_journal_for_work_event_root(
    project_state_root: &Path,
    work_event_root: &Path,
    update: WorkspaceProjectionUpdate,
) -> Result<WorkspaceJournalEntry> {
    let current_path = gwt_workspace_projection_path_for_repo_path(project_state_root);
    let journal_path = gwt_workspace_journal_path_for_repo_path(project_state_root);
    let _ = migrate_legacy_workspace_projection(project_state_root, &current_path)?;
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(work_event_root);
    let _ = migrate_legacy_workspace_work_items(work_event_root, &work_items_path)?;
    let events_path = repo_local_work_events_path_with_migration(work_event_root)?;
    with_workspace_current_and_work_items_lock(&current_path, &work_items_path, || {
        let current_precondition = workspace_state_file_fingerprint(&current_path)?;
        let work_items_precondition = workspace_state_file_fingerprint(&work_items_path)?;
        copy_legacy_workspace_file_if_needed(
            &legacy_workspace_journal_path_for_repo_path(project_state_root),
            &journal_path,
        )?;
        let mut projection =
            load_or_default_workspace_projection_from_path(&current_path, project_state_root)?;
        projection.project_root = project_state_root.to_path_buf();
        let mut work_items = load_or_synthesize_workspace_work_items_from_paths(
            &work_items_path,
            &current_path,
            &journal_path,
            project_state_root,
        )?;
        let entry = projection.apply_update(update, Utc::now());
        let event =
            workspace_work_event_from_journal_entry_for_root(&projection, &entry, work_event_root);
        if work_items.apply_event(event.clone()) == WorkEventApplyOutcome::RejectedSessionConflict {
            return Err(GwtError::Other(format!(
                "workspace journal event rejected Session conflict for Work {}",
                event.work_item_id
            )));
        }

        let transaction = PendingWorkspaceStateTransaction {
            version: WORKSPACE_STATE_TRANSACTION_VERSION,
            transaction_id: Some(Uuid::new_v4().to_string()),
            current_path: current_path.clone(),
            work_items_path: work_items_path.clone(),
            current_precondition: Some(current_precondition),
            work_items_precondition: Some(work_items_precondition),
            projection,
            work_items: Some(work_items),
            events_path: Some(events_path.clone()),
            events: vec![event],
            journal_path: Some(journal_path.clone()),
            journal_entries: vec![entry.clone()],
        };
        persist_workspace_state_transaction_locked(&current_path, &transaction)?;
        Ok(entry)
    })
}

/// Immutable identity selected by the Session-bound Work resolver before a
/// sparse mutation enters the project transaction.
///
/// The transaction treats every field as a precondition. Mutable request
/// values live in [`WorkspaceProjectionUpdate`]; callers cannot replace the
/// Work, Session, branch, worktree, or event root through that payload.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionBoundWorkspaceMutationTarget {
    pub project_state_root: PathBuf,
    pub work_event_root: PathBuf,
    pub session_id: String,
    pub branch_identity: String,
    pub worktree_identity: PathBuf,
    pub work_id: String,
}

/// Host-authenticated Session/runtime identity used for terminalization.
///
/// Unlike [`SessionBoundWorkspaceMutationTarget`], this target intentionally
/// carries no Work id. The latest assignment is resolved only after the
/// current/WorkItems dual lock is held, so authenticated callers cannot close
/// a Work selected before a concurrent reassignment.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionBoundWorkspaceTerminalTarget {
    pub project_state_root: PathBuf,
    pub work_event_root: PathBuf,
    pub session_id: String,
    pub branch_identity: String,
    pub worktree_identity: PathBuf,
}

impl std::fmt::Debug for SessionBoundWorkspaceTerminalTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionBoundWorkspaceTerminalTarget")
            .field("project_state_root", &"<redacted>")
            .field("work_event_root", &"<redacted>")
            .field("session_id", &"<redacted>")
            .field("branch_identity", &"<redacted>")
            .field("worktree_identity", &"<redacted>")
            .finish()
    }
}

impl std::fmt::Debug for SessionBoundWorkspaceMutationTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionBoundWorkspaceMutationTarget")
            .field("project_state_root", &"<redacted>")
            .field("work_event_root", &"<redacted>")
            .field("session_id", &"<redacted>")
            .field("branch_identity", &"<redacted>")
            .field("worktree_identity", &"<redacted>")
            .field("work_id", &"<redacted>")
            .finish()
    }
}

/// Apply one sparse Session-bound Work update under the same lock and
/// recovery transaction as current projection, WorkItems, tracked event, and
/// journal persistence.
///
/// `revalidate` runs after both project locks are held and after the persisted
/// assignment/Work/container preconditions have been reloaded. The gwt layer
/// uses it to reload the Session ledger and runtime binding; core-only callers
/// can validate any additional authority source without moving that authority
/// into the request payload.
pub fn update_workspace_projection_with_journal_for_resolved_work_target(
    target: &SessionBoundWorkspaceMutationTarget,
    update: WorkspaceProjectionUpdate,
    revalidate: impl FnOnce(&WorkspaceProjection, &WorkItemsProjection) -> Result<()>,
) -> Result<WorkspaceJournalEntry> {
    validate_session_bound_target_shape(target, &update)?;

    let current_path = gwt_workspace_projection_path_for_repo_path(&target.project_state_root);
    let journal_path = gwt_workspace_journal_path_for_repo_path(&target.project_state_root);
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(&target.work_event_root);
    let events_path = gwt_repo_local_work_events_path(&target.work_event_root);

    // This strict entry point never synthesizes or migrates authority state.
    // Target resolution already proved each surface; disappearance between
    // resolution and lock acquisition is a conflict, not a fallback signal.
    for (path, label) in [
        (&current_path, "current projection"),
        (&work_items_path, "WorkItems projection"),
        (&events_path, "tracked Work event log"),
    ] {
        match path.try_exists() {
            Ok(true) => {}
            Ok(false) => {
                return Err(GwtError::Other(format!(
                    "Session-bound workspace transaction requires an existing {label}"
                )))
            }
            Err(_) => {
                return Err(GwtError::Other(format!(
                    "Session-bound workspace transaction could not verify the {label}"
                )))
            }
        }
    }

    with_workspace_current_and_work_items_lock(&current_path, &work_items_path, || {
        let current_precondition = workspace_state_file_fingerprint(&current_path)?;
        let work_items_precondition = workspace_state_file_fingerprint(&work_items_path)?;
        let mut projection =
            load_workspace_projection_from_path(&current_path)?.ok_or_else(|| {
                GwtError::Other(
                    "Session-bound workspace transaction lost its current projection".to_string(),
                )
            })?;
        let mut work_items =
            load_workspace_work_items_from_path(&work_items_path)?.ok_or_else(|| {
                GwtError::Other(
                    "Session-bound workspace transaction lost its WorkItems projection".to_string(),
                )
            })?;

        let locked = validate_session_bound_target_locked(
            &projection,
            &work_items,
            target,
            update.owner.as_deref(),
            false,
        )?;
        revalidate(&projection, &work_items)?;

        let updated_at = Utc::now();
        let entry = apply_sparse_update_to_locked_projection(
            &mut projection,
            target,
            &update,
            locked.target_owner.clone(),
            updated_at,
        );
        let event = session_bound_work_event(
            &projection,
            target,
            &update,
            locked.target_owner,
            locked.execution_container,
            updated_at,
        );
        if work_items.apply_event(event.clone()) == WorkEventApplyOutcome::RejectedSessionConflict {
            return Err(GwtError::Other(
                "Session-bound workspace transaction rejected a conflicting Session attachment"
                    .to_string(),
            ));
        }

        let transaction = PendingWorkspaceStateTransaction {
            version: WORKSPACE_STATE_TRANSACTION_VERSION,
            transaction_id: Some(Uuid::new_v4().to_string()),
            current_path: current_path.clone(),
            work_items_path: work_items_path.clone(),
            current_precondition: Some(current_precondition),
            work_items_precondition: Some(work_items_precondition),
            projection,
            work_items: Some(work_items),
            events_path: Some(events_path.clone()),
            events: vec![event],
            journal_path: Some(journal_path.clone()),
            journal_entries: vec![entry.clone()],
        };
        persist_workspace_state_transaction_locked(&current_path, &transaction)?;
        Ok(entry)
    })
}

/// Emit one Done/Discarded close event for a previously resolved
/// Session-bound Work while preserving the same authority boundary as sparse
/// workspace updates.
///
/// The target assignment and execution container are reloaded under the
/// current/WorkItems dual lock. `revalidate` then lets the caller reload
/// authority that lives outside core (the Host Session ledger and runtime
/// binding) before any close event is persisted. Retries against an already
/// terminal target are allowed so the caller receives an explicit
/// [`WorkspaceTerminalEventOutcome`] without appending another event.
pub fn emit_workspace_terminal_event_for_resolved_work_target(
    target: &SessionBoundWorkspaceTerminalTarget,
    close_kind: WorkCloseKind,
    updated_at: DateTime<Utc>,
    revalidate: impl FnOnce(&WorkspaceProjection, &WorkItemsProjection) -> Result<()>,
) -> Result<WorkspaceTerminalEventOutcome> {
    validate_session_bound_target_identity_shape(target)?;

    let current_path = gwt_workspace_projection_path_for_repo_path(&target.project_state_root);
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(&target.work_event_root);
    let events_path = gwt_workspace_work_events_closed_path_for_repo_path(&target.work_event_root);
    for (path, label) in [
        (&current_path, "current projection"),
        (&work_items_path, "WorkItems projection"),
    ] {
        match path.try_exists() {
            Ok(true) => {}
            Ok(false) => {
                return Err(GwtError::Other(format!(
                    "Session-bound Work terminalization requires an existing {label}"
                )))
            }
            Err(_) => {
                return Err(GwtError::Other(format!(
                    "Session-bound Work terminalization could not verify the {label}"
                )))
            }
        }
    }

    with_workspace_current_and_work_items_lock(&current_path, &work_items_path, || {
        let projection = load_workspace_projection_from_path(&current_path)?.ok_or_else(|| {
            GwtError::Other(
                "Session-bound Work terminalization lost its current projection".to_string(),
            )
        })?;
        let work_items =
            load_workspace_work_items_from_path(&work_items_path)?.ok_or_else(|| {
                GwtError::Other(
                    "Session-bound Work terminalization lost its WorkItems projection".to_string(),
                )
            })?;
        let locked =
            resolve_session_bound_terminal_target_locked(&projection, &work_items, target)?;
        revalidate(&projection, &work_items)?;
        let work_id = match locked {
            LockedSessionBoundTerminalTarget::NoTarget => {
                return Ok(WorkspaceTerminalEventOutcome::NoTarget)
            }
            LockedSessionBoundTerminalTarget::AssignedWorkMissing(work_id) => {
                return Ok(WorkspaceTerminalEventOutcome::AssignedWorkMissing(work_id))
            }
            LockedSessionBoundTerminalTarget::Existing(work_id) => work_id,
        };

        let mut event = match close_kind {
            WorkCloseKind::Done => {
                let mut event = WorkEvent::new(WorkEventKind::Done, &work_id, updated_at);
                event.status_category = Some(WorkspaceStatusCategory::Done);
                event
            }
            WorkCloseKind::Discarded => {
                WorkEvent::new(WorkEventKind::Discard, &work_id, updated_at)
            }
        };
        event.agent_session_id = Some(target.session_id.clone());
        emit_workspace_terminal_event_outcome_locked(
            &work_items_path,
            &events_path,
            event,
            true,
            false,
        )
    })
}

enum LockedSessionBoundTerminalTarget {
    NoTarget,
    AssignedWorkMissing(String),
    Existing(String),
}

fn resolve_session_bound_terminal_target_locked(
    projection: &WorkspaceProjection,
    work_items: &WorkItemsProjection,
    target: &SessionBoundWorkspaceTerminalTarget,
) -> Result<LockedSessionBoundTerminalTarget> {
    let Some(agent) = projection.latest_agent_for_session(&target.session_id) else {
        return Ok(LockedSessionBoundTerminalTarget::NoTarget);
    };
    if agent.affiliation_status != WorkspaceAgentAffiliationStatus::Assigned {
        return Ok(LockedSessionBoundTerminalTarget::NoTarget);
    }
    let Some(work_id) = agent
        .workspace_id
        .as_deref()
        .filter(|work_id| !work_id.trim().is_empty())
    else {
        return Ok(LockedSessionBoundTerminalTarget::NoTarget);
    };
    if canonical_session_bound_branch(agent.branch.as_deref().unwrap_or_default())
        != canonical_session_bound_branch(&target.branch_identity)
        || !session_bound_paths_match(
            agent.worktree_path.as_deref(),
            Some(target.worktree_identity.as_path()),
        )?
    {
        return Err(GwtError::Other(
            "Session-bound Work terminalization assignment container changed before commit"
                .to_string(),
        ));
    }

    let mut matches = work_items
        .work_items
        .iter()
        .filter(|item| item.id == work_id);
    let Some(item) = matches.next() else {
        return Ok(LockedSessionBoundTerminalTarget::AssignedWorkMissing(
            work_id.to_string(),
        ));
    };
    if matches.next().is_some() {
        return Err(GwtError::Other(
            "Session-bound Work terminalization target became ambiguous".to_string(),
        ));
    }
    let mut matching_containers = 0;
    for container in &item.execution_containers {
        let branch_matches =
            canonical_session_bound_branch(container.branch.as_deref().unwrap_or_default())
                == canonical_session_bound_branch(&target.branch_identity);
        if branch_matches
            && session_bound_paths_match(
                container.worktree_path.as_deref(),
                Some(target.worktree_identity.as_path()),
            )?
        {
            matching_containers += 1;
        }
    }
    if matching_containers != 1 {
        return Err(GwtError::Other(
            "Session-bound Work terminalization target container changed or became ambiguous"
                .to_string(),
        ));
    }
    if work_items.work_items.iter().any(|other| {
        other.id != work_id
            && !other.is_terminal()
            && other
                .agents
                .iter()
                .any(|agent| agent.session_id == target.session_id)
    }) {
        return Err(GwtError::Other(
            "Session-bound Work terminalization Session is attached to multiple active Works"
                .to_string(),
        ));
    }
    Ok(LockedSessionBoundTerminalTarget::Existing(
        work_id.to_string(),
    ))
}

struct LockedSessionBoundTarget {
    target_owner: Option<String>,
    execution_container: WorkspaceExecutionContainerRef,
}

fn validate_session_bound_target_shape(
    target: &SessionBoundWorkspaceMutationTarget,
    update: &WorkspaceProjectionUpdate,
) -> Result<()> {
    validate_session_bound_target_identity_shape(target)?;
    if target.work_id.trim().is_empty() {
        return Err(GwtError::Other(
            "Session-bound workspace transaction received an incomplete target".to_string(),
        ));
    }
    if update.agent_session_id.as_deref() != Some(target.session_id.as_str()) {
        return Err(GwtError::Other(
            "Session-bound workspace update Session does not match its resolved target".to_string(),
        ));
    }
    Ok(())
}

fn validate_session_bound_target_identity_shape(
    target: &impl SessionBoundTargetIdentity,
) -> Result<()> {
    if target.session_id().trim().is_empty()
        || canonical_session_bound_branch(target.branch_identity()).is_empty()
        || target.project_state_root().as_os_str().is_empty()
        || target.work_event_root().as_os_str().is_empty()
        || target.worktree_identity().as_os_str().is_empty()
    {
        return Err(GwtError::Other(
            "Session-bound workspace transaction received an incomplete target".to_string(),
        ));
    }
    Ok(())
}

trait SessionBoundTargetIdentity {
    fn project_state_root(&self) -> &Path;
    fn work_event_root(&self) -> &Path;
    fn session_id(&self) -> &str;
    fn branch_identity(&self) -> &str;
    fn worktree_identity(&self) -> &Path;
}

impl SessionBoundTargetIdentity for SessionBoundWorkspaceMutationTarget {
    fn project_state_root(&self) -> &Path {
        &self.project_state_root
    }
    fn work_event_root(&self) -> &Path {
        &self.work_event_root
    }
    fn session_id(&self) -> &str {
        &self.session_id
    }
    fn branch_identity(&self) -> &str {
        &self.branch_identity
    }
    fn worktree_identity(&self) -> &Path {
        &self.worktree_identity
    }
}

impl SessionBoundTargetIdentity for SessionBoundWorkspaceTerminalTarget {
    fn project_state_root(&self) -> &Path {
        &self.project_state_root
    }
    fn work_event_root(&self) -> &Path {
        &self.work_event_root
    }
    fn session_id(&self) -> &str {
        &self.session_id
    }
    fn branch_identity(&self) -> &str {
        &self.branch_identity
    }
    fn worktree_identity(&self) -> &Path {
        &self.worktree_identity
    }
}

fn validate_session_bound_target_locked(
    projection: &WorkspaceProjection,
    work_items: &WorkItemsProjection,
    target: &SessionBoundWorkspaceMutationTarget,
    owner_claim: Option<&str>,
    allow_terminal: bool,
) -> Result<LockedSessionBoundTarget> {
    let agent = projection
        .latest_agent_for_session(&target.session_id)
        .filter(|agent| {
            agent.affiliation_status == WorkspaceAgentAffiliationStatus::Assigned
                && agent.workspace_id.as_deref() == Some(target.work_id.as_str())
        })
        .ok_or_else(|| {
            GwtError::Other("Session-bound workspace assignment changed before commit".to_string())
        })?;
    if canonical_session_bound_branch(agent.branch.as_deref().unwrap_or_default())
        != canonical_session_bound_branch(&target.branch_identity)
        || !session_bound_paths_match(
            agent.worktree_path.as_deref(),
            Some(target.worktree_identity.as_path()),
        )?
    {
        return Err(GwtError::Other(
            "Session-bound workspace assignment container changed before commit".to_string(),
        ));
    }

    let mut matches = work_items
        .work_items
        .iter()
        .filter(|item| item.id == target.work_id);
    let item = matches.next().ok_or_else(|| {
        GwtError::Other("Session-bound workspace target disappeared before commit".to_string())
    })?;
    if matches.next().is_some() || (item.is_terminal() && !allow_terminal) {
        return Err(GwtError::Other(
            "Session-bound workspace target is ambiguous or terminal".to_string(),
        ));
    }

    let mut execution_container = None;
    for container in &item.execution_containers {
        let branch_matches =
            canonical_session_bound_branch(container.branch.as_deref().unwrap_or_default())
                == canonical_session_bound_branch(&target.branch_identity);
        if branch_matches
            && session_bound_paths_match(
                container.worktree_path.as_deref(),
                Some(target.worktree_identity.as_path()),
            )?
        {
            if execution_container.is_some() {
                return Err(GwtError::Other(
                    "Session-bound workspace target container became ambiguous".to_string(),
                ));
            }
            execution_container = Some(container.clone());
        }
    }
    let execution_container = execution_container.ok_or_else(|| {
        GwtError::Other(
            "Session-bound workspace target container changed before commit".to_string(),
        )
    })?;

    if work_items.work_items.iter().any(|other| {
        other.id != target.work_id
            && !other.is_terminal()
            && other
                .agents
                .iter()
                .any(|agent| agent.session_id == target.session_id)
    }) {
        return Err(GwtError::Other(
            "Session-bound workspace Session is attached to multiple active Works".to_string(),
        ));
    }

    let target_owner = item.owner.clone();
    validate_session_bound_owner_claim(owner_claim, target_owner.as_deref())?;

    Ok(LockedSessionBoundTarget {
        target_owner,
        execution_container,
    })
}

fn validate_session_bound_owner_claim(
    claimed_owner: Option<&str>,
    target_owner: Option<&str>,
) -> Result<()> {
    let Some(claimed_owner) = claimed_owner else {
        return Ok(());
    };
    let claimed_owner = claimed_owner.trim();
    if claimed_owner.is_empty() || Some(claimed_owner) != target_owner.map(str::trim) {
        return Err(GwtError::Other(
            "Session-bound workspace owner claim conflicts with the target Work".to_string(),
        ));
    }
    Ok(())
}

fn session_bound_paths_match(left: Option<&Path>, right: Option<&Path>) -> Result<bool> {
    let (Some(left), Some(right)) = (left, right) else {
        return Ok(false);
    };
    let canonicalize = |path: &Path| {
        fs::canonicalize(path)
            .map(|path| crate::paths::normalize_windows_child_process_path(&path))
            .map_err(|_| {
                GwtError::Other(
                    "Session-bound workspace path could not be canonicalized".to_string(),
                )
            })
    };
    Ok(canonicalize(left)? == canonicalize(right)?)
}

fn canonical_session_bound_branch(branch: &str) -> String {
    let branch = branch.trim();
    let branch = branch.strip_prefix("refs/heads/").unwrap_or(branch);
    canonical_work_branch_identity(branch)
}

fn apply_sparse_update_to_locked_projection(
    projection: &mut WorkspaceProjection,
    target: &SessionBoundWorkspaceMutationTarget,
    update: &WorkspaceProjectionUpdate,
    target_owner: Option<String>,
    updated_at: DateTime<Utc>,
) -> WorkspaceJournalEntry {
    if projection.id == target.work_id {
        let mut projection_update = update.clone();
        projection_update.owner = target_owner.clone();
        return projection.apply_update(projection_update, updated_at);
    }

    if let Some(agent) = projection.latest_agent_for_session_mut(&target.session_id) {
        let mut changed = false;
        if let Some(focus) = update
            .agent_current_focus
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            agent.current_focus = Some(focus.clone());
            changed = true;
        }
        if let Some(title) = update
            .agent_title_summary
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            agent.title_summary = Some(title.clone());
            changed = true;
        }
        if changed {
            agent.updated_at = updated_at;
            projection.updated_at = updated_at;
        }
    }

    WorkspaceJournalEntry {
        id: Uuid::new_v4().to_string(),
        project_root: target.project_state_root.clone(),
        title: update.title.clone(),
        status_category: update.status_category,
        status_text: update.status_text.clone(),
        owner: target_owner,
        next_action: update.next_action.clone(),
        summary: update.summary.clone(),
        progress_summary: update.progress_summary.clone(),
        agent_session_id: Some(target.session_id.clone()),
        agent_current_focus: update.agent_current_focus.clone(),
        agent_title_summary: update.agent_title_summary.clone(),
        updated_at,
    }
}

fn session_bound_work_event(
    projection: &WorkspaceProjection,
    target: &SessionBoundWorkspaceMutationTarget,
    update: &WorkspaceProjectionUpdate,
    target_owner: Option<String>,
    execution_container: WorkspaceExecutionContainerRef,
    updated_at: DateTime<Utc>,
) -> WorkEvent {
    let kind = match update.status_category {
        Some(WorkspaceStatusCategory::Done) => WorkEventKind::Done,
        Some(WorkspaceStatusCategory::Blocked) => WorkEventKind::Blocked,
        _ => WorkEventKind::Update,
    };
    let mut event = WorkEvent::new(kind, &target.work_id, updated_at);
    event.title = update
        .title
        .clone()
        .or_else(|| update.agent_title_summary.clone());
    event.intent = update.agent_current_focus.clone();
    event.summary = update
        .summary
        .clone()
        .or_else(|| update.status_text.clone());
    event.progress_summary = update.progress_summary.clone();
    event.status_category = update.status_category;
    event.owner = target_owner;
    event.next_action = update.next_action.clone();
    event.agent_session_id = Some(target.session_id.clone());
    if let Some(agent) = projection.latest_agent_for_session(&target.session_id) {
        event.agent_id = (!agent.agent_id.trim().is_empty()).then(|| agent.agent_id.clone());
        event.display_name =
            (!agent.display_name.trim().is_empty()).then(|| agent.display_name.clone());
    }
    event.execution_container = Some(execution_container);
    event
}

pub fn mark_workspace_agent_stopped(
    repo_path: &Path,
    session_id: &str,
    window_id: Option<&str>,
) -> Result<bool> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    mark_workspace_agent_stopped_at(&current_path, repo_path, session_id, window_id, Utc::now())
}

pub fn load_workspace_projection_from_path(path: &Path) -> Result<Option<WorkspaceProjection>> {
    match fs::read(path) {
        Ok(bytes) => {
            let mut projection: WorkspaceProjection = serde_json::from_slice(&bytes)
                .map_err(|error| GwtError::Other(format!("workspace projection json: {error}")))?;
            migrate_workspace_to_work_terminology(&mut projection);
            Ok(Some(projection))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn migrate_workspace_to_work_terminology(projection: &mut WorkspaceProjection) {
    if projection.title == "Workspace" {
        projection.title = "Work".to_string();
    } else if projection.title.starts_with("Workspace ") {
        let suffix = &projection.title["Workspace ".len()..];
        projection.title = format!("Work {suffix}");
    } else if projection.title.ends_with(" workspace") {
        let prefix = &projection.title[..projection.title.len() - " workspace".len()];
        projection.title = format!("{prefix} work");
    }
}

pub fn load_workspace_work_items(repo_path: &Path) -> Result<Option<WorkItemsProjection>> {
    migrate_legacy_workspace_work_items(
        repo_path,
        &gwt_workspace_work_items_path_for_repo_path(repo_path),
    )
}

fn classify_json_decode_error(context: &'static str, error: serde_json::Error) -> GwtError {
    let kind = match error.classify() {
        serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
            JsonDecodeKind::Malformed
        }
        serde_json::error::Category::Data => JsonDecodeKind::IncompatibleSchema,
        serde_json::error::Category::Io => {
            return GwtError::Io(std::io::Error::new(
                error.io_error_kind().unwrap_or(std::io::ErrorKind::Other),
                error,
            ));
        }
    };
    GwtError::JsonDecode {
        context,
        kind,
        message: error.to_string(),
    }
}

pub fn load_workspace_work_items_from_path(path: &Path) -> Result<Option<WorkItemsProjection>> {
    match fs::read(path) {
        Ok(bytes) => {
            let mut items: WorkItemsProjection = serde_json::from_slice(&bytes)
                .map_err(|error| classify_json_decode_error("workspace work items json", error))?;
            for item in &mut items.work_items {
                if item.title == "Workspace" {
                    item.title = "Work".to_string();
                } else if item.title.starts_with("Workspace ") {
                    let suffix = &item.title["Workspace ".len()..];
                    item.title = format!("Work {suffix}");
                } else if item.title.ends_with(" workspace") {
                    let prefix = &item.title[..item.title.len() - " workspace".len()];
                    item.title = format!("{prefix} work");
                }
            }
            items.refresh_derived_progress_summaries();
            Ok(Some(items))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn load_or_synthesize_workspace_work_items(repo_path: &Path) -> Result<WorkItemsProjection> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let journal_path = gwt_workspace_journal_path_for_repo_path(repo_path);
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    with_workspace_current_and_work_items_lock(&current_path, &work_items_path, || {
        copy_legacy_workspace_file_if_needed(
            &legacy_workspace_journal_path_for_repo_path(repo_path),
            &journal_path,
        )?;
        load_or_synthesize_workspace_work_items_from_paths(
            &work_items_path,
            &current_path,
            &journal_path,
            repo_path,
        )
    })
}

pub fn load_or_synthesize_workspace_work_items_from_paths(
    work_items_path: &Path,
    current_path: &Path,
    journal_path: &Path,
    project_root: &Path,
) -> Result<WorkItemsProjection> {
    if let Some(projection) = load_workspace_work_items_from_path(work_items_path)? {
        return Ok(projection);
    }
    synthesize_workspace_work_items_from_legacy_paths(current_path, journal_path, project_root)
}

/// SPEC-2359 (close-latency root fix, 2026-06-11): mtime+size-keyed cache in
/// front of [`load_or_synthesize_workspace_work_items`]. The home works.json
/// grows to megabytes (hundreds of Work items × thousands of events) and the
/// UI event loop rebuilds the Workspace projection on every broadcast-bearing
/// action; re-parsing the file each time stalls the queue. A cache hit clones
/// the parsed projection instead. Synthesized fallbacks (works.json absent)
/// are never cached — they must observe legacy-file changes.
#[derive(Default)]
pub struct WorkItemsCache {
    entries: HashMap<PathBuf, CachedWorkItemsProjection>,
    /// Lifetime parse counter; tests assert the steady state stops parsing.
    pub parse_count: u64,
}

struct CachedWorkItemsProjection {
    mtime: std::time::SystemTime,
    size: u64,
    projection: WorkItemsProjection,
}

impl WorkItemsCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cached equivalent of [`load_or_synthesize_workspace_work_items`].
    pub fn load_or_synthesize(&mut self, repo_path: &Path) -> Result<WorkItemsProjection> {
        let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
        if let Some(hit) = self.lookup(&work_items_path) {
            return Ok(hit);
        }
        let projection = load_or_synthesize_workspace_work_items(repo_path)?;
        self.store(&work_items_path, &projection);
        Ok(projection)
    }

    /// Paths-injected variant for tests and path-explicit callers (#3022).
    pub fn load_or_synthesize_from_paths(
        &mut self,
        work_items_path: &Path,
        current_path: &Path,
        journal_path: &Path,
        project_root: &Path,
    ) -> Result<WorkItemsProjection> {
        if let Some(hit) = self.lookup(work_items_path) {
            return Ok(hit);
        }
        let projection = load_or_synthesize_workspace_work_items_from_paths(
            work_items_path,
            current_path,
            journal_path,
            project_root,
        )?;
        self.store(work_items_path, &projection);
        Ok(projection)
    }

    fn lookup(&self, work_items_path: &Path) -> Option<WorkItemsProjection> {
        let meta = fs::metadata(work_items_path).ok()?;
        let mtime = meta.modified().ok()?;
        let hit = self.entries.get(work_items_path)?;
        (hit.mtime == mtime && hit.size == meta.len()).then(|| hit.projection.clone())
    }

    fn store(&mut self, work_items_path: &Path, projection: &WorkItemsProjection) {
        self.parse_count += 1;
        let Ok(meta) = fs::metadata(work_items_path) else {
            // works.json absent: the result was synthesized from legacy
            // sources — do not cache it against a missing file.
            self.entries.remove(work_items_path);
            return;
        };
        let Ok(mtime) = meta.modified() else {
            self.entries.remove(work_items_path);
            return;
        };
        self.entries.insert(
            work_items_path.to_path_buf(),
            CachedWorkItemsProjection {
                mtime,
                size: meta.len(),
                projection: projection.clone(),
            },
        );
    }
}

pub fn save_workspace_work_items_projection_to_path(
    path: &Path,
    projection: &WorkItemsProjection,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(projection)
        .map_err(|error| GwtError::Other(format!("workspace work items json: {error}")))?;
    write_atomic(path, &bytes)
}

pub(crate) fn with_workspace_work_items_lock<T>(
    work_items_path: &Path,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    with_workspace_transaction_recovery(
        vec![work_items_path.to_path_buf()],
        vec![pending_workspace_state_transaction_path_for_work_items(
            work_items_path,
        )],
        operation,
    )
}

fn with_workspace_work_items_locks<T>(
    work_items_paths: &[PathBuf],
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let mut lock_paths = work_items_paths
        .iter()
        .map(|path| path.with_extension("lock"))
        .collect::<Vec<_>>();
    lock_paths.sort();
    lock_paths.dedup();
    let mut locks = Vec::with_capacity(lock_paths.len());
    for lock_path in lock_paths {
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        lock.lock_exclusive()?;
        locks.push(lock);
    }
    let result = operation();
    drop(locks);
    result
}

pub(crate) fn with_workspace_current_and_work_items_lock<T>(
    current_path: &Path,
    work_items_path: &Path,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let current_work_items_path = current_path.with_file_name("works.json");
    with_workspace_transaction_recovery(
        vec![current_work_items_path, work_items_path.to_path_buf()],
        vec![
            pending_workspace_state_transaction_path(current_path),
            pending_workspace_state_transaction_path_for_work_items(work_items_path),
        ],
        operation,
    )
}

fn with_workspace_transaction_recovery<T>(
    base_lock_targets: Vec<PathBuf>,
    base_marker_paths: Vec<PathBuf>,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let mut operation = Some(operation);
    loop {
        let mut marker_paths = base_marker_paths.clone();
        marker_paths.extend(discover_pending_workspace_state_transaction_coordinators(
            &base_lock_targets,
        )?);
        marker_paths.sort();
        marker_paths.dedup();
        let discovered = match find_pending_workspace_state_transaction(&marker_paths) {
            Ok(discovered) => discovered,
            Err(GwtError::JsonDecode {
                kind: JsonDecodeKind::Malformed,
                ..
            }) => None,
            Err(error) => return Err(error),
        };

        let mut lock_targets = base_lock_targets.clone();
        if let Some(transaction) = discovered.as_ref() {
            lock_targets.push(transaction.current_path.with_file_name("works.json"));
            lock_targets.push(transaction.work_items_path.clone());
            marker_paths.extend(pending_workspace_state_transaction_paths(transaction));
        }
        lock_targets.sort();
        lock_targets.dedup();
        marker_paths.sort();
        marker_paths.dedup();

        let outcome = with_workspace_work_items_locks(&lock_targets, || {
            marker_paths.extend(discover_pending_workspace_state_transaction_coordinators(
                &lock_targets,
            )?);
            marker_paths.sort();
            marker_paths.dedup();
            loop {
                let pending = match find_pending_workspace_state_transaction(&marker_paths) {
                    Ok(pending) => pending,
                    Err(
                        error @ GwtError::JsonDecode {
                            kind: JsonDecodeKind::Malformed,
                            ..
                        },
                    ) => {
                        quarantine_invalid_pending_workspace_state_transactions(
                            &marker_paths,
                            &error,
                        )?;
                        return Err(error);
                    }
                    Err(error) => return Err(error),
                };
                let Some(transaction) = pending else {
                    break;
                };
                let required_locks = [
                    transaction.current_path.with_file_name("works.json"),
                    transaction.work_items_path.clone(),
                ];
                if required_locks
                    .iter()
                    .any(|required| !lock_targets.iter().any(|locked| locked == required))
                {
                    return Ok(None);
                }
                apply_workspace_state_transaction_locked(
                    &transaction.current_path,
                    &transaction,
                    true,
                )?;
            }
            let operation = operation
                .take()
                .expect("workspace state operation must run exactly once");
            operation().map(Some)
        })?;
        if let Some(result) = outcome {
            return Ok(result);
        }
    }
}

fn find_pending_workspace_state_transaction(
    marker_paths: &[PathBuf],
) -> Result<Option<PendingWorkspaceStateTransaction>> {
    for marker_path in marker_paths {
        let bytes = match fs::read(marker_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let transaction: PendingWorkspaceStateTransaction = serde_json::from_slice(&bytes)
            .map_err(|error| {
                classify_json_decode_error("workspace state transaction json", error)
            })?;
        let valid_marker_paths = pending_workspace_state_transaction_paths(&transaction);
        if !(MIN_WORKSPACE_STATE_TRANSACTION_VERSION..=WORKSPACE_STATE_TRANSACTION_VERSION)
            .contains(&transaction.version)
        {
            return Err(GwtError::JsonDecode {
                context: "workspace state transaction json",
                kind: JsonDecodeKind::IncompatibleSchema,
                message: format!(
                    "unsupported transaction version {} at {}",
                    transaction.version,
                    marker_path.display()
                ),
            });
        }
        if (transaction.version >= 2
            && (transaction.transaction_id.is_none()
                || transaction.current_precondition.is_none()
                || transaction.work_items_precondition.is_none()))
            || !valid_marker_paths.iter().any(|path| path == marker_path)
        {
            return Err(GwtError::JsonDecode {
                context: "workspace state transaction json",
                kind: JsonDecodeKind::Malformed,
                message: format!("invalid current transaction at {}", marker_path.display()),
            });
        }
        return Ok(Some(transaction));
    }
    Ok(None)
}

fn quarantine_invalid_pending_workspace_state_transactions(
    marker_paths: &[PathBuf],
    source_error: &GwtError,
) -> Result<()> {
    let mut quarantined = Vec::new();
    for marker_path in marker_paths {
        if !marker_path.exists() {
            continue;
        }
        match find_pending_workspace_state_transaction(std::slice::from_ref(marker_path)) {
            Ok(_) => continue,
            Err(GwtError::JsonDecode {
                kind: JsonDecodeKind::Malformed,
                ..
            }) => {}
            Err(_) => continue,
        }
        let file_name = marker_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("pending-state-transaction.json");
        let quarantine_path =
            marker_path.with_file_name(format!("{file_name}.corrupt-{}", Uuid::new_v4()));
        fs::rename(marker_path, &quarantine_path)?;
        quarantined.push(quarantine_path);
    }
    if quarantined.is_empty() {
        return Err(GwtError::Other(format!(
            "workspace state transaction could not be quarantined: {source_error}"
        )));
    }
    Err(GwtError::Other(format!(
        "quarantined corrupt workspace state transaction at {}: {source_error}",
        quarantined
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

fn persist_workspace_state_transaction_locked(
    current_path: &Path,
    transaction: &PendingWorkspaceStateTransaction,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(transaction)
        .map_err(|error| GwtError::Other(format!("workspace state transaction json: {error}")))?;
    let coordinator_path = pending_workspace_state_transaction_coordinator_path(transaction);
    let mut marker_paths = pending_workspace_state_transaction_paths(transaction);
    marker_paths.sort_by_key(|path| coordinator_path.as_ref() != Some(path));
    let mut written_markers = Vec::new();
    for marker_path in &marker_paths {
        if let Err(error) = write_atomic(marker_path, &bytes) {
            for written in written_markers {
                let _ = fs::remove_file(written);
            }
            return Err(error);
        }
        written_markers.push(marker_path);
    }
    apply_workspace_state_transaction_locked(current_path, transaction, false)
}

fn apply_workspace_state_transaction_locked(
    current_path: &Path,
    transaction: &PendingWorkspaceStateTransaction,
    recovering: bool,
) -> Result<()> {
    if transaction.current_path != current_path {
        return Err(GwtError::Other(format!(
            "workspace state transaction current path mismatch: {} != {}",
            transaction.current_path.display(),
            current_path.display()
        )));
    }
    if let Some(journal_path) = transaction.journal_path.as_deref() {
        if recovering {
            append_workspace_journal_entries_if_missing(
                journal_path,
                &transaction.journal_entries,
            )?;
        } else {
            for entry in &transaction.journal_entries {
                append_workspace_journal_entry_to_path(journal_path, entry)?;
            }
        }
    }
    if let Some(events_path) = transaction.events_path.as_deref() {
        if recovering {
            append_workspace_work_events_if_missing(events_path, &transaction.events)?;
        } else {
            append_workspace_work_events_to_path(events_path, &transaction.events)?;
        }
    }
    if let Some(work_items) = transaction.work_items.as_ref() {
        let may_write = !recovering
            || workspace_state_snapshot_matches_precondition(
                &transaction.work_items_path,
                transaction.work_items_precondition.as_deref(),
            )?;
        if may_write {
            save_workspace_work_items_projection_to_path(&transaction.work_items_path, work_items)?;
        }
    }
    if !recovering
        || workspace_state_snapshot_matches_precondition(
            current_path,
            transaction.current_precondition.as_deref(),
        )?
    {
        save_workspace_projection_to_path_unlocked(current_path, &transaction.projection)?;
    }
    let coordinator_path = pending_workspace_state_transaction_coordinator_path(transaction);
    let mut marker_paths = pending_workspace_state_transaction_paths(transaction);
    marker_paths.sort_by_key(|path| coordinator_path.as_ref() == Some(path));
    for marker_path in marker_paths {
        match fs::remove_file(marker_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn workspace_state_snapshot_matches_precondition(
    path: &Path,
    precondition: Option<&str>,
) -> Result<bool> {
    let Some(precondition) = precondition else {
        return Ok(true);
    };
    Ok(workspace_state_file_fingerprint(path)? == precondition)
}

fn existing_jsonl_ids(path: &Path) -> Result<HashSet<String>> {
    repair_jsonl_tail(path)?;
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(error) => return Err(error.into()),
    };
    Ok(content
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
        .filter_map(|value| {
            value
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .collect())
}

fn repair_jsonl_tail(path: &Path) -> Result<()> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if bytes.is_empty() || bytes.last() == Some(&b'\n') {
        return Ok(());
    }

    let tail_start = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let mut file = fs::OpenOptions::new().append(true).open(path)?;
    if serde_json::from_slice::<serde_json::Value>(&bytes[tail_start..]).is_ok() {
        file.write_all(b"\n")?;
    } else {
        file.set_len(tail_start as u64)?;
    }
    file.sync_all()?;
    Ok(())
}

fn append_workspace_journal_entries_if_missing(
    path: &Path,
    entries: &[WorkspaceJournalEntry],
) -> Result<()> {
    let mut seen = existing_jsonl_ids(path)?;
    for entry in entries {
        if seen.insert(entry.id.clone()) {
            append_workspace_journal_entry_to_path(path, entry)?;
        }
    }
    Ok(())
}

fn append_workspace_work_events_if_missing(path: &Path, events: &[WorkEvent]) -> Result<()> {
    let mut seen = existing_jsonl_ids(path)?;
    let missing = events
        .iter()
        .filter(|event| seen.insert(event.id.clone()))
        .cloned()
        .collect::<Vec<_>>();
    append_workspace_work_events_to_path(path, &missing)
}

pub fn record_workspace_work_event(repo_path: &Path, event: WorkEvent) -> Result<()> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    let events_path = repo_local_work_events_path_with_migration(repo_path)?;
    record_workspace_work_event_paths(&work_items_path, &events_path, event)
}

pub fn record_workspace_work_event_paths(
    work_items_path: &Path,
    events_path: &Path,
    event: WorkEvent,
) -> Result<()> {
    record_workspace_work_events_paths(work_items_path, events_path, vec![event])
}

pub fn record_workspace_work_events_paths(
    work_items_path: &Path,
    events_path: &Path,
    events: Vec<WorkEvent>,
) -> Result<()> {
    with_workspace_work_items_lock(work_items_path, || {
        let initial_updated_at = events
            .first()
            .map(|event| event.updated_at)
            .unwrap_or_else(Utc::now);
        let mut projection = load_workspace_work_items_from_path(work_items_path)?
            .unwrap_or_else(|| WorkItemsProjection::empty(initial_updated_at));
        persist_workspace_work_events_locked(
            work_items_path,
            events_path,
            &mut projection,
            events,
        )?;
        Ok(())
    })
}

fn persist_workspace_work_events_locked(
    work_items_path: &Path,
    events_path: &Path,
    projection: &mut WorkItemsProjection,
    events: Vec<WorkEvent>,
) -> Result<usize> {
    if events.is_empty() {
        return Ok(0);
    }
    let mut candidate = projection.clone();
    for event in &events {
        if candidate.apply_event(event.clone()) == WorkEventApplyOutcome::RejectedSessionConflict {
            return Ok(0);
        }
    }
    append_workspace_work_events_to_path(events_path, &events)?;
    *projection = candidate;
    save_workspace_work_items_projection_to_path(work_items_path, projection)?;
    Ok(events.len())
}

/// SPEC-2359 Phase W-15 (FR-379/FR-381): one locally existing worktree as
/// reconcile input. `branch == None` (detached worktree) is never backfilled,
/// which encodes FR-381 — records are only generated for branches that have a
/// real working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeReconcileSource {
    pub branch: Option<String>,
    pub worktree_path: PathBuf,
}

/// SPEC-2359 Phase W-15 (FR-380 idempotency / SC-255): pure decision step of
/// worktree reconciliation. Returns the `(canonical work id, source)` pairs
/// that still need a Backfill record. A source is skipped when any existing
/// item already covers it: matching id, matching execution-container branch
/// (compared via the canonical branch identity, so `origin/x` == `x`), or
/// matching worktree path. Terminal (Done/Discarded) items also match here,
/// which keeps closed Work closed (US-61 — backfill never re-opens).
pub fn worktree_sources_needing_backfill(
    projection: &WorkItemsProjection,
    project_root: &Path,
    sources: &[WorktreeReconcileSource],
) -> Vec<(String, WorktreeReconcileSource)> {
    let mut pending = Vec::new();
    for source in sources {
        let Some(branch) = source
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(work_id) = canonical_work_id(project_root, Some(branch), None) else {
            continue;
        };
        let branch_identity = canonical_work_branch_identity(branch);
        let worktree_identity = canonical_worktree_identity(&source.worktree_path);
        let covered = projection.work_items.iter().any(|item| {
            item.id == work_id
                || item.execution_containers.iter().any(|container| {
                    container.branch.as_deref().is_some_and(|existing| {
                        canonical_work_branch_identity(existing) == branch_identity
                    }) || container.worktree_path.as_deref().is_some_and(|existing| {
                        canonical_worktree_identity(existing) == worktree_identity
                    })
                })
        });
        if covered || pending.iter().any(|(id, _)| *id == work_id) {
            continue;
        }
        pending.push((work_id, source.clone()));
    }
    pending
}

/// SPEC-2359 Phase W-15 (FR-380): record a single Backfill event for a
/// worktree that has no Work record. `status_category` stays `None` so a
/// re-ingested copy of this event (W-16 intake on another machine) cannot
/// regress a terminal item; the Idle surface state comes from the kind
/// mapping in `workspace_work_event_status`.
pub fn record_workspace_backfill_event_paths(
    work_items_path: &Path,
    events_path: &Path,
    work_id: &str,
    branch: &str,
    worktree_path: &Path,
    updated_at: DateTime<Utc>,
) -> Result<()> {
    let mut event = WorkEvent::new(WorkEventKind::Backfill, work_id, updated_at);
    event.title = Some(branch.to_string());
    event.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some(branch.to_string()),
        worktree_path: Some(worktree_path.to_path_buf()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    record_workspace_work_event_paths(work_items_path, events_path, event)
}

/// #3065: find the Work item that owns a given execution container. The
/// match mirrors the backfill coverage rule: canonical work id, canonical
/// branch identity (`origin/x` == `x`), or canonical worktree path. Used to
/// source the Workspace Resume context from the resumed Work itself instead
/// of the repo-shared current projection (whose identity may belong to a
/// different Work).
pub fn find_work_item_for_container<'a>(
    projection: &'a WorkItemsProjection,
    project_root: &Path,
    branch: Option<&str>,
    worktree_path: Option<&Path>,
) -> Option<&'a WorkItem> {
    let branch = branch.map(str::trim).filter(|value| !value.is_empty());
    let canonical_id = canonical_work_id(project_root, branch, worktree_path);
    let branch_identity = branch.map(canonical_work_branch_identity);
    let worktree_identity = worktree_path.map(canonical_worktree_identity);
    projection.work_items.iter().find(|item| {
        canonical_id
            .as_deref()
            .is_some_and(|work_id| item.id == work_id)
            || item.execution_containers.iter().any(|container| {
                branch_identity.as_deref().is_some_and(|identity| {
                    container.branch.as_deref().is_some_and(|existing| {
                        canonical_work_branch_identity(existing) == identity
                    })
                }) || worktree_identity.as_deref().is_some_and(|identity| {
                    container
                        .worktree_path
                        .as_deref()
                        .is_some_and(|existing| canonical_worktree_identity(existing) == identity)
                })
            })
    })
}

/// SPEC-2359 Phase W-15 (FR-379/FR-380): reconcile locally existing worktrees
/// against the Work records and backfill the missing ones. Each Backfill
/// event is appended to the *owning worktree's* repo-local event log
/// (`<worktree>/.gwt/work/events.jsonl`) so it travels with that branch, and
/// applied to the shared home works projection. Returns the number of
/// backfilled worktrees.
pub fn reconcile_worktree_work_items_paths(
    work_items_path: &Path,
    project_root: &Path,
    sources: &[WorktreeReconcileSource],
    now: DateTime<Utc>,
) -> Result<usize> {
    let projection = load_workspace_work_items_from_path(work_items_path)?
        .unwrap_or_else(|| WorkItemsProjection::empty(now));
    let pending = worktree_sources_needing_backfill(&projection, project_root, sources);
    for (work_id, source) in &pending {
        let Some(branch) = source.branch.as_deref() else {
            continue;
        };
        let events_path = gwt_repo_local_work_events_path(&source.worktree_path);
        // FR-403: the baseline timestamp is the worktree's last real activity
        // — the HEAD committer time for git worktrees (directory mtime is
        // polluted by unrelated writes such as the backfill itself creating
        // `.gwt/`), falling back to the directory mtime and finally to `now`.
        // Reconciliation runs at bootstrap/open (not on the projection build
        // hot path), so one git spawn per newly backfilled worktree is fine.
        let baseline = worktree_head_commit_time(&source.worktree_path)
            .or_else(|| {
                fs::metadata(&source.worktree_path)
                    .and_then(|metadata| metadata.modified())
                    .map(DateTime::<Utc>::from)
                    .ok()
            })
            .unwrap_or(now)
            .min(now);
        record_workspace_backfill_event_paths(
            work_items_path,
            &events_path,
            work_id,
            branch,
            &source.worktree_path,
            baseline,
        )?;
    }
    Ok(pending.len())
}

/// HEAD committer time of a git worktree (`git log -1 --format=%ct`).
/// Returns `None` for non-repositories or unborn branches.
fn worktree_head_commit_time(worktree_path: &Path) -> Option<DateTime<Utc>> {
    let output = crate::process::hidden_command("git")
        .arg("-C")
        .arg(worktree_path)
        .args(["log", "-1", "--format=%ct"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let seconds: i64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    DateTime::<Utc>::from_timestamp(seconds, 0)
}

/// SPEC-2359 Phase W-16 (FR-393): decompose legacy mega-items. The pre-W-12
/// implementation keyed journal/board events to the projection's single UUID
/// (or `workspace-<millis>`), fusing dozens of branches and thousands of
/// sessions into one Work record. Each event still carries its own
/// `execution_container.branch`, so any item whose events span two or more
/// canonical branch identities is re-keyed here: its events are replayed per
/// branch into canonical branch-derived items (`canonical_work_id`), titles
/// and agents follow each branch's events, and the legacy shell (including
/// branchless heartbeat events) is dropped. Canonical/per-session items have
/// a single branch, so a second run finds nothing to decompose (idempotent —
/// no marker file needed). Returns the number of decomposed legacy items.
pub fn decompose_legacy_multi_branch_work_items_paths(
    work_items_path: &Path,
    project_root: &Path,
) -> Result<usize> {
    with_workspace_work_items_lock(work_items_path, || {
        decompose_legacy_multi_branch_work_items_paths_locked(work_items_path, project_root)
    })
}

fn decompose_legacy_multi_branch_work_items_paths_locked(
    work_items_path: &Path,
    project_root: &Path,
) -> Result<usize> {
    let Some(mut projection) = load_workspace_work_items_from_path(work_items_path)? else {
        return Ok(0);
    };
    let mut decomposed = 0usize;
    let mut replacement: Vec<WorkItem> = Vec::new();
    let mut pending_events: Vec<WorkEvent> = Vec::new();
    for item in projection.work_items.drain(..) {
        let mut branch_identities: Vec<String> = Vec::new();
        for event in &item.events {
            if let Some(branch) = event
                .execution_container
                .as_ref()
                .and_then(|container| container.branch.as_deref())
                .map(canonical_work_branch_identity)
            {
                if !branch_identities.contains(&branch) {
                    branch_identities.push(branch);
                }
            }
        }
        if branch_identities.len() < 2 {
            replacement.push(item);
            continue;
        }
        decomposed += 1;
        for event in &item.events {
            let Some(branch) = event
                .execution_container
                .as_ref()
                .and_then(|container| container.branch.as_deref())
            else {
                // Branchless heartbeat of the legacy shell — dropped with it.
                continue;
            };
            let Some(work_id) = canonical_work_id(project_root, Some(branch), None) else {
                continue;
            };
            let mut event = event.clone();
            event.work_item_id = work_id;
            pending_events.push(event);
        }
    }
    if decomposed == 0 {
        projection.work_items = replacement;
        return Ok(0);
    }
    pending_events.sort_by_key(|event| event.updated_at);
    projection.work_items = replacement;
    for event in pending_events {
        projection.apply_event(event);
    }
    projection.updated_at = chrono::Utc::now();
    save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    Ok(decomposed)
}

/// Convenience wrapper resolving the home works projection for `repo_path`
/// (with the legacy migration applied), then delegating to
/// [`decompose_legacy_multi_branch_work_items_paths`].
pub fn decompose_legacy_multi_branch_work_items(repo_path: &Path) -> Result<usize> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    decompose_legacy_multi_branch_work_items_paths(&work_items_path, repo_path)
}

/// Convenience wrapper resolving the home works projection for `repo_path`
/// (with the legacy migration applied), then delegating to
/// [`reconcile_worktree_work_items_paths`].
pub fn reconcile_worktree_work_items(
    repo_path: &Path,
    sources: &[WorktreeReconcileSource],
    now: DateTime<Utc>,
) -> Result<usize> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    reconcile_worktree_work_items_paths(&work_items_path, repo_path, sources, now)
}

/// #3065: report of one resume-owner-bleed repair pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumeOwnerBleedRepairReport {
    /// Resume events whose stamped identity fields were cleared.
    pub sanitized_events: usize,
    /// Whether the shared current projection's identity was cleared.
    pub cleared_current: bool,
    /// Stray agents pruned from the shared current projection.
    pub pruned_current_agents: usize,
}

impl ResumeOwnerBleedRepairReport {
    pub fn changed(&self) -> bool {
        self.sanitized_events > 0 || self.cleared_current || self.pruned_current_agents > 0
    }
}

/// #3065: minimum number of distinct Work items sharing one identical resume
/// payload before it is treated as a bleed signature. Two branches may
/// legitimately resume the same SPEC with the same wording; three or more
/// identical (title, owner, next_action) stamps across different Work items
/// only arise from the shared-projection replay bug.
const RESUME_OWNER_BLEED_MIN_ITEMS: usize = 3;

fn resume_bleed_key(event: &WorkEvent) -> Option<(String, String, String)> {
    if event.kind != WorkEventKind::Resume {
        return None;
    }
    let owner = event
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let title = event.title.as_deref().map(str::trim).unwrap_or("");
    let next_action = event.next_action.as_deref().map(str::trim).unwrap_or("");
    Some((
        title.to_string(),
        owner.to_string(),
        next_action.to_string(),
    ))
}

/// #3065: the contaminated identity pair carried by one bleed event —
/// `(title, owner)`, both trimmed, owner non-empty.
fn bleed_identity_pair(event: &WorkEvent) -> Option<(String, String)> {
    let owner = event
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let title = event.title.as_deref().map(str::trim).unwrap_or("");
    Some((title.to_string(), owner.to_string()))
}

/// #3065: detection-based, idempotent repair for the resume owner bleed.
/// Detection: an identical (title, owner, next_action) resume payload stamped
/// onto `RESUME_OWNER_BLEED_MIN_ITEMS`+ distinct Work items. Sanitization,
/// in two strengths:
///
/// - events carrying a contaminated (title, owner) identity pair — the same
///   shared-projection snapshot also leaked through pause / update stamps —
///   have ALL identity fields cleared;
/// - events carrying a contaminated owner VALUE with a different title (the
///   update/done/pause leak stamped the poisoned owner alongside an
///   agent-authored title) lose only their owner.
///
/// Event ids are kept so the intake dedup still skips re-ingestion, and the
/// Work items are re-folded from their events. The shared current projection
/// is cleared when its (title, owner) pair carries the contamination (full
/// clear) or its owner value alone does (owner-only clear). Stray agents
/// assigned to a different work id are pruned from the current projection in
/// the same pass. Runs after every work-events ingest; converges to a no-op
/// once the data is clean.
pub fn repair_resume_owner_bleed_paths(
    work_items_path: &Path,
    current_projection_path: &Path,
    now: DateTime<Utc>,
) -> Result<ResumeOwnerBleedRepairReport> {
    with_workspace_current_and_work_items_lock(current_projection_path, work_items_path, || {
        repair_resume_owner_bleed_paths_locked(work_items_path, current_projection_path, now)
    })
}

fn repair_resume_owner_bleed_paths_locked(
    work_items_path: &Path,
    current_projection_path: &Path,
    now: DateTime<Utc>,
) -> Result<ResumeOwnerBleedRepairReport> {
    use std::collections::HashSet;

    let mut report = ResumeOwnerBleedRepairReport::default();
    let Some(mut works) = load_workspace_work_items_from_path(work_items_path)? else {
        return Ok(report);
    };

    let mut stamped: HashMap<(String, String, String), HashSet<String>> = HashMap::new();
    for item in &works.work_items {
        for event in &item.events {
            if let Some(key) = resume_bleed_key(event) {
                stamped.entry(key).or_default().insert(item.id.clone());
            }
        }
    }
    let contaminated_pairs: HashSet<(String, String)> = stamped
        .into_iter()
        .filter(|(_, items)| items.len() >= RESUME_OWNER_BLEED_MIN_ITEMS)
        .map(|((title, owner, _), _)| (title, owner))
        .collect();
    let contaminated_owners: HashSet<String> = contaminated_pairs
        .iter()
        .map(|(_, owner)| owner.clone())
        .collect();

    if !contaminated_pairs.is_empty() {
        let projection_updated_at = works.updated_at;
        let mut all_events: Vec<WorkEvent> = Vec::new();
        let mut eventless_items: Vec<WorkItem> = Vec::new();
        for item in works.work_items.drain(..) {
            if item.events.is_empty() {
                eventless_items.push(item);
                continue;
            }
            all_events.extend(item.events);
        }
        let mut sanitized = 0usize;
        for event in &mut all_events {
            let Some(pair) = bleed_identity_pair(event) else {
                continue;
            };
            if contaminated_pairs.contains(&pair) {
                event.title = None;
                event.intent = None;
                event.summary = None;
                event.owner = None;
                event.next_action = None;
                sanitized += 1;
            } else if contaminated_owners.contains(&pair.1) {
                event.owner = None;
                sanitized += 1;
            }
        }
        all_events.sort_by_key(|event| event.updated_at);
        let mut rebuilt = WorkItemsProjection::empty(projection_updated_at);
        for event in all_events {
            rebuilt.apply_event(event);
        }
        rebuilt.work_items.extend(eventless_items);
        if projection_updated_at > rebuilt.updated_at {
            rebuilt.updated_at = projection_updated_at;
        }
        save_workspace_work_items_projection_to_path(work_items_path, &rebuilt)?;
        report.sanitized_events = sanitized;
    }

    let Some(mut current) = load_workspace_projection_from_path(current_projection_path)? else {
        return Ok(report);
    };
    let mut current_changed = false;
    let current_pair = (
        current.title.trim().to_string(),
        current
            .owner
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string(),
    );
    if !current_pair.1.is_empty() && contaminated_pairs.contains(&current_pair) {
        current.title = "Work".to_string();
        current.owner = None;
        current.summary = None;
        current.next_action = None;
        current.agents.clear();
        current.status_category = WorkspaceStatusCategory::Idle;
        current.status_text = "No active work".to_string();
        current.updated_at = now;
        report.cleared_current = true;
        current_changed = true;
    } else {
        if !current_pair.1.is_empty() && contaminated_owners.contains(&current_pair.1) {
            // The title drifted (agent-authored) but the owner value is the
            // contaminated one — drop only the owner.
            current.owner = None;
            current.updated_at = now;
            report.cleared_current = true;
            current_changed = true;
        }
        let before = current.agents.len();
        let current_id = current.id.clone();
        current.agents.retain(|agent| {
            agent
                .workspace_id
                .as_deref()
                .is_none_or(|assigned| assigned == current_id)
        });
        let pruned = before - current.agents.len();
        if pruned > 0 {
            report.pruned_current_agents = pruned;
            current.updated_at = now;
            current_changed = true;
        }
    }
    if current_changed {
        save_workspace_projection_to_path_unlocked(current_projection_path, &current)?;
    }
    Ok(report)
}

/// #3065: repo-path wrapper for [`repair_resume_owner_bleed_paths`] resolving
/// the canonical home-projection paths.
pub fn repair_resume_owner_bleed_for_repo(
    repo_path: &Path,
    now: DateTime<Utc>,
) -> Result<ResumeOwnerBleedRepairReport> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    repair_resume_owner_bleed_paths(&work_items_path, &current_path, now)
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): record a `Pause` work event so a
/// Work whose owning agent session stopped is retained in the work history
/// (and on the Work surface) until the user explicitly closes it. `work_item_id`
/// is the session-derived canonical Work id (`work-session-<session_id>`), which
/// matches the live-agent grouping id so a later resume dedupes to one row.
/// Idempotent for already-closed (Done) Work: the `apply_event` Done-preservation
/// keeps a terminal Work terminal because the Pause event carries no explicit
/// `status_category`.
#[allow(clippy::too_many_arguments)]
pub fn record_workspace_work_paused_event_paths(
    work_items_path: &Path,
    events_path: &Path,
    work_item_id: &str,
    title: Option<&str>,
    summary: Option<&str>,
    owner: Option<&str>,
    board_refs: &[String],
    execution_container: Option<WorkspaceExecutionContainerRef>,
    agent_session_id: Option<&str>,
    updated_at: DateTime<Utc>,
) -> Result<()> {
    let mut event = WorkEvent::new(WorkEventKind::Pause, work_item_id, updated_at);
    event.title = non_empty_clone(title);
    event.summary = non_empty_clone(summary);
    event.owner = non_empty_clone(owner);
    event.agent_session_id = non_empty_clone(agent_session_id);
    event.execution_container = execution_container;
    // Pause must not regress a terminal Work, so leave status_category implicit;
    // record the board refs (if any) so the retained row keeps its provenance.
    let mut events = Vec::with_capacity(board_refs.len() + 1);
    for board_ref in board_refs {
        if let Some(board_ref) = non_empty_clone(Some(board_ref.as_str())) {
            let mut ref_event = WorkEvent::new(WorkEventKind::Update, work_item_id, updated_at);
            ref_event.board_entry_id = Some(board_ref);
            events.push(ref_event);
        }
    }
    events.push(event);
    record_workspace_work_events_paths(work_items_path, events_path, events)
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): convenience wrapper resolving the
/// project-scoped work_items and close-event paths from `repo_path` and
/// invoking [`record_workspace_work_paused_event_paths`].
///
/// SPEC-2359 Phase W-15 (FR-384): Pause is a close-kind event, so it is
/// home-persisted only (`work-events-closed.jsonl`) and never enters the
/// git-tracked repo-local log.
#[allow(clippy::too_many_arguments)]
pub fn record_workspace_work_paused_event(
    repo_path: &Path,
    work_item_id: &str,
    title: Option<&str>,
    summary: Option<&str>,
    owner: Option<&str>,
    board_refs: &[String],
    execution_container: Option<WorkspaceExecutionContainerRef>,
    agent_session_id: Option<&str>,
    updated_at: DateTime<Utc>,
) -> Result<()> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    let events_path = gwt_workspace_work_events_closed_path_for_repo_path(repo_path);
    record_workspace_work_paused_event_paths(
        &work_items_path,
        &events_path,
        work_item_id,
        title,
        summary,
        owner,
        board_refs,
        execution_container,
        agent_session_id,
        updated_at,
    )
}

/// SPEC-2359 US-37 / FR-117..FR-120: Emit a single Done `WorkEvent`
/// for `work_item_id` iff its current projection is not terminal. This is the
/// canonical write path for auto-done emission from PR merge detection,
/// user-confirmed cleanup, and startup retroactive migration. A Work that was
/// explicitly reopened after an earlier Done may receive a new Done event;
/// retries while it remains terminal are idempotent noops.
pub fn emit_workspace_done_event_if_absent_paths(
    work_items_path: &Path,
    events_path: &Path,
    work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    let mut event = WorkEvent::new(WorkEventKind::Done, work_item_id, updated_at);
    event.status_category = Some(WorkspaceStatusCategory::Done);
    emit_workspace_terminal_event_if_absent_paths(work_items_path, events_path, event)
}

/// SPEC-2359 US-37 / FR-117..FR-120: Convenience wrapper resolving the
/// project-scoped work_items and work_events paths from `repo_path` and
/// invoking [`emit_workspace_done_event_if_absent_paths`].
/// SPEC-2359 Phase W-15 (FR-384): Done is a close-kind event — home-persisted
/// only, never written to the git-tracked repo-local log.
pub fn emit_workspace_done_event_if_absent(
    repo_path: &Path,
    work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    emit_workspace_done_event_if_absent_paths(
        &gwt_workspace_work_items_path_for_repo_path(repo_path),
        &gwt_workspace_work_events_closed_path_for_repo_path(repo_path),
        work_item_id,
        updated_at,
    )
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): Emit a single Discard
/// `WorkEvent` for `work_item_id` iff the Work is not already
/// terminal (Done or already Discarded). This is the canonical write path for a
/// user-initiated Discard close from the Work surface. Returns `Ok(true)` when
/// a new Discard event was appended, `Ok(false)` when the Work was already
/// terminal (idempotent noop so a re-close does nothing).
pub fn emit_workspace_discard_event_if_absent_paths(
    work_items_path: &Path,
    events_path: &Path,
    work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    let event = WorkEvent::new(WorkEventKind::Discard, work_item_id, updated_at);
    emit_workspace_terminal_event_if_absent_paths(work_items_path, events_path, event)
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): Convenience wrapper resolving the
/// project-scoped work_items and work_events paths from `repo_path` and
/// invoking [`emit_workspace_discard_event_if_absent_paths`].
/// SPEC-2359 Phase W-15 (FR-384): Discard is a close-kind event —
/// home-persisted only, never written to the git-tracked repo-local log.
pub fn emit_workspace_discard_event_if_absent(
    repo_path: &Path,
    work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    emit_workspace_discard_event_if_absent_paths(
        &gwt_workspace_work_items_path_for_repo_path(repo_path),
        &gwt_workspace_work_events_closed_path_for_repo_path(repo_path),
        work_item_id,
        updated_at,
    )
}

/// Resolve the latest Session assignment and emit Done while holding the
/// Work projection transaction lock. `legacy_work_item_id` is used only when
/// no Session row exists; an explicit Unassigned row disables fallback.
#[allow(clippy::too_many_arguments)]
pub fn emit_workspace_done_event_for_session_paths(
    current_path: &Path,
    work_items_path: &Path,
    events_path: &Path,
    session_id: &str,
    legacy_work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    Ok(matches!(
        emit_workspace_done_event_for_session_outcome_paths(
            current_path,
            work_items_path,
            events_path,
            session_id,
            legacy_work_item_id,
            updated_at,
        )?,
        WorkspaceTerminalEventOutcome::Emitted
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn emit_workspace_done_event_for_session_outcome_paths(
    current_path: &Path,
    work_items_path: &Path,
    events_path: &Path,
    session_id: &str,
    legacy_work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<WorkspaceTerminalEventOutcome> {
    let mut event = WorkEvent::new(WorkEventKind::Done, legacy_work_item_id, updated_at);
    event.status_category = Some(WorkspaceStatusCategory::Done);
    emit_workspace_terminal_event_for_session_paths(
        current_path,
        work_items_path,
        events_path,
        session_id,
        legacy_work_item_id,
        event,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn emit_workspace_discard_event_for_session_paths(
    current_path: &Path,
    work_items_path: &Path,
    events_path: &Path,
    session_id: &str,
    legacy_work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    Ok(matches!(
        emit_workspace_discard_event_for_session_outcome_paths(
            current_path,
            work_items_path,
            events_path,
            session_id,
            legacy_work_item_id,
            updated_at,
        )?,
        WorkspaceTerminalEventOutcome::Emitted
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn emit_workspace_discard_event_for_session_outcome_paths(
    current_path: &Path,
    work_items_path: &Path,
    events_path: &Path,
    session_id: &str,
    legacy_work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<WorkspaceTerminalEventOutcome> {
    let event = WorkEvent::new(WorkEventKind::Discard, legacy_work_item_id, updated_at);
    emit_workspace_terminal_event_for_session_paths(
        current_path,
        work_items_path,
        events_path,
        session_id,
        legacy_work_item_id,
        event,
    )
}

pub fn emit_workspace_done_event_for_session(
    project_state_root: &Path,
    work_event_root: &Path,
    session_id: &str,
    legacy_work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    Ok(matches!(
        emit_workspace_done_event_for_session_outcome(
            project_state_root,
            work_event_root,
            session_id,
            legacy_work_item_id,
            updated_at,
        )?,
        WorkspaceTerminalEventOutcome::Emitted
    ))
}

pub fn emit_workspace_done_event_for_session_outcome(
    project_state_root: &Path,
    work_event_root: &Path,
    session_id: &str,
    legacy_work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<WorkspaceTerminalEventOutcome> {
    let current_path = gwt_workspace_projection_path_for_repo_path(project_state_root);
    let _ = migrate_legacy_workspace_projection(project_state_root, &current_path)?;
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(work_event_root);
    let _ = migrate_legacy_workspace_work_items(work_event_root, &work_items_path)?;
    emit_workspace_done_event_for_session_outcome_paths(
        &current_path,
        &work_items_path,
        &gwt_workspace_work_events_closed_path_for_repo_path(work_event_root),
        session_id,
        legacy_work_item_id,
        updated_at,
    )
}

pub fn emit_workspace_discard_event_for_session(
    project_state_root: &Path,
    work_event_root: &Path,
    session_id: &str,
    legacy_work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    Ok(matches!(
        emit_workspace_discard_event_for_session_outcome(
            project_state_root,
            work_event_root,
            session_id,
            legacy_work_item_id,
            updated_at,
        )?,
        WorkspaceTerminalEventOutcome::Emitted
    ))
}

pub fn emit_workspace_discard_event_for_session_outcome(
    project_state_root: &Path,
    work_event_root: &Path,
    session_id: &str,
    legacy_work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<WorkspaceTerminalEventOutcome> {
    let current_path = gwt_workspace_projection_path_for_repo_path(project_state_root);
    let _ = migrate_legacy_workspace_projection(project_state_root, &current_path)?;
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(work_event_root);
    let _ = migrate_legacy_workspace_work_items(work_event_root, &work_items_path)?;
    emit_workspace_discard_event_for_session_outcome_paths(
        &current_path,
        &work_items_path,
        &gwt_workspace_work_events_closed_path_for_repo_path(work_event_root),
        session_id,
        legacy_work_item_id,
        updated_at,
    )
}

fn emit_workspace_terminal_event_for_session_paths(
    current_path: &Path,
    work_items_path: &Path,
    events_path: &Path,
    session_id: &str,
    legacy_work_item_id: &str,
    mut event: WorkEvent,
) -> Result<WorkspaceTerminalEventOutcome> {
    with_workspace_current_and_work_items_lock(current_path, work_items_path, || {
        let assignment = load_workspace_projection_from_path(current_path)?
            .as_ref()
            .map(|projection| workspace_assignment_for_session(projection, session_id))
            .unwrap_or(WorkspaceSessionAssignment::Missing);
        let (target, assigned_target) = match assignment {
            WorkspaceSessionAssignment::Assigned(work_id) => (work_id, true),
            WorkspaceSessionAssignment::Unassigned => {
                return Ok(WorkspaceTerminalEventOutcome::NoTarget)
            }
            WorkspaceSessionAssignment::Missing => (legacy_work_item_id.to_string(), false),
        };
        event.work_item_id = target;
        emit_workspace_terminal_event_outcome_locked(
            work_items_path,
            events_path,
            event,
            assigned_target,
            false,
        )
    })
}

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): true when `work_item_id` is already
/// in a terminal close state (Done or discarded) in the saved projection. Used
/// to make Done / Discard close emission idempotent.
fn emit_workspace_terminal_event_if_absent_paths(
    work_items_path: &Path,
    events_path: &Path,
    event: WorkEvent,
) -> Result<bool> {
    emit_workspace_terminal_event_if_absent_paths_inner(work_items_path, events_path, event, false)
}

fn emit_workspace_terminal_event_if_absent_paths_inner(
    work_items_path: &Path,
    events_path: &Path,
    event: WorkEvent,
    allow_missing_target: bool,
) -> Result<bool> {
    with_workspace_work_items_lock(work_items_path, || {
        emit_workspace_terminal_event_if_absent_locked(
            work_items_path,
            events_path,
            event,
            allow_missing_target,
        )
    })
}

fn emit_workspace_terminal_event_if_absent_locked(
    work_items_path: &Path,
    events_path: &Path,
    event: WorkEvent,
    allow_missing_target: bool,
) -> Result<bool> {
    Ok(matches!(
        emit_workspace_terminal_event_outcome_locked(
            work_items_path,
            events_path,
            event,
            false,
            allow_missing_target,
        )?,
        WorkspaceTerminalEventOutcome::Emitted
    ))
}

fn emit_workspace_terminal_event_outcome_locked(
    work_items_path: &Path,
    events_path: &Path,
    event: WorkEvent,
    assigned_target: bool,
    allow_missing_target: bool,
) -> Result<WorkspaceTerminalEventOutcome> {
    let mut projection = load_workspace_work_items_from_path(work_items_path)?
        .unwrap_or_else(|| WorkItemsProjection::empty(event.updated_at));
    let recovered = recover_unprojected_workspace_work_events_locked(&mut projection, events_path)?;
    let item = projection
        .work_items
        .iter()
        .find(|item| item.id == event.work_item_id);
    if item.is_none() && !allow_missing_target {
        if recovered {
            save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
        }
        return Ok(if assigned_target {
            WorkspaceTerminalEventOutcome::AssignedWorkMissing(event.work_item_id)
        } else {
            WorkspaceTerminalEventOutcome::NoTarget
        });
    }
    let outcome = item.and_then(|item| {
        if item.status_category == WorkspaceStatusCategory::Done && item.discarded {
            Some(WorkspaceTerminalEventOutcome::AmbiguousTerminal)
        } else {
            let matches_requested_terminal = match event.kind {
                WorkEventKind::Done => {
                    item.status_category == WorkspaceStatusCategory::Done && !item.discarded
                }
                WorkEventKind::Discard => item.discarded,
                _ => false,
            };
            if matches_requested_terminal {
                Some(WorkspaceTerminalEventOutcome::AlreadyMatching)
            } else if item.is_terminal() {
                Some(WorkspaceTerminalEventOutcome::WrongTerminal)
            } else {
                None
            }
        }
    });
    if let Some(outcome) = outcome {
        if recovered {
            save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
        }
        return Ok(outcome);
    }
    persist_workspace_work_events_locked(
        work_items_path,
        events_path,
        &mut projection,
        vec![event],
    )?;
    Ok(WorkspaceTerminalEventOutcome::Emitted)
}

fn recover_unprojected_workspace_work_events_locked(
    projection: &mut WorkItemsProjection,
    events_path: &Path,
) -> Result<bool> {
    let records = read_workspace_work_event_records_from_path(events_path)?;
    let seen_event_ids = projection
        .work_items
        .iter()
        .flat_map(|item| item.events.iter().map(|event| event.id.clone()))
        .collect::<HashSet<_>>();
    let mut durable_events = Vec::new();
    for record in records {
        let Some(event) = record.into_known_event() else {
            continue;
        };
        if seen_event_ids.contains(&event.id) {
            continue;
        }
        durable_events.push(event);
    }
    if durable_events.is_empty() {
        return Ok(false);
    }

    let rebuilt =
        crate::work_events_intake::refold_work_events_projection(projection, durable_events)?;
    let changed = rebuilt.work_items != projection.work_items;
    if changed {
        *projection = rebuilt;
    }
    Ok(changed)
}

/// SPEC-2359 US-37 / FR-119: Scan WorkItems and the current Workspace
/// projection and emit a Done event for each eligible target that is still
/// incomplete and not yet recorded as Done.
///
/// Two eligibility sources are consulted:
///
/// 1. Each WorkItem in `work_items.json`: at least one
///    [`WorkspaceExecutionContainerRef`] whose `branch` starts with `"work/"`
///    and whose `pr_state` matches `"merged"` case-insensitively.
/// 2. The current Workspace projection at `current_path`: `git_details.branch`
///    starts with `"work/"`, `pr_state` matches `"merged"` case-insensitively,
///    and `created_by_start_work` is true. This catches the case where an
///    older gwtd version updated `current.json` after a PR merged but never
///    emitted a corresponding Done event, which would otherwise leave the
///    WorkItem stuck outside the Completed column.
///
/// Emission is delegated to [`emit_workspace_done_event_if_absent_paths`] so
/// repeated invocations are idempotent. Returns the number of WorkItems that
/// were newly Done'd. Missing files (`work_items.json`, `current.json`) skip
/// silently without surfacing an error.
pub fn retroactive_auto_done_scan_paths(
    current_path: &Path,
    work_items_path: &Path,
    events_path: &Path,
    now: DateTime<Utc>,
) -> Result<usize> {
    let mut emitted = 0;

    if let Some(work_items_projection) = load_workspace_work_items_from_path(work_items_path)? {
        let candidates: Vec<String> = work_items_projection
            .work_items
            .iter()
            .filter(|item| item.is_incomplete())
            .filter(|item| work_item_is_eligible_for_auto_done(item))
            .map(|item| item.id.clone())
            .collect();
        for work_item_id in candidates {
            if emit_workspace_done_event_if_absent_paths(
                work_items_path,
                events_path,
                &work_item_id,
                now,
            )? {
                emitted += 1;
            }
        }
    }

    if let Some(current) = load_workspace_projection_from_path(current_path)? {
        if workspace_projection_is_eligible_for_auto_done(&current) {
            let mut event = WorkEvent::new(WorkEventKind::Done, &current.id, now);
            event.status_category = Some(WorkspaceStatusCategory::Done);
            if emit_workspace_terminal_event_if_absent_paths_inner(
                work_items_path,
                events_path,
                event,
                true,
            )? {
                emitted += 1;
            }
        }
    }

    Ok(emitted)
}

/// SPEC-2359 US-37: Schema version recorded in `work_items.migration.json`.
/// Bumping this value forces [`rebuild_work_items_from_events_paths`] and
/// [`rebuild_work_items_from_events_for_repo`] to re-run on existing data.
/// Version 1 corresponds to the terminal-Done apply_event fix; prior
/// projections may show stale non-Done status_category for items whose
/// latest event regressed Done.
pub const WORK_ITEMS_REBUILD_VERSION: u32 = 1;

/// SPEC-2359 US-37: Outcome of the work_items.json rebuild migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkItemsRebuildOutcome {
    /// `work_events.jsonl` does not exist. Nothing to rebuild.
    Missing,
    /// Marker already records the current rebuild version. Skip silently.
    AlreadyMigrated,
    /// Rebuilt `work_items.json` from the event log and wrote the marker.
    Applied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkItemsRebuildMarker {
    version: u32,
    #[serde(default)]
    migrated_at: Option<DateTime<Utc>>,
}

/// SPEC-2359 US-37: Rebuild `work_items.json` by replaying every event in
/// `work_events.jsonl` through the (fixed) apply_event semantics. This
/// recovers historical Done state that the legacy apply_event regressed
/// when subsequent heartbeat update events arrived after the Done event.
/// The rebuild is idempotent across daemon restarts: a marker file records
/// the current schema version.
pub fn rebuild_work_items_from_events_paths(
    work_items_path: &Path,
    events_path: &Path,
    marker_path: &Path,
) -> Result<WorkItemsRebuildOutcome> {
    if rebuild_marker_at_or_above(marker_path, WORK_ITEMS_REBUILD_VERSION)? {
        return Ok(WorkItemsRebuildOutcome::AlreadyMigrated);
    }
    if !events_path.exists() {
        return Ok(WorkItemsRebuildOutcome::Missing);
    }
    let mut events = read_workspace_work_event_records_from_path(events_path)?
        .into_iter()
        .filter_map(WorkEventLogRecord::into_known_event)
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.updated_at);
    let initial_updated_at = events
        .first()
        .map(|event| event.updated_at)
        .unwrap_or_else(chrono::Utc::now);
    let mut projection = WorkItemsProjection::empty(initial_updated_at);
    for event in events {
        projection.apply_event(event);
    }
    projection.updated_at = chrono::Utc::now();
    save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    write_rebuild_marker(marker_path)?;
    Ok(WorkItemsRebuildOutcome::Applied)
}

/// SPEC-2359 US-37 — SUPERSEDED by the W-16 intake consumer
/// (`work_events_intake` + the gwt-side `work_events_ingest` orchestrator).
/// The bootstrap no longer calls this; the permanently-installed idempotent
/// intake covers the same repo-local source plus worktree filesystems and
/// fetched `origin/*` refs. The `work_items.migration.json` marker file is
/// no longer read but is intentionally left on disk. Kept for tests and as
/// a manual recovery tool.
///
/// SPEC-2359 Phase W-15 (FR-384) caveat: close-kind events recorded after
/// W-15 live only in the home close log (`work-events-closed.jsonl`). A
/// replay of solely the repo-local log would resurrect closed Work — any
/// such replay must also merge the home close log.
pub fn rebuild_work_items_from_events_for_repo(
    repo_path: &Path,
) -> Result<WorkItemsRebuildOutcome> {
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    let events_path = repo_local_work_events_path_with_migration(repo_path)?;
    let marker_path = work_items_path
        .parent()
        .map(|dir| dir.join("work_items.migration.json"))
        .unwrap_or_else(|| PathBuf::from("work_items.migration.json"));
    rebuild_work_items_from_events_paths(&work_items_path, &events_path, &marker_path)
}

fn rebuild_marker_at_or_above(path: &Path, required: u32) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let body = fs::read_to_string(path)?;
    Ok(serde_json::from_str::<WorkItemsRebuildMarker>(&body)
        .map(|marker| marker.version >= required)
        .unwrap_or(false))
}

fn write_rebuild_marker(path: &Path) -> Result<()> {
    let marker = WorkItemsRebuildMarker {
        version: WORK_ITEMS_REBUILD_VERSION,
        migrated_at: Some(chrono::Utc::now()),
    };
    let body = serde_json::to_vec_pretty(&marker)
        .map_err(|error| GwtError::Other(format!("work items migration marker: {error}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(path, &body)
}

/// SPEC-2359 Phase W-11 (US-58 / FR-346): schema version for the one-time
/// agent identity reset. Bumping this re-runs [`reset_legacy_agent_identity_at`]
/// on existing data. Version 1 clears `title_summary` / `current_focus`
/// written by the legacy prompt-derivation hook so the display fallback and
/// agent re-authoring take over.
pub const WORKSPACE_AGENT_IDENTITY_RESET_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AgentIdentityResetMarker {
    version: u32,
    #[serde(default)]
    migrated_at: Option<DateTime<Utc>>,
}

fn agent_identity_reset_marker_path(current_path: &Path) -> PathBuf {
    current_path
        .parent()
        .map(|dir| dir.join("agent_identity.migration.json"))
        .unwrap_or_else(|| PathBuf::from("agent_identity.migration.json"))
}

fn agent_identity_reset_marker_at_or_above(path: &Path, required: u32) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let body = fs::read_to_string(path)?;
    Ok(serde_json::from_str::<AgentIdentityResetMarker>(&body)
        .map(|marker| marker.version >= required)
        .unwrap_or(false))
}

fn write_agent_identity_reset_marker(path: &Path) -> Result<()> {
    let marker = AgentIdentityResetMarker {
        version: WORKSPACE_AGENT_IDENTITY_RESET_VERSION,
        migrated_at: Some(chrono::Utc::now()),
    };
    let body = serde_json::to_vec_pretty(&marker)
        .map_err(|error| GwtError::Other(format!("agent identity reset marker: {error}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(path, &body)
}

/// SPEC-2359 Phase W-11 (US-58 / FR-346): clear legacy `title_summary` /
/// `current_focus` from the canonical projection at `current_path` exactly
/// once, guarded by a version marker. After this reset those fields are
/// authored only by the agent (`workspace.update` / `board.post`),
/// and empty values resolve through the display fallback chain. Returns
/// `true` when the reset marker was newly written, `false` when the marker
/// already records the current version (so agent-authored values written
/// after the reset are never cleared again).
pub fn reset_legacy_agent_identity_at(current_path: &Path) -> Result<bool> {
    let work_items_path = current_path.with_file_name("works.json");
    with_workspace_work_items_lock(&work_items_path, || {
        let marker_path = agent_identity_reset_marker_path(current_path);
        if agent_identity_reset_marker_at_or_above(
            &marker_path,
            WORKSPACE_AGENT_IDENTITY_RESET_VERSION,
        )? {
            return Ok(false);
        }
        if let Some(mut projection) = load_workspace_projection_from_path(current_path)? {
            let mut changed = false;
            for agent in &mut projection.agents {
                if agent.title_summary.take().is_some() {
                    changed = true;
                }
                if agent.current_focus.take().is_some() {
                    changed = true;
                }
            }
            if changed {
                save_workspace_projection_to_path_unlocked(current_path, &projection)?;
            }
        }
        write_agent_identity_reset_marker(&marker_path)?;
        Ok(true)
    })
}

/// SPEC-2359 Phase W-11 (US-58 / FR-346): repo-scoped convenience wrapper for
/// the startup bootstrap. Resolves the canonical projection path and runs the
/// version-guarded one-time legacy identity reset. Call this once at startup
/// (alongside the work-items rebuild), not on every projection load, so a
/// freshly agent-authored title is never cleared.
pub fn reset_legacy_agent_identity_for_repo(repo_path: &Path) -> Result<bool> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    reset_legacy_agent_identity_at(&current_path)
}

/// SPEC-2359 US-37 / FR-119: Convenience wrapper resolving the project-scoped
/// current, work_items, and work_events paths from `repo_path` and invoking
/// [`retroactive_auto_done_scan_paths`].
/// SPEC-2359 Phase W-15 (FR-384): the emitted Done events are close-kind, so
/// they go to the home close log, never to the git-tracked repo-local log.
pub fn retroactive_auto_done_scan(repo_path: &Path, now: DateTime<Utc>) -> Result<usize> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let work_items_path = gwt_workspace_work_items_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    let events_path = gwt_workspace_work_events_closed_path_for_repo_path(repo_path);
    retroactive_auto_done_scan_paths(&current_path, &work_items_path, &events_path, now)
}

fn work_item_is_eligible_for_auto_done(item: &WorkItem) -> bool {
    item.execution_containers.iter().any(|container| {
        let branch_starts_with_work = container
            .branch
            .as_deref()
            .is_some_and(|branch| branch.starts_with("work/"));
        let pr_state_merged = container
            .pr_state
            .as_deref()
            .is_some_and(|state| state.eq_ignore_ascii_case("merged"));
        branch_starts_with_work && pr_state_merged
    })
}

fn workspace_projection_is_eligible_for_auto_done(projection: &WorkspaceProjection) -> bool {
    let Some(details) = projection.git_details.as_ref() else {
        return false;
    };
    let branch_starts_with_work = details
        .branch
        .as_deref()
        .is_some_and(|branch| branch.starts_with("work/"));
    let pr_state_merged = details
        .pr_state
        .as_deref()
        .is_some_and(|state| state.eq_ignore_ascii_case("merged"));
    branch_starts_with_work && pr_state_merged && details.created_by_start_work
}

/// SPEC-2359 US-37 / FR-118: Emit a Done WorkEvent for the Workspace
/// WorkItem currently associated with `branch`. The function loads the current
/// projection at `current_path` and emits Done iff
/// `projection.git_details.branch` matches `branch`. Used by user-confirmed
/// cleanup to mark the matching Workspace as completed after worktree/branch
/// deletion succeeds. Idempotent per `work_item_id` via
/// [`emit_workspace_done_event_if_absent_paths`].
pub fn emit_workspace_done_event_for_branch_paths(
    current_path: &Path,
    work_items_path: &Path,
    events_path: &Path,
    branch: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let Some(projection) = load_workspace_projection_from_path(current_path)? else {
        return Ok(false);
    };
    let matches = projection
        .git_details
        .as_ref()
        .and_then(|details| details.branch.as_deref())
        .is_some_and(|stored_branch| stored_branch == branch);
    if !matches {
        return Ok(false);
    }
    emit_workspace_done_event_if_absent_paths(work_items_path, events_path, &projection.id, now)
}

/// SPEC-2359 US-37 / FR-118: Convenience wrapper resolving project-scoped
/// paths from `repo_path` and invoking
/// [`emit_workspace_done_event_for_branch_paths`].
pub fn emit_workspace_done_event_for_branch(
    repo_path: &Path,
    branch: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    emit_workspace_done_event_for_branch_paths(
        &gwt_workspace_projection_path_for_repo_path(repo_path),
        &gwt_workspace_work_items_path_for_repo_path(repo_path),
        &gwt_workspace_work_events_closed_path_for_repo_path(repo_path),
        branch,
        now,
    )
}

pub fn load_or_default_workspace_projection_from_path(
    path: &Path,
    project_root: &Path,
) -> Result<WorkspaceProjection> {
    Ok(load_workspace_projection_from_path(path)?
        .unwrap_or_else(|| WorkspaceProjection::default_for_project(project_root)))
}

pub fn save_workspace_projection_to_path(
    path: &Path,
    projection: &WorkspaceProjection,
) -> Result<()> {
    let work_items_path = path.with_file_name("works.json");
    with_workspace_work_items_lock(&work_items_path, || {
        save_workspace_projection_to_path_unlocked(path, projection)
    })
}

fn save_workspace_projection_to_path_unlocked(
    path: &Path,
    projection: &WorkspaceProjection,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(projection)
        .map_err(|error| GwtError::Other(format!("workspace projection json: {error}")))?;
    write_atomic(path, &bytes)
}

pub fn update_workspace_projection_with_journal_paths(
    current_path: &Path,
    journal_path: &Path,
    project_root: &Path,
    update: WorkspaceProjectionUpdate,
) -> Result<WorkspaceJournalEntry> {
    update_workspace_projection_with_journal_paths_at(
        current_path,
        journal_path,
        project_root,
        update,
        Utc::now(),
    )
}

pub fn update_workspace_projection_with_journal_paths_at(
    current_path: &Path,
    journal_path: &Path,
    project_root: &Path,
    update: WorkspaceProjectionUpdate,
    updated_at: DateTime<Utc>,
) -> Result<WorkspaceJournalEntry> {
    let work_items_path = current_path.with_file_name("works.json");
    with_workspace_current_and_work_items_lock(current_path, &work_items_path, || {
        let current_precondition = workspace_state_file_fingerprint(current_path)?;
        let work_items_precondition = workspace_state_file_fingerprint(&work_items_path)?;
        let mut projection =
            load_or_default_workspace_projection_from_path(current_path, project_root)?;
        projection.project_root = project_root.to_path_buf();
        let entry = projection.apply_update(update, updated_at);
        let transaction = PendingWorkspaceStateTransaction {
            version: WORKSPACE_STATE_TRANSACTION_VERSION,
            transaction_id: Some(Uuid::new_v4().to_string()),
            current_path: current_path.to_path_buf(),
            work_items_path: work_items_path.clone(),
            current_precondition: Some(current_precondition),
            work_items_precondition: Some(work_items_precondition),
            projection,
            work_items: None,
            events_path: None,
            events: Vec::new(),
            journal_path: Some(journal_path.to_path_buf()),
            journal_entries: vec![entry.clone()],
        };
        persist_workspace_state_transaction_locked(current_path, &transaction)?;
        Ok(entry)
    })
}

pub fn mark_workspace_agent_stopped_at(
    current_path: &Path,
    project_root: &Path,
    session_id: &str,
    window_id: Option<&str>,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    let work_items_path = current_path.with_file_name("works.json");
    with_workspace_work_items_lock(&work_items_path, || {
        let Some(mut projection) = load_workspace_projection_from_path(current_path)? else {
            return Ok(false);
        };
        projection.project_root = project_root.to_path_buf();
        let changed = projection.remove_agent_session(session_id, window_id, updated_at);
        if changed {
            save_workspace_projection_to_path_unlocked(current_path, &projection)?;
        }
        Ok(changed)
    })
}

pub fn append_workspace_journal_entry_to_path(
    path: &Path,
    entry: &WorkspaceJournalEntry,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, entry)
        .map_err(|error| GwtError::Other(format!("workspace journal json: {error}")))?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub fn append_workspace_work_event_to_path(path: &Path, event: &WorkEvent) -> Result<()> {
    append_workspace_work_events_to_path(path, std::slice::from_ref(event))
}

/// Release A is reader-first: future event kinds can only enter this binary as
/// opaque records read from an existing log. Production writers remain typed
/// by the closed [`WorkEventKind`] enum until the release-B reader floor is
/// established.
#[cfg(test)]
const WORK_EVENT_CORRECTION_WRITER_ENABLED: bool = false;

/// Release-A view of one direct Work-event log line. Unknown event kinds are
/// intentionally opaque, while known kinds expose only the current typed
/// fields after additive top-level fields have been ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecodedWorkspaceWorkEvent {
    Known(Box<WorkEvent>),
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkEventLogRecord {
    Known {
        event: Box<WorkEvent>,
        original_line: Option<Vec<u8>>,
    },
    Opaque {
        original_line: Vec<u8>,
    },
}

impl WorkEventLogRecord {
    fn from_local_event(event: WorkEvent) -> Self {
        Self::Known {
            event: Box::new(event),
            original_line: None,
        }
    }

    fn into_known_event(self) -> Option<WorkEvent> {
        match self {
            Self::Known { event, .. } => Some(*event),
            Self::Opaque { .. } => None,
        }
    }
}

fn work_event_field_is_known(field: &str) -> bool {
    matches!(
        field,
        "id" | "work_item_id"
            | "kind"
            | "title"
            | "intent"
            | "summary"
            | "progress_summary"
            | "status_category"
            | "owner"
            | "next_action"
            | "agent_session_id"
            | "agent_id"
            | "display_name"
            | "board_entry_id"
            | "execution_container"
            | "related_work_item_id"
            | "updated_at"
    )
}

fn trim_ascii_json_line(mut line: &[u8]) -> &[u8] {
    while line.first().is_some_and(u8::is_ascii_whitespace) {
        line = &line[1..];
    }
    while line.last().is_some_and(u8::is_ascii_whitespace) {
        line = &line[..line.len() - 1];
    }
    line
}

pub(crate) fn decode_workspace_work_event_line(line: &[u8]) -> Result<DecodedWorkspaceWorkEvent> {
    let json_line = trim_ascii_json_line(line);
    let value: serde_json::Value = serde_json::from_slice(json_line)
        .map_err(|error| GwtError::Other(format!("workspace work event json: {error}")))?;
    let object = value.as_object().ok_or_else(|| {
        GwtError::Other("workspace work event json: expected an object".to_string())
    })?;
    let kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            GwtError::Other("workspace work event json: kind must be a string".to_string())
        })?;
    let known_kind =
        serde_json::from_value::<WorkEventKind>(serde_json::Value::String(kind.to_string())).ok();

    // Only the event object's top-level extension surface is opaque. Strip
    // those additive fields for the release-A projection view, but keep all
    // known identity/container fields so their existing strict decoders still
    // fail closed on an incompatible schema.
    let mut compatible = object
        .iter()
        .filter(|(field, _)| work_event_field_is_known(field))
        .map(|(field, value)| (field.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>();
    if known_kind.is_none() {
        compatible.insert(
            "kind".to_string(),
            serde_json::Value::String("update".to_string()),
        );
    }
    let event: WorkEvent = serde_json::from_value(serde_json::Value::Object(compatible))
        .map_err(|error| GwtError::Other(format!("workspace work event json: {error}")))?;

    match known_kind {
        Some(_) => Ok(DecodedWorkspaceWorkEvent::Known(Box::new(event))),
        None => Ok(DecodedWorkspaceWorkEvent::Opaque),
    }
}

fn decode_workspace_work_event_record(line: &[u8]) -> Result<WorkEventLogRecord> {
    match decode_workspace_work_event_line(line)? {
        DecodedWorkspaceWorkEvent::Known(event) => Ok(WorkEventLogRecord::Known {
            event,
            original_line: Some(line.to_vec()),
        }),
        DecodedWorkspaceWorkEvent::Opaque => Ok(WorkEventLogRecord::Opaque {
            original_line: line.to_vec(),
        }),
    }
}

fn read_workspace_work_event_records_from_path(path: &Path) -> Result<Vec<WorkEventLogRecord>> {
    repair_jsonl_tail(path)?;
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    content
        .split(|byte| *byte == b'\n')
        .filter(|line| !trim_ascii_json_line(line).is_empty())
        .map(decode_workspace_work_event_record)
        .collect()
}

fn append_workspace_work_events_to_path(path: &Path, events: &[WorkEvent]) -> Result<()> {
    let records = events
        .iter()
        .cloned()
        .map(WorkEventLogRecord::from_local_event)
        .collect::<Vec<_>>();
    append_workspace_work_event_records_to_path(path, &records)
}

fn append_workspace_work_event_records_to_path(
    path: &Path,
    records: &[WorkEventLogRecord],
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = Vec::new();
    for record in records {
        match record {
            WorkEventLogRecord::Known {
                original_line: Some(original_line),
                ..
            }
            | WorkEventLogRecord::Opaque { original_line } => {
                bytes.extend_from_slice(original_line);
            }
            WorkEventLogRecord::Known {
                event,
                original_line: None,
            } => {
                serde_json::to_writer(&mut bytes, event).map_err(|error| {
                    GwtError::Other(format!("workspace work event json: {error}"))
                })?;
            }
        }
        bytes.push(b'\n');
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn load_recent_workspace_journal_entries(
    repo_path: &Path,
    limit: usize,
) -> Result<Vec<WorkspaceJournalEntry>> {
    load_recent_workspace_journal_entries_from_path(
        &gwt_workspace_journal_path_for_repo_path(repo_path),
        limit,
    )
}

pub fn load_recent_workspace_journal_entries_from_path(
    path: &Path,
    limit: usize,
) -> Result<Vec<WorkspaceJournalEntry>> {
    if limit == 0 || !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    let mut entries = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<WorkspaceJournalEntry>(line)
                .map_err(|error| GwtError::Other(format!("workspace journal json: {error}")))
        })
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.updated_at));
    entries.truncate(limit);
    Ok(entries)
}

fn synthesize_workspace_work_items_from_legacy_paths(
    current_path: &Path,
    journal_path: &Path,
    project_root: &Path,
) -> Result<WorkItemsProjection> {
    let projection = load_workspace_projection_from_path(current_path)?;
    let mut journal_entries =
        load_recent_workspace_journal_entries_from_path(journal_path, usize::MAX)?;
    journal_entries.sort_by_key(|entry| entry.updated_at);
    let updated_at = projection
        .as_ref()
        .map(|projection| projection.updated_at)
        .or_else(|| journal_entries.last().map(|entry| entry.updated_at))
        .unwrap_or_else(Utc::now);
    let Some(item) = synthesize_workspace_work_item_from_legacy(
        projection.as_ref(),
        &journal_entries,
        project_root,
    ) else {
        return Ok(WorkItemsProjection::empty(updated_at));
    };
    Ok(WorkItemsProjection {
        updated_at: item.updated_at,
        work_items: vec![item],
    })
}

fn synthesize_workspace_work_item_from_legacy(
    projection: Option<&WorkspaceProjection>,
    journal_entries: &[WorkspaceJournalEntry],
    _project_root: &Path,
) -> Option<WorkItem> {
    if projection.is_none() && journal_entries.is_empty() {
        return None;
    }
    if let Some(projection) = projection {
        let has_workspace_identity = projection.assigned_agents().next().is_some()
            || projection.git_details.is_some()
            || !projection.board_refs.is_empty()
            || projection.status_category != WorkspaceStatusCategory::Unknown;
        if journal_entries.is_empty() && !has_workspace_identity {
            return None;
        }
    }
    let first_entry = journal_entries.first();
    let last_entry = journal_entries.last();
    let id = projection
        .map(|projection| projection.id.clone())
        .or_else(|| first_entry.map(|entry| format!("legacy-{}", entry.id)))?;
    let title = projection
        .map(|projection| projection.title.clone())
        .or_else(|| {
            first_entry.and_then(|entry| {
                non_empty_clone(entry.title.as_deref())
                    .or_else(|| non_empty_clone(entry.agent_title_summary.as_deref()))
                    .or_else(|| non_empty_clone(entry.summary.as_deref()))
            })
        })
        .unwrap_or_else(|| "Work history".to_string());
    let status_category = projection
        .map(WorkspaceProjection::effective_status_category)
        .or_else(|| last_entry.and_then(|entry| entry.status_category))
        .unwrap_or(WorkspaceStatusCategory::Unknown);
    let summary = projection
        .and_then(|projection| projection.summary.clone())
        .or_else(|| last_entry.and_then(|entry| entry.summary.clone()))
        .or_else(|| last_entry.and_then(|entry| entry.status_text.clone()));
    let progress_summary = projection
        .and_then(|projection| projection.progress_summary.clone())
        .or_else(|| last_entry.and_then(|entry| entry.progress_summary.clone()));
    let owner = projection
        .and_then(|projection| projection.owner.clone())
        .or_else(|| last_entry.and_then(|entry| entry.owner.clone()));
    let created_at = first_entry
        .map(|entry| entry.updated_at)
        .or_else(|| projection.map(|projection| projection.updated_at))
        .unwrap_or_else(Utc::now);
    let updated_at = projection
        .map(|projection| projection.updated_at)
        .or_else(|| last_entry.map(|entry| entry.updated_at))
        .unwrap_or(created_at);
    let completed_at = (status_category == WorkspaceStatusCategory::Done).then_some(updated_at);
    let mut item = WorkItem {
        id: id.clone(),
        title,
        intent: summary.clone(),
        summary,
        progress_summary,
        status_category,
        owner,
        created_at,
        updated_at,
        completed_at,
        agents: Vec::new(),
        execution_containers: Vec::new(),
        board_refs: projection
            .map(|projection| projection.board_refs.clone())
            .unwrap_or_default(),
        related_work_item_ids: Vec::new(),
        events: Vec::new(),
        legacy_metadata_snapshot: None,
        legacy_metadata_authoritative: false,
        legacy_metadata_snapshot_at: None,
        duplicate_event_containers: Default::default(),
        discarded: false,
        discarded_at: None,
    };
    if let Some(projection) = projection {
        item.agents
            .extend(projection.assigned_agents().map(|agent| WorkAgentRef {
                session_id: agent.session_id.clone(),
                agent_id: Some(agent.agent_id.clone()),
                display_name: Some(agent.display_name.clone()),
                updated_at: agent.updated_at,
                // Synthesized from the projection, not from a work event.
                attached_by: None,
            }));
        if let Some(details) = projection.git_details.as_ref() {
            item.execution_containers
                .push(WorkspaceExecutionContainerRef {
                    branch: details.branch.clone(),
                    worktree_path: details.worktree_path.clone(),
                    pr_number: details.pr_number,
                    pr_url: details.pr_url.clone(),
                    pr_state: details.pr_state.clone(),
                });
        }
    }
    for (index, entry) in journal_entries.iter().enumerate() {
        let mut event = WorkEvent::new(
            workspace_work_event_kind_from_journal(index, entry),
            id.clone(),
            entry.updated_at,
        );
        event.id = format!("legacy-journal-{}", entry.id);
        event.title = entry
            .title
            .clone()
            .or_else(|| entry.agent_title_summary.clone());
        event.intent = entry
            .agent_current_focus
            .clone()
            .or_else(|| entry.summary.clone());
        event.summary = entry.summary.clone().or_else(|| entry.status_text.clone());
        event.progress_summary = entry.progress_summary.clone();
        event.status_category = entry.status_category;
        event.owner = entry.owner.clone();
        event.next_action = entry.next_action.clone();
        event.agent_session_id = entry.agent_session_id.clone();
        if let Some(session_id) = non_empty_clone(entry.agent_session_id.as_deref()) {
            if !item
                .agents
                .iter()
                .any(|agent| agent.session_id == session_id)
            {
                item.agents.push(WorkAgentRef {
                    session_id,
                    agent_id: None,
                    display_name: entry.agent_title_summary.clone(),
                    updated_at: entry.updated_at,
                    // Synthesized from a legacy journal entry, not a work event.
                    attached_by: None,
                });
            }
        }
        item.events.push(event);
    }
    if item.events.is_empty() {
        let mut event = WorkEvent::new(
            workspace_work_event_kind_from_status(status_category, 0),
            id,
            updated_at,
        );
        event.title = Some(item.title.clone());
        event.summary = item.summary.clone();
        event.progress_summary = item.progress_summary.clone();
        event.status_category = Some(status_category);
        event.owner = item.owner.clone();
        item.events.push(event);
    }
    item.events.sort_by_key(|event| event.updated_at);
    Some(item)
}

#[cfg(test)]
fn workspace_work_event_from_journal_entry(
    projection: &WorkspaceProjection,
    entry: &WorkspaceJournalEntry,
) -> WorkEvent {
    workspace_work_event_from_journal_entry_for_root(projection, entry, &projection.project_root)
}

fn workspace_work_event_from_journal_entry_for_root(
    projection: &WorkspaceProjection,
    entry: &WorkspaceJournalEntry,
    work_event_root: &Path,
) -> WorkEvent {
    let (work_item_id, execution_container) = workspace_work_event_target_from_projection(
        projection,
        entry.agent_session_id.as_deref(),
        work_event_root,
    );
    let mut event = WorkEvent::new(
        workspace_work_event_kind_from_status(
            entry.status_category.unwrap_or(projection.status_category),
            1,
        ),
        work_item_id,
        entry.updated_at,
    );
    event.title = entry
        .title
        .clone()
        .or_else(|| entry.agent_title_summary.clone())
        .or_else(|| Some(projection.title.clone()));
    event.intent = entry
        .agent_current_focus
        .clone()
        .or_else(|| entry.summary.clone())
        .or_else(|| projection.summary.clone());
    event.summary = entry.summary.clone().or_else(|| entry.status_text.clone());
    event.progress_summary = entry
        .progress_summary
        .clone()
        .or_else(|| projection.progress_summary.clone());
    event.status_category = Some(entry.status_category.unwrap_or(projection.status_category));
    event.owner = entry.owner.clone().or_else(|| projection.owner.clone());
    event.next_action = entry.next_action.clone();
    event.agent_session_id = entry.agent_session_id.clone();
    if let Some(session_id) = entry.agent_session_id.as_deref() {
        if let Some(agent) = projection.latest_agent_for_session(session_id) {
            event.agent_id = Some(agent.agent_id.clone());
            event.display_name = Some(agent.display_name.clone());
        }
    }
    event.execution_container = execution_container;
    event
}

pub fn workspace_work_event_from_board_entry(
    projection: &WorkspaceProjection,
    entry: &BoardEntry,
) -> WorkEvent {
    let (work_item_id, execution_container) = workspace_work_event_target_from_projection(
        projection,
        entry.origin_session_id.as_deref(),
        &projection.project_root,
    );
    let mut event = WorkEvent::new(
        workspace_work_event_kind_from_board_entry(entry),
        work_item_id,
        entry.updated_at,
    );
    // SPEC-3075 FR-003/FR-004: a Board post body is the Work *status*
    // (a point-in-time snapshot), never its *purpose* (identity). The identity
    // comes from the agent-declared `title_summary`; if absent, the existing
    // Work title is preserved (the body must not become the title). The body is
    // retained only as `summary`. This stops "the summary is a status snapshot,
    // not what the Work is".
    event.title = entry
        .title_summary
        .clone()
        .or_else(|| Some(projection.title.clone()));
    event.intent = entry.title_summary.clone();
    event.summary = Some(entry.body.clone());
    event.status_category = Some(match entry.kind {
        BoardEntryKind::Blocked => WorkspaceStatusCategory::Blocked,
        BoardEntryKind::Next
        | BoardEntryKind::Status
        | BoardEntryKind::Claim
        | BoardEntryKind::Handoff
        | BoardEntryKind::Decision => WorkspaceStatusCategory::Active,
        BoardEntryKind::Request | BoardEntryKind::Impact | BoardEntryKind::Question => {
            projection.status_category
        }
    });
    event.owner = entry
        .related_owners
        .first()
        .cloned()
        .or_else(|| projection.owner.clone());
    event.agent_session_id = entry.origin_session_id.clone();
    event.agent_id = entry.origin_agent_id.clone();
    event.board_entry_id = Some(entry.id.clone());
    event.execution_container = execution_container;
    event
}

/// Resolve a Board entry to one existing, current Session-owned Work.
///
/// Board history is independently durable, so an unverifiable origin is a
/// normal `None`: callers keep the Board entry but must not mutate current or
/// append a Work event. The returned event derives Work identity and Agent
/// metadata only from the locked projections; untrusted origin Agent metadata
/// from the Board payload never enters the tracked Work log.
pub fn resolve_workspace_work_event_from_board_entry(
    projection: &WorkspaceProjection,
    work_items: &WorkItemsProjection,
    entry: &BoardEntry,
) -> Option<WorkEvent> {
    let session_id = entry
        .origin_session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| board_origin_session_id_is_safe(session_id))?;
    let agent = projection
        .latest_agent_for_session(session_id)
        .filter(|agent| {
            agent.affiliation_status == WorkspaceAgentAffiliationStatus::Assigned
                && entry.updated_at >= agent.updated_at
        })?;
    let work_item_id = agent
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|work_item_id| !work_item_id.is_empty())?;

    if let Some(origin_branch) = entry
        .origin_branch
        .as_deref()
        .map(canonical_session_bound_branch)
        .filter(|branch| !branch.is_empty())
    {
        let assigned_branch =
            canonical_session_bound_branch(agent.branch.as_deref().unwrap_or_default());
        if assigned_branch.is_empty() || origin_branch != assigned_branch {
            return None;
        }
    }

    let mut target_matches = work_items
        .work_items
        .iter()
        .filter(|item| item.id == work_item_id);
    let target = target_matches.next()?;
    if target_matches.next().is_some()
        || target.is_terminal()
        || entry.updated_at < target.updated_at
    {
        return None;
    }

    let mut session_targets = work_items.work_items.iter().filter(|item| {
        !item.is_terminal()
            && item
                .agents
                .iter()
                .any(|work_agent| work_agent.session_id == session_id)
    });
    if session_targets.next().map(|item| item.id.as_str()) != Some(work_item_id)
        || session_targets.next().is_some()
    {
        return None;
    }

    let execution_container = resolve_board_target_execution_container(target, agent)?;
    let mut event = WorkEvent::new(
        workspace_work_event_kind_from_board_entry(entry),
        work_item_id,
        entry.updated_at,
    );
    event.title = non_empty_clone(entry.title_summary.as_deref());
    event.intent = non_empty_clone(entry.title_summary.as_deref());
    event.summary = non_empty_clone(Some(entry.body.as_str()));
    event.status_category = Some(match entry.kind {
        BoardEntryKind::Blocked => WorkspaceStatusCategory::Blocked,
        BoardEntryKind::Next
        | BoardEntryKind::Status
        | BoardEntryKind::Claim
        | BoardEntryKind::Handoff
        | BoardEntryKind::Decision => WorkspaceStatusCategory::Active,
        BoardEntryKind::Request | BoardEntryKind::Impact | BoardEntryKind::Question => {
            target.status_category
        }
    });
    event.owner = target.owner.clone();
    event.agent_session_id = Some(agent.session_id.clone());
    event.agent_id = non_empty_clone(Some(agent.agent_id.as_str()));
    event.display_name = non_empty_clone(Some(agent.display_name.as_str()));
    event.board_entry_id = Some(entry.id.clone());
    event.execution_container = Some(execution_container);
    Some(event)
}

fn board_origin_session_id_is_safe(session_id: &str) -> bool {
    let bytes = session_id.as_bytes();
    !session_id.is_empty()
        && !matches!(session_id, "." | "..")
        && !session_id.contains('/')
        && !session_id.contains('\\')
        && !session_id.contains('\0')
        && !(bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

fn resolve_board_target_execution_container(
    target: &WorkItem,
    agent: &WorkspaceAgentSummary,
) -> Option<WorkspaceExecutionContainerRef> {
    if target.execution_containers.is_empty() {
        return None;
    }

    let assigned_branch =
        canonical_session_bound_branch(agent.branch.as_deref().unwrap_or_default());
    let assigned_worktree = agent.worktree_path.as_deref();
    if assigned_branch.is_empty() && assigned_worktree.is_none() {
        return None;
    }

    let mut matches = target.execution_containers.iter().filter(|container| {
        let branch_matches = assigned_branch.is_empty()
            || canonical_session_bound_branch(container.branch.as_deref().unwrap_or_default())
                == assigned_branch;
        let worktree_matches = assigned_worktree
            .is_none_or(|worktree| container.worktree_path.as_deref() == Some(worktree));
        branch_matches && worktree_matches
    });
    let container = matches.next()?.clone();
    matches.next().is_none().then_some(container)
}

fn workspace_work_event_kind_from_board_entry(entry: &BoardEntry) -> WorkEventKind {
    match entry.kind {
        BoardEntryKind::Claim => WorkEventKind::Claim,
        BoardEntryKind::Blocked => WorkEventKind::Blocked,
        BoardEntryKind::Handoff => WorkEventKind::Handoff,
        BoardEntryKind::Next
        | BoardEntryKind::Status
        | BoardEntryKind::Decision
        | BoardEntryKind::Request
        | BoardEntryKind::Impact
        | BoardEntryKind::Question => WorkEventKind::Update,
    }
}

fn workspace_work_event_kind_from_journal(
    index: usize,
    entry: &WorkspaceJournalEntry,
) -> WorkEventKind {
    workspace_work_event_kind_from_status(
        entry
            .status_category
            .unwrap_or(WorkspaceStatusCategory::Unknown),
        index,
    )
}

fn workspace_work_event_kind_from_status(
    status_category: WorkspaceStatusCategory,
    index: usize,
) -> WorkEventKind {
    match status_category {
        WorkspaceStatusCategory::Done => WorkEventKind::Done,
        WorkspaceStatusCategory::Blocked => WorkEventKind::Blocked,
        _ if index == 0 => WorkEventKind::Start,
        _ => WorkEventKind::Update,
    }
}

fn workspace_execution_container_from_projection(
    projection: &WorkspaceProjection,
) -> Option<WorkspaceExecutionContainerRef> {
    projection
        .git_details
        .as_ref()
        .map(|details| WorkspaceExecutionContainerRef {
            branch: details.branch.clone(),
            worktree_path: details.worktree_path.clone(),
            pr_number: details.pr_number,
            pr_url: details.pr_url.clone(),
            pr_state: details.pr_state.clone(),
        })
}

fn workspace_work_event_target_from_projection(
    projection: &WorkspaceProjection,
    agent_session_id: Option<&str>,
    work_event_root: &Path,
) -> (String, Option<WorkspaceExecutionContainerRef>) {
    if let Some(agent) = agent_session_id
        .and_then(|session_id| latest_workspace_agent_for_session(projection, session_id))
    {
        let container = workspace_execution_container_from_agent(agent);
        let assigned_work_id = (!agent.is_unassigned())
            .then(|| agent.workspace_id.clone())
            .flatten();
        let work_item_id = assigned_work_id
            .or_else(|| {
                container.as_ref().and_then(|container| {
                    canonical_work_id(
                        work_event_root,
                        container.branch.as_deref(),
                        container.worktree_path.as_deref(),
                    )
                })
            })
            .unwrap_or_else(|| projection.id.clone());
        return (work_item_id, container);
    }

    (
        projection.id.clone(),
        workspace_execution_container_from_projection(projection),
    )
}

fn workspace_execution_container_from_agent(
    agent: &WorkspaceAgentSummary,
) -> Option<WorkspaceExecutionContainerRef> {
    let branch = non_empty_clone(agent.branch.as_deref());
    let worktree_path = agent.worktree_path.clone();
    if branch.is_none() && worktree_path.is_none() {
        return None;
    }
    Some(WorkspaceExecutionContainerRef {
        branch,
        worktree_path,
        pr_number: None,
        pr_url: None,
        pr_state: None,
    })
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("current.json");
    let tmp = path.with_file_name(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// SPEC-2359 US-41 (FR-151): classify a Workspace projection as stale based
/// on Git side-effect signals (worktree existence, linked PR state) and a
/// time threshold (`now - updated_at` > `config.archive_after_days`).
///
/// Returns `None` when no stale signal applies; returns the single matching
/// [`StaleReason`] when exactly one applies; returns [`StaleReason::Compound`]
/// when more than one applies so callers can prioritize compound evidence.
///
/// Network-free: PR state is read from `projection.linked_prs[].state` and
/// `git_details.pr_state`, which are populated by the existing GitHub Issue
/// cache via `gh` API (the caller keeps that cache fresh).
pub fn workspace_projection_stale_reason(
    projection: &WorkspaceProjection,
    config: &WorkspaceRetentionConfig,
    now: DateTime<Utc>,
) -> Option<StaleReason> {
    let mut reasons: Vec<StaleReason> = Vec::with_capacity(3);

    if let Some(git) = &projection.git_details {
        if let Some(worktree_path) = &git.worktree_path {
            if !worktree_path.exists() {
                reasons.push(StaleReason::WorktreeMissing);
            }
        }
    }

    let mut pr_closed = projection
        .git_details
        .as_ref()
        .and_then(|git| git.pr_state.as_deref())
        .map(pr_state_is_closed)
        .unwrap_or(false);
    if !pr_closed {
        pr_closed = projection
            .linked_prs
            .iter()
            .any(|pr| pr.state.as_deref().map(pr_state_is_closed).unwrap_or(false));
    }
    if pr_closed {
        reasons.push(StaleReason::PrClosed);
    }

    let threshold = chrono::Duration::days(config.archive_after_days as i64);
    if now.signed_duration_since(projection.updated_at) > threshold {
        reasons.push(StaleReason::TimeThreshold);
    }

    match reasons.len() {
        0 => None,
        1 => Some(reasons[0]),
        _ => Some(StaleReason::Compound),
    }
}

fn pr_state_is_closed(state: &str) -> bool {
    matches!(state.to_ascii_lowercase().as_str(), "merged" | "closed")
}

/// SPEC-2359 US-41 (FR-155): why a Workspace projection is left untouched by
/// the pruner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PruneSkipReason {
    /// `workspace_projection_stale_reason()` returned `None`.
    NotStale,
    /// One or more agents on this Workspace are still affiliated or have a
    /// live window (FR-155).
    ActiveAgent,
    /// `lifecycle_stage = Archived` but `delete_after_archive_days` has not
    /// elapsed yet.
    ArchivedTooSoon,
}

/// SPEC-2359 US-41 (FR-154): action the pruner intends to take on a single
/// Workspace projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PruneAction {
    /// Leave the projection untouched.
    Skip { reason: PruneSkipReason },
    /// Transition `lifecycle_stage` from `Active` (or any non-Archived stage)
    /// to `Archived` and persist the change. The directory is preserved so
    /// users can recover it manually.
    Archive,
    /// Physically remove `~/.gwt/projects/<repo-hash>/workspace/` after the
    /// archive grace period.
    Delete,
}

/// SPEC-2359 US-41: a single Workspace projection classified by
/// [`classify_workspace_projections`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifiedProjection {
    pub workspace_id: String,
    pub project_root: PathBuf,
    pub workspace_dir: PathBuf,
    pub lifecycle_stage: WorkspaceLifecycleStage,
    pub updated_at: DateTime<Utc>,
    pub stale_reason: Option<StaleReason>,
    pub action: PruneAction,
}

/// SPEC-2359 US-41 (FR-153): aggregate counts returned by [`apply_prune_plan`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneSummary {
    pub skipped: usize,
    pub archived: usize,
    pub deleted: usize,
}

/// SPEC-2359 US-41 (FR-153, FR-154, FR-155): walk `scan_root` (typically
/// `~/.gwt/projects/`), load each Workspace projection, and classify it as
/// `Skip` / `Archive` / `Delete`.
///
/// `is_active_session` lets the caller bridge in the live-window registry or
/// agent affiliation state held outside this crate, so the pruner stays
/// network-free and registry-free at the core layer.
pub fn classify_workspace_projections<F>(
    scan_root: &Path,
    config: &WorkspaceRetentionConfig,
    now: DateTime<Utc>,
    is_active_session: F,
) -> Vec<ClassifiedProjection>
where
    F: Fn(&WorkspaceProjection) -> bool,
{
    let mut results = Vec::new();

    let entries = match fs::read_dir(scan_root) {
        Ok(entries) => entries,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let project_dir = entry.path();
        if !project_dir.is_dir() {
            continue;
        }
        let state_dir = project_dir.join("project-state");
        let legacy_dir = project_dir.join("workspace");
        let workspace_dir = if state_dir.join("current.json").is_file() {
            state_dir
        } else if legacy_dir.join("current.json").is_file() {
            legacy_dir
        } else {
            continue;
        };
        let current_json = workspace_dir.join("current.json");
        let projection = match load_workspace_projection_from_path(&current_json) {
            Ok(Some(p)) => p,
            _ => continue,
        };

        let stale_reason = workspace_projection_stale_reason(&projection, config, now);

        let action = if is_active_session(&projection) {
            PruneAction::Skip {
                reason: PruneSkipReason::ActiveAgent,
            }
        } else if workspace_projection_is_empty_default(&projection) {
            PruneAction::Delete
        } else {
            match projection.lifecycle_stage {
                WorkspaceLifecycleStage::Archived => {
                    let elapsed = now.signed_duration_since(projection.updated_at);
                    let threshold = chrono::Duration::days(config.delete_after_archive_days as i64);
                    if elapsed > threshold {
                        PruneAction::Delete
                    } else {
                        PruneAction::Skip {
                            reason: PruneSkipReason::ArchivedTooSoon,
                        }
                    }
                }
                _ => match stale_reason {
                    Some(_) => PruneAction::Archive,
                    None => PruneAction::Skip {
                        reason: PruneSkipReason::NotStale,
                    },
                },
            }
        };

        results.push(ClassifiedProjection {
            workspace_id: projection.id.clone(),
            project_root: projection.project_root.clone(),
            workspace_dir,
            lifecycle_stage: projection.lifecycle_stage,
            updated_at: projection.updated_at,
            stale_reason: if workspace_projection_is_empty_default(&projection) {
                Some(StaleReason::EmptyProjection)
            } else {
                stale_reason
            },
            action,
        });
    }

    results
}

fn workspace_projection_is_empty_default(projection: &WorkspaceProjection) -> bool {
    matches!(projection.title.as_str(), "Work" | "Workspace")
        && projection.status_category == WorkspaceStatusCategory::Unknown
        && matches!(projection.status_text.as_str(), "" | "No active work")
        && projection.summary.as_deref().is_none_or(str::is_empty)
        && projection
            .progress_summary
            .as_deref()
            .is_none_or(str::is_empty)
        && projection.owner.as_deref().is_none_or(str::is_empty)
        && projection.next_action.as_deref().is_none_or(str::is_empty)
        && projection.agents.iter().all(workspace_agent_is_empty_stub)
        && projection.git_details.is_none()
        && projection.board_refs.is_empty()
        && projection.lifecycle_stage == WorkspaceLifecycleStage::Planning
        && projection
            .blocked_reason
            .as_deref()
            .is_none_or(str::is_empty)
        && projection.linked_issues.is_empty()
        && projection.linked_prs.is_empty()
        && projection.tags.is_empty()
        && projection.progress_pct.is_none()
}

fn workspace_agent_is_empty_stub(agent: &WorkspaceAgentSummary) -> bool {
    agent.window_id.is_none()
        && agent.current_focus.as_deref().is_none_or(str::is_empty)
        && agent.title_summary.as_deref().is_none_or(str::is_empty)
        && agent.worktree_path.is_none()
        && agent.branch.as_deref().is_none_or(str::is_empty)
        && agent.last_board_entry_id.is_none()
        && agent.last_board_entry_kind.is_none()
        && agent.coordination_scope.is_none()
}

/// SPEC-2359 US-41 (FR-153, FR-154): apply a previously-classified plan.
///
/// When `dry_run` is `true`, the function counts the actions without touching
/// the filesystem so callers can preview the outcome. When `dry_run` is
/// `false`, `Archive` entries are persisted via `save_workspace_projection_to_path`
/// and `Delete` entries are removed via `fs::remove_dir_all` on the workspace
/// directory.
pub fn apply_prune_plan(plan: &[ClassifiedProjection], dry_run: bool) -> Result<PruneSummary> {
    let mut summary = PruneSummary::default();
    for item in plan {
        match &item.action {
            PruneAction::Skip { .. } => {
                summary.skipped += 1;
            }
            PruneAction::Archive => {
                if !dry_run {
                    let current_json = item.workspace_dir.join("current.json");
                    let work_items_path = current_json.with_file_name("works.json");
                    with_workspace_work_items_lock(&work_items_path, || {
                        if let Ok(Some(mut projection)) =
                            load_workspace_projection_from_path(&current_json)
                        {
                            projection.lifecycle_stage = WorkspaceLifecycleStage::Archived;
                            projection.updated_at = Utc::now();
                            save_workspace_projection_to_path_unlocked(&current_json, &projection)?;
                        }
                        Ok(())
                    })?;
                }
                summary.archived += 1;
            }
            PruneAction::Delete => {
                if !dry_run {
                    remove_workspace_dir_and_empty_project_dir(&item.workspace_dir)?;
                }
                summary.deleted += 1;
            }
        }
    }
    Ok(summary)
}

fn remove_workspace_dir_and_empty_project_dir(workspace_dir: &Path) -> Result<()> {
    fs::remove_dir_all(workspace_dir).map_err(|err| {
        GwtError::Other(format!(
            "failed to remove workspace dir {}: {}",
            workspace_dir.display(),
            err
        ))
    })?;
    if let Some(project_dir) = workspace_dir.parent() {
        let _ = fs::remove_dir(project_dir);
    }
    Ok(())
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
