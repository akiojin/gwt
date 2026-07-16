//! SPEC-2359 W-16 (FR-387/FR-388): cross-machine Work skeleton intake.
//!
//! A permanently-installed, idempotent consumer that merges `events.jsonl`
//! content from any source (local worktree filesystems, the base branch,
//! fetched `origin/*` refs) into the home works projection. Replaces the
//! one-shot `rebuild_work_items_from_events_for_repo` migration gate.
//!
//! Contract (plan §Architecture Decisions 3):
//! - dedup is the diff against the FULL event-id set already inside
//!   works.json — deleting the intake state cache never breaks idempotence
//!   (SC-260);
//! - close kinds {Pause, Done, Discard} are never ingested from ANY source
//!   (defence against contaminated logs, #3023; close state is owned by the
//!   machine-local close log per FR-384);
//! - malformed lines are skipped leniently (warn, keep going);
//! - terminal (Done / Discarded) items never apply events stamped at or
//!   before their close time (no re-open, no `updated_at` rollback);
//! - only works.json is written — sessions / current.json / journal.jsonl
//!   are untouched (SC-261).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{GwtError, JsonDecodeKind, Result};
use crate::workspace_projection::{
    load_workspace_work_items_from_path, save_workspace_work_items_projection_to_path,
    DuplicateWorkEventProvenance, WorkEvent, WorkEventApplyOutcome, WorkEventKind, WorkItem,
    WorkItemsProjection, WorkspaceExecutionContainerRef,
};

/// Per-run accounting so callers (and tests) can see what happened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkEventsIntakeReport {
    pub applied: usize,
    pub skipped_duplicate: usize,
    pub skipped_close_kind: usize,
    pub skipped_invalid: usize,
    pub skipped_terminal: usize,
}

impl WorkEventsIntakeReport {
    pub fn changed(&self) -> bool {
        self.applied > 0
    }
}

fn is_close_kind(kind: WorkEventKind) -> bool {
    matches!(
        kind,
        WorkEventKind::Pause | WorkEventKind::Done | WorkEventKind::Discard
    )
}

/// The first durable terminal close instant. Metadata and heartbeat events may
/// advance `updated_at`, but must never move this cutoff.
fn terminal_close_time(item: &crate::workspace_projection::WorkItem) -> Option<DateTime<Utc>> {
    if !item.is_terminal() {
        return None;
    }
    item.completed_at
        .into_iter()
        .chain(item.discarded_at)
        .chain(
            item.events
                .iter()
                .filter(|event| matches!(event.kind, WorkEventKind::Done | WorkEventKind::Discard))
                .map(|event| event.updated_at),
        )
        .min()
        .or(Some(item.updated_at))
}

/// Ingest one source's raw `events.jsonl` content into the works projection
/// at `work_items_path`. Pure file-paths API (#3022): no HOME resolution.
pub fn ingest_work_events_content(
    work_items_path: &Path,
    content: &str,
) -> Result<WorkEventsIntakeReport> {
    ingest_work_events_contents(work_items_path, std::iter::once(content))
}

/// Ingest multiple source logs as one globally ordered event stream. Source
/// discovery order must not decide which Work owns a session.
pub fn ingest_work_events_contents<'a>(
    work_items_path: &Path,
    contents: impl IntoIterator<Item = &'a str>,
) -> Result<WorkEventsIntakeReport> {
    crate::workspace_projection::with_workspace_work_items_lock(work_items_path, || {
        ingest_work_events_contents_locked(work_items_path, contents, None)
    })
}

/// Ingest changed shared sources plus the strict machine-local lifecycle log.
/// The local file is read while holding the same projection transaction lock
/// used by event writers, which also recovers an event appended before a
/// failed or interrupted projection save.
pub fn ingest_work_events_with_local_path<'a>(
    work_items_path: &Path,
    shared_contents: impl IntoIterator<Item = &'a str>,
    local_path: Option<&Path>,
) -> Result<(WorkEventsIntakeReport, Option<String>)> {
    crate::workspace_projection::with_workspace_work_items_lock(work_items_path, || {
        let local_content = match local_path.map(std::fs::read_to_string) {
            Some(Ok(content)) => Some(content),
            Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Some(Err(error)) => return Err(error.into()),
            None => None,
        };
        let local_fingerprint = local_content.as_deref().map(content_fingerprint);
        let report = ingest_work_events_contents_locked(
            work_items_path,
            shared_contents,
            local_content.as_deref(),
        )?;
        Ok((report, local_fingerprint))
    })
}

fn ingest_work_events_contents_locked<'a>(
    work_items_path: &Path,
    contents: impl IntoIterator<Item = &'a str>,
    local_content: Option<&str>,
) -> Result<WorkEventsIntakeReport> {
    let mut report = WorkEventsIntakeReport::default();
    let previous = load_workspace_work_items_from_path(work_items_path)?
        .unwrap_or_else(|| WorkItemsProjection::empty(Utc::now()));
    let seen_event_ids: HashSet<String> = previous
        .work_items
        .iter()
        .flat_map(|item| item.events.iter().map(|event| event.id.clone()))
        .collect();
    let mut incoming = collect_work_events(contents, WorkEventContentKind::Shared, &mut report)?;
    if let Some(content) = local_content {
        incoming.extend(collect_work_events(
            std::iter::once(content),
            WorkEventContentKind::MachineLocalLifecycle,
            &mut report,
        )?);
    }
    if incoming.is_empty() {
        return Ok(report);
    }

    // Keep the existing per-run accounting behavior, then independently
    // materialize the canonical result from accepted history plus this batch.
    // A source discovered in a later run may contain an earlier event, so
    // applying only to the hot projection would make source arrival order
    // decide Session ownership.
    let mut incrementally_reported = previous.clone();
    fold_work_events(
        &mut incrementally_reported,
        &seen_event_ids,
        incoming.clone(),
        &mut report,
    );
    let replayable = incoming
        .into_iter()
        .filter(|(event, _)| {
            seen_event_ids.contains(&event.id)
                || previous
                    .work_items
                    .iter()
                    .find(|item| item.id == event.work_item_id)
                    .and_then(terminal_close_time)
                    .is_none_or(|closed_at| event.updated_at > closed_at)
        })
        .collect();
    let mut projection = refold_work_events_projection_with_keys(&previous, replayable)?;
    let changed = !work_item_sets_equal(&projection, &previous);
    reconcile_refold_report(
        &previous,
        &projection,
        &seen_event_ids,
        changed,
        &mut report,
    );
    if changed {
        projection.updated_at = Utc::now();
        save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    }
    Ok(report)
}

fn work_item_sets_equal(left: &WorkItemsProjection, right: &WorkItemsProjection) -> bool {
    let mut left_items = left.work_items.iter().collect::<Vec<_>>();
    let mut right_items = right.work_items.iter().collect::<Vec<_>>();
    left_items.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    right_items.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    left_items == right_items
}

/// Deterministically replay accepted projection history together with newly
/// durable events. Used by the intake path and by terminal-write recovery so
/// both honor the same global chronology across process runs.
pub(crate) fn refold_work_events_projection(
    previous: &WorkItemsProjection,
    incoming: Vec<WorkEvent>,
) -> Result<WorkItemsProjection> {
    let incoming = incoming
        .into_iter()
        .map(|event| {
            let stable_key = serde_json::to_string(&event).map_err(|error| {
                GwtError::Other(format!("work events replay stable key: {error}"))
            })?;
            Ok((event, stable_key))
        })
        .collect::<Result<Vec<_>>>()?;
    refold_work_events_projection_with_keys(previous, incoming)
}

fn refold_work_events_projection_with_keys(
    previous: &WorkItemsProjection,
    incoming: Vec<(WorkEvent, String)>,
) -> Result<WorkItemsProjection> {
    let legacy_items = previous
        .work_items
        .iter()
        .filter(|item| item.events.is_empty() || item.legacy_metadata_authoritative)
        .cloned()
        .collect::<Vec<_>>();
    let mut all_events = previous
        .work_items
        .iter()
        .flat_map(|item| item.events.iter().cloned())
        .map(|event| {
            let stable_key = serde_json::to_string(&event).map_err(|error| {
                GwtError::Other(format!("work events history stable key: {error}"))
            })?;
            Ok((event, stable_key))
        })
        .collect::<Result<Vec<_>>>()?;
    all_events.extend(incoming);

    let mut rebuilt = WorkItemsProjection::empty(previous.updated_at);
    let mut replay_report = WorkEventsIntakeReport::default();
    fold_work_events(
        &mut rebuilt,
        &HashSet::new(),
        all_events,
        &mut replay_report,
    );
    for item in legacy_items {
        merge_eventless_legacy_item(&mut rebuilt, item);
    }

    // A duplicate event from another source may have contributed only an
    // execution container. Retain it only while the canonical copy of that
    // exact event remains accepted after the chronological refold.
    let previous_provenance = previous
        .work_items
        .iter()
        .flat_map(|item| {
            item.duplicate_event_containers
                .iter()
                .flat_map(move |(event_id, entries)| {
                    entries
                        .iter()
                        .cloned()
                        .map(move |entry| (item.id.clone(), event_id.clone(), entry))
                })
        })
        .collect::<Vec<_>>();
    for (work_item_id, event_id, provenance) in previous_provenance {
        let canonical_remains = rebuilt.work_items.iter().any(|item| {
            item.id == work_item_id && item.events.iter().any(|event| event.id == event_id)
        });
        if !canonical_remains {
            continue;
        }
        if provenance.event().is_some_and(|event| {
            event.work_item_id != work_item_id || rebuilt.would_reject_session_attach(event)
        }) {
            continue;
        }
        let Some(container) = provenance.container().cloned() else {
            continue;
        };
        let Some(item) = rebuilt
            .work_items
            .iter_mut()
            .find(|item| item.id == work_item_id)
        else {
            continue;
        };
        if !item
            .execution_containers
            .iter()
            .any(|existing| execution_container_same(existing, &container))
        {
            item.execution_containers.push(container.clone());
        }
        let entries = item.duplicate_event_containers.entry(event_id).or_default();
        if !entries.contains(&provenance) {
            entries.push(provenance);
        }
    }
    rebuilt
        .work_items
        .sort_by_key(|item| std::cmp::Reverse(item.updated_at));
    Ok(rebuilt)
}

