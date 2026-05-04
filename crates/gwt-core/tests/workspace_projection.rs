use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use gwt_core::{
    paths::gwt_workspace_projection_path,
    repo_hash::compute_repo_hash,
    workspace_projection::{
        load_or_default_workspace_projection_from_path, load_workspace_projection_from_path,
        save_workspace_projection_to_path, GitDetails, WorkspaceAgentSummary, WorkspaceProjection,
        WorkspaceStatusCategory,
    },
};

fn projection(project_root: &Path) -> WorkspaceProjection {
    WorkspaceProjection {
        id: "work-1".to_string(),
        project_root: project_root.to_path_buf(),
        title: "Start payment cleanup".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        status_text: "Refining the backend change".to_string(),
        owner: Some("SPEC-2359".to_string()),
        next_action: Some("Run focused tests".to_string()),
        agents: vec![WorkspaceAgentSummary {
            session_id: "sess-1".to_string(),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("backend foundation".to_string()),
            worktree_path: Some(project_root.join("../work/20260504-1200")),
            branch: Some("work/20260504-1200".to_string()),
            last_board_entry_id: Some("board-1".to_string()),
            updated_at: Utc.with_ymd_and_hms(2026, 5, 4, 12, 0, 0).unwrap(),
        }],
        git_details: Some(GitDetails {
            branch: Some("work/20260504-1200".to_string()),
            worktree_path: Some(project_root.join("../work/20260504-1200")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: None,
            pr_state: None,
            created_by_start_work: true,
            created_at: Utc.with_ymd_and_hms(2026, 5, 4, 12, 0, 0).unwrap(),
        }),
        board_refs: vec!["board-1".to_string()],
        updated_at: Utc.with_ymd_and_hms(2026, 5, 4, 12, 1, 0).unwrap(),
    }
}

#[test]
fn workspace_projection_path_is_project_scoped() {
    let repo_hash = compute_repo_hash("https://github.com/example/project.git");

    let path = gwt_workspace_projection_path(&repo_hash);

    assert!(path.ends_with(PathBuf::from(repo_hash.as_str()).join("workspace/current.json")));
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
