use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use gwt_core::{
    paths::{
        gwt_project_state_projection_path, gwt_workspace_projection_path,
        gwt_workspace_projection_path_for_repo_path,
    },
    repo_hash::compute_repo_hash,
    workspace_projection::{
        load_or_default_workspace_projection_from_path, load_workspace_projection_from_path,
        save_workspace_projection_to_path, GitDetails, WorkspaceAgentAffiliationStatus,
        WorkspaceAgentSummary, WorkspaceProjection, WorkspaceStatusCategory,
    },
};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .expect("env lock")
}

fn projection(project_root: &Path) -> WorkspaceProjection {
    WorkspaceProjection {
        id: "work-1".to_string(),
        project_root: project_root.to_path_buf(),
        title: "Start payment cleanup".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        status_text: "Refining the backend change".to_string(),
        summary: Some("Payment cleanup is in progress.".to_string()),
        owner: Some("SPEC-2359".to_string()),
        next_action: Some("Run focused tests".to_string()),
        agents: vec![WorkspaceAgentSummary {
            session_id: "sess-1".to_string(),
            window_id: Some("tab-1::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("backend foundation".to_string()),
            title_summary: None,
            worktree_path: Some(project_root.join("../work/20260504-1200")),
            branch: Some("work/20260504-1200".to_string()),
            last_board_entry_id: Some("board-1".to_string()),
            last_board_entry_kind: Some(gwt_core::coordination::BoardEntryKind::Status),
            coordination_scope: Some("SPEC-2359 / start-work".to_string()),
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: Some("work-1".to_string()),
            updated_at: Utc.with_ymd_and_hms(2026, 5, 4, 12, 0, 0).unwrap(),
        }],
        git_details: Some(GitDetails {
            branch: Some("work/20260504-1200".to_string()),
            worktree_path: Some(project_root.join("../work/20260504-1200")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: Utc.with_ymd_and_hms(2026, 5, 4, 12, 0, 0).unwrap(),
        }),
        board_refs: vec!["board-1".to_string()],
        updated_at: Utc.with_ymd_and_hms(2026, 5, 4, 12, 1, 0).unwrap(),
        created_at: Utc.with_ymd_and_hms(2026, 5, 4, 11, 30, 0).unwrap(),
        creator: Some("codex".to_string()),
        lifecycle_stage: gwt_core::workspace_projection::WorkspaceLifecycleStage::Active,
        blocked_reason: None,
        linked_issues: Vec::new(),
        linked_prs: Vec::new(),
        tags: Vec::new(),
        progress_pct: None,
    }
}

/// SPEC-2359 Phase U-6 (FR-132, FR-139, FR-143): `recompute_lifecycle_stage`
/// derives the high-level lifecycle stage from runtime status + linked PRs.
/// Done overrides everything; open PR routes to InReview; other activity
/// signals route to Active; Unknown stays in Planning.
#[test]
fn recompute_lifecycle_stage_follows_documented_precedence_rules() {
    use gwt_core::workspace_projection::{
        recompute_lifecycle_stage, WorkspaceLifecycleStage, WorkspacePrLink,
    };

    // Rule 1: Done status wins regardless of PR state.
    let open_pr = vec![WorkspacePrLink {
        number: 42,
        title: None,
        url: None,
        state: Some("open".to_string()),
    }];
    assert_eq!(
        recompute_lifecycle_stage(WorkspaceStatusCategory::Done, &open_pr),
        WorkspaceLifecycleStage::Done
    );

    // Rule 2: open PR routes Active to InReview.
    assert_eq!(
        recompute_lifecycle_stage(WorkspaceStatusCategory::Active, &open_pr),
        WorkspaceLifecycleStage::InReview
    );
    // Open is case-insensitive.
    let upper_open = vec![WorkspacePrLink {
        number: 7,
        title: None,
        url: None,
        state: Some("OPEN".to_string()),
    }];
    assert_eq!(
        recompute_lifecycle_stage(WorkspaceStatusCategory::Active, &upper_open),
        WorkspaceLifecycleStage::InReview
    );

    // Rule 3: Active / Blocked / Idle without an open PR route to Active.
    let no_pr: Vec<WorkspacePrLink> = Vec::new();
    assert_eq!(
        recompute_lifecycle_stage(WorkspaceStatusCategory::Active, &no_pr),
        WorkspaceLifecycleStage::Active
    );
    assert_eq!(
        recompute_lifecycle_stage(WorkspaceStatusCategory::Blocked, &no_pr),
        WorkspaceLifecycleStage::Active
    );
    assert_eq!(
        recompute_lifecycle_stage(WorkspaceStatusCategory::Idle, &no_pr),
        WorkspaceLifecycleStage::Active
    );

    // Rule 4: Unknown stays in Planning.
    assert_eq!(
        recompute_lifecycle_stage(WorkspaceStatusCategory::Unknown, &no_pr),
        WorkspaceLifecycleStage::Planning
    );

    // A merged PR (state != "open") does not promote to InReview.
    let merged_pr = vec![WorkspacePrLink {
        number: 9,
        title: None,
        url: None,
        state: Some("merged".to_string()),
    }];
    assert_eq!(
        recompute_lifecycle_stage(WorkspaceStatusCategory::Active, &merged_pr),
        WorkspaceLifecycleStage::Active
    );
}

/// SPEC-2359 Phase U-6 (FR-140): when a Workspace's `summary` is missing,
/// the first incoming Status / Claim / Handoff / Decision Board milestone
/// must backfill it from the entry body so Workspace Overview Detail pane
/// never stays on the "No Workspace summary yet" placeholder for in-progress
/// work.
#[test]
fn record_board_milestone_backfills_summary_when_missing_for_status_entry() {
    use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind};

    let mut projection = WorkspaceProjection::default_for_project("/repo");
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-1".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: None,
        branch: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some("workspace-1".to_string()),
        updated_at: Utc.with_ymd_and_hms(2026, 5, 13, 10, 0, 0).unwrap(),
    });
    projection.summary = None;
    let entry = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "Implementing the workspace summary backfill",
        None,
        None,
        Vec::new(),
        Vec::new(),
    )
    .with_origin_session_id("session-1");

    projection.record_board_milestone(&entry);

    assert_eq!(
        projection.summary.as_deref(),
        Some("Implementing the workspace summary backfill"),
        "summary must be backfilled from Status entry body when previously None"
    );
}