fn reconcile_refold_report(
    previous: &WorkItemsProjection,
    rebuilt: &WorkItemsProjection,
    previous_event_ids: &HashSet<String>,
    changed: bool,
    report: &mut WorkEventsIntakeReport,
) {
    if !changed {
        report.skipped_duplicate += report.applied;
        report.applied = 0;
        return;
    }

    let newly_accepted = rebuilt
        .work_items
        .iter()
        .flat_map(|item| item.events.iter())
        .filter(|event| !previous_event_ids.contains(&event.id))
        .map(|event| event.id.as_str())
        .collect::<HashSet<_>>()
        .len();
    let required_applied = newly_accepted.max(1);
    if required_applied <= report.applied {
        return;
    }
    let mut remaining = required_applied - report.applied;
    let from_duplicates = remaining.min(report.skipped_duplicate);
    report.skipped_duplicate -= from_duplicates;
    remaining -= from_duplicates;
    let from_terminal = remaining.min(report.skipped_terminal);
    report.skipped_terminal -= from_terminal;
    report.applied = required_applied;

    debug_assert_ne!(previous.work_items, rebuilt.work_items);
}

/// Rebuild the event-derived projection with the current deterministic fold.
/// Shared logs reject close kinds; the machine-local lifecycle log accepts
/// close events and their auxiliary updates, and is parsed strictly so a
/// damaged local record never gets silently dropped. Eventless legacy rows are
/// carried forward as authoritative local metadata.
pub fn rebuild_work_events_contents<'a>(
    work_items_path: &Path,
    shared_contents: impl IntoIterator<Item = &'a str>,
    close_content: Option<&str>,
) -> Result<WorkEventsIntakeReport> {
    crate::workspace_projection::with_workspace_work_items_lock(work_items_path, || {
        rebuild_work_events_contents_locked(work_items_path, shared_contents, close_content)
    })
}

/// Rebuild using the machine-local lifecycle log at `close_path`. The file is
/// read only after the projection transaction lock is held, so a concurrent
/// terminal writer is either fully before or fully after this rebuild.
pub fn rebuild_work_events_paths<'a>(
    work_items_path: &Path,
    shared_contents: impl IntoIterator<Item = &'a str>,
    close_path: Option<&Path>,
) -> Result<WorkEventsIntakeReport> {
    crate::workspace_projection::with_workspace_work_items_lock(work_items_path, || {
        let close_content = match close_path.map(std::fs::read_to_string) {
            Some(Ok(content)) => Some(content),
            Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Some(Err(error)) => return Err(error.into()),
            None => None,
        };
        rebuild_work_events_contents_locked(
            work_items_path,
            shared_contents,
            close_content.as_deref(),
        )
    })
}

/// Rebuild from shared source contents loaded only after the project-scoped
/// projection lock is held. The loader lets the orchestrator discover and
/// fingerprint sources first, then re-read mutable worktree logs at the
/// transaction boundary so a writer cannot be overwritten by a stale
/// pre-lock snapshot.
pub fn rebuild_work_events_with_shared_loader<F, T>(
    work_items_path: &Path,
    load_shared_contents: F,
    close_path: Option<&Path>,
) -> Result<(WorkEventsIntakeReport, T, Option<String>)>
where
    F: FnOnce() -> Result<(Vec<String>, T)>,
{
    crate::workspace_projection::with_workspace_work_items_lock(work_items_path, || {
        let (shared_contents, loaded_metadata) = load_shared_contents()?;
        let close_content = match close_path.map(std::fs::read_to_string) {
            Some(Ok(content)) => Some(content),
            Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Some(Err(error)) => return Err(error.into()),
            None => None,
        };
        let close_fingerprint = close_content.as_deref().map(content_fingerprint);
        let report = rebuild_work_events_contents_locked(
            work_items_path,
            shared_contents.iter().map(String::as_str),
            close_content.as_deref(),
        )?;
        Ok((report, loaded_metadata, close_fingerprint))
    })
}

fn rebuild_work_events_contents_locked<'a>(
    work_items_path: &Path,
    shared_contents: impl IntoIterator<Item = &'a str>,
    close_content: Option<&str>,
) -> Result<WorkEventsIntakeReport> {
    let previous = match load_workspace_work_items_from_path(work_items_path) {
        Ok(previous) => previous,
        Err(GwtError::JsonDecode {
            kind: JsonDecodeKind::Malformed,
            message: error,
            ..
        }) => {
            tracing::warn!(
                %error,
                path = %work_items_path.display(),
                "work events rebuild: discarding corrupt projection"
            );
            None
        }
        Err(error) => return Err(error),
    };
    let previous_updated_at = previous.as_ref().map(|projection| projection.updated_at);
    let eventless_items = previous
        .into_iter()
        .flat_map(|projection| projection.work_items)
        .filter(|item| item.events.is_empty() || item.legacy_metadata_authoritative)
        .collect::<Vec<_>>();

    let mut report = WorkEventsIntakeReport::default();
    let mut incoming =
        collect_work_events(shared_contents, WorkEventContentKind::Shared, &mut report)?;
    if let Some(content) = close_content {
        incoming.extend(collect_work_events(
            std::iter::once(content),
            WorkEventContentKind::MachineLocalLifecycle,
            &mut report,
        )?);
    }

    let initial_updated_at = incoming
        .iter()
        .map(|(event, _)| event.updated_at)
        .min()
        .or(previous_updated_at)
        .unwrap_or_else(Utc::now);
    let mut projection = WorkItemsProjection::empty(initial_updated_at);
    fold_work_events(&mut projection, &HashSet::new(), incoming, &mut report);
    for item in eventless_items {
        merge_eventless_legacy_item(&mut projection, item);
    }
    projection
        .work_items
        .sort_by_key(|item| std::cmp::Reverse(item.updated_at));
    projection.updated_at = Utc::now();
    save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    Ok(report)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkEventContentKind {
    Shared,
    MachineLocalLifecycle,
}

fn collect_work_events<'a>(
    contents: impl IntoIterator<Item = &'a str>,
    content_kind: WorkEventContentKind,
    report: &mut WorkEventsIntakeReport,
) -> Result<Vec<(WorkEvent, String)>> {
    let mut incoming = Vec::new();
    for content in contents {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let event: WorkEvent = match serde_json::from_str(line) {
                Ok(event) => event,
                Err(error) if content_kind == WorkEventContentKind::Shared => {
                    report.skipped_invalid += 1;
                    tracing::warn!(%error, "work events intake: skipping malformed line");
                    continue;
                }
                Err(error) => {
                    return Err(GwtError::Other(format!(
                        "machine-local close event json: {error}"
                    )));
                }
            };
            match content_kind {
                WorkEventContentKind::Shared if is_close_kind(event.kind) => {
                    report.skipped_close_kind += 1;
                    continue;
                }
                _ => {}
            }
            let stable_key = serde_json::to_string(&event).map_err(|error| {
                GwtError::Other(format!("work events intake stable key: {error}"))
            })?;
            incoming.push((event, stable_key));
        }
    }
    Ok(incoming)
}

fn fold_work_events(
    projection: &mut WorkItemsProjection,
    seen_event_ids: &HashSet<String>,
    mut incoming: Vec<(WorkEvent, String)>,
    report: &mut WorkEventsIntakeReport,
) {
    incoming.sort_by(|(left, left_key), (right, right_key)| {
        left.updated_at
            .cmp(&right.updated_at)
            .then_with(|| work_event_sort_rank(left.kind).cmp(&work_event_sort_rank(right.kind)))
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left_key.cmp(right_key))
    });

    let mut event_groups = Vec::<Vec<WorkEvent>>::new();
    for (event, _) in incoming {
        let joins_last_tie = event_groups.last().is_some_and(|group| {
            let first = &group[0];
            first.id == event.id
                && first.updated_at == event.updated_at
                && work_event_sort_rank(first.kind) == work_event_sort_rank(event.kind)
        });
        if joins_last_tie {
            event_groups.last_mut().unwrap().push(event);
        } else {
            event_groups.push(vec![event]);
        }
    }

    let mut processed_event_ids = seen_event_ids.clone();
    let mut reported_applied_event_ids = HashSet::new();
    for mut events in event_groups {
        let event_id = events[0].id.clone();
        if processed_event_ids.contains(&event_id) {
            let mut repaired = false;
            for event in events {
                if projection.would_reject_session_attach(&event) {
                    warn_rejected_session_conflict(&event);
                    report.skipped_duplicate += 1;
                } else if repair_duplicate_event_container(projection, &event) {
                    repaired = true;
                } else {
                    report.skipped_duplicate += 1;
                }
            }
            if repaired && reported_applied_event_ids.insert(event_id) {
                report.applied += 1;
            }
            continue;
        }

        let event_count = events.len();
        let duplicate_count = event_count.saturating_sub(1);
        let Some(canonical_index) = events
            .iter()
            .position(|event| !projection.would_reject_session_attach(event))
        else {
            if let Some(rejected) = events.first() {
                warn_rejected_session_conflict(rejected);
            }
            report.skipped_duplicate += event_count;
            continue;
        };

        let event = events.remove(canonical_index);
        let terminal_cutoff = projection
            .work_items
            .iter()
            .find(|item| item.id == event.work_item_id)
            .and_then(terminal_close_time);
        if let Some(closed_at) = terminal_cutoff {
            if event.updated_at <= closed_at {
                report.skipped_terminal += 1;
                report.skipped_duplicate += duplicate_count;
                continue;
            }
        }
        if projection.apply_event(event) == WorkEventApplyOutcome::Applied {
            if reported_applied_event_ids.insert(event_id.clone()) {
                report.applied += 1;
            }
            processed_event_ids.insert(event_id);
        } else {
            report.skipped_duplicate += event_count;
            continue;
        }
        for duplicate in events {
            if projection.would_reject_session_attach(&duplicate) {
                warn_rejected_session_conflict(&duplicate);
                report.skipped_duplicate += 1;
            } else if !repair_duplicate_event_container(projection, &duplicate) {
                report.skipped_duplicate += 1;
            }
        }
    }
}

