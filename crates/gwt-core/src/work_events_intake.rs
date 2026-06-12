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

use crate::error::{GwtError, Result};
use crate::work_projection::{
    load_workspace_work_items_from_path, save_workspace_work_items_projection_to_path, WorkEvent,
    WorkEventKind, WorkItemsProjection,
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

/// The moment a terminal item closed: `completed_at` when recorded, else its
/// last update. Events at or before this instant must not re-apply.
fn terminal_close_time(item: &crate::work_projection::WorkItem) -> Option<DateTime<Utc>> {
    item.is_terminal()
        .then(|| item.completed_at.unwrap_or(item.updated_at))
}

/// Ingest one source's raw `events.jsonl` content into the works projection
/// at `work_items_path`. Pure file-paths API (#3022): no HOME resolution.
pub fn ingest_work_events_content(
    work_items_path: &Path,
    content: &str,
) -> Result<WorkEventsIntakeReport> {
    let mut report = WorkEventsIntakeReport::default();
    let mut projection = load_workspace_work_items_from_path(work_items_path)?
        .unwrap_or_else(|| WorkItemsProjection::empty(Utc::now()));

    let mut seen_event_ids: HashSet<String> = projection
        .work_items
        .iter()
        .flat_map(|item| item.events.iter().map(|event| event.id.clone()))
        .collect();

    let mut incoming: Vec<WorkEvent> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let event: WorkEvent = match serde_json::from_str(line) {
            Ok(event) => event,
            Err(error) => {
                report.skipped_invalid += 1;
                tracing::warn!(%error, "work events intake: skipping malformed line");
                continue;
            }
        };
        if is_close_kind(event.kind) {
            report.skipped_close_kind += 1;
            continue;
        }
        if !seen_event_ids.insert(event.id.clone()) {
            // Covers both already-ingested events and duplicate lines inside
            // one source (git union-merge artifacts).
            report.skipped_duplicate += 1;
            continue;
        }
        incoming.push(event);
    }
    if incoming.is_empty() {
        return Ok(report);
    }
    incoming.sort_by_key(|event| event.updated_at);

    for event in incoming {
        let terminal_cutoff = projection
            .work_items
            .iter()
            .find(|item| item.id == event.work_item_id)
            .and_then(terminal_close_time);
        if let Some(closed_at) = terminal_cutoff {
            if event.updated_at <= closed_at {
                report.skipped_terminal += 1;
                continue;
            }
        }
        projection.apply_event(event);
        report.applied += 1;
    }

    if report.applied > 0 {
        projection.updated_at = Utc::now();
        save_workspace_work_items_projection_to_path(work_items_path, &projection)?;
    }
    Ok(report)
}

/// Fingerprint cache mapping a source key (worktree path / ref name) to the
/// last-ingested content fingerprint (git blob oid or content sha256). A pure
/// optimization: deleting the file only costs re-reading sources, never
/// correctness (dedup is event-id based).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkEventsIntakeState {
    #[serde(default)]
    pub sources: BTreeMap<String, String>,
}

impl WorkEventsIntakeState {
    /// True when `source` already ingested content with `fingerprint`.
    pub fn is_current(&self, source: &str, fingerprint: &str) -> bool {
        self.sources.get(source).map(String::as_str) == Some(fingerprint)
    }

    pub fn record(&mut self, source: impl Into<String>, fingerprint: impl Into<String>) {
        self.sources.insert(source.into(), fingerprint.into());
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
    crate::work_projection::write_atomic(path, &body)
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
    use crate::work_projection::{WorkItemsProjection, WorkspaceStatusCategory};
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

    /// Terminal items never apply events stamped at or before their close
    /// time: no re-open, no `updated_at` rollback (re-ingest of a Backfill
    /// committed elsewhere after the local close, FR-380 note).
    #[test]
    fn terminal_items_skip_events_at_or_before_close_time() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let works = tmp.path().join("works.json");
        let closed_at = chrono::Utc.with_ymd_and_hms(2026, 6, 5, 12, 0, 0).unwrap();
        let mut projection = WorkItemsProjection::empty(closed_at);
        let mut done = crate::work_projection::WorkEvent::new(
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
}
