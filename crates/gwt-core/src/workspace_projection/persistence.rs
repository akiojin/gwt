//! Persistence and history maintenance for the workspace projection family:
//! load/save of `current.json` / `journal.jsonl` / `work_items.json` /
//! `events.jsonl`, legacy-path migration, work-event recording and backfill
//! reconciliation, rebuild-from-events, legacy synthesis, and the
//! stale-detection / classify / prune pipeline.

use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    coordination::{BoardEntry, BoardEntryKind},
    error::{GwtError, Result},
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
}

impl StaleReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorktreeMissing => "worktree_missing",
            Self::PrClosed => "pr_closed",
            Self::TimeThreshold => "time_threshold",
            Self::Compound => "compound",
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
    fs::copy(legacy_path, canonical_path)?;
    Ok(())
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
    if canonical_path.exists() {
        return load_workspace_projection_from_path(canonical_path);
    }

    let legacy_path = legacy_workspace_projection_path_for_repo_path(repo_path);
    if legacy_path == canonical_path {
        return load_workspace_projection_from_path(canonical_path);
    }
    let Some(projection) = load_workspace_projection_from_path(&legacy_path)? else {
        return Ok(None);
    };
    save_workspace_projection_to_path(canonical_path, &projection)?;
    Ok(Some(projection))
}

fn migrate_legacy_workspace_work_items(
    repo_path: &Path,
    canonical_path: &Path,
) -> Result<Option<WorkItemsProjection>> {
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
    let projection = load_workspace_projection(repo_path).ok().flatten()?;
    projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session_id)
        .filter(|agent| !agent.is_unassigned())
        .and_then(|agent| agent.workspace_id.clone())
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
    projection
        .agents
        .iter()
        .find(|agent| match target_kind {
            "session" => agent.session_id == target_value,
            "agent" => {
                agent.agent_id == target_value
                    || agent.display_name == target_value
                    || agent.display_name.eq_ignore_ascii_case(target_value)
            }
            _ => false,
        })
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

