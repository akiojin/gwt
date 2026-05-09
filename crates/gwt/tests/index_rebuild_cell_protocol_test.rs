use gwt::{FrontendEvent, IndexRebuildScope};
use serde_json::json;

#[test]
fn frontend_event_rebuild_index_cell_deserializes_with_all_scopes() {
    let issues = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "rebuild_index_cell",
        "project_root": "/abs/repo",
        "scope": "issues"
    }))
    .expect("rebuild_index_cell scope=issues should deserialize");
    match issues {
        FrontendEvent::RebuildIndexCell {
            project_root,
            scope,
            worktree_hash,
        } => {
            assert_eq!(project_root, "/abs/repo");
            assert_eq!(scope, IndexRebuildScope::Issues);
            assert_eq!(worktree_hash, None);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let files = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "rebuild_index_cell",
        "project_root": "/abs/repo",
        "scope": "files",
        "worktree_hash": "wtAhash"
    }))
    .expect("rebuild_index_cell scope=files should deserialize");
    match files {
        FrontendEvent::RebuildIndexCell {
            project_root,
            scope,
            worktree_hash,
        } => {
            assert_eq!(project_root, "/abs/repo");
            assert_eq!(scope, IndexRebuildScope::Files);
            assert_eq!(worktree_hash, Some("wtAhash".to_string()));
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let files_docs = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "rebuild_index_cell",
        "project_root": "/abs/repo",
        "scope": "files-docs",
        "worktree_hash": "wtBhash"
    }))
    .expect("rebuild_index_cell scope=files-docs should deserialize");
    match files_docs {
        FrontendEvent::RebuildIndexCell {
            project_root: _,
            scope,
            worktree_hash,
        } => {
            assert_eq!(scope, IndexRebuildScope::FilesDocs);
            assert_eq!(worktree_hash, Some("wtBhash".to_string()));
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
