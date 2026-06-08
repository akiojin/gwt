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
            operation_id,
        } => {
            assert_eq!(id, "branches-1");
            assert_eq!(branches, vec!["work/old"]);
            assert!(!delete_remote);
            assert!(!force_filesystem_delete);
            assert_eq!(operation_id, None);
        }
        other => panic!("expected RunBranchCleanup, got {other:?}"),
    }
}

#[test]
fn run_branch_cleanup_accepts_operation_id() {
    let event = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "run_branch_cleanup",
        "id": "branches-1",
        "branches": ["work/old"],
        "delete_remote": false,
        "force_filesystem_delete": true,
        "operation_id": "cleanup-op-1"
    }))
    .expect("run_branch_cleanup should deserialize operation_id");

    match event {
        FrontendEvent::RunBranchCleanup {
            operation_id,
            force_filesystem_delete,
            ..
        } => {
            assert!(force_filesystem_delete);
            assert_eq!(operation_id.as_deref(), Some("cleanup-op-1"));
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
            operation_id,
        } => {
            assert_eq!(branch, "work/old");
            assert!(delete_remote);
            assert!(!force_filesystem_delete);
            assert_eq!(operation_id, None);
        }
        other => panic!("expected RunWorkspaceCleanup, got {other:?}"),
    }
}

#[test]
fn sync_branch_cleanup_round_trips() {
    let event = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "sync_branch_cleanup",
        "id": "branches-1",
        "operation_id": "cleanup-op-1"
    }))
    .expect("sync_branch_cleanup should deserialize");

    match event {
        FrontendEvent::SyncBranchCleanup { id, operation_id } => {
            assert_eq!(id, "branches-1");
            assert_eq!(operation_id, "cleanup-op-1");
        }
        other => panic!("expected SyncBranchCleanup, got {other:?}"),
    }
}

#[test]
fn clear_branch_cleanup_status_round_trips() {
    let event = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "clear_branch_cleanup_status",
        "id": "branches-1",
        "operation_id": "cleanup-op-1"
    }))
    .expect("clear_branch_cleanup_status should deserialize");

    match event {
        FrontendEvent::ClearBranchCleanupStatus { id, operation_id } => {
            assert_eq!(id, "branches-1");
            assert_eq!(operation_id, "cleanup-op-1");
        }
        other => panic!("expected ClearBranchCleanupStatus, got {other:?}"),
    }
}

#[test]
fn branch_cleanup_progress_serializes_as_backend_event() {
    let event = BackendEvent::BranchCleanupProgress {
        id: "branches-1".to_string(),
        operation_id: Some("cleanup-op-1".to_string()),
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
    assert_eq!(value["operation_id"], "cleanup-op-1");
    assert_eq!(value["branch"], "work/old");
    assert_eq!(value["execution_branch"], "work/old");
    assert_eq!(value["index"], 2);
    assert_eq!(value["total"], 5);
    assert_eq!(value["phase"], "running");
    assert_eq!(value["message"], "Removing work/old");
}

#[test]
fn branch_cleanup_result_serializes_operation_id() {
    let event = BackendEvent::BranchCleanupResult {
        id: "branches-1".to_string(),
        operation_id: Some("cleanup-op-1".to_string()),
        results: Vec::new(),
    };

    let value = serde_json::to_value(&event).expect("serialize result event");

    assert_eq!(value["kind"], "branch_cleanup_result");
    assert_eq!(value["id"], "branches-1");
    assert_eq!(value["operation_id"], "cleanup-op-1");
    assert!(value["results"].as_array().is_some_and(Vec::is_empty));
}