/// SPEC-2359 Phase U-6 (FR-141): a `Blocked` Board milestone must populate
/// `blocked_reason` distinctly from `status_text` so the Detail pane can
/// render a dedicated Blocked Reason section.
#[test]
fn record_board_milestone_sets_blocked_reason_separately_from_status_text() {
    use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind};

    let mut projection = WorkspaceProjection::default_for_project("/repo");
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-1".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: None,
        branch: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some("workspace-1".to_string()),
        updated_at: Utc.with_ymd_and_hms(2026, 5, 13, 10, 0, 0).unwrap(),
    });
    projection.blocked_reason = None;
    let entry = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Blocked,
        "Waiting for review on PR #2671",
        None,
        None,
        Vec::new(),
        Vec::new(),
    )
    .with_origin_session_id("session-1");

    projection.record_board_milestone(&entry);

    assert_eq!(
        projection.blocked_reason.as_deref(),
        Some("Waiting for review on PR #2671"),
        "blocked_reason must capture the entry body for Blocked milestones"
    );
    assert_eq!(
        projection.status_text, "Waiting for review on PR #2671",
        "status_text continues to mirror the milestone body for backward compatibility"
    );
}

/// SPEC-2359 Phase U-6 (FR-140): when `summary` is already populated, an
/// incoming Status / Claim / Handoff / Decision must NOT overwrite it. The
/// backfill is one-way and only fills empty values.
#[test]
fn record_board_milestone_preserves_existing_summary_on_status_entry() {
    use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind};

    let mut projection = WorkspaceProjection::default_for_project("/repo");
    projection.agents.push(WorkspaceAgentSummary {
        session_id: "session-1".to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: None,
        branch: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some("workspace-1".to_string()),
        updated_at: Utc.with_ymd_and_hms(2026, 5, 13, 10, 0, 0).unwrap(),
    });
    projection.summary = Some("Existing summary that should survive".to_string());
    let entry = BoardEntry::new(
        AuthorKind::Agent,
        "Codex",
        BoardEntryKind::Status,
        "A newer milestone body that should NOT clobber the summary",
        None,
        None,
        Vec::new(),
        Vec::new(),
    )
    .with_origin_session_id("session-1");

    projection.record_board_milestone(&entry);

    assert_eq!(
        projection.summary.as_deref(),
        Some("Existing summary that should survive"),
        "existing summary must survive Board milestone updates"
    );
}

