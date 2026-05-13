//! SPEC-2359 Phase U-6 (FR-142..145): retroactive migration for legacy
//! `workspace.json` files. Backfills the Phase U-6 schema additions
//! (`summary`, `created_at`, `creator`, `lifecycle_stage`) without
//! overwriting any values that the user / agent has already set.
//!
//! Boundary with SPEC-2359 US-37 (PR #2670): US-37 introduces a separate
//! `retroactive_scan` that emits `Done` events for previously-merged
//! WorkItems. The two migrations are independent — US-37 operates on
//! `journal.jsonl` / `work_events.jsonl`, while this module operates on
//! `workspace.json` metadata. They can run in either order during daemon
//! startup.
//!
//! The migration is exactly-once per Workspace via a sibling marker file
//! `workspace.migration.json` containing `{"version": 6}`. Subsequent
//! startups detect the marker and skip the Workspace. Missing or
//! unreadable files are silently skipped so daemon startup is never
//! blocked by file I/O errors.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{GwtError, Result};
use crate::paths::gwt_workspace_projection_path_for_repo_path;
#[cfg(test)]
use crate::workspace_projection::WorkspaceProjection;
use crate::workspace_projection::{
    load_workspace_projection_from_path, save_workspace_projection_to_path,
    workspace_projection_default_created_at, WorkspaceLifecycleStage, WorkspaceStatusCategory,
};

/// Schema version recorded in `workspace.migration.json`. When the
/// projection schema grows new fields in a future Phase, bump this
/// constant so the migration re-runs on existing data.
pub const WORKSPACE_PROJECTION_MIGRATION_VERSION: u32 = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkspaceProjectionMigrationMarker {
    version: u32,
    #[serde(default)]
    migrated_at: Option<DateTime<Utc>>,
}

/// Result of running [`migrate_workspace_projection_path`] on a single
/// projection file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceProjectionMigrationOutcome {
    /// The projection file does not exist (e.g. the Workspace has never
    /// been written). Nothing to do.
    Missing,
    /// A marker with the current version already exists. Skip silently.
    AlreadyMigrated,
    /// Migration backfilled at least one field on the projection and the
    /// marker was written. The amended projection has been saved.
    Applied,
    /// All schema fields already had real values, so no backfill was
    /// needed. The marker was still written so we do not re-scan.
    NoBackfillNeeded,
}

/// Run the Phase U-6 retroactive migration against a single
/// `workspace.json` file. The accompanying marker is written to
/// `workspace.migration.json` in the same directory.
///
/// Errors are surfaced for the caller to log; the daemon-level wrapper
/// downgrades all errors to debug logs so startup is not blocked.
pub fn migrate_workspace_projection_path(
    workspace_json_path: &Path,
) -> Result<WorkspaceProjectionMigrationOutcome> {
    if !workspace_json_path.exists() {
        return Ok(WorkspaceProjectionMigrationOutcome::Missing);
    }

    let marker_path = marker_path_for_projection(workspace_json_path);
    if marker_already_current(&marker_path)? {
        return Ok(WorkspaceProjectionMigrationOutcome::AlreadyMigrated);
    }

    let Some(mut projection) = load_workspace_projection_from_path(workspace_json_path)? else {
        return Ok(WorkspaceProjectionMigrationOutcome::Missing);
    };

    let mut changed = false;

    // FR-143: summary <- title when None. Skip if title is the
    // hard-coded default ("Workspace") so we do not inject the
    // placeholder into legitimately untitled Workspaces.
    if projection
        .summary
        .as_deref()
        .is_none_or(|s| s.trim().is_empty())
        && projection.title.trim() != "Workspace"
        && !projection.title.trim().is_empty()
    {
        projection.summary = Some(projection.title.clone());
        changed = true;
    }

    // FR-143: created_at <- updated_at when sentinel (legacy data).
    if projection.created_at == workspace_projection_default_created_at() {
        projection.created_at = projection.updated_at;
        changed = true;
    }

    // FR-143: lifecycle_stage <- derived from status_category when the
    // field is still at the schema default and the projection has any
    // signal that says "this is real work" (status_category != Unknown).
    if projection.lifecycle_stage == WorkspaceLifecycleStage::Planning
        && projection.status_category != WorkspaceStatusCategory::Unknown
    {
        projection.lifecycle_stage = lifecycle_stage_from_status(projection.status_category);
        changed = true;
    }

    // FR-143: creator <- first agent's agent_id (or fallback "system").
    if projection.creator.is_none() {
        let candidate = projection
            .agents
            .iter()
            .find(|agent| !agent.agent_id.trim().is_empty())
            .map(|agent| agent.agent_id.clone())
            .unwrap_or_else(|| "system".to_string());
        projection.creator = Some(candidate);
        changed = true;
    }

    if changed {
        save_workspace_projection_to_path(workspace_json_path, &projection)?;
    }
    write_marker(&marker_path)?;

    Ok(if changed {
        WorkspaceProjectionMigrationOutcome::Applied
    } else {
        WorkspaceProjectionMigrationOutcome::NoBackfillNeeded
    })
}