fn merge_eventless_legacy_item(projection: &mut WorkItemsProjection, mut legacy: WorkItem) {
    legacy.legacy_metadata_authoritative = true;
    let legacy_snapshot_at = legacy
        .legacy_metadata_snapshot_at
        .unwrap_or(legacy.updated_at);
    legacy.legacy_metadata_snapshot_at = Some(legacy_snapshot_at);
    let immutable_snapshot = legacy.legacy_metadata_snapshot.clone().unwrap_or_else(|| {
        let mut snapshot = legacy.clone();
        snapshot.events.clear();
        snapshot.legacy_metadata_snapshot = None;
        snapshot.duplicate_event_containers.clear();
        Box::new(snapshot)
    });
    let mut legacy_base = (*immutable_snapshot).clone();
    legacy_base.events.clear();
    legacy_base.legacy_metadata_snapshot = Some(immutable_snapshot.clone());
    legacy_base.legacy_metadata_authoritative = true;
    legacy_base.legacy_metadata_snapshot_at = Some(legacy_snapshot_at);
    legacy_base.duplicate_event_containers.clear();
    let Some(rebuilt) = projection
        .work_items
        .iter_mut()
        .find(|item| item.id == legacy.id)
    else {
        projection.work_items.push(legacy_base);
        return;
    };

    let rebuilt_events = std::mem::take(&mut rebuilt.events);
    let newer_events = rebuilt_events
        .iter()
        .filter(|event| event.updated_at > legacy_snapshot_at)
        .cloned()
        .collect::<Vec<_>>();
    let mut snapshot_projection = WorkItemsProjection {
        updated_at: legacy_base.updated_at,
        work_items: vec![legacy_base],
    };
    for event in newer_events {
        snapshot_projection.apply_event(event);
    }
    let mut legacy = snapshot_projection.work_items.pop().unwrap();
    legacy.legacy_metadata_snapshot = Some(immutable_snapshot);
    legacy.legacy_metadata_authoritative = true;
    legacy.legacy_metadata_snapshot_at = Some(legacy_snapshot_at);
    legacy.events = rebuilt_events;
    for agent in std::mem::take(&mut rebuilt.agents) {
        if !legacy
            .agents
            .iter()
            .any(|existing| existing.session_id == agent.session_id)
        {
            legacy.agents.push(agent);
        }
    }
    for container in std::mem::take(&mut rebuilt.execution_containers) {
        if !legacy
            .execution_containers
            .iter()
            .any(|existing| execution_container_same(existing, &container))
        {
            legacy.execution_containers.push(container);
        }
    }
    for (event_id, provenance) in std::mem::take(&mut rebuilt.duplicate_event_containers) {
        let target = legacy
            .duplicate_event_containers
            .entry(event_id)
            .or_default();
        for entry in provenance {
            if entry.container().is_none() {
                continue;
            }
            if !target.contains(&entry) {
                target.push(entry);
            }
        }
    }
    for board_ref in std::mem::take(&mut rebuilt.board_refs) {
        if !legacy.board_refs.contains(&board_ref) {
            legacy.board_refs.push(board_ref);
        }
    }
    for related in std::mem::take(&mut rebuilt.related_work_item_ids) {
        if !legacy.related_work_item_ids.contains(&related) {
            legacy.related_work_item_ids.push(related);
        }
    }
    legacy.created_at = legacy.created_at.min(rebuilt.created_at);
    *rebuilt = legacy;
}

fn work_event_sort_rank(kind: WorkEventKind) -> u8 {
    match kind {
        WorkEventKind::Start => 0,
        WorkEventKind::Backfill => 1,
        WorkEventKind::Claim | WorkEventKind::Resume => 2,
        WorkEventKind::Update | WorkEventKind::Handoff | WorkEventKind::Pr => 3,
        WorkEventKind::Blocked | WorkEventKind::Split | WorkEventKind::Merge => 4,
        WorkEventKind::Pause | WorkEventKind::Done | WorkEventKind::Discard => 5,
    }
}

fn warn_rejected_session_conflict(event: &WorkEvent) {
    tracing::warn!(
        target: "gwt::workspace_projection",
        work_item_id = %event.work_item_id,
        session_id = event.agent_session_id.as_deref().unwrap_or_default(),
        event_kind = ?event.kind,
        "refused duplicate session attach: session already bound to a Work \
         with a conflicting git identity (Issue #3216)"
    );
}

fn repair_duplicate_event_container(
    projection: &mut WorkItemsProjection,
    event: &WorkEvent,
) -> bool {
    let Some(container) = event.execution_container.as_ref() else {
        return false;
    };
    let Some(item) = projection
        .work_items
        .iter_mut()
        .find(|item| item.id == event.work_item_id)
    else {
        return false;
    };
    if !item.events.iter().any(|existing| existing.id == event.id) {
        return false;
    }
    let mut changed = false;
    if !item
        .execution_containers
        .iter()
        .any(|existing| execution_container_same(existing, container))
    {
        item.execution_containers.push(container.clone());
        changed = true;
    }
    let provenance = DuplicateWorkEventProvenance::Event(Box::new(event.clone()));
    let entries = item
        .duplicate_event_containers
        .entry(event.id.clone())
        .or_default();
    if !entries.contains(&provenance) {
        entries.push(provenance);
        changed = true;
    }
    changed
}

fn execution_container_same(
    left: &WorkspaceExecutionContainerRef,
    right: &WorkspaceExecutionContainerRef,
) -> bool {
    (left.branch.is_some() && left.branch == right.branch)
        || (left.worktree_path.is_some() && left.worktree_path == right.worktree_path)
        || (left.pr_number.is_some() && left.pr_number == right.pr_number)
        || (left.pr_url.is_some() && left.pr_url == right.pr_url)
}

/// Fingerprint cache mapping a source key (worktree path / ref name) to the
/// last-ingested content fingerprint (git blob oid or content sha256). A pure
/// optimization: deleting the file only costs re-reading sources, never
/// correctness (dedup is event-id based).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkEventsIntakeState {
    #[serde(default)]
    pub sources: BTreeMap<String, String>,
    #[serde(default)]
    pub projection_version: Option<String>,
}

impl WorkEventsIntakeState {
    /// True when `source` already ingested content with `fingerprint`.
    pub fn is_current(&self, source: &str, fingerprint: &str) -> bool {
        self.sources.get(source).map(String::as_str) == Some(fingerprint)
    }

    pub fn record(&mut self, source: impl Into<String>, fingerprint: impl Into<String>) {
        self.sources.insert(source.into(), fingerprint.into());
    }

    pub fn projection_is_current(&self, required: &str) -> bool {
        self.projection_version.as_deref() == Some(required)
    }

    pub fn record_projection_version(&mut self, version: impl Into<String>) {
        self.projection_version = Some(version.into());
    }
}

/// Load the intake state; missing or corrupt files yield the default state
/// (the cache is advisory).
pub fn load_work_events_intake_state(path: &Path) -> WorkEventsIntakeState {
    let Ok(body) = std::fs::read_to_string(path) else {
        return WorkEventsIntakeState::default();
    };
    serde_json::from_str(&body).unwrap_or_default()
}

pub fn save_work_events_intake_state(path: &Path, state: &WorkEventsIntakeState) -> Result<()> {
    let body = serde_json::to_vec_pretty(state)
        .map_err(|error| GwtError::Other(format!("work events intake state: {error}")))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::workspace_projection::write_atomic(path, &body)
}

