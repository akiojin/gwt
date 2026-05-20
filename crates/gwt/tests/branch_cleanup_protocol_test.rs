use gwt::{BackendEvent, BranchCleanupProgressPhase, FrontendEvent};
use serde_json::json;

#[test]
fn run_branch_cleanup_defaults_force_filesystem_delete_to_false() {
    let event = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "run_branch_cleanup",
        "id": "branches-1",
        "branches": ["work/old"],
        "delete_remote": false
    }))
    .expect("run_branch_cleanup should deserialize");

    match event {
        FrontendEvent::RunBranchCleanup {
            id,
            branches,
            delete_remote,
            force_filesystem_delete,
        } => {
            assert_eq!(id, "branches-1");
            assert_eq!(branches, vec!["work/old"]);
            assert!(!delete_remote);
            assert!(!force_filesystem_delete);
        }
        other => panic!("expected RunBranchCleanup, got {other:?}"),
    }
}

#[test]
fn run_workspace_cleanup_defaults_force_filesystem_delete_to_false() {
    let event = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "run_workspace_cleanup",
        "branch": "work/old",
        "delete_remote": true
    }))
    .expect("run_workspace_cleanup should deserialize");

    match event {
        FrontendEvent::RunWorkspaceCleanup {
            branch,
            delete_remote,
            force_filesystem_delete,
        } => {
            assert_eq!(branch, "work/old");
            assert!(delete_remote);
            assert!(!force_filesystem_delete);
        }
        other => panic!("expected RunWorkspaceCleanup, got {other:?}"),
    }
}

#[test]
fn branch_cleanup_progress_serializes_as_backend_event() {
    let event = BackendEvent::BranchCleanupProgress {
        id: "branches-1".to_string(),
        branch: "work/old".to_string(),
        execution_branch: Some("work/old".to_string()),
        index: 2,
        total: 5,
        phase: BranchCleanupProgressPhase::Running,
        message: "Removing work/old".to_string(),
    };

    let value = serde_json::to_value(&event).expect("serialize progress event");

    assert_eq!(value["kind"], "branch_cleanup_progress");
    assert_eq!(value["id"], "branches-1");
    assert_eq!(value["branch"], "work/old");
    assert_eq!(value["execution_branch"], "work/old");
    assert_eq!(value["index"], 2);
    assert_eq!(value["total"], 5);
    assert_eq!(value["phase"], "running");
    assert_eq!(value["message"], "Removing work/old");
}