pub fn update_workspace_projection_with_journal(
    repo_path: &Path,
    update: WorkspaceProjectionUpdate,
) -> Result<WorkspaceJournalEntry> {
    let current_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    let journal_path = gwt_workspace_journal_path_for_repo_path(repo_path);
    let _ = migrate_legacy_workspace_projection(repo_path, &current_path)?;
    copy_legacy_workspace_file_if_needed(
        &legacy_workspace_journal_path_for_repo_path(repo_path),
        &journal_path,
    )?;
    let entry = update_workspace_projection_with_journal_paths(
        &current_path,
        &journal_path,
        repo_path,
        update,
    )?;
    if let Some(projection) = load_workspace_projection(repo_path)? {
        let event = workspace_work_event_from_journal_entry(&projection, &entry);
        record_workspace_work_event(repo_path, event)?;
    }
    Ok(entry)
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

pub fn load_workspace_work_items_from_path(path: &Path) -> Result<Option<WorkItemsProjection>> {
    match fs::read(path) {
        Ok(bytes) => {
            let mut items: WorkItemsProjection = serde_json::from_slice(&bytes)
                .map_err(|error| GwtError::Other(format!("workspace work items json: {error}")))?;
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
    copy_legacy_workspace_file_if_needed(
        &legacy_workspace_journal_path_for_repo_path(repo_path),
        &journal_path,
    )?;
    let _ = migrate_legacy_workspace_work_items(repo_path, &work_items_path)?;
    load_or_synthesize_workspace_work_items_from_paths(
        &work_items_path,
        &current_path,
        &journal_path,
        repo_path,
    )
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
    let mut projection = load_workspace_work_items_from_path(work_items_path)?
        .unwrap_or_else(|| WorkItemsProjection::empty(event.updated_at));
    projection.apply_event(event.clone());
    save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    append_workspace_work_event_to_path(events_path, &event)?;
    Ok(())
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
        save_workspace_projection_to_path(current_projection_path, &current)?;
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
    record_workspace_work_event_paths(work_items_path, events_path, event)?;
    for board_ref in board_refs {
        if let Some(board_ref) = non_empty_clone(Some(board_ref.as_str())) {
            let mut ref_event = WorkEvent::new(WorkEventKind::Update, work_item_id, updated_at);
            ref_event.board_entry_id = Some(board_ref);
            record_workspace_work_event_paths(work_items_path, events_path, ref_event)?;
        }
    }
    Ok(())
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
/// for `work_item_id` iff no Done event has been recorded for it yet. This is
/// the canonical write path for auto-done emission from PR merge detection,
/// user-confirmed cleanup, and startup retroactive migration. Returns
/// `Ok(true)` when a new Done event was appended, `Ok(false)` when an
/// existing Done event was found (idempotent noop).
pub fn emit_workspace_done_event_if_absent_paths(
    work_items_path: &Path,
    events_path: &Path,
    work_item_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    if work_item_has_done_event_in_projection(work_items_path, work_item_id)? {
        return Ok(false);
    }
    let mut event = WorkEvent::new(WorkEventKind::Done, work_item_id, updated_at);
    event.status_category = Some(WorkspaceStatusCategory::Done);
    record_workspace_work_event_paths(work_items_path, events_path, event)?;
    Ok(true)
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

fn work_item_has_done_event_in_projection(
    work_items_path: &Path,
    work_item_id: &str,
) -> Result<bool> {
    let Some(projection) = load_workspace_work_items_from_path(work_items_path)? else {
        return Ok(false);
    };
    Ok(projection
        .work_items
        .iter()
        .filter(|item| item.id == work_item_id)
        .any(|item| {
            item.events
                .iter()
                .any(|event| event.kind == WorkEventKind::Done)
        }))
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
    if work_item_is_terminal_in_projection(work_items_path, work_item_id)? {
        return Ok(false);
    }
    let event = WorkEvent::new(WorkEventKind::Discard, work_item_id, updated_at);
    record_workspace_work_event_paths(work_items_path, events_path, event)?;
    Ok(true)
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

/// SPEC-2359 Phase W-12 Slice 4 (FR-352): true when `work_item_id` is already
/// in a terminal close state (Done or discarded) in the saved projection. Used
/// to make Done / Discard close emission idempotent.
fn work_item_is_terminal_in_projection(work_items_path: &Path, work_item_id: &str) -> Result<bool> {
    let Some(projection) = load_workspace_work_items_from_path(work_items_path)? else {
        return Ok(false);
    };
    Ok(projection
        .work_items
        .iter()
        .filter(|item| item.id == work_item_id)
        .any(|item| item.is_terminal()))
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
        if workspace_projection_is_eligible_for_auto_done(&current)
            && emit_workspace_done_event_if_absent_paths(
                work_items_path,
                events_path,
                &current.id,
                now,
            )?
        {
            emitted += 1;
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
    let content = fs::read_to_string(events_path)?;
    let mut events: Vec<WorkEvent> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<WorkEvent>(line)
                .map_err(|err| GwtError::Other(format!("workspace work event json: {err}")))
        })
        .collect::<Result<Vec<_>>>()?;
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
            save_workspace_projection_to_path(current_path, &projection)?;
        }
    }
    write_agent_identity_reset_marker(&marker_path)?;
    Ok(true)
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
    let mut projection =
        load_or_default_workspace_projection_from_path(current_path, project_root)?;
    projection.project_root = project_root.to_path_buf();
    let entry = projection.apply_update(update, updated_at);
    save_workspace_projection_to_path(current_path, &projection)?;
    append_workspace_journal_entry_to_path(journal_path, &entry)?;
    Ok(entry)
}

pub fn mark_workspace_agent_stopped_at(
    current_path: &Path,
    project_root: &Path,
    session_id: &str,
    window_id: Option<&str>,
    updated_at: DateTime<Utc>,
) -> Result<bool> {
    let Some(mut projection) = load_workspace_projection_from_path(current_path)? else {
        return Ok(false);
    };
    projection.project_root = project_root.to_path_buf();
    let changed = projection.remove_agent_session(session_id, window_id, updated_at);
    if changed {
        save_workspace_projection_to_path(current_path, &projection)?;
    }
    Ok(changed)
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, event)
        .map_err(|error| GwtError::Other(format!("workspace work event json: {error}")))?;
    file.write_all(b"\n")?;
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
        discarded: false,
    };
    if let Some(projection) = projection {
        item.agents
            .extend(projection.assigned_agents().map(|agent| WorkAgentRef {
                session_id: agent.session_id.clone(),
                agent_id: Some(agent.agent_id.clone()),
                display_name: Some(agent.display_name.clone()),
                updated_at: agent.updated_at,
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
        event.status_category = Some(status_category);
        event.owner = item.owner.clone();
        item.events.push(event);
    }
    item.events.sort_by_key(|event| event.updated_at);
    Some(item)
}

fn workspace_work_event_from_journal_entry(
    projection: &WorkspaceProjection,
    entry: &WorkspaceJournalEntry,
) -> WorkEvent {
    let mut event = WorkEvent::new(
        workspace_work_event_kind_from_status(
            entry.status_category.unwrap_or(projection.status_category),
            1,
        ),
        projection.id.clone(),
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
    event.status_category = Some(entry.status_category.unwrap_or(projection.status_category));
    event.owner = entry.owner.clone().or_else(|| projection.owner.clone());
    event.next_action = entry.next_action.clone();
    event.agent_session_id = entry.agent_session_id.clone();
    if let Some(session_id) = entry.agent_session_id.as_deref() {
        if let Some(agent) = projection
            .agents
            .iter()
            .find(|agent| agent.session_id == session_id)
        {
            event.agent_id = Some(agent.agent_id.clone());
            event.display_name = Some(agent.display_name.clone());
        }
    }
    event.execution_container = workspace_execution_container_from_projection(projection);
    event
}

pub fn workspace_work_event_from_board_entry(
    projection: &WorkspaceProjection,
    entry: &BoardEntry,
) -> WorkEvent {
    let mut event = WorkEvent::new(
        workspace_work_event_kind_from_board_entry(entry),
        projection.id.clone(),
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
    event.execution_container = workspace_execution_container_from_projection(projection);
    event
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
            stale_reason,
            action,
        });
    }

    results
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
                    if let Ok(Some(mut projection)) =
                        load_workspace_projection_from_path(&current_json)
                    {
                        projection.lifecycle_stage = WorkspaceLifecycleStage::Archived;
                        projection.updated_at = Utc::now();
                        save_workspace_projection_to_path(&current_json, &projection)?;
                    }
                }
                summary.archived += 1;
            }
            PruneAction::Delete => {
                if !dry_run {
                    fs::remove_dir_all(&item.workspace_dir).map_err(|err| {
                        GwtError::Other(format!(
                            "failed to remove workspace dir {}: {}",
                            item.workspace_dir.display(),
                            err
                        ))
                    })?;
                }
                summary.deleted += 1;
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