/// Content sha256 used as the fingerprint for filesystem sources (git blob
/// oids serve the same purpose for ref sources).
pub fn content_fingerprint(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_projection::{WorkItemsProjection, WorkspaceStatusCategory};
    use chrono::TimeZone;

    fn event_json(id: &str, work_id: &str, kind: &str, updated_at: &str, extra: &str) -> String {
        format!(
            "{{\"id\":\"{id}\",\"work_item_id\":\"{work_id}\",\"kind\":\"{kind}\",\"updated_at\":\"{updated_at}\"{extra}}}"
        )
    }

    #[test]
    fn ingest_restores_work_skeleton_from_source_content() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let content = [
            event_json(
                "evt-1",
                "work-feature-aaaa1111",
                "start",
                "2026-06-01T10:00:00Z",
                ",\"title\":\"feature work\",\"intent\":\"build it\",\"status_category\":\"active\",\"board_entry_id\":\"board-1\",\"execution_container\":{\"branch\":\"work/feature\"}",
            ),
            event_json(
                "evt-2",
                "work-feature-aaaa1111",
                "update",
                "2026-06-02T11:00:00Z",
                ",\"summary\":\"halfway\"",
            ),
        ]
        .join("\n");

        let report = ingest_work_events_content(&works, &content).expect("ingest");
        assert_eq!(report.applied, 2);

        let projection = load_workspace_work_items_from_path(&works)
            .expect("load")
            .expect("projection exists");
        assert_eq!(projection.work_items.len(), 1);
        let item = &projection.work_items[0];
        assert_eq!(item.id, "work-feature-aaaa1111");
        assert_eq!(item.title, "feature work");
        assert_eq!(item.intent.as_deref(), Some("build it"));
        assert_eq!(item.summary.as_deref(), Some("halfway"));
        assert_eq!(
            item.created_at,
            chrono::Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap()
        );
        assert_eq!(
            item.updated_at,
            chrono::Utc.with_ymd_and_hms(2026, 6, 2, 11, 0, 0).unwrap()
        );
        assert!(item
            .execution_containers
            .iter()
            .any(|container| container.branch.as_deref() == Some("work/feature")));
        assert!(item.board_refs.iter().any(|entry| entry == "board-1"));
    }

    /// SC-260: re-ingesting the same source is a no-op, with or without any
    /// intake state cache — dedup rides the works.json event-id set.
    #[test]
    fn double_ingest_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let content = event_json(
            "evt-1",
            "work-x-bbbb2222",
            "start",
            "2026-06-01T10:00:00Z",
            ",\"title\":\"x\",\"status_category\":\"active\"",
        );

        let first = ingest_work_events_content(&works, &content).expect("first ingest");
        assert_eq!(first.applied, 1);
        let second = ingest_work_events_content(&works, &content).expect("second ingest");
        assert_eq!(second.applied, 0);
        assert_eq!(second.skipped_duplicate, 1);

        let projection = load_workspace_work_items_from_path(&works)
            .expect("load")
            .expect("projection");
        assert_eq!(projection.work_items.len(), 1);
        assert_eq!(projection.work_items[0].events.len(), 1);
    }

    #[test]
    fn duplicate_event_can_repair_missing_execution_container() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let legacy_content = event_json(
            "evt-legacy",
            "work-legacy-cccc3333",
            "update",
            "2026-06-01T10:00:00Z",
            ",\"title\":\"legacy work\",\"status_category\":\"active\"",
        );
        let repaired_content = event_json(
            "evt-legacy",
            "work-legacy-cccc3333",
            "update",
            "2026-06-01T10:00:00Z",
            ",\"title\":\"legacy work\",\"status_category\":\"active\",\"execution_container\":{\"branch\":\"work/legacy\"}",
        );

        ingest_work_events_content(&works, &legacy_content).expect("legacy ingest");
        let legacy_projection = load_workspace_work_items_from_path(&works)
            .expect("load legacy")
            .expect("legacy projection");
        assert!(
            legacy_projection.work_items[0]
                .execution_containers
                .is_empty(),
            "legacy event starts branch-less"
        );

        let repair = ingest_work_events_content(&works, &repaired_content).expect("repair ingest");
        assert_eq!(repair.applied, 1, "container repair is a projection change");

        let projection = load_workspace_work_items_from_path(&works)
            .expect("load repaired")
            .expect("repaired projection");
        assert!(projection.work_items[0]
            .execution_containers
            .iter()
            .any(|container| container.branch.as_deref() == Some("work/legacy")));

        let duplicate = ingest_work_events_content(&works, &repaired_content)
            .expect("duplicate repaired ingest");
        assert_eq!(duplicate.applied, 0);
        assert_eq!(duplicate.skipped_duplicate, 1);
    }

    #[test]
    fn accepted_event_duplicate_cannot_repair_a_conflicting_session_container() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let mut projection = WorkItemsProjection::empty(t0);

        let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t0);
        owner.id = "evt-z-owner".to_string();
        owner.agent_session_id = Some("session-owner".to_string());
        owner.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/owner".to_string()),
            worktree_path: Some("/repo/work/owner".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        projection.apply_event(owner);

        let mut accepted = WorkEvent::new(WorkEventKind::Start, "work-target", t0);
        accepted.id = "evt-accepted".to_string();
        accepted.agent_session_id = Some("session-target".to_string());
        accepted.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/target".to_string()),
            worktree_path: Some("/repo/feature/target".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        projection.apply_event(accepted.clone());
        save_workspace_work_items_projection_to_path(&works, &projection).unwrap();

        let mut conflicting_copy = accepted;
        conflicting_copy.updated_at = t1;
        conflicting_copy.agent_session_id = Some("session-owner".to_string());
        conflicting_copy.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/foreign".to_string()),
            worktree_path: Some("/repo/feature/foreign".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let content = serde_json::to_string(&conflicting_copy).unwrap();

        let report = ingest_work_events_content(&works, &content).expect("duplicate ingest");

        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped_duplicate, 1);
        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        let target = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-target")
            .unwrap();
        assert_eq!(target.execution_containers.len(), 1);
        assert!(target
            .execution_containers
            .iter()
            .all(|container| container.branch.as_deref() != Some("feature/foreign")));
    }

    #[test]
    fn single_conflicting_primary_is_counted_as_skipped() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let mut projection = WorkItemsProjection::empty(t0);

        let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t0);
        owner.agent_session_id = Some("session-owner".to_string());
        owner.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/owner".to_string()),
            worktree_path: Some("/repo/work/owner".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        projection.apply_event(owner);
        save_workspace_work_items_projection_to_path(&works, &projection).unwrap();

        let mut conflicting = WorkEvent::new(WorkEventKind::Update, "work-phantom", t1);
        conflicting.id = "evt-conflict".to_string();
        conflicting.agent_session_id = Some("session-owner".to_string());
        conflicting.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/foreign".to_string()),
            worktree_path: Some("/repo/feature/foreign".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });

        let content = serde_json::to_string(&conflicting).unwrap();
        let report = ingest_work_events_content(&works, &content).unwrap();

        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped_duplicate, 1);
        assert_eq!(report.skipped_close_kind, 0);
        assert_eq!(report.skipped_invalid, 0);
        assert_eq!(report.skipped_terminal, 0);
    }

    /// Close kinds are never ingested from any source (#3023 defence +
    /// FR-384: close state is machine-local).
    #[test]
    fn close_kinds_are_never_ingested() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let content = [
            event_json(
                "evt-1",
                "work-x-cccc3333",
                "start",
                "2026-06-01T10:00:00Z",
                ",\"title\":\"x\",\"status_category\":\"active\"",
            ),
            event_json(
                "evt-2",
                "work-x-cccc3333",
                "pause",
                "2026-06-01T11:00:00Z",
                "",
            ),
            event_json(
                "evt-3",
                "work-x-cccc3333",
                "done",
                "2026-06-01T12:00:00Z",
                "",
            ),
            event_json(
                "evt-4",
                "work-x-cccc3333",
                "discard",
                "2026-06-01T13:00:00Z",
                "",
            ),
        ]
        .join("\n");

        let report = ingest_work_events_content(&works, &content).expect("ingest");
        assert_eq!(report.applied, 1);
        assert_eq!(report.skipped_close_kind, 3);

        let projection = load_workspace_work_items_from_path(&works)
            .expect("load")
            .expect("projection");
        let item = &projection.work_items[0];
        assert!(!item.is_terminal(), "close kinds must not close the item");
        assert_ne!(item.status_category, WorkspaceStatusCategory::Done);
    }

    #[test]
    fn malformed_lines_are_skipped_leniently() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let content = format!(
            "not json at all\n{{\"id\":\"broken\"}}\n{}",
            event_json(
                "evt-1",
                "work-x-dddd4444",
                "start",
                "2026-06-01T10:00:00Z",
                ",\"title\":\"x\",\"status_category\":\"active\"",
            )
        );

        let report = ingest_work_events_content(&works, &content).expect("ingest");
        assert_eq!(report.applied, 1);
        assert_eq!(report.skipped_invalid, 2);
    }

    #[test]
    fn rebuild_propagates_unrelated_projection_io_errors() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works-is-a-directory");
        std::fs::create_dir(&works).expect("projection directory");
        let shared = event_json(
            "evt-recovery",
            "work-recovery",
            "start",
            "2026-07-16T10:00:00Z",
            ",\"title\":\"Recovered from events\",\"status_category\":\"active\"",
        );

        assert!(
            rebuild_work_events_contents(&works, [shared.as_str()], None).is_err(),
            "unrelated projection I/O failures must remain errors"
        );
    }

    #[test]
    fn rebuild_recovers_syntactically_corrupt_projection_from_complete_event_source() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        std::fs::write(&works, b"{\"updated_at\":").expect("corrupt projection");
        let shared = event_json(
            "evt-recovered",
            "work-recovered",
            "start",
            "2026-07-16T07:00:00Z",
            ",\"title\":\"Recovered from events\",\"status_category\":\"active\"",
        );

        let report = rebuild_work_events_contents(&works, [shared.as_str()], None)
            .expect("complete event history must replace corrupt projection JSON");

        assert_eq!(report.applied, 1);
        let projection = load_workspace_work_items_from_path(&works)
            .expect("load rebuilt projection")
            .expect("rebuilt projection");
        let recovered = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-recovered")
            .expect("Work recovered from complete event history");
        assert_eq!(recovered.title, "Recovered from events");
        assert!(recovered
            .events
            .iter()
            .any(|event| event.id == "evt-recovered"));
    }

    #[test]
    fn rebuild_preserves_syntactically_valid_incompatible_projection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let now = Utc.with_ymd_and_hms(2026, 7, 16, 7, 30, 0).unwrap();
        let mut projection = WorkItemsProjection::empty(now);
        projection.apply_event(WorkEvent::new(
            WorkEventKind::Start,
            "work-future-schema",
            now,
        ));
        let mut incompatible = serde_json::to_value(&projection).expect("projection json");
        incompatible["work_items"][0]
            .as_object_mut()
            .expect("Work item object")
            .insert(
                "future_schema_field".to_string(),
                serde_json::json!({ "preserve": true }),
            );
        let original = serde_json::to_vec_pretty(&incompatible).expect("incompatible json");
        std::fs::write(&works, &original).expect("write incompatible projection");
        let shared = event_json(
            "evt-known-copy",
            "work-known-copy",
            "start",
            "2026-07-16T08:00:00Z",
            ",\"title\":\"Known copy\",\"status_category\":\"active\"",
        );

        let error = rebuild_work_events_contents(&works, [shared.as_str()], None)
            .expect_err("valid incompatible projection must fail closed");

        assert!(
            !matches!(error, GwtError::Io(_)),
            "schema incompatibility must remain distinct from filesystem I/O"
        );
        assert_eq!(
            std::fs::read(&works).expect("read preserved projection"),
            original,
            "rebuild must not overwrite valid JSON written by a newer schema"
        );
    }

    #[test]
    fn conflicting_session_event_cannot_pollute_terminal_work_after_reload_or_reingest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let done_at = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let stray_at = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();
        let mut projection = WorkItemsProjection::empty(t0);

        let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t0);
        owner.agent_session_id = Some("session-owner".to_string());
        owner.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/issue-3272".to_string()),
            worktree_path: Some("/repo/work/issue-3272".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        projection.apply_event(owner);

        let mut target = WorkEvent::new(WorkEventKind::Start, "work-target", t0);
        target.agent_session_id = Some("session-target".to_string());
        target.title = Some("Original title".to_string());
        target.progress_summary = Some("Original progress".to_string());
        target.next_action = Some("Original next".to_string());
        target.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/spec-3273".to_string()),
            worktree_path: Some("/repo/feature/spec-3273".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        projection.apply_event(target);
        let mut done = WorkEvent::new(WorkEventKind::Done, "work-target", done_at);
        done.status_category = Some(WorkspaceStatusCategory::Done);
        projection.apply_event(done);
        save_workspace_work_items_projection_to_path(&works, &projection).unwrap();

        let mut stray = WorkEvent::new(WorkEventKind::Update, "work-target", stray_at);
        stray.agent_session_id = Some("session-owner".to_string());
        stray.status_category = Some(WorkspaceStatusCategory::Active);
        stray.title = Some("Foreign title".to_string());
        stray.progress_summary = Some("Foreign progress".to_string());
        stray.next_action = Some("Foreign next".to_string());
        stray.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/foreign".to_string()),
            worktree_path: Some("/repo/feature/foreign".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let line = serde_json::to_string(&stray).unwrap();
        let content = format!("{line}\n{line}");

        let first = ingest_work_events_content(&works, &content).unwrap();
        let second = ingest_work_events_content(&works, &content).unwrap();

        assert_eq!(first.applied, 0, "rejected event is not applied");
        assert_eq!(second.applied, 0, "rejected replay is not applied");
        assert!(!first.changed());
        assert!(!second.changed());

        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        assert_eq!(
            projection.updated_at, done_at,
            "rejected intake must not advance projection recency"
        );
        let target = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-target")
            .unwrap();
        assert_eq!(target.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(target.completed_at, Some(done_at));
        assert_eq!(target.updated_at, done_at);
        assert_eq!(target.title, "Original title");
        assert_eq!(
            target.progress_summary.as_deref(),
            Some("Original progress")
        );
        assert_eq!(target.latest_next_action(), Some("Original next"));
        assert_eq!(target.execution_containers.len(), 1);
        assert_eq!(target.events.len(), 2);
    }

    #[test]
    fn rebuild_accepts_mixed_machine_local_lifecycle_log() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();

        let mut start = WorkEvent::new(WorkEventKind::Start, "work-mixed", t0);
        start.id = "evt-start".to_string();
        start.title = Some("Mixed local lifecycle".to_string());
        let mut pause = WorkEvent::new(WorkEventKind::Pause, "work-mixed", t1);
        pause.id = "evt-pause".to_string();
        let mut board_ref = WorkEvent::new(WorkEventKind::Update, "work-mixed", t1);
        board_ref.id = "evt-board-ref".to_string();
        board_ref.board_entry_id = Some("board-verified".to_string());
        let mut done = WorkEvent::new(WorkEventKind::Done, "work-mixed", t2);
        done.id = "evt-done".to_string();
        done.status_category = Some(WorkspaceStatusCategory::Done);

        let shared = format!("{}\n", serde_json::to_string(&start).unwrap());
        let local = [pause, board_ref, done]
            .into_iter()
            .map(|event| serde_json::to_string(&event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");

        rebuild_work_events_contents(&works, std::iter::once(shared.as_str()), Some(&local))
            .expect("valid mixed local lifecycle log must rebuild");

        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-mixed")
            .unwrap();
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.completed_at, Some(t2));
        assert!(item
            .board_refs
            .iter()
            .any(|entry| entry == "board-verified"));
    }

    #[test]
    fn rebuild_merges_same_id_eventless_legacy_metadata_with_rebuilt_history() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();

        let mut legacy_projection = WorkItemsProjection::empty(t0);
        let mut legacy = WorkEvent::new(WorkEventKind::Start, "work-legacy", t0);
        legacy.title = Some("Legacy authoritative title".to_string());
        legacy.summary = Some("Legacy summary".to_string());
        legacy.owner = Some("legacy-owner".to_string());
        legacy_projection.apply_event(legacy);
        let mut done = WorkEvent::new(WorkEventKind::Done, "work-legacy", t1);
        done.status_category = Some(WorkspaceStatusCategory::Done);
        legacy_projection.apply_event(done);
        legacy_projection.work_items[0].events.clear();
        save_workspace_work_items_projection_to_path(&works, &legacy_projection).unwrap();

        let shared = event_json(
            "evt-shared",
            "work-legacy",
            "start",
            "2026-07-15T07:30:00Z",
            ",\"title\":\"Shared title\",\"status_category\":\"active\",\"execution_container\":{\"branch\":\"feature/spec-3273\"}",
        );
        rebuild_work_events_contents(&works, std::iter::once(shared.as_str()), None).unwrap();
        rebuild_work_events_contents(&works, std::iter::once(shared.as_str()), None).unwrap();

        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-legacy")
            .unwrap();
        assert_eq!(item.title, "Legacy authoritative title");
        assert_eq!(item.summary.as_deref(), Some("Legacy summary"));
        assert_eq!(item.owner.as_deref(), Some("legacy-owner"));
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.completed_at, Some(t1));
        assert!(item.events.iter().any(|event| event.id == "evt-shared"));
        assert!(item
            .execution_containers
            .iter()
            .any(|container| { container.branch.as_deref() == Some("feature/spec-3273") }));
    }

    #[test]
    fn rebuild_applies_lifecycle_events_newer_than_persisted_legacy_snapshots() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2026, 7, 15, 10, 0, 0).unwrap();

        let mut legacy_projection = WorkItemsProjection::empty(t0);
        for work_id in ["work-done", "work-discard", "work-reopen"] {
            let mut start = WorkEvent::new(WorkEventKind::Start, work_id, t0);
            start.title = Some(format!("Legacy {work_id}"));
            start.owner = Some("legacy-owner".to_string());
            start.status_category = Some(WorkspaceStatusCategory::Active);
            legacy_projection.apply_event(start);
        }
        let mut legacy_done = WorkEvent::new(WorkEventKind::Done, "work-reopen", t1);
        legacy_done.status_category = Some(WorkspaceStatusCategory::Done);
        legacy_projection.apply_event(legacy_done);
        for item in &mut legacy_projection.work_items {
            item.events.clear();
        }
        save_workspace_work_items_projection_to_path(&works, &legacy_projection).unwrap();

        let baseline = ["work-done", "work-discard", "work-reopen"]
            .into_iter()
            .map(|work_id| {
                event_json(
                    &format!("evt-baseline-{work_id}"),
                    work_id,
                    "start",
                    "2026-07-15T07:00:00Z",
                    ",\"status_category\":\"active\"",
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        rebuild_work_events_contents(&works, std::iter::once(baseline.as_str()), None).unwrap();

        let mut done = WorkEvent::new(WorkEventKind::Done, "work-done", t2);
        done.id = "evt-new-done".to_string();
        done.status_category = Some(WorkspaceStatusCategory::Done);
        let mut discard = WorkEvent::new(WorkEventKind::Discard, "work-discard", t2);
        discard.id = "evt-new-discard".to_string();
        let mut reopen = WorkEvent::new(WorkEventKind::Resume, "work-reopen", t2);
        reopen.id = "evt-new-reopen".to_string();
        reopen.status_category = Some(WorkspaceStatusCategory::Active);
        let local = [done, discard, reopen]
            .into_iter()
            .map(|event| serde_json::to_string(&event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");

        let mut metadata = WorkEvent::new(WorkEventKind::Update, "work-done", t3);
        metadata.id = "evt-new-metadata".to_string();
        metadata.title = Some("New title".to_string());
        metadata.summary = Some("New summary".to_string());
        metadata.owner = Some("new-owner".to_string());
        let shared = format!("{baseline}\n{}", serde_json::to_string(&metadata).unwrap());

        rebuild_work_events_contents(&works, std::iter::once(shared.as_str()), Some(&local))
            .unwrap();

        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        let done_item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-done")
            .unwrap();
        assert_eq!(done_item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(done_item.completed_at, Some(t2));
        assert_eq!(done_item.title, "New title");
        assert_eq!(done_item.summary.as_deref(), Some("New summary"));
        assert_eq!(done_item.owner.as_deref(), Some("new-owner"));
        assert!(done_item
            .events
            .iter()
            .any(|event| event.id == "evt-new-done"));

        let discarded_item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-discard")
            .unwrap();
        assert!(discarded_item.discarded);
        assert!(discarded_item.is_terminal());

        let reopened_item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-reopen")
            .unwrap();
        assert_eq!(
            reopened_item.status_category,
            WorkspaceStatusCategory::Active
        );
        assert!(!reopened_item.is_terminal());
        assert_eq!(reopened_item.completed_at, None);
    }

    #[test]
    fn legacy_snapshot_cutoff_does_not_advance_past_later_discovered_metadata() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2026, 7, 15, 10, 0, 0).unwrap();

        let mut legacy_projection = WorkItemsProjection::empty(t0);
        let mut legacy = WorkEvent::new(WorkEventKind::Start, "work-cutoff", t0);
        legacy.title = Some("Legacy title".to_string());
        legacy.owner = Some("legacy-owner".to_string());
        legacy.status_category = Some(WorkspaceStatusCategory::Active);
        legacy_projection.apply_event(legacy);
        legacy_projection.work_items[0].events.clear();
        save_workspace_work_items_projection_to_path(&works, &legacy_projection).unwrap();

        let mut baseline = WorkEvent::new(WorkEventKind::Start, "work-cutoff", t0);
        baseline.id = "evt-cutoff-baseline".to_string();
        baseline.status_category = Some(WorkspaceStatusCategory::Active);
        let mut heartbeat = WorkEvent::new(WorkEventKind::Update, "work-cutoff", t3);
        heartbeat.id = "evt-cutoff-heartbeat".to_string();
        let first_shared = [baseline.clone(), heartbeat.clone()]
            .into_iter()
            .map(|event| serde_json::to_string(&event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        rebuild_work_events_contents(&works, std::iter::once(first_shared.as_str()), None).unwrap();

        let mut discovered = WorkEvent::new(WorkEventKind::Update, "work-cutoff", t2);
        discovered.id = "evt-cutoff-discovered".to_string();
        discovered.title = Some("Discovered title".to_string());
        discovered.owner = Some("discovered-owner".to_string());
        let second_shared = [baseline, discovered, heartbeat]
            .into_iter()
            .map(|event| serde_json::to_string(&event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        rebuild_work_events_contents(&works, std::iter::once(second_shared.as_str()), None)
            .unwrap();
        rebuild_work_events_contents(&works, std::iter::once(second_shared.as_str()), None)
            .unwrap();

        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-cutoff")
            .unwrap();
        assert_eq!(item.title, "Discovered title");
        assert_eq!(item.owner.as_deref(), Some("discovered-owner"));
        assert!(item
            .events
            .iter()
            .any(|event| event.id == "evt-cutoff-discovered"));
    }

    #[test]
    fn later_duplicate_is_evaluated_after_intervening_session_owner_event() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();

        let mut primary = WorkEvent::new(WorkEventKind::Start, "work-target", t0);
        primary.id = "evt-duplicate".to_string();
        primary.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/target".to_string()),
            worktree_path: Some("/repo/feature/target".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t1);
        owner.id = "evt-owner".to_string();
        owner.agent_session_id = Some("session-owner".to_string());
        owner.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/owner".to_string()),
            worktree_path: Some("/repo/work/owner".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let mut later_copy = primary.clone();
        later_copy.updated_at = t2;
        later_copy.agent_session_id = Some("session-owner".to_string());
        later_copy.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/foreign".to_string()),
            worktree_path: Some("/repo/feature/foreign".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let content = [primary, owner, later_copy]
            .into_iter()
            .map(|event| serde_json::to_string(&event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");

        let report = ingest_work_events_content(&works, &content).unwrap();

        assert_eq!(report.applied, 2);
        assert_eq!(report.skipped_duplicate, 1);
        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        let target = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-target")
            .unwrap();
        assert!(target
            .execution_containers
            .iter()
            .all(|container| { container.branch.as_deref() != Some("feature/foreign") }));
        assert!(target
            .agents
            .iter()
            .all(|agent| agent.session_id != "session-owner"));
    }

    #[test]
    fn rejected_same_id_copy_does_not_hide_later_valid_timestamp_copy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 16, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 16, 8, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 16, 9, 0, 0).unwrap();

        let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t0);
        owner.id = "evt-owner".to_string();
        owner.agent_session_id = Some("session-owner".to_string());
        owner.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/owner".to_string()),
            worktree_path: Some("/repo/work/owner".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let mut conflicting = WorkEvent::new(WorkEventKind::Update, "work-target", t1);
        conflicting.id = "evt-retried-copy".to_string();
        conflicting.title = Some("Rejected copy".to_string());
        conflicting.agent_session_id = Some("session-owner".to_string());
        conflicting.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/foreign".to_string()),
            worktree_path: Some("/repo/feature/foreign".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let mut valid = conflicting.clone();
        valid.updated_at = t2;
        valid.title = Some("Accepted later copy".to_string());
        valid.agent_session_id = Some("session-target".to_string());
        valid.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/target".to_string()),
            worktree_path: Some("/repo/feature/target".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let shared = [owner, conflicting, valid]
            .into_iter()
            .map(|event| serde_json::to_string(&event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");

        let report = rebuild_work_events_contents(&works, [shared.as_str()], None).unwrap();

        assert_eq!(report.applied, 2, "owner and later valid copy must apply");
        assert_eq!(report.skipped_duplicate, 1, "only the conflict is skipped");
        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        let target = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-target")
            .expect("later valid copy must create the target Work");
        assert_eq!(target.title, "Accepted later copy");
        assert!(target
            .agents
            .iter()
            .any(|agent| agent.session_id == "session-target"));
        assert!(target
            .events
            .iter()
            .any(|event| event.id == "evt-retried-copy" && event.updated_at == t2));
    }

    #[test]
    fn cross_source_duplicate_conflict_keeps_deterministic_canonical_in_both_source_orders() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let forward_works = tmp.path().join("forward-works.json");
        let reverse_works = tmp.path().join("reverse-works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let mut seed = WorkItemsProjection::empty(t0);

        let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t0);
        owner.agent_session_id = Some("session-owner".to_string());
        owner.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/owner".to_string()),
            worktree_path: Some("/repo/work/owner".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        seed.apply_event(owner);
        save_workspace_work_items_projection_to_path(&forward_works, &seed).unwrap();
        save_workspace_work_items_projection_to_path(&reverse_works, &seed).unwrap();

        let mut compatible = WorkEvent::new(WorkEventKind::Update, "work-target", t1);
        compatible.id = "evt-duplicate".to_string();
        compatible.agent_session_id = Some("session-owner".to_string());
        compatible.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/owner".to_string()),
            worktree_path: Some("/repo/work/owner".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let mut conflicting = compatible.clone();
        conflicting.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/foreign".to_string()),
            worktree_path: Some("/repo/feature/foreign".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });

        let compatible_line = serde_json::to_string(&compatible).unwrap();
        let conflicting_line = serde_json::to_string(&conflicting).unwrap();
        let forward = [compatible_line.as_str(), conflicting_line.as_str()];
        let reverse = [conflicting_line.as_str(), compatible_line.as_str()];

        let forward_report =
            ingest_work_events_contents(&forward_works, forward).expect("forward ingest");
        let reverse_report =
            ingest_work_events_contents(&reverse_works, reverse).expect("reverse ingest");

        assert_eq!(forward_report.applied, 1);
        assert_eq!(reverse_report.applied, 1);
        let forward_projection = load_workspace_work_items_from_path(&forward_works)
            .unwrap()
            .unwrap();
        let reverse_projection = load_workspace_work_items_from_path(&reverse_works)
            .unwrap()
            .unwrap();
        assert_eq!(forward_projection.work_items, reverse_projection.work_items);
        let target = forward_projection
            .work_items
            .iter()
            .find(|item| item.id == "work-target")
            .expect("the compatible canonical copy must survive full rebuild");
        assert!(target
            .execution_containers
            .iter()
            .any(|container| { container.branch.as_deref() == Some("work/owner") }));
        assert!(target
            .execution_containers
            .iter()
            .all(|container| { container.branch.as_deref() != Some("feature/foreign") }));
    }

    #[test]
    fn duplicate_conflict_incremental_and_full_rebuild_converge() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let incremental_path = tmp.path().join("incremental.json");
        let rebuilt_path = tmp.path().join("rebuilt.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();

        let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t0);
        owner.id = "evt-owner".to_string();
        owner.agent_session_id = Some("session-owner".to_string());
        owner.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/owner".to_string()),
            worktree_path: Some("/repo/work/owner".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let mut compatible = WorkEvent::new(WorkEventKind::Update, "work-target", t1);
        compatible.id = "evt-duplicate".to_string();
        compatible.agent_session_id = Some("session-owner".to_string());
        compatible.execution_container = owner.execution_container.clone();
        let mut conflicting = compatible.clone();
        conflicting.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/foreign".to_string()),
            worktree_path: Some("/repo/feature/foreign".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });

        for event in [&owner, &compatible, &conflicting] {
            ingest_work_events_content(&incremental_path, &serde_json::to_string(event).unwrap())
                .unwrap();
        }
        let full_content = [&owner, &compatible, &conflicting]
            .into_iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        rebuild_work_events_contents(&rebuilt_path, [full_content.as_str()], None).unwrap();

        let incremental = load_workspace_work_items_from_path(&incremental_path)
            .unwrap()
            .unwrap();
        let rebuilt = load_workspace_work_items_from_path(&rebuilt_path)
            .unwrap()
            .unwrap();
        assert_eq!(incremental.work_items, rebuilt.work_items);
        assert!(incremental
            .work_items
            .iter()
            .any(|item| item.id == "work-target"));
    }

    /// Terminal items never apply events stamped at or before their close
    /// time: no re-open, no `updated_at` rollback (re-ingest of a Backfill
    /// committed elsewhere after the local close, FR-380 note).
    #[test]
    fn terminal_items_skip_events_at_or_before_close_time() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let closed_at = chrono::Utc.with_ymd_and_hms(2026, 6, 5, 12, 0, 0).unwrap();
        let mut projection = WorkItemsProjection::empty(closed_at);
        let mut done = crate::workspace_projection::WorkEvent::new(
            WorkEventKind::Done,
            "work-x-eeee5555",
            closed_at,
        );
        done.status_category = Some(WorkspaceStatusCategory::Done);
        done.title = Some("closed work".to_string());
        projection.apply_event(done);
        save_workspace_work_items_projection_to_path(&works, &projection).expect("seed");

        let content = [
            // Before the close: must be skipped.
            event_json(
                "evt-old",
                "work-x-eeee5555",
                "update",
                "2026-06-04T10:00:00Z",
                ",\"summary\":\"stale\"",
            ),
            // After the close: applies through normal apply_event semantics.
            event_json(
                "evt-new",
                "work-x-eeee5555",
                "update",
                "2026-06-06T10:00:00Z",
                ",\"summary\":\"fresh\"",
            ),
        ]
        .join("\n");

        let report = ingest_work_events_content(&works, &content).expect("ingest");
        assert_eq!(report.skipped_terminal, 1);
        assert_eq!(report.applied, 1);

        let projection = load_workspace_work_items_from_path(&works)
            .expect("load")
            .expect("projection");
        let item = &projection.work_items[0];
        assert!(
            item.updated_at >= closed_at,
            "updated_at must never roll back behind the close time"
        );
        assert_ne!(item.summary.as_deref(), Some("stale"));
    }

    /// Union-merge artifacts: the same event id appearing twice within one
    /// source content applies once.
    #[test]
    fn duplicate_event_ids_within_one_source_dedup() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let line = event_json(
            "evt-1",
            "work-x-ffff6666",
            "start",
            "2026-06-01T10:00:00Z",
            ",\"title\":\"x\",\"status_category\":\"active\"",
        );
        let content = format!("{line}\n{line}");

        let report = ingest_work_events_content(&works, &content).expect("ingest");
        assert_eq!(report.applied, 1);
        assert_eq!(report.skipped_duplicate, 1);
    }

    /// SC-261: intake writes works.json only — sibling state files are
    /// untouched byte-for-byte.
    #[test]
    fn ingest_leaves_sessions_current_and_journal_untouched() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let current = tmp.path().join("current.json");
        let journal = tmp.path().join("journal.jsonl");
        std::fs::write(&current, b"{\"current\":true}").expect("seed current");
        std::fs::write(&journal, b"{\"entry\":1}\n").expect("seed journal");

        let content = event_json(
            "evt-1",
            "work-x-aaaa7777",
            "start",
            "2026-06-01T10:00:00Z",
            ",\"title\":\"x\",\"status_category\":\"active\"",
        );
        ingest_work_events_content(&works, &content).expect("ingest");

        assert_eq!(
            std::fs::read(&current).expect("current"),
            b"{\"current\":true}"
        );
        assert_eq!(
            std::fs::read(&journal).expect("journal"),
            b"{\"entry\":1}\n"
        );
    }

    #[test]
    fn intake_state_roundtrip_and_lenient_load() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("work-events-intake.json");

        // Missing file → default.
        assert_eq!(
            load_work_events_intake_state(&path),
            WorkEventsIntakeState::default()
        );

        let mut state = WorkEventsIntakeState::default();
        state.record("origin/work/x", "blob-oid-1");
        save_work_events_intake_state(&path, &state).expect("save");
        let loaded = load_work_events_intake_state(&path);
        assert!(loaded.is_current("origin/work/x", "blob-oid-1"));
        assert!(!loaded.is_current("origin/work/x", "blob-oid-2"));

        // Corrupt file → default (advisory cache).
        std::fs::write(&path, b"{ not json").expect("corrupt");
        assert_eq!(
            load_work_events_intake_state(&path),
            WorkEventsIntakeState::default()
        );
    }

    #[test]
    fn content_fingerprint_is_stable_sha256() {
        assert_eq!(content_fingerprint("abc"), content_fingerprint("abc"));
        assert_ne!(content_fingerprint("abc"), content_fingerprint("abd"));
        assert_eq!(content_fingerprint("abc").len(), 64);
    }

    #[test]
    fn incremental_intake_refolds_late_earlier_events_with_accepted_history() {
        let temp = tempfile::tempdir().expect("tempdir");
        let incremental_path = temp.path().join("incremental.json");
        let one_pass_path = temp.path().join("one-pass.json");
        let later = event_json(
            "evt-later-owner",
            "work-later",
            "start",
            "2026-07-15T09:00:00Z",
            ",\"agent_session_id\":\"session-shared\",\"execution_container\":{\"branch\":\"work/later\",\"worktree_path\":\"/repo/work/later\"}",
        );
        let earlier = event_json(
            "evt-earlier-owner",
            "work-earlier",
            "start",
            "2026-07-15T08:00:00Z",
            ",\"agent_session_id\":\"session-shared\",\"execution_container\":{\"branch\":\"work/earlier\",\"worktree_path\":\"/repo/work/earlier\"}",
        );

        ingest_work_events_content(&incremental_path, &later).expect("later event first");
        let second = ingest_work_events_content(&incremental_path, &earlier)
            .expect("late discovery of earlier event");
        assert_eq!(second.applied, 1);

        ingest_work_events_contents(&one_pass_path, [later.as_str(), earlier.as_str()])
            .expect("globally ordered one-pass intake");
        let incremental = load_workspace_work_items_from_path(&incremental_path)
            .unwrap()
            .unwrap();
        let one_pass = load_workspace_work_items_from_path(&one_pass_path)
            .unwrap()
            .unwrap();
        assert_eq!(
            incremental.work_items, one_pass.work_items,
            "source arrival across runs must not change canonical Session ownership"
        );
        assert!(incremental
            .work_items
            .iter()
            .any(|item| item.id == "work-earlier"));
        assert!(incremental
            .work_items
            .iter()
            .all(|item| item.id != "work-later"));
    }

    #[test]
    fn late_owner_refold_restores_eventless_legacy_snapshot_without_rejected_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let incremental_path = temp.path().join("incremental.json");
        let one_pass_path = temp.path().join("one-pass.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();

        let mut legacy = WorkItemsProjection::empty(t0);
        let mut legacy_start = WorkEvent::new(WorkEventKind::Start, "work-later", t0);
        legacy_start.title = Some("Trusted legacy title".to_string());
        legacy_start.owner = Some("trusted-owner".to_string());
        legacy_start.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/later".to_string()),
            worktree_path: Some("/repo/work/later".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        legacy.apply_event(legacy_start);
        legacy.work_items[0].events.clear();
        save_workspace_work_items_projection_to_path(&incremental_path, &legacy).unwrap();
        save_workspace_work_items_projection_to_path(&one_pass_path, &legacy).unwrap();

        let later = event_json(
            "evt-later",
            "work-later",
            "update",
            "2026-07-15T09:00:00Z",
            ",\"title\":\"Rejected event title\",\"owner\":\"rejected-owner\",\"agent_session_id\":\"session-shared\",\"execution_container\":{\"branch\":\"work/later\",\"worktree_path\":\"/repo/work/later\"}",
        );
        let earlier = event_json(
            "evt-earlier",
            "work-owner",
            "start",
            "2026-07-15T08:00:00Z",
            ",\"agent_session_id\":\"session-shared\",\"execution_container\":{\"branch\":\"work/owner\",\"worktree_path\":\"/repo/work/owner\"}",
        );

        ingest_work_events_content(&incremental_path, &later).unwrap();
        ingest_work_events_content(&incremental_path, &earlier).unwrap();
        ingest_work_events_contents(&one_pass_path, [later.as_str(), earlier.as_str()]).unwrap();

        let incremental = load_workspace_work_items_from_path(&incremental_path)
            .unwrap()
            .unwrap();
        let one_pass = load_workspace_work_items_from_path(&one_pass_path)
            .unwrap()
            .unwrap();
        assert_eq!(incremental.work_items, one_pass.work_items);
        let restored = incremental
            .work_items
            .iter()
            .find(|item| item.id == "work-later")
            .unwrap();
        assert_eq!(restored.title, "Trusted legacy title");
        assert_eq!(restored.owner.as_deref(), Some("trusted-owner"));
        assert!(restored.events.is_empty());
        assert!(restored
            .agents
            .iter()
            .all(|agent| agent.session_id != "session-shared"));
    }

    #[test]
    fn late_owner_refold_drops_container_from_newly_rejected_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        let works = temp.path().join("works.json");
        let target = event_json(
            "evt-target",
            "work-target",
            "start",
            "2026-07-15T07:00:00Z",
            ",\"execution_container\":{\"branch\":\"work/target\",\"worktree_path\":\"/repo/work/target\"}",
        );
        let later = event_json(
            "evt-later",
            "work-target",
            "update",
            "2026-07-15T09:00:00Z",
            ",\"agent_session_id\":\"session-shared\",\"execution_container\":{\"branch\":\"feature/foreign\",\"worktree_path\":\"/repo/feature/foreign\"}",
        );
        let earlier = event_json(
            "evt-earlier",
            "work-owner",
            "start",
            "2026-07-15T08:00:00Z",
            ",\"agent_session_id\":\"session-shared\",\"execution_container\":{\"branch\":\"work/owner\",\"worktree_path\":\"/repo/work/owner\"}",
        );

        ingest_work_events_contents(&works, [target.as_str(), later.as_str()]).unwrap();
        ingest_work_events_content(&works, &earlier).unwrap();

        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        let target = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-target")
            .unwrap();
        assert!(target
            .execution_containers
            .iter()
            .all(|container| container.branch.as_deref() != Some("feature/foreign")));
        assert!(target.events.iter().all(|event| event.id != "evt-later"));
    }

    #[test]
    fn discarded_close_cutoff_is_stable_after_later_updates() {
        let temp = tempfile::tempdir().expect("tempdir");
        let incremental_path = temp.path().join("incremental.json");
        let rebuilt_path = temp.path().join("rebuilt.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2026, 7, 15, 10, 0, 0).unwrap();

        let mut start = WorkEvent::new(WorkEventKind::Start, "work-discarded", t0);
        start.id = "evt-start".to_string();
        let mut discard = WorkEvent::new(WorkEventKind::Discard, "work-discarded", t1);
        discard.id = "evt-discard".to_string();
        let mut discovered = WorkEvent::new(WorkEventKind::Update, "work-discarded", t2);
        discovered.id = "evt-discovered".to_string();
        discovered.summary = Some("late-discovered metadata".to_string());
        let mut heartbeat = WorkEvent::new(WorkEventKind::Update, "work-discarded", t3);
        heartbeat.id = "evt-heartbeat".to_string();

        let mut incremental = WorkItemsProjection::empty(t0);
        for event in [start.clone(), discard.clone(), heartbeat.clone()] {
            incremental.apply_event(event);
        }
        save_workspace_work_items_projection_to_path(&incremental_path, &incremental).unwrap();
        ingest_work_events_content(
            &incremental_path,
            &serde_json::to_string(&discovered).unwrap(),
        )
        .unwrap();

        let shared = [start, discovered, heartbeat]
            .into_iter()
            .map(|event| serde_json::to_string(&event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let local = serde_json::to_string(&discard).unwrap();
        rebuild_work_events_contents(&rebuilt_path, [shared.as_str()], Some(&local)).unwrap();

        let incremental = load_workspace_work_items_from_path(&incremental_path)
            .unwrap()
            .unwrap();
        let rebuilt = load_workspace_work_items_from_path(&rebuilt_path)
            .unwrap()
            .unwrap();
        assert_eq!(
            incremental.work_items, rebuilt.work_items,
            "incremental intake and full rebuild must use the original Discard instant"
        );
        let item = &incremental.work_items[0];
        assert!(item.discarded);
        assert!(item.events.iter().any(|event| event.id == "evt-discovered"));
    }

    #[test]
    fn late_session_owner_rechecks_duplicate_event_provenance() {
        let temp = tempfile::tempdir().expect("tempdir");
        let works = temp.path().join("works.json");
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();

        let mut primary = WorkEvent::new(WorkEventKind::Update, "work-target", t2);
        primary.id = "evt-duplicate".to_string();
        primary.title = Some("A canonical copy".to_string());
        primary.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/target".to_string()),
            worktree_path: Some("/repo/work/target".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let mut duplicate = primary.clone();
        duplicate.title = Some("Z duplicate copy".to_string());
        duplicate.agent_session_id = Some("session-shared".to_string());
        duplicate.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("feature/foreign".to_string()),
            worktree_path: Some("/repo/feature/foreign".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let first = [primary, duplicate]
            .into_iter()
            .map(|event| serde_json::to_string(&event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        ingest_work_events_content(&works, &first).unwrap();

        let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t1);
        owner.id = "evt-owner".to_string();
        owner.agent_session_id = Some("session-shared".to_string());
        owner.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/owner".to_string()),
            worktree_path: Some("/repo/work/owner".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        ingest_work_events_content(&works, &serde_json::to_string(&owner).unwrap()).unwrap();

        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        let target = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-target")
            .unwrap();
        assert!(target
            .execution_containers
            .iter()
            .all(|container| container.branch.as_deref() != Some("feature/foreign")));
        assert!(target
            .agents
            .iter()
            .all(|agent| agent.session_id != "session-shared"));
    }

    #[test]
    fn incremental_intake_retains_all_provenance_for_the_same_duplicate_container() {
        let temp = tempfile::tempdir().expect("tempdir");
        let incremental_path = temp.path().join("incremental.json");
        let rebuilt_path = temp.path().join("rebuilt.json");
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();

        let mut canonical = WorkEvent::new(WorkEventKind::Update, "work-target", t2);
        canonical.id = "evt-duplicate".to_string();
        canonical.title = Some("A canonical copy".to_string());
        let duplicate_container = WorkspaceExecutionContainerRef {
            branch: Some("feature/shared".to_string()),
            worktree_path: Some("/repo/feature/shared".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        };
        let mut conflicting_duplicate = canonical.clone();
        conflicting_duplicate.title = Some("B conflicting duplicate".to_string());
        conflicting_duplicate.agent_session_id = Some("session-shared".to_string());
        conflicting_duplicate.execution_container = Some(duplicate_container.clone());
        let mut valid_duplicate = canonical.clone();
        valid_duplicate.title = Some("C valid duplicate".to_string());
        valid_duplicate.execution_container = Some(duplicate_container.clone());

        let initial = [&canonical, &conflicting_duplicate, &valid_duplicate]
            .into_iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let initial_report = ingest_work_events_content(&incremental_path, &initial).unwrap();
        assert_eq!(
            initial_report.applied, 1,
            "multiple provenance variants for one event id count as one applied event"
        );

        let mut owner = WorkEvent::new(WorkEventKind::Start, "work-owner", t1);
        owner.id = "evt-owner".to_string();
        owner.agent_session_id = Some("session-shared".to_string());
        owner.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/owner".to_string()),
            worktree_path: Some("/repo/work/owner".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        ingest_work_events_content(&incremental_path, &serde_json::to_string(&owner).unwrap())
            .unwrap();

        let full_content = [&canonical, &conflicting_duplicate, &valid_duplicate, &owner]
            .into_iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        rebuild_work_events_contents(&rebuilt_path, [full_content.as_str()], None).unwrap();

        let incremental = load_workspace_work_items_from_path(&incremental_path)
            .unwrap()
            .unwrap();
        let rebuilt = load_workspace_work_items_from_path(&rebuilt_path)
            .unwrap()
            .unwrap();
        assert_eq!(incremental.work_items, rebuilt.work_items);
        let target = incremental
            .work_items
            .iter()
            .find(|item| item.id == "work-target")
            .unwrap();
        assert!(target
            .execution_containers
            .iter()
            .any(|container| execution_container_same(container, &duplicate_container)));
    }

    #[test]
    fn incremental_intake_counts_one_applied_event_across_timestamp_variants() {
        let temp = tempfile::tempdir().expect("tempdir");
        let works = temp.path().join("works.json");
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();

        let mut canonical = WorkEvent::new(WorkEventKind::Update, "work-target", t1);
        canonical.id = "evt-duplicate".to_string();
        canonical.title = Some("Canonical copy".to_string());

        let duplicate_container = WorkspaceExecutionContainerRef {
            branch: Some("feature/shared".to_string()),
            worktree_path: Some("/repo/feature/shared".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        };
        let mut later_duplicate = canonical.clone();
        later_duplicate.updated_at = t2;
        later_duplicate.execution_container = Some(duplicate_container.clone());

        let content = [&canonical, &later_duplicate]
            .into_iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let report = ingest_work_events_content(&works, &content).unwrap();

        assert_eq!(
            report.applied, 1,
            "one event id counts once even when provenance variants use different timestamps"
        );
        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        assert!(projection.work_items[0]
            .execution_containers
            .iter()
            .any(|container| execution_container_same(container, &duplicate_container)));
    }

    #[test]
    fn eventless_legacy_merge_retains_duplicate_event_provenance() {
        let temp = tempfile::tempdir().expect("tempdir");
        let works = temp.path().join("works.json");
        let t0 = Utc.with_ymd_and_hms(2026, 7, 15, 7, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 7, 15, 8, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap();

        let mut legacy = WorkItemsProjection::empty(t0);
        let mut legacy_start = WorkEvent::new(WorkEventKind::Start, "work-legacy", t0);
        legacy_start.title = Some("Legacy title".to_string());
        legacy.apply_event(legacy_start);
        legacy.work_items[0].events.clear();
        save_workspace_work_items_projection_to_path(&works, &legacy).unwrap();

        let mut primary = WorkEvent::new(WorkEventKind::Update, "work-legacy", t1);
        primary.id = "evt-duplicate".to_string();
        primary.title = Some("A canonical copy".to_string());
        primary.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/legacy".to_string()),
            worktree_path: Some("/repo/work/legacy".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let mut duplicate = primary.clone();
        duplicate.title = Some("Z duplicate copy".to_string());
        duplicate.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("origin/work/legacy".to_string()),
            worktree_path: Some("/other-machine/work/legacy".into()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        });
        let shared = [primary, duplicate]
            .into_iter()
            .map(|event| serde_json::to_string(&event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        rebuild_work_events_contents(&works, [shared.as_str()], None).unwrap();

        let mut unrelated = WorkEvent::new(WorkEventKind::Start, "work-unrelated", t2);
        unrelated.id = "evt-unrelated".to_string();
        ingest_work_events_content(&works, &serde_json::to_string(&unrelated).unwrap()).unwrap();

        let projection = load_workspace_work_items_from_path(&works)
            .unwrap()
            .unwrap();
        let legacy = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-legacy")
            .unwrap();
        assert!(legacy.execution_containers.iter().any(|container| {
            container.worktree_path.as_deref()
                == Some(std::path::Path::new("/other-machine/work/legacy"))
        }));
        assert!(
            !legacy.duplicate_event_containers.is_empty(),
            "legacy merge must carry duplicate provenance into the next refold"
        );
    }
}