/// SPEC-2359 Phase U-6 (FR-131, FR-143): legacy `workspace.json` files
/// written before the schema extension must continue to deserialize. The
/// new fields populate via `#[serde(default)]` so reads stay
/// backward-compatible; the retroactive migration backfills meaningful
/// values on the next startup.
#[test]
fn workspace_projection_legacy_json_deserializes_with_serde_defaults() {
    let legacy_json = serde_json::json!({
        "id": "legacy-1",
        "project_root": "/repo",
        "title": "Legacy workspace",
        "status_category": "active",
        "status_text": "Existing work",
        "summary": null,
        "owner": null,
        "next_action": null,
        "agents": [],
        "git_details": null,
        "board_refs": [],
        "updated_at": "2026-04-01T12:00:00Z"
    });

    let projection: WorkspaceProjection =
        serde_json::from_value(legacy_json).expect("legacy projection deserializes");

    assert_eq!(projection.id, "legacy-1");
    assert_eq!(projection.title, "Legacy workspace");
    // New fields populate via serde defaults so legacy data does not panic.
    assert_eq!(
        projection.created_at,
        gwt_core::workspace_projection::workspace_projection_default_created_at(),
        "legacy data lacks created_at; default is UNIX_EPOCH sentinel for migration"
    );
    assert_eq!(projection.creator, None);
    assert_eq!(
        projection.lifecycle_stage,
        gwt_core::workspace_projection::WorkspaceLifecycleStage::Planning,
        "legacy data defaults to Planning until migration recomputes"
    );
    assert_eq!(projection.blocked_reason, None);
    assert!(projection.linked_issues.is_empty());
    assert!(projection.linked_prs.is_empty());
    assert!(projection.tags.is_empty());
    assert_eq!(projection.progress_pct, None);
}

/// SPEC-2359 Phase U-6 (FR-131): every new schema field must survive a
/// serde round-trip so the retroactive migration backfill, GUI mutations,
/// and CLI updates all persist losslessly.
#[test]
fn workspace_projection_serde_round_trip_preserves_new_fields() {
    use gwt_core::workspace_projection::{
        WorkspaceIssueLink, WorkspaceLifecycleStage, WorkspacePrLink,
    };

    let original = WorkspaceProjection {
        id: "round-trip-1".to_string(),
        project_root: PathBuf::from("/repo"),
        title: "Schema round-trip".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        status_text: "Implementing Phase U-6".to_string(),
        summary: Some("Schema additions for Workspace Content Coherence".to_string()),
        owner: Some("SPEC-2359".to_string()),
        next_action: Some("Auto-populate".to_string()),
        agents: Vec::new(),
        git_details: None,
        board_refs: Vec::new(),
        updated_at: Utc.with_ymd_and_hms(2026, 5, 13, 12, 0, 0).unwrap(),
        created_at: Utc.with_ymd_and_hms(2026, 5, 13, 9, 0, 0).unwrap(),
        creator: Some("codex".to_string()),
        lifecycle_stage: WorkspaceLifecycleStage::InReview,
        blocked_reason: Some("Waiting for review".to_string()),
        linked_issues: vec![WorkspaceIssueLink {
            number: 2359,
            title: Some("SPEC-2359".to_string()),
            url: Some("https://github.com/akiojin/gwt/issues/2359".to_string()),
        }],
        linked_prs: vec![WorkspacePrLink {
            number: 2671,
            title: Some("Phase U-5 PR".to_string()),
            url: Some("https://github.com/akiojin/gwt/pull/2671".to_string()),
            state: Some("open".to_string()),
        }],
        tags: vec!["title-sync".to_string(), "phase-u-6".to_string()],
        progress_pct: Some(40),
    };

    let json = serde_json::to_string(&original).expect("serialize");
    let restored: WorkspaceProjection = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(restored, original);
}