/// Convenience wrapper used by the daemon startup hook: takes a repository
/// root path, resolves the canonical Workspace projection JSON path via
/// [`gwt_workspace_projection_path_for_repo_path`], and runs the Phase U-6
/// migration. Mirrors the signature of
/// `workspace_projection::retroactive_auto_done_scan` (peer SPEC-2359 US-37)
/// so the two scans can be called from `AppRuntime::bootstrap` side by
/// side.
pub fn migrate_workspace_projection_for_repo(
    repo_path: &Path,
) -> Result<WorkspaceProjectionMigrationOutcome> {
    let projection_path = gwt_workspace_projection_path_for_repo_path(repo_path);
    migrate_workspace_projection_path(&projection_path)
}

fn marker_path_for_projection(workspace_json_path: &Path) -> PathBuf {
    workspace_json_path
        .parent()
        .map(|dir| dir.join("workspace.migration.json"))
        .unwrap_or_else(|| PathBuf::from("workspace.migration.json"))
}

fn marker_already_current(marker_path: &Path) -> Result<bool> {
    if !marker_path.exists() {
        return Ok(false);
    }
    let body = std::fs::read_to_string(marker_path)?;
    match serde_json::from_str::<WorkspaceProjectionMigrationMarker>(&body) {
        Ok(parsed) => Ok(parsed.version >= WORKSPACE_PROJECTION_MIGRATION_VERSION),
        Err(_) => Ok(false),
    }
}

fn write_marker(marker_path: &Path) -> Result<()> {
    if let Some(parent) = marker_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let marker = WorkspaceProjectionMigrationMarker {
        version: WORKSPACE_PROJECTION_MIGRATION_VERSION,
        migrated_at: Some(Utc::now()),
    };
    let body = serde_json::to_string_pretty(&marker)
        .map_err(|err| GwtError::Other(format!("serialize migration marker: {err}")))?;
    std::fs::write(marker_path, body)?;
    Ok(())
}