#[test]
fn workspace_projection_path_is_project_scoped() {
    let repo_hash = compute_repo_hash("https://github.com/example/project.git");

    let path = gwt_workspace_projection_path(&repo_hash);

    assert!(path.ends_with(PathBuf::from(repo_hash.as_str()).join("project-state/current.json")));
    assert_eq!(path, gwt_project_state_projection_path(&repo_hash));
}

#[test]
fn workspace_projection_repo_path_migrates_legacy_workspace_current_json() {
    let temp_home = tempfile::tempdir().expect("home");
    let repo = tempfile::tempdir().expect("repo");
    let _guard = env_lock();
    let original_home = std::env::var_os("HOME");
    std::env::set_var("HOME", temp_home.path());

    let legacy_path =
        gwt_core::paths::gwt_project_dir_for_repo_path(repo.path()).join("workspace/current.json");
    save_workspace_projection_to_path(&legacy_path, &projection(repo.path()))
        .expect("save legacy projection");
    let new_path = gwt_workspace_projection_path_for_repo_path(repo.path());
    assert!(
        new_path.ends_with("project-state/current.json"),
        "new canonical path must be project-state/current.json"
    );
    assert!(
        !new_path.exists(),
        "test precondition: canonical project-state file should not exist"
    );

    let loaded = gwt_core::workspace_projection::load_workspace_projection(repo.path())
        .expect("migrate")
        .expect("migrated projection");

    assert_eq!(loaded.title, "Start payment cleanup");
    assert!(
        new_path.is_file(),
        "load must materialize migrated project-state file"
    );
    assert!(
        legacy_path.is_file(),
        "legacy workspace/current.json remains for non-destructive migration"
    );

    match original_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
}

#[test]
fn missing_projection_file_returns_default_for_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let path = temp.path().join("missing/current.json");

    let loaded =
        load_or_default_workspace_projection_from_path(&path, &project_root).expect("load default");

    assert_eq!(loaded.project_root, project_root);
    assert_eq!(loaded.status_category, WorkspaceStatusCategory::Unknown);
    assert!(loaded.agents.is_empty());
    assert!(loaded.git_details.is_none());
}

#[test]
fn projection_round_trips_through_json_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let path = temp.path().join("workspace/current.json");
    let expected = projection(&project_root);

    save_workspace_projection_to_path(&path, &expected).expect("save projection");
    let loaded = load_workspace_projection_from_path(&path)
        .expect("load projection")
        .expect("projection exists");

    assert_eq!(loaded, expected);
}

#[test]
fn save_projection_is_atomic_and_cleans_temp_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("repo");
    let path = temp.path().join("workspace/current.json");

    save_workspace_projection_to_path(&path, &projection(&project_root)).expect("save projection");

    let entries = std::fs::read_dir(path.parent().unwrap())
        .expect("read workspace dir")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(entries, vec!["current.json".to_string()]);
}