fn lifecycle_stage_from_status(status: WorkspaceStatusCategory) -> WorkspaceLifecycleStage {
    match status {
        WorkspaceStatusCategory::Active => WorkspaceLifecycleStage::Active,
        WorkspaceStatusCategory::Blocked => WorkspaceLifecycleStage::Active,
        WorkspaceStatusCategory::Idle => WorkspaceLifecycleStage::Active,
        WorkspaceStatusCategory::Done => WorkspaceLifecycleStage::Done,
        WorkspaceStatusCategory::Unknown => WorkspaceLifecycleStage::Planning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// FR-142, FR-143: a legacy projection with empty summary / sentinel
    /// created_at / default lifecycle / no creator gets backfilled on
    /// first run.
    #[test]
    fn migration_backfills_legacy_projection_on_first_run() {
        let temp = tempdir().expect("tempdir");
        let workspace_dir = temp.path().join("workspace");
        fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let projection_path = workspace_dir.join("current.json");
        let legacy_json = serde_json::json!({
            "id": "legacy-1",
            "project_root": "/repo",
            "title": "Legacy ticket title",
            "status_category": "active",
            "status_text": "Working on legacy data",
            "summary": null,
            "owner": null,
            "next_action": null,
            "agents": [],
            "git_details": null,
            "board_refs": [],
            "updated_at": "2026-04-15T10:00:00Z"
        });
        fs::write(
            &projection_path,
            serde_json::to_string(&legacy_json).expect("legacy json"),
        )
        .expect("write legacy json");

        let outcome =
            migrate_workspace_projection_path(&projection_path).expect("migrate legacy projection");

        assert_eq!(outcome, WorkspaceProjectionMigrationOutcome::Applied);
        let migrated: WorkspaceProjection =
            serde_json::from_slice(&fs::read(&projection_path).expect("read migrated"))
                .expect("parse migrated");
        assert_eq!(
            migrated.summary.as_deref(),
            Some("Legacy ticket title"),
            "summary must be backfilled from title"
        );
        assert_ne!(
            migrated.created_at,
            workspace_projection_default_created_at(),
            "created_at must move off the sentinel after migration"
        );
        assert_eq!(
            migrated.lifecycle_stage,
            WorkspaceLifecycleStage::Active,
            "lifecycle_stage must be derived from status_category"
        );
        assert_eq!(
            migrated.creator.as_deref(),
            Some("system"),
            "creator falls back to 'system' when no agent metadata is available"
        );

        let marker_path = workspace_dir.join("workspace.migration.json");
        assert!(marker_path.exists(), "migration marker must be persisted");
    }

    /// FR-144: marker prevents the migration from running twice on the
    /// same Workspace, even if the projection has new schema-default
    /// values after some intentional reset.
    #[test]
    fn migration_is_idempotent_when_marker_exists() {
        let temp = tempdir().expect("tempdir");
        let workspace_dir = temp.path().join("workspace");
        fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let projection_path = workspace_dir.join("current.json");
        let legacy_json = serde_json::json!({
            "id": "legacy-2",
            "project_root": "/repo",
            "title": "Already migrated",
            "status_category": "active",
            "status_text": "Done",
            "agents": [],
            "git_details": null,
            "board_refs": [],
            "updated_at": "2026-04-15T10:00:00Z"
        });
        fs::write(
            &projection_path,
            serde_json::to_string(&legacy_json).expect("json"),
        )
        .expect("write legacy");

        let first = migrate_workspace_projection_path(&projection_path).expect("first run");
        assert_eq!(first, WorkspaceProjectionMigrationOutcome::Applied);

        let second = migrate_workspace_projection_path(&projection_path).expect("second run");
        assert_eq!(second, WorkspaceProjectionMigrationOutcome::AlreadyMigrated);
    }

    /// FR-145: missing workspace.json must not error.
    #[test]
    fn migration_returns_missing_for_absent_projection_file() {
        let temp = tempdir().expect("tempdir");
        let projection_path = temp.path().join("workspace/current.json");

        let outcome = migrate_workspace_projection_path(&projection_path).expect("migrate missing");

        assert_eq!(outcome, WorkspaceProjectionMigrationOutcome::Missing);
    }

    /// FR-143: when no backfill is needed (e.g. all fields already have
    /// real values) the marker is still written so subsequent startups
    /// skip the file.
    #[test]
    fn migration_writes_marker_even_when_no_backfill_needed() {
        let temp = tempdir().expect("tempdir");
        let workspace_dir = temp.path().join("workspace");
        fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let projection_path = workspace_dir.join("current.json");
        let fresh = WorkspaceProjection {
            id: "fresh-1".to_string(),
            project_root: PathBuf::from("/repo"),
            title: "Fresh workspace".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            status_text: "Started".to_string(),
            summary: Some("Real summary".to_string()),
            owner: None,
            next_action: None,
            agents: Vec::new(),
            git_details: None,
            board_refs: Vec::new(),
            updated_at: Utc::now(),
            created_at: Utc::now(),
            creator: Some("codex".to_string()),
            lifecycle_stage: WorkspaceLifecycleStage::Active,
            blocked_reason: None,
            linked_issues: Vec::new(),
            linked_prs: Vec::new(),
            tags: Vec::new(),
            progress_pct: None,
        };
        fs::write(
            &projection_path,
            serde_json::to_string(&fresh).expect("json"),
        )
        .expect("write fresh");

        let outcome = migrate_workspace_projection_path(&projection_path).expect("migrate fresh");
        assert_eq!(
            outcome,
            WorkspaceProjectionMigrationOutcome::NoBackfillNeeded
        );
        let marker_path = workspace_dir.join("workspace.migration.json");
        assert!(
            marker_path.exists(),
            "marker must be written even when no fields changed"
        );
    }
}
