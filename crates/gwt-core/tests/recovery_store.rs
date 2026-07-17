use chrono::{Duration, TimeZone, Utc};
use gwt_core::recovery::{
    build_checkpoint_continuation_prompt, build_checkpoint_continuation_prompt_with_attachments,
    BindingQuality, BoardMilestoneIntent, CheckpointCoverage, CreateRecovery, ProviderRootBinding,
    ProviderRootCandidate, ProviderRootRole, RecoveryAttachmentPayload, RecoveryContinuationLink,
    RecoveryLaunchStage, RecoveryLease, RecoveryLifecycle, RecoveryRecord, RecoverySessionKind,
    RecoveryStore, RecoveryStoreError, RecoveryStoreFaultPoint, RootTurnUpdate, SemanticCheckpoint,
    VisibleDiscussionItem,
};
use gwt_core::repo_hash::compute_repo_hash;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
struct SnapshotBodyFixture {
    schema_version: u32,
    recovery_id: String,
    generation: u64,
    record: RecoveryRecord,
    #[serde(default)]
    operation_digests: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct StoredSnapshotFixture {
    body: SnapshotBodyFixture,
    checksum: String,
}

fn recovery_payload_dir(root: &std::path::Path, recovery_id: &str) -> std::path::PathBuf {
    root.join("recoveries").join(recovery_id)
}

fn durable_attachment_path(
    root: &std::path::Path,
    attachment: &gwt_core::recovery::RecoveryAttachmentRef,
) -> std::path::PathBuf {
    let digest = attachment.content_id.strip_prefix("sha256:").unwrap();
    root.join("attachments")
        .join("sha256")
        .join(&digest[..2])
        .join(digest)
}

fn operation_receipt_path_for(
    root: &std::path::Path,
    recovery_id: &str,
    operation_id: &str,
) -> std::path::PathBuf {
    recovery_payload_dir(root, recovery_id)
        .join("operations")
        .join(format!(
            "{}.json",
            hex::encode(Sha256::digest(operation_id.as_bytes()))
        ))
}

fn rewrite_latest_snapshot(
    root: &std::path::Path,
    recovery_id: &str,
    update: impl FnOnce(&mut SnapshotBodyFixture),
) {
    let snapshots_dir = recovery_payload_dir(root, recovery_id).join("snapshots");
    let mut snapshots = std::fs::read_dir(&snapshots_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    snapshots.sort();
    let path = snapshots.pop().expect("latest recovery snapshot");
    let mut stored: StoredSnapshotFixture =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    update(&mut stored.body);
    stored.checksum = hex::encode(Sha256::digest(serde_json::to_vec(&stored.body).unwrap()));
    std::fs::write(path, serde_json::to_vec_pretty(&stored).unwrap()).unwrap();
}

fn create_request(recovery_id: &str, worktree: &std::path::Path) -> CreateRecovery {
    CreateRecovery {
        recovery_id: recovery_id.to_string(),
        session_id: "session-intake-1".to_string(),
        repo_id: "repo-123".to_string(),
        session_kind: RecoverySessionKind::Intake,
        worktree_path: worktree.to_path_buf(),
        launch_base_ref: Some("origin/develop".to_string()),
        launch_base_oid: "1111111111111111111111111111111111111111".to_string(),
        launch_head_oid: "1111111111111111111111111111111111111111".to_string(),
        provider: "codex".to_string(),
        model: Some("gpt-5.5".to_string()),
        runtime: "host".to_string(),
        initial_prompt: "Investigate Intake recovery".to_string(),
        created_at: Utc.with_ymd_and_hms(2026, 7, 16, 1, 2, 3).unwrap(),
    }
}

fn create_exact_recovery(
    store: &RecoveryStore,
    recovery_id: &str,
    session_id: &str,
    worktree: &std::path::Path,
    provider_root_id: &str,
) {
    let mut request = create_request(recovery_id, worktree);
    request.session_id = session_id.to_string();
    store
        .create(request, format!("create-{recovery_id}"))
        .unwrap();
    store
        .bind_root(
            recovery_id,
            ProviderRootBinding {
                root_id: provider_root_id.to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            format!("bind-{recovery_id}"),
        )
        .unwrap();
}

fn recovery_lease(token: &str, acquired_at: chrono::DateTime<Utc>) -> RecoveryLease {
    RecoveryLease {
        lease_id: token.to_string(),
        holder_id: format!("holder-{token}"),
        acquired_at,
        expires_at: acquired_at + Duration::minutes(2),
    }
}

#[test]
fn recovery_record_roundtrips_immutable_launch_identity() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("intake");
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let request = create_request("recovery-a", &worktree);

    let created = store.create(request.clone(), "create-a").unwrap();
    let loaded = store.load("recovery-a").unwrap().unwrap();

    assert_eq!(created, loaded);
    assert_eq!(loaded.generation, 1);
    assert_eq!(loaded.session_kind, RecoverySessionKind::Intake);
    assert_eq!(loaded.worktree_path, worktree);
    assert_eq!(loaded.launch_base_ref.as_deref(), Some("origin/develop"));
    assert_eq!(loaded.launch_base_oid, request.launch_base_oid);
    assert_eq!(loaded.initial_prompt, "Investigate Intake recovery");
    assert_eq!(loaded.lifecycle, RecoveryLifecycle::Launching);
    assert_eq!(
        loaded.launch_stage,
        RecoveryLaunchStage::WorktreeMaterialized
    );
    assert_eq!(loaded.root_role, ProviderRootRole::Unknown);
}

#[test]
fn recovery_launch_stage_and_root_role_are_durable_monotonic_store_state() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();

    let spawned = store
        .advance_launch_stage(
            "recovery-a",
            RecoveryLaunchStage::ProcessSpawned,
            None,
            "spawned-a",
        )
        .unwrap();
    assert_eq!(spawned.launch_stage, RecoveryLaunchStage::ProcessSpawned);
    assert_eq!(spawned.root_role, ProviderRootRole::Unknown);

    let bound = store
        .bind_root_semantic(
            "recovery-a",
            "root-a",
            None,
            BindingQuality::Verified,
            "bind-a",
        )
        .unwrap();
    assert_eq!(bound.launch_stage, RecoveryLaunchStage::ProviderBound);
    assert_eq!(bound.root_role, ProviderRootRole::Root);

    let ready = store
        .advance_launch_stage(
            "recovery-a",
            RecoveryLaunchStage::Ready,
            Some(ProviderRootRole::Root),
            "ready-a",
        )
        .unwrap();
    assert_eq!(ready.launch_stage, RecoveryLaunchStage::Ready);

    let replayed_older_boundary = store
        .advance_launch_stage(
            "recovery-a",
            RecoveryLaunchStage::ProcessSpawned,
            None,
            "spawned-replayed-a",
        )
        .unwrap();
    assert_eq!(
        replayed_older_boundary.launch_stage,
        RecoveryLaunchStage::Ready
    );

    let tombstone = store
        .finalize_and_purge(
            "recovery-a",
            RecoveryLifecycle::Resolved,
            Utc::now(),
            "resolve-a",
        )
        .unwrap();
    assert_eq!(tombstone.launch_stage, RecoveryLaunchStage::Resolved);
    assert_eq!(tombstone.root_role, ProviderRootRole::Root);
}

#[test]
fn preassigned_exact_root_keeps_two_phase_spawn_boundary_observable() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    let preassigned = store
        .bind_root(
            "recovery-a",
            ProviderRootBinding {
                root_id: "root-a".to_string(),
                session_tree_id: None,
                quality: BindingQuality::Preassigned,
                bound_at: Utc::now(),
            },
            "preassign-a",
        )
        .unwrap();
    assert_eq!(
        preassigned.launch_stage,
        RecoveryLaunchStage::WorktreeMaterialized,
        "historical exact target selection is not a live provider bind"
    );
    assert_eq!(preassigned.root_role, ProviderRootRole::Root);

    let requested = store
        .advance_launch_stage(
            "recovery-a",
            RecoveryLaunchStage::SpawnRequested,
            Some(ProviderRootRole::Root),
            "spawn-requested-a",
        )
        .unwrap();
    assert_eq!(requested.launch_stage, RecoveryLaunchStage::SpawnRequested);

    let spawned = store
        .advance_launch_stage(
            "recovery-a",
            RecoveryLaunchStage::ProcessSpawned,
            Some(ProviderRootRole::Root),
            "process-spawned-a",
        )
        .unwrap();
    assert_eq!(spawned.launch_stage, RecoveryLaunchStage::ProcessSpawned);

    let rebound = store
        .bind_root_semantic(
            "recovery-a",
            "root-a",
            None,
            BindingQuality::Verified,
            "verified-bind-a",
        )
        .unwrap();
    assert_eq!(rebound.launch_stage, RecoveryLaunchStage::ProviderBound);
}

#[test]
fn spawn_requested_event_survives_snapshot_crash_and_prevents_unknown_relaunch_state() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    RecoveryStore::new(&root)
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    let faulted = RecoveryStore::new(&root)
        .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterEventPublication);

    assert!(matches!(
        faulted.advance_launch_stage(
            "recovery-a",
            RecoveryLaunchStage::SpawnRequested,
            None,
            "spawn-requested-a",
        ),
        Err(RecoveryStoreError::InjectedFault(
            RecoveryStoreFaultPoint::AfterEventPublication
        ))
    ));

    let restarted = RecoveryStore::new(&root)
        .load("recovery-a")
        .unwrap()
        .unwrap();
    assert_eq!(restarted.launch_stage, RecoveryLaunchStage::SpawnRequested);
}

#[test]
fn subagent_role_cannot_bind_or_author_root_checkpoint_content() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    store
        .advance_launch_stage(
            "recovery-a",
            RecoveryLaunchStage::WorktreeMaterialized,
            Some(ProviderRootRole::Subagent),
            "observe-subagent-a",
        )
        .unwrap();

    assert!(matches!(
        store.bind_root_semantic(
            "recovery-a",
            "child-root",
            None,
            BindingQuality::Verified,
            "bind-child-a",
        ),
        Err(RecoveryStoreError::RootRoleRejected(
            ProviderRootRole::Subagent
        ))
    ));
    assert!(matches!(
        store.replace_checkpoint(
            "recovery-a",
            "child-root",
            0,
            SemanticCheckpoint::default(),
            "checkpoint-child-a",
        ),
        Err(RecoveryStoreError::RootRoleRejected(
            ProviderRootRole::Subagent
        ))
    ));
}

#[test]
fn recovery_record_without_stage_and_role_remains_deserializable() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let record = store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    let mut legacy = serde_json::to_value(record).unwrap();
    let object = legacy.as_object_mut().unwrap();
    object.remove("launch_stage");
    object.remove("root_role");

    let restored: gwt_core::recovery::RecoveryRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.launch_stage, RecoveryLaunchStage::Created);
    assert_eq!(restored.root_role, ProviderRootRole::Unknown);
}

#[test]
fn operation_id_retry_is_idempotent_and_payload_change_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let request = create_request("recovery-a", &temp.path().join("intake"));

    let first = store.create(request.clone(), "create-a").unwrap();
    let retry = store.create(request.clone(), "create-a").unwrap();
    assert_eq!(retry.generation, first.generation);

    let mut changed = request;
    changed.initial_prompt = "different payload".to_string();
    assert!(matches!(
        store.create(changed, "create-a"),
        Err(RecoveryStoreError::OperationConflict { .. })
    ));
}

#[test]
fn operation_receipts_keep_long_session_snapshots_bounded_and_paths_private() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let store = RecoveryStore::new(&root);
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();

    for index in 0..128 {
        let operation_id = format!("../../untrusted-operation-{index}-{}", "x".repeat(3_500));
        store
            .ack_board_entry("recovery-a", &format!("board-entry-{index}"), operation_id)
            .unwrap();
    }

    let loaded = store.load("recovery-a").unwrap().unwrap();
    assert_eq!(loaded.board_entry_ids.len(), 128);
    assert_eq!(loaded.board_entry_ids.first().unwrap(), "board-entry-0");
    assert_eq!(loaded.board_entry_ids.last().unwrap(), "board-entry-127");

    let payload_dir = recovery_payload_dir(&root, "recovery-a");
    let operation_paths = std::fs::read_dir(payload_dir.join("operations"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(operation_paths.len(), 129);
    assert!(operation_paths.iter().all(|path| {
        let file_name = path.file_name().unwrap().to_string_lossy();
        file_name.len() == 69
            && file_name.ends_with(".json")
            && file_name[..64]
                .bytes()
                .all(|value| value.is_ascii_hexdigit())
            && !file_name.contains("untrusted-operation")
            && !file_name.contains("..")
    }));

    for snapshot in std::fs::read_dir(payload_dir.join("snapshots")).unwrap() {
        let path = snapshot.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(path).unwrap();
        let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stored["body"]["operation_digests"], serde_json::json!({}));
        assert!(bytes.len() < 128 * 1_024);
    }

    let final_operation_id = format!("../../untrusted-operation-127-{}", "x".repeat(3_500));
    let generation = loaded.generation;
    let retried = store
        .ack_board_entry("recovery-a", "board-entry-127", final_operation_id)
        .unwrap();
    assert_eq!(retried.generation, generation);
}

#[test]
fn legacy_inline_operation_map_migrates_before_the_next_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let store = RecoveryStore::new(&root);
    let request = create_request("recovery-a", &temp.path().join("intake"));
    let created = store.create(request.clone(), "create-a").unwrap();
    let receipt_path = operation_receipt_path_for(&root, "recovery-a", "create-a");
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    let mutation_digest = receipt["body"]["mutation_digest"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::remove_file(&receipt_path).unwrap();
    rewrite_latest_snapshot(&root, "recovery-a", |body| {
        body.operation_digests
            .insert("create-a".to_string(), mutation_digest);
    });

    assert!(store
        .has_committed_operation("recovery-a", "create-a")
        .unwrap());
    let faulted = RecoveryStore::new(&root)
        .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterOperationReceiptPublication);
    assert!(matches!(
        faulted.set_lifecycle(
            "recovery-a",
            RecoveryLifecycle::Running,
            None,
            "new-mutation-after-legacy",
        ),
        Err(RecoveryStoreError::InjectedFault(
            RecoveryStoreFaultPoint::AfterOperationReceiptPublication
        ))
    ));
    let still_legacy = store.load("recovery-a").unwrap().unwrap();
    assert_eq!(still_legacy.generation, created.generation);
    assert_eq!(still_legacy.lifecycle, RecoveryLifecycle::Launching);

    let advanced = store
        .set_lifecycle(
            "recovery-a",
            RecoveryLifecycle::Running,
            None,
            "new-mutation-after-legacy",
        )
        .unwrap();
    assert_eq!(advanced.generation, created.generation + 1);
    assert!(receipt_path.is_file());

    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    assert_eq!(receipt["body"]["generation"], created.generation);
    assert_eq!(receipt["body"]["operation_id"], "create-a");

    let retried = store.create(request.clone(), "create-a").unwrap();
    assert_eq!(retried.generation, advanced.generation);
    let loaded = store.load("recovery-a").unwrap().unwrap();
    assert_eq!(loaded.generation, advanced.generation);
    let mut changed = request;
    changed.initial_prompt = "different payload".to_string();
    assert!(matches!(
        store.create(changed, "create-a"),
        Err(RecoveryStoreError::OperationConflict { .. })
    ));
}

#[test]
fn event_and_receipt_publication_crashes_preserve_exact_operation_retries() {
    for fault in [
        RecoveryStoreFaultPoint::AfterEventPublication,
        RecoveryStoreFaultPoint::AfterOperationReceiptPublication,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("recovery");
        let request = create_request("recovery-a", &temp.path().join("intake"));
        let faulted = RecoveryStore::new(&root).with_fault_injection_for_test(fault);
        assert!(matches!(
            faulted.create(request.clone(), "create-a"),
            Err(RecoveryStoreError::InjectedFault(actual)) if actual == fault
        ));

        let receipt_path = operation_receipt_path_for(&root, "recovery-a", "create-a");
        assert_eq!(
            receipt_path.is_file(),
            fault == RecoveryStoreFaultPoint::AfterOperationReceiptPublication
        );
        let restarted = RecoveryStore::new(&root);
        assert!(restarted
            .has_committed_operation("recovery-a", "create-a")
            .unwrap());
        let retried = restarted.create(request.clone(), "create-a").unwrap();
        assert_eq!(retried.generation, 1);
        assert!(receipt_path.is_file());

        let mut changed = request;
        changed.initial_prompt = "different payload".to_string();
        assert!(matches!(
            restarted.create(changed, "create-a"),
            Err(RecoveryStoreError::OperationConflict { .. })
        ));
    }
}

#[test]
fn corrupt_or_future_generation_operation_receipts_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let store = RecoveryStore::new(&root);
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    let receipt_path = operation_receipt_path_for(&root, "recovery-a", "create-a");
    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    stored["body"]["generation"] = serde_json::json!(2);
    let checksum = hex::encode(Sha256::digest(serde_json::to_vec(&stored["body"]).unwrap()));
    stored["checksum"] = serde_json::Value::String(checksum);
    std::fs::write(&receipt_path, serde_json::to_vec_pretty(&stored).unwrap()).unwrap();

    assert!(matches!(
        store.has_committed_operation("recovery-a", "create-a"),
        Err(RecoveryStoreError::Io(error)) if error.kind() == std::io::ErrorKind::InvalidData
    ));

    stored["body"]["generation"] = serde_json::json!(1);
    stored["checksum"] = serde_json::Value::String("0".repeat(64));
    std::fs::write(&receipt_path, serde_json::to_vec_pretty(&stored).unwrap()).unwrap();
    assert!(matches!(
        store.has_committed_operation("recovery-a", "create-a"),
        Err(RecoveryStoreError::Io(error)) if error.kind() == std::io::ErrorKind::InvalidData
    ));
}

#[test]
fn legacy_board_history_is_normalized_to_the_recent_window_on_load() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let store = RecoveryStore::new(&root);
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    rewrite_latest_snapshot(&root, "recovery-a", |body| {
        body.record.board_entry_ids = (0..1_100)
            .map(|index| format!("legacy-board-{index}"))
            .collect();
    });

    let loaded = store.load("recovery-a").unwrap().unwrap();
    assert_eq!(loaded.board_entry_ids.len(), 1_024);
    assert_eq!(loaded.board_entry_ids.first().unwrap(), "legacy-board-76");
    assert_eq!(loaded.board_entry_ids.last().unwrap(), "legacy-board-1099");

    let persisted = store
        .set_lifecycle(
            "recovery-a",
            RecoveryLifecycle::Running,
            None,
            "normalize-legacy-board",
        )
        .unwrap();
    assert_eq!(persisted.board_entry_ids.len(), 1_024);
}

#[test]
fn board_history_rollover_keeps_the_latest_1024_entries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let store = RecoveryStore::new(&root);
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    let old_ack = store
        .ack_board_entry("recovery-a", "board-old", "board-old-ack")
        .unwrap();
    rewrite_latest_snapshot(&root, "recovery-a", |body| {
        body.record.board_entry_ids = (0..1_024).map(|index| format!("board-{index}")).collect();
    });

    let exact_retry = store
        .ack_board_entry("recovery-a", "board-old", "board-old-ack")
        .unwrap();
    assert_eq!(exact_retry.generation, old_ack.generation);
    assert!(!exact_retry
        .board_entry_ids
        .contains(&"board-old".to_string()));

    let rolled = store
        .ack_board_entry("recovery-a", "board-new", "board-new-ack")
        .unwrap();
    assert_eq!(rolled.board_entry_ids.len(), 1_024);
    assert_eq!(rolled.board_entry_ids.first().unwrap(), "board-1");
    assert_eq!(rolled.board_entry_ids.last().unwrap(), "board-new");
}

#[test]
fn recovery_store_rejects_oversized_initial_prompt_before_persisting() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let mut request = create_request("recovery-a", &temp.path().join("intake"));
    request.initial_prompt = "x".repeat(262_145);

    assert!(matches!(
        store.create(request, "create-a"),
        Err(RecoveryStoreError::ContentLimitExceeded { field, .. })
            if field == "initial_prompt"
    ));
    assert!(store.load("recovery-a").unwrap().is_none());
}

#[test]
fn recovery_store_rejects_oversized_checkpoint_and_root_turn_collections() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    store
        .bind_root_semantic(
            "recovery-a",
            "root-1",
            None,
            BindingQuality::Verified,
            "bind-a",
        )
        .unwrap();

    let oversized_checkpoint = SemanticCheckpoint {
        summary: "x".repeat(65_537),
        ..SemanticCheckpoint::default()
    };
    assert!(matches!(
        store.replace_checkpoint(
            "recovery-a",
            "root-1",
            0,
            oversized_checkpoint,
            "checkpoint-a",
        ),
        Err(RecoveryStoreError::ContentLimitExceeded { field, .. })
            if field == "checkpoint.summary"
    ));

    let total_oversized_checkpoint = SemanticCheckpoint {
        visible_items: (0..9)
            .map(|index| VisibleDiscussionItem {
                role: "assistant".to_string(),
                kind: "message".to_string(),
                text: format!("{index}{}", "x".repeat(60_000)),
                partial: false,
            })
            .collect(),
        ..SemanticCheckpoint::default()
    };
    assert!(matches!(
        store.replace_checkpoint(
            "recovery-a",
            "root-1",
            0,
            total_oversized_checkpoint,
            "checkpoint-total-a",
        ),
        Err(RecoveryStoreError::ContentLimitExceeded { field, .. })
            if field == "checkpoint"
    ));

    let visible_items = (0..129)
        .map(|index| VisibleDiscussionItem {
            role: "assistant".to_string(),
            kind: "message".to_string(),
            text: format!("item-{index}"),
            partial: false,
        })
        .collect();
    assert!(matches!(
        store.record_root_turn(
            "recovery-a",
            RootTurnUpdate {
                root_id: "root-1".to_string(),
                turn_id: "turn-1".to_string(),
                input_text: None,
                visible_items,
                attachment_refs: Vec::new(),
            },
            "turn-a",
        ),
        Err(RecoveryStoreError::ContentLimitExceeded { field, .. })
            if field == "root_turn.visible_items"
    ));

    let loaded = store.load("recovery-a").unwrap().unwrap();
    assert_eq!(loaded.generation, 2);
    assert_eq!(loaded.checkpoint_revision, 0);
}

#[test]
fn root_turn_with_attachment_is_atomic_across_event_publication_crash() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let healthy = RecoveryStore::new(&root);
    healthy
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    healthy
        .bind_root_semantic(
            "recovery-a",
            "root-1",
            Some("tree-1".to_string()),
            BindingQuality::Verified,
            "bind-root",
        )
        .unwrap();
    let update = RootTurnUpdate {
        root_id: "root-1".to_string(),
        turn_id: "turn-1".to_string(),
        input_text: Some("Answer the recovery question".to_string()),
        visible_items: vec![VisibleDiscussionItem {
            role: "user".to_string(),
            kind: "structured_answer".to_string(),
            text: "Answer the recovery question".to_string(),
            partial: false,
        }],
        attachment_refs: Vec::new(),
    };
    let attachment = RecoveryAttachmentPayload {
        file_name: "evidence.png".to_string(),
        bytes: b"atomic attachment".to_vec(),
    };
    let faulted = RecoveryStore::new(&root)
        .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterEventPublication);

    let error = faulted
        .record_root_turn_with_attachments(
            "recovery-a",
            update.clone(),
            vec![attachment.clone()],
            "semantic-turn-1",
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RecoveryStoreError::InjectedFault(RecoveryStoreFaultPoint::AfterEventPublication)
    ));

    let recovered = healthy.load("recovery-a").unwrap().unwrap();
    assert_eq!(
        recovered
            .latest_root_input
            .as_ref()
            .map(|input| input.text.as_str()),
        Some("Answer the recovery question")
    );
    let checkpoint = recovered.checkpoint.as_ref().expect("atomic checkpoint");
    assert_eq!(checkpoint.visible_items.len(), 1);
    assert_eq!(checkpoint.attachment_refs.len(), 1);
    healthy
        .verify_attachment(&checkpoint.attachment_refs[0])
        .unwrap();
    let generation = recovered.generation;

    let retry = healthy
        .record_root_turn_with_attachments(
            "recovery-a",
            update.clone(),
            vec![attachment],
            "semantic-turn-1",
        )
        .unwrap();
    assert_eq!(retry.generation, generation);
    assert_eq!(retry.checkpoint_revision, 1);

    let mut changed = update;
    changed.input_text = Some("changed payload".to_string());
    assert!(matches!(
        healthy.record_root_turn("recovery-a", changed, "semantic-turn-1"),
        Err(RecoveryStoreError::OperationConflict { .. })
    ));
}

#[test]
fn checkpoint_with_path_attachment_is_atomic_across_event_publication_crash() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let healthy = RecoveryStore::new(&root);
    healthy
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    healthy
        .bind_root_semantic(
            "recovery-a",
            "root-1",
            Some("tree-1".to_string()),
            BindingQuality::Verified,
            "bind-root",
        )
        .unwrap();
    let source = temp.path().join("checkpoint-evidence.png");
    std::fs::write(&source, b"checkpoint attachment").unwrap();
    let checkpoint = SemanticCheckpoint {
        summary: "Persist the checkpoint and its evidence together".to_string(),
        visible_items: vec![VisibleDiscussionItem {
            role: "assistant".to_string(),
            kind: "discussion_checkpoint".to_string(),
            text: "The checkpoint is ready".to_string(),
            partial: false,
        }],
        ..SemanticCheckpoint::default()
    };
    let attachment_paths = vec![source];
    let faulted = RecoveryStore::new(&root)
        .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterEventPublication);

    let error = faulted
        .replace_checkpoint_with_attachments(
            "recovery-a",
            "root-1",
            0,
            checkpoint.clone(),
            &attachment_paths,
            "checkpoint-with-evidence",
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RecoveryStoreError::InjectedFault(RecoveryStoreFaultPoint::AfterEventPublication)
    ));

    let recovered = healthy.load("recovery-a").unwrap().unwrap();
    assert_eq!(recovered.checkpoint_revision, 1);
    let durable = recovered.checkpoint.as_ref().expect("checkpoint");
    assert_eq!(durable.attachment_refs.len(), 1);
    healthy
        .verify_attachment(&durable.attachment_refs[0])
        .unwrap();
    let generation = recovered.generation;

    let retry = healthy
        .replace_checkpoint_with_attachments(
            "recovery-a",
            "root-1",
            0,
            checkpoint,
            &attachment_paths,
            "checkpoint-with-evidence",
        )
        .unwrap();
    assert_eq!(retry.generation, generation);
    assert_eq!(retry.checkpoint_revision, 1);
    assert_eq!(retry.checkpoint.unwrap().attachment_refs.len(), 1);
}

#[test]
fn recovery_lease_claim_is_generation_cas_and_expired_claims_can_be_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let created = store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 1, 3, 0).unwrap();
    let first_lease = RecoveryLease {
        lease_id: "lease-first".to_string(),
        holder_id: "window-first".to_string(),
        acquired_at,
        expires_at: acquired_at + Duration::minutes(2),
    };
    let claimed = store
        .claim_recovery(
            "recovery-a",
            created.generation,
            first_lease.clone(),
            "Recovery Center exact resume",
            "claim-first",
        )
        .unwrap();
    assert_eq!(claimed.recovery_lease, Some(first_lease.clone()));
    assert_eq!(claimed.lifecycle, RecoveryLifecycle::Recovering);

    let competing = RecoveryLease {
        lease_id: "lease-competing".to_string(),
        holder_id: "window-competing".to_string(),
        acquired_at: acquired_at + Duration::seconds(10),
        expires_at: acquired_at + Duration::minutes(3),
    };
    assert!(matches!(
        store.claim_recovery(
            "recovery-a",
            created.generation,
            competing.clone(),
            "competing stale claim",
            "claim-stale",
        ),
        Err(RecoveryStoreError::GenerationMismatch {
            expected: 1,
            actual: 2
        })
    ));
    assert!(matches!(
        store.claim_recovery(
            "recovery-a",
            claimed.generation,
            competing,
            "competing active claim",
            "claim-active",
        ),
        Err(RecoveryStoreError::LeaseConflict { .. })
    ));

    let takeover_at = first_lease.expires_at;
    let takeover = RecoveryLease {
        lease_id: "lease-takeover".to_string(),
        holder_id: "window-takeover".to_string(),
        acquired_at: takeover_at,
        expires_at: takeover_at + Duration::minutes(2),
    };
    let replaced = store
        .claim_recovery(
            "recovery-a",
            claimed.generation,
            takeover.clone(),
            "expired lease takeover",
            "claim-takeover",
        )
        .unwrap();
    assert_eq!(replaced.recovery_lease, Some(takeover));

    let attention = store
        .set_lifecycle(
            "recovery-a",
            RecoveryLifecycle::Attention,
            Some("launch failed".to_string()),
            "attention-a",
        )
        .unwrap();
    assert!(attention.recovery_lease.is_none());
}

#[test]
fn provider_root_claim_blocks_a_second_recovery_for_the_same_authoritative_root() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    create_exact_recovery(
        &store,
        "recovery-first",
        "session-first",
        &temp.path().join("first"),
        "shared-provider-root",
    );
    create_exact_recovery(
        &store,
        "recovery-second",
        "session-second",
        &temp.path().join("second"),
        "shared-provider-root",
    );
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 0, 0).unwrap();
    let first = store.load("recovery-first").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-first",
            first.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-first", acquired_at),
            "window-first",
            "manual exact recovery",
            "claim-first-operation",
        )
        .unwrap();

    let second = store.load("recovery-second").unwrap().unwrap();
    let error = store
        .claim_recovery_with_provider_root(
            "recovery-second",
            second.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-second", acquired_at + Duration::seconds(1)),
            "window-second",
            "manual exact recovery",
            "claim-second-operation",
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RecoveryStoreError::ProviderRootClaimConflict {
            holder_recovery_id,
            holder_session_id,
            holder_window_id,
        } if holder_recovery_id == "recovery-first"
            && holder_session_id == "session-first"
            && holder_window_id == "window-first"
    ));
}

#[test]
fn provider_root_claim_concurrent_race_has_exactly_one_winner() {
    use std::sync::{Arc, Barrier};

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let store = RecoveryStore::new(&root);
    for suffix in ["a", "b"] {
        create_exact_recovery(
            &store,
            &format!("recovery-{suffix}"),
            &format!("session-{suffix}"),
            &temp.path().join(suffix),
            "shared-provider-root",
        );
    }
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for suffix in ["a", "b"] {
        let store = RecoveryStore::new(&root);
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            let recovery_id = format!("recovery-{suffix}");
            let record = store.load(&recovery_id).unwrap().unwrap();
            barrier.wait();
            store.claim_recovery_with_provider_root(
                &recovery_id,
                record.generation,
                "shared-provider-root",
                false,
                recovery_lease(
                    &format!("claim-{suffix}"),
                    Utc.with_ymd_and_hms(2026, 7, 16, 3, 10, 0).unwrap(),
                ),
                &format!("window-{suffix}"),
                "concurrent exact recovery",
                format!("claim-operation-{suffix}"),
            )
        }));
    }
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(
                result,
                Err(RecoveryStoreError::ProviderRootClaimConflict { .. })
            ))
            .count(),
        1
    );
}

#[test]
fn expired_provider_root_claim_is_replaceable_and_stale_release_is_cas_safe() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    for suffix in ["first", "second"] {
        create_exact_recovery(
            &store,
            &format!("recovery-{suffix}"),
            &format!("session-{suffix}"),
            &temp.path().join(suffix),
            "shared-provider-root",
        );
    }
    let first_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 20, 0).unwrap();
    let first = store.load("recovery-first").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-first",
            first.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-first", first_at),
            "window-first",
            "first exact recovery",
            "claim-first-operation",
        )
        .unwrap();

    let takeover_at = first_at + Duration::minutes(2);
    let second = store.load("recovery-second").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-second",
            second.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-second", takeover_at),
            "window-second",
            "take over expired exact recovery",
            "claim-second-operation",
        )
        .unwrap();

    assert!(!store
        .release_provider_root_claim_for_recovery(
            "recovery-first",
            "claim-first",
            takeover_at + Duration::seconds(1),
        )
        .unwrap());
    let active = store
        .active_provider_root_claim(
            "codex",
            "shared-provider-root",
            takeover_at + Duration::seconds(1),
        )
        .unwrap()
        .unwrap();
    assert_eq!(active.claim_token, "claim-second");
    assert_eq!(active.holder_recovery_id, "recovery-second");
}

#[test]
fn provider_root_claim_renewal_updates_window_and_enforces_ttl_bound() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    create_exact_recovery(
        &store,
        "recovery-a",
        "session-a",
        &temp.path().join("a"),
        "provider-root-a",
    );
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 30, 0).unwrap();
    let record = store.load("recovery-a").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-a",
            record.generation,
            "provider-root-a",
            false,
            recovery_lease("claim-a", acquired_at),
            "pending-window-a",
            "exact recovery",
            "claim-a-operation",
        )
        .unwrap();
    let renewed_at = acquired_at + Duration::seconds(30);
    let renewed = store
        .renew_provider_root_claim_for_recovery(
            "recovery-a",
            "claim-a",
            "window-a",
            renewed_at,
            renewed_at + Duration::minutes(3),
        )
        .unwrap();
    assert_eq!(renewed.holder_window_id, "window-a");
    assert_eq!(renewed.expires_at, renewed_at + Duration::minutes(3));

    assert!(matches!(
        store.renew_provider_root_claim_for_recovery(
            "recovery-a",
            "claim-a",
            "window-a",
            renewed_at,
            renewed_at + Duration::minutes(11),
        ),
        Err(RecoveryStoreError::InvalidLease(_))
    ));
}

#[test]
fn provider_root_claim_renewal_keeps_a_slow_pre_ready_launch_exclusive() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    for suffix in ["slow", "competitor"] {
        create_exact_recovery(
            &store,
            &format!("recovery-{suffix}"),
            &format!("session-{suffix}"),
            &temp.path().join(suffix),
            "shared-provider-root",
        );
    }
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 35, 0).unwrap();
    let slow = store.load("recovery-slow").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-slow",
            slow.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-slow", acquired_at),
            "pending-slow-window",
            "slow exact recovery",
            "claim-slow-operation",
        )
        .unwrap();
    store
        .advance_launch_stage(
            "recovery-slow",
            RecoveryLaunchStage::ProcessSpawned,
            Some(ProviderRootRole::Root),
            "slow-process-spawned",
        )
        .unwrap();

    let renewed_at = acquired_at + Duration::seconds(90);
    store
        .renew_provider_root_claim_for_recovery(
            "recovery-slow",
            "claim-slow",
            "slow-window",
            renewed_at,
            renewed_at + Duration::minutes(2),
        )
        .unwrap();

    let competitor = store.load("recovery-competitor").unwrap().unwrap();
    let after_original_ttl = acquired_at + Duration::seconds(121);
    assert!(matches!(
        store.claim_recovery_with_provider_root(
            "recovery-competitor",
            competitor.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-competitor", after_original_ttl),
            "competitor-window",
            "competing exact recovery",
            "claim-competitor-operation",
        ),
        Err(RecoveryStoreError::ProviderRootClaimConflict { .. })
    ));
}

#[test]
fn startup_interruption_clears_only_an_expired_unrenewed_recovery_lease() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    create_exact_recovery(
        &store,
        "recovery-crashed",
        "session-crashed",
        &temp.path().join("crashed"),
        "provider-root-crashed",
    );
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 35, 0).unwrap();
    let record = store.load("recovery-crashed").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-crashed",
            record.generation,
            "provider-root-crashed",
            false,
            recovery_lease("claim-crashed", acquired_at),
            "crashed-window",
            "slow exact recovery",
            "claim-crashed-operation",
        )
        .unwrap();
    let renewed_at = acquired_at + Duration::seconds(90);
    store
        .renew_provider_root_claim_for_recovery(
            "recovery-crashed",
            "claim-crashed",
            "crashed-window",
            renewed_at,
            renewed_at + Duration::minutes(2),
        )
        .unwrap();

    let original_record_expired_at = acquired_at + Duration::seconds(121);
    let still_owned = store
        .interrupt_expired_recovery_lease(
            "recovery-crashed",
            original_record_expired_at,
            "startup observed interrupted Session",
            "interrupt-expired-crashed",
        )
        .unwrap();
    assert_eq!(still_owned.lifecycle, RecoveryLifecycle::Recovering);
    assert!(still_owned.recovery_lease.is_some());

    let renewed_claim_expired_at = renewed_at + Duration::minutes(2);
    let interrupted = store
        .interrupt_expired_recovery_lease(
            "recovery-crashed",
            renewed_claim_expired_at,
            "startup observed interrupted Session",
            "interrupt-expired-crashed",
        )
        .unwrap();
    assert_eq!(interrupted.lifecycle, RecoveryLifecycle::Interrupted);
    assert!(interrupted.recovery_lease.is_none());
    assert!(store
        .active_provider_root_claim("codex", "provider-root-crashed", renewed_claim_expired_at,)
        .unwrap()
        .is_none());
}

#[test]
fn claimed_provider_ready_promotes_the_target_before_releasing_the_source_claim() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    for suffix in ["source", "target", "competitor"] {
        create_exact_recovery(
            &store,
            &format!("recovery-{suffix}"),
            &format!("session-{suffix}"),
            &temp.path().join(suffix),
            "shared-provider-root",
        );
    }
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 36, 0).unwrap();
    let source = store.load("recovery-source").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-source",
            source.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-source", acquired_at),
            "target-window",
            "exact recovery",
            "claim-source-operation",
        )
        .unwrap();

    let ready = store
        .complete_claimed_provider_ready(
            "recovery-source",
            "recovery-target",
            "claim-source",
            acquired_at + Duration::seconds(30),
            "complete-target-ready",
        )
        .unwrap();
    assert_eq!(ready.lifecycle, RecoveryLifecycle::Running);
    assert_eq!(ready.launch_stage, RecoveryLaunchStage::Ready);
    assert!(store
        .active_provider_root_claim(
            "codex",
            "shared-provider-root",
            acquired_at + Duration::seconds(31),
        )
        .unwrap()
        .is_none());

    let competitor = store.load("recovery-competitor").unwrap().unwrap();
    assert!(matches!(
        store.claim_recovery_with_provider_root(
            "recovery-competitor",
            competitor.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-competitor", acquired_at + Duration::minutes(3),),
            "competitor-window",
            "competing exact recovery",
            "claim-competitor-after-ready",
        ),
        Err(RecoveryStoreError::ProviderRootClaimConflict { .. })
    ));
}

#[test]
fn normal_provider_ready_atomically_publishes_ready_and_running_after_every_crash_boundary() {
    for fault in [
        RecoveryStoreFaultPoint::AfterEventPublication,
        RecoveryStoreFaultPoint::AfterOperationReceiptPublication,
        RecoveryStoreFaultPoint::AfterSnapshotPublication,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("recovery");
        let healthy = RecoveryStore::new(&root);
        healthy
            .create(
                create_request("recovery-normal-ready", &temp.path().join("intake")),
                "create-normal-ready",
            )
            .unwrap();
        healthy
            .bind_root_semantic(
                "recovery-normal-ready",
                "normal-ready-root",
                None,
                BindingQuality::Verified,
                "bind-normal-ready",
            )
            .unwrap();
        let ready_at = Utc.with_ymd_and_hms(2026, 7, 16, 5, 0, 0).unwrap();
        let faulted = RecoveryStore::new(&root).with_fault_injection_for_test(fault);
        assert!(matches!(
            faulted.complete_provider_ready(
                "recovery-normal-ready",
                ready_at,
                "complete-normal-ready",
            ),
            Err(RecoveryStoreError::InjectedFault(actual)) if actual == fault
        ));

        let restarted = RecoveryStore::new(&root);
        let recovered = restarted.load("recovery-normal-ready").unwrap().unwrap();
        assert_eq!(recovered.launch_stage, RecoveryLaunchStage::Ready);
        assert_eq!(recovered.lifecycle, RecoveryLifecycle::Running);
        assert_eq!(recovered.root_role, ProviderRootRole::Root);
        assert!(recovered.recovery_lease.is_none());
        let retry = restarted
            .complete_provider_ready(
                "recovery-normal-ready",
                ready_at + Duration::seconds(1),
                "complete-normal-ready",
            )
            .unwrap();
        assert_eq!(retry.generation, recovered.generation);
        assert_eq!(retry.launch_stage, RecoveryLaunchStage::Ready);
        assert_eq!(retry.lifecycle, RecoveryLifecycle::Running);
    }
}

#[test]
fn ready_recovery_requires_durable_supervisor_stop_proof_and_dedicated_claim() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let stopped_at = Utc.with_ymd_and_hms(2026, 7, 16, 5, 30, 0).unwrap();

    for (suffix, exact) in [("exact", true), ("semantic", false)] {
        let recovery_id = format!("recovery-ready-{suffix}");
        let session_id = format!("session-ready-{suffix}");
        let provider_root_id = format!("provider-ready-{suffix}");
        create_exact_recovery(
            &store,
            &recovery_id,
            &session_id,
            &temp.path().join(suffix),
            &provider_root_id,
        );
        store
            .complete_provider_ready(
                &recovery_id,
                stopped_at - Duration::seconds(10),
                format!("ready-{suffix}"),
            )
            .unwrap();
        let ready = store.load(&recovery_id).unwrap().unwrap();

        assert!(matches!(
            store.claim_recovery(
                &recovery_id,
                ready.generation,
                recovery_lease(&format!("ordinary-{suffix}"), stopped_at),
                "ordinary claim must stay closed",
                format!("ordinary-claim-{suffix}"),
            ),
            Err(RecoveryStoreError::InvalidLease(_))
        ));

        let interrupted = store
            .interrupt_after_supervisor_stop(
                &recovery_id,
                ready.generation,
                &session_id,
                stopped_at,
                "Cold startup owns no live PTY for this Intake Session",
                format!("supervisor-stop-{suffix}"),
            )
            .unwrap();
        assert_eq!(interrupted.lifecycle, RecoveryLifecycle::Interrupted);
        assert_eq!(interrupted.launch_stage, RecoveryLaunchStage::Ready);
        assert_eq!(
            interrupted
                .supervisor_stop_proof
                .as_ref()
                .map(|proof| proof.session_id.as_str()),
            Some(session_id.as_str())
        );

        let lease = recovery_lease(&format!("interrupted-{suffix}"), stopped_at);
        let claimed = if exact {
            store
                .claim_interrupted_recovery_with_provider_root(
                    &recovery_id,
                    interrupted.generation,
                    &provider_root_id,
                    false,
                    lease,
                    &format!("ready-window-{suffix}"),
                    "Launch a successor after durable supervisor stop",
                    format!("interrupted-claim-{suffix}"),
                )
                .unwrap()
        } else {
            store
                .claim_interrupted_recovery(
                    &recovery_id,
                    interrupted.generation,
                    lease,
                    "Launch a semantic successor after durable supervisor stop",
                    format!("interrupted-claim-{suffix}"),
                )
                .unwrap()
        };
        assert_eq!(claimed.lifecycle, RecoveryLifecycle::Recovering);
        assert_eq!(claimed.launch_stage, RecoveryLaunchStage::Ready);
        assert!(claimed.recovery_lease.is_some());
    }
}

#[test]
fn ready_execution_recovery_accepts_the_same_durable_supervisor_stop_proof() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let recovery_id = "recovery-ready-execution";
    let session_id = "session-ready-execution";
    let provider_root_id = "provider-ready-execution";
    let stopped_at = Utc.with_ymd_and_hms(2026, 7, 17, 3, 0, 0).unwrap();
    let mut request = create_request(recovery_id, &temp.path().join("execution"));
    request.session_id = session_id.to_string();
    request.session_kind = RecoverySessionKind::Execution;
    store.create(request, "create-ready-execution").unwrap();
    store
        .bind_root(
            recovery_id,
            ProviderRootBinding {
                root_id: provider_root_id.to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: stopped_at - Duration::seconds(20),
            },
            "bind-ready-execution",
        )
        .unwrap();
    store
        .complete_provider_ready(
            recovery_id,
            stopped_at - Duration::seconds(10),
            "ready-execution",
        )
        .unwrap();
    let ready = store.load(recovery_id).unwrap().unwrap();

    let interrupted = store
        .interrupt_after_supervisor_stop(
            recovery_id,
            ready.generation,
            session_id,
            stopped_at,
            "gwt observed the Execution provider stop",
            "supervisor-stop-ready-execution",
        )
        .unwrap();

    assert_eq!(interrupted.lifecycle, RecoveryLifecycle::Interrupted);
    assert_eq!(interrupted.launch_stage, RecoveryLaunchStage::Ready);
    assert_eq!(
        interrupted
            .supervisor_stop_proof
            .as_ref()
            .map(|proof| proof.session_id.as_str()),
        Some(session_id)
    );
    let claimed = store
        .claim_interrupted_recovery_with_provider_root(
            recovery_id,
            interrupted.generation,
            provider_root_id,
            false,
            recovery_lease("execution-successor", stopped_at),
            "execution-successor-window",
            "resume interrupted Execution",
            "claim-ready-execution",
        )
        .unwrap();
    assert_eq!(claimed.lifecycle, RecoveryLifecycle::Recovering);
}

#[test]
fn attention_supervisor_stop_requires_the_same_observed_retry_reason() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let recovery_id = "recovery-attention-stop";
    let session_id = "session-attention-stop";
    create_exact_recovery(
        &store,
        recovery_id,
        session_id,
        &temp.path().join("attention-stop"),
        "provider-attention-stop",
    );
    let attention_reason = "legacy_import_attention:missing_intake_worktree";
    let attention = store
        .set_lifecycle(
            recovery_id,
            RecoveryLifecycle::Attention,
            Some(attention_reason.to_string()),
            "attention-stop-state",
        )
        .unwrap();
    let stopped_at = Utc.with_ymd_and_hms(2026, 7, 16, 5, 45, 0).unwrap();

    assert!(matches!(
        store.interrupt_after_supervisor_stop(
            recovery_id,
            attention.generation,
            session_id,
            stopped_at,
            "ordinary live-state proof must not erase Attention",
            "ordinary-attention-stop",
        ),
        Err(RecoveryStoreError::InvalidLease(_))
    ));
    assert!(matches!(
        store.interrupt_attention_after_supervisor_stop(
            recovery_id,
            attention.generation,
            session_id,
            "legacy_import_attention:multiple_provider_roots",
            stopped_at,
            "mismatched evidence must stay closed",
            "mismatched-attention-stop",
        ),
        Err(RecoveryStoreError::InvalidLease(_))
    ));
    let unchanged = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(unchanged.generation, attention.generation);
    assert_eq!(unchanged.lifecycle, RecoveryLifecycle::Attention);
    assert!(unchanged.supervisor_stop_proof.is_none());

    let interrupted = store
        .interrupt_attention_after_supervisor_stop(
            recovery_id,
            attention.generation,
            session_id,
            attention_reason,
            stopped_at,
            "Cold startup classified the missing Intake as retryable",
            "matching-attention-stop",
        )
        .unwrap();
    assert_eq!(interrupted.lifecycle, RecoveryLifecycle::Interrupted);
    assert_eq!(interrupted.launch_stage, RecoveryLaunchStage::ProviderBound);
    assert_eq!(
        interrupted
            .supervisor_stop_proof
            .as_ref()
            .map(|proof| proof.session_id.as_str()),
        Some(session_id)
    );
}

#[test]
fn stale_ready_token_cannot_promote_a_target_after_claim_takeover() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    for suffix in ["source", "target", "takeover"] {
        create_exact_recovery(
            &store,
            &format!("recovery-{suffix}"),
            &format!("session-{suffix}"),
            &temp.path().join(suffix),
            "shared-provider-root",
        );
    }
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 37, 0).unwrap();
    let source = store.load("recovery-source").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-source",
            source.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-source", acquired_at),
            "source-window",
            "exact recovery",
            "claim-source-operation",
        )
        .unwrap();
    let takeover_at = acquired_at + Duration::minutes(2);
    let takeover = store.load("recovery-takeover").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-takeover",
            takeover.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-takeover", takeover_at),
            "takeover-window",
            "expired claim takeover",
            "claim-takeover-operation",
        )
        .unwrap();

    assert!(matches!(
        store.complete_claimed_provider_ready(
            "recovery-source",
            "recovery-target",
            "claim-source",
            takeover_at + Duration::seconds(1),
            "stale-target-ready",
        ),
        Err(RecoveryStoreError::ProviderRootClaimConflict { .. })
            | Err(RecoveryStoreError::InvalidLease(_))
    ));
    let target = store.load("recovery-target").unwrap().unwrap();
    assert!(target.launch_stage < RecoveryLaunchStage::Ready);
    assert_ne!(target.lifecycle, RecoveryLifecycle::Running);
    assert_eq!(
        store
            .active_provider_root_claim(
                "codex",
                "shared-provider-root",
                takeover_at + Duration::seconds(1),
            )
            .unwrap()
            .unwrap()
            .claim_token,
        "claim-takeover"
    );
}

#[test]
fn a_crashed_process_spawned_attempt_is_replaceable_after_claim_expiry() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    for suffix in ["crashed", "takeover"] {
        create_exact_recovery(
            &store,
            &format!("recovery-{suffix}"),
            &format!("session-{suffix}"),
            &temp.path().join(suffix),
            "shared-provider-root",
        );
    }
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 38, 0).unwrap();
    let crashed = store.load("recovery-crashed").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-crashed",
            crashed.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-crashed", acquired_at),
            "crashed-window",
            "exact recovery",
            "claim-crashed-operation",
        )
        .unwrap();
    store
        .advance_launch_stage(
            "recovery-crashed",
            RecoveryLaunchStage::ProcessSpawned,
            Some(ProviderRootRole::Root),
            "crashed-process-spawned",
        )
        .unwrap();

    let takeover_at = acquired_at + Duration::minutes(2);
    let takeover = store.load("recovery-takeover").unwrap().unwrap();
    let claimed = store
        .claim_recovery_with_provider_root(
            "recovery-takeover",
            takeover.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-takeover", takeover_at),
            "takeover-window",
            "crash takeover after expiry",
            "claim-takeover-operation",
        )
        .expect("pre-Ready crash must become recoverable after its bounded claim expires");
    assert_eq!(claimed.lifecycle, RecoveryLifecycle::Recovering);
}

#[test]
fn provider_root_claim_publication_survives_crash_before_record_claim() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let healthy = RecoveryStore::new(&root);
    create_exact_recovery(
        &healthy,
        "recovery-a",
        "session-a",
        &temp.path().join("a"),
        "provider-root-a",
    );
    let record = healthy.load("recovery-a").unwrap().unwrap();
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 40, 0).unwrap();
    let faulted = RecoveryStore::new(&root)
        .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterProviderRootClaimPublication);
    assert!(matches!(
        faulted.claim_recovery_with_provider_root(
            "recovery-a",
            record.generation,
            "provider-root-a",
            false,
            recovery_lease("claim-a", acquired_at),
            "window-a",
            "exact recovery",
            "claim-a-operation",
        ),
        Err(RecoveryStoreError::InjectedFault(
            RecoveryStoreFaultPoint::AfterProviderRootClaimPublication
        ))
    ));

    let restarted = RecoveryStore::new(&root);
    let unchanged = restarted.load("recovery-a").unwrap().unwrap();
    assert_eq!(unchanged.generation, record.generation);
    let active = restarted
        .active_provider_root_claim(
            "codex",
            "provider-root-a",
            acquired_at + Duration::seconds(1),
        )
        .unwrap()
        .unwrap();
    assert_eq!(active.claim_token, "claim-a");
}

#[test]
fn record_event_crash_keeps_provider_claim_and_semantic_retry_converges() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let healthy = RecoveryStore::new(&root);
    for suffix in ["a", "b"] {
        create_exact_recovery(
            &healthy,
            &format!("recovery-{suffix}"),
            &format!("session-{suffix}"),
            &temp.path().join(suffix),
            "shared-provider-root",
        );
    }
    let before = healthy.load("recovery-a").unwrap().unwrap();
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 50, 0).unwrap();
    let lease = recovery_lease("claim-a", acquired_at);
    let faulted = RecoveryStore::new(&root)
        .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterEventPublication);
    assert!(matches!(
        faulted.claim_recovery_with_provider_root(
            "recovery-a",
            before.generation,
            "shared-provider-root",
            false,
            lease.clone(),
            "window-a",
            "exact recovery",
            "claim-a-operation",
        ),
        Err(RecoveryStoreError::InjectedFault(
            RecoveryStoreFaultPoint::AfterEventPublication
        ))
    ));

    let restarted = RecoveryStore::new(&root);
    let committed = restarted.load("recovery-a").unwrap().unwrap();
    assert_eq!(committed.lifecycle, RecoveryLifecycle::Recovering);
    assert_eq!(
        restarted
            .active_provider_root_claim(
                "codex",
                "shared-provider-root",
                acquired_at + Duration::seconds(1),
            )
            .unwrap()
            .unwrap()
            .claim_token,
        "claim-a"
    );
    let second = restarted.load("recovery-b").unwrap().unwrap();
    assert!(matches!(
        restarted.claim_recovery_with_provider_root(
            "recovery-b",
            second.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-b", acquired_at + Duration::seconds(1)),
            "window-b",
            "competing exact recovery",
            "claim-b-operation",
        ),
        Err(RecoveryStoreError::ProviderRootClaimConflict { .. })
    ));

    let retried = restarted
        .claim_recovery_with_provider_root(
            "recovery-a",
            before.generation,
            "shared-provider-root",
            false,
            lease,
            "window-a",
            "exact recovery",
            "claim-a-operation",
        )
        .expect("semantic retry must converge");
    assert_eq!(retried.generation, committed.generation);
}

#[test]
fn provider_root_claim_retry_converges_after_operation_receipt_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let healthy = RecoveryStore::new(&root);
    create_exact_recovery(
        &healthy,
        "recovery-a",
        "session-a",
        &temp.path().join("a"),
        "provider-root-a",
    );
    let before = healthy.load("recovery-a").unwrap().unwrap();
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 3, 55, 0).unwrap();
    let lease = recovery_lease("claim-a", acquired_at);
    let faulted = RecoveryStore::new(&root)
        .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterOperationReceiptPublication);
    assert!(matches!(
        faulted.claim_recovery_with_provider_root(
            "recovery-a",
            before.generation,
            "provider-root-a",
            false,
            lease.clone(),
            "window-a",
            "exact recovery",
            "claim-a-operation",
        ),
        Err(RecoveryStoreError::InjectedFault(
            RecoveryStoreFaultPoint::AfterOperationReceiptPublication
        ))
    ));

    let restarted = RecoveryStore::new(&root);
    let retried = restarted
        .claim_recovery_with_provider_root(
            "recovery-a",
            before.generation,
            "provider-root-a",
            false,
            lease,
            "window-a",
            "exact recovery",
            "claim-a-operation",
        )
        .unwrap();
    assert_eq!(retried.generation, before.generation + 1);
    assert_eq!(retried.lifecycle, RecoveryLifecycle::Recovering);
    assert_eq!(
        restarted
            .active_provider_root_claim(
                "codex",
                "provider-root-a",
                acquired_at + Duration::seconds(1),
            )
            .unwrap()
            .unwrap()
            .claim_token,
        "claim-a"
    );
}

#[test]
fn terminal_finalize_releases_only_its_current_provider_root_claim() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    create_exact_recovery(
        &store,
        "recovery-a",
        "session-a",
        &temp.path().join("a"),
        "provider-root-a",
    );
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 4, 0, 0).unwrap();
    let record = store.load("recovery-a").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-a",
            record.generation,
            "provider-root-a",
            false,
            recovery_lease("claim-a", acquired_at),
            "window-a",
            "exact recovery",
            "claim-a-operation",
        )
        .unwrap();
    store
        .finalize_and_purge(
            "recovery-a",
            RecoveryLifecycle::Discarded,
            acquired_at + Duration::seconds(1),
            "discard-a",
        )
        .unwrap();
    assert!(store
        .active_provider_root_claim(
            "codex",
            "provider-root-a",
            acquired_at + Duration::seconds(2),
        )
        .unwrap()
        .is_none());
}

#[test]
fn ready_owner_blocks_same_root_after_launch_claim_release_until_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    for suffix in ["a", "b"] {
        create_exact_recovery(
            &store,
            &format!("recovery-{suffix}"),
            &format!("session-{suffix}"),
            &temp.path().join(suffix),
            "shared-provider-root",
        );
    }
    let acquired_at = Utc.with_ymd_and_hms(2026, 7, 16, 4, 10, 0).unwrap();
    let first = store.load("recovery-a").unwrap().unwrap();
    store
        .claim_recovery_with_provider_root(
            "recovery-a",
            first.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-a", acquired_at),
            "window-a",
            "exact recovery",
            "claim-a-operation",
        )
        .unwrap();
    store
        .advance_launch_stage(
            "recovery-a",
            RecoveryLaunchStage::Ready,
            Some(ProviderRootRole::Root),
            "ready-a",
        )
        .unwrap();
    assert!(store
        .release_provider_root_claim_for_recovery(
            "recovery-a",
            "claim-a",
            acquired_at + Duration::seconds(1),
        )
        .unwrap());
    store
        .set_lifecycle("recovery-a", RecoveryLifecycle::Running, None, "running-a")
        .unwrap();

    let second = store.load("recovery-b").unwrap().unwrap();
    assert!(matches!(
        store.claim_recovery_with_provider_root(
            "recovery-b",
            second.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-b", acquired_at + Duration::minutes(3)),
            "window-b",
            "duplicate exact recovery",
            "claim-b-operation",
        ),
        Err(RecoveryStoreError::ProviderRootClaimConflict { .. })
    ));
    assert!(store
        .provider_root_owned_by_other_recovery(
            "codex",
            "shared-provider-root",
            "recovery-b",
            acquired_at + Duration::minutes(3),
        )
        .unwrap());

    store
        .finalize_and_purge(
            "recovery-a",
            RecoveryLifecycle::Resolved,
            acquired_at + Duration::minutes(3),
            "resolve-a",
        )
        .unwrap();
    let resumed = store
        .claim_recovery_with_provider_root(
            "recovery-b",
            second.generation,
            "shared-provider-root",
            false,
            recovery_lease("claim-b", acquired_at + Duration::minutes(3)),
            "window-b",
            "exact recovery after terminal owner",
            "claim-b-operation-after-terminal",
        )
        .expect("terminal owner must release durable ownership");
    assert_eq!(resumed.lifecycle, RecoveryLifecycle::Recovering);
}

#[test]
fn root_binding_only_upgrades_and_never_silently_changes_exact_id() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();

    store
        .bind_root(
            "recovery-a",
            ProviderRootBinding {
                root_id: "root-1".to_string(),
                session_tree_id: Some("tree-1".to_string()),
                quality: BindingQuality::Inferred,
                bound_at: Utc::now(),
            },
            "bind-inferred",
        )
        .unwrap();
    let upgraded = store
        .bind_root(
            "recovery-a",
            ProviderRootBinding {
                root_id: "root-1".to_string(),
                session_tree_id: Some("tree-1".to_string()),
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-verified",
        )
        .unwrap();
    assert_eq!(
        upgraded.provider_root.unwrap().quality,
        BindingQuality::Verified
    );

    assert!(matches!(
        store.bind_root(
            "recovery-a",
            ProviderRootBinding {
                root_id: "root-other".to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-conflict",
        ),
        Err(RecoveryStoreError::RootBindingConflict { .. })
    ));
}

#[test]
fn ambiguous_provider_candidates_are_persisted_and_confirmation_claims_one_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let created = store
        .create(
            create_request("recovery-ambiguous", &temp.path().join("intake")),
            "create-ambiguous",
        )
        .unwrap();
    let observed_at = Utc.with_ymd_and_hms(2026, 7, 16, 1, 4, 0).unwrap();
    let candidates = vec![
        ProviderRootCandidate {
            root_id: "provider-a".to_string(),
            evidence: vec!["current session id".to_string()],
            observed_at,
        },
        ProviderRootCandidate {
            root_id: "provider-b".to_string(),
            evidence: vec!["provider history".to_string()],
            observed_at,
        },
    ];
    let recorded = store
        .record_provider_root_candidates(
            "recovery-ambiguous",
            candidates.clone(),
            "record-provider-candidates",
        )
        .unwrap();
    assert_eq!(recorded.provider_root, None);
    assert_eq!(recorded.provider_root_candidates, candidates);

    let lease = RecoveryLease {
        lease_id: "lease-confirmed".to_string(),
        holder_id: "recovery-center".to_string(),
        acquired_at: observed_at + Duration::minutes(1),
        expires_at: observed_at + Duration::minutes(3),
    };
    assert!(matches!(
        store.claim_recovery_with_confirmed_root(
            "recovery-ambiguous",
            recorded.generation,
            "provider-unknown",
            lease.clone(),
            "confirm exact provider root",
            "claim-unknown",
        ),
        Err(RecoveryStoreError::UnknownProviderRootCandidate { .. })
    ));

    let claimed = store
        .claim_recovery_with_confirmed_root(
            "recovery-ambiguous",
            recorded.generation,
            "provider-b",
            lease.clone(),
            "confirm exact provider root",
            "claim-provider-b",
        )
        .unwrap();
    assert_eq!(
        claimed.provider_root,
        Some(ProviderRootBinding {
            root_id: "provider-b".to_string(),
            session_tree_id: None,
            quality: BindingQuality::Confirmed,
            bound_at: lease.acquired_at,
        })
    );
    assert_eq!(claimed.provider_root_candidates, candidates);
    assert_eq!(claimed.recovery_lease, Some(lease));
    assert_eq!(claimed.lifecycle, RecoveryLifecycle::Recovering);
    assert_eq!(store.load("recovery-ambiguous").unwrap(), Some(claimed));

    assert!(matches!(
        store.claim_recovery_with_confirmed_root(
            "recovery-ambiguous",
            created.generation,
            "provider-a",
            RecoveryLease {
                lease_id: "lease-stale".to_string(),
                holder_id: "other-window".to_string(),
                acquired_at: observed_at + Duration::minutes(4),
                expires_at: observed_at + Duration::minutes(6),
            },
            "stale confirmation",
            "claim-stale-provider-a",
        ),
        Err(RecoveryStoreError::GenerationMismatch { .. })
    ));
}

#[test]
fn checkpoint_is_root_scoped_complete_replacement_with_cas() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    store
        .bind_root(
            "recovery-a",
            ProviderRootBinding {
                root_id: "root-1".to_string(),
                session_tree_id: Some("tree-1".to_string()),
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-a",
        )
        .unwrap();

    let checkpoint = SemanticCheckpoint {
        summary: "The base OID is lost before materialization.".to_string(),
        confirmed_decisions: vec!["Persist base OID before spawn.".to_string()],
        open_questions: vec!["How should legacy ambiguity be shown?".to_string()],
        next_action: Some("Build Recovery Center".to_string()),
        as_of_turn_id: Some("turn-7".to_string()),
        visible_items: vec![VisibleDiscussionItem {
            role: "assistant".to_string(),
            kind: "message".to_string(),
            text: "Visible answer\u{1b}[31m\0".to_string(),
            partial: false,
        }],
        attachment_refs: Vec::new(),
        board_intents: Vec::new(),
    };

    let replaced = store
        .replace_checkpoint("recovery-a", "root-1", 0, checkpoint, "checkpoint-1")
        .unwrap();
    assert_eq!(replaced.checkpoint_revision, 1);
    assert_eq!(replaced.checkpoint_coverage, CheckpointCoverage::Explicit);
    let visible = &replaced.checkpoint.unwrap().visible_items[0].text;
    assert_eq!(visible, "Visible answer[31m");

    assert!(matches!(
        store.replace_checkpoint(
            "recovery-a",
            "child-root",
            1,
            SemanticCheckpoint::default(),
            "checkpoint-child",
        ),
        Err(RecoveryStoreError::RootMismatch { .. })
    ));
    assert!(matches!(
        store.replace_checkpoint(
            "recovery-a",
            "root-1",
            0,
            SemanticCheckpoint::default(),
            "checkpoint-stale",
        ),
        Err(RecoveryStoreError::RevisionMismatch {
            expected: 0,
            actual: 1
        })
    ));
}

#[test]
fn new_root_input_marks_explicit_checkpoint_stale() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    store
        .bind_root(
            "recovery-a",
            ProviderRootBinding {
                root_id: "root-1".to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-a",
        )
        .unwrap();
    store
        .replace_checkpoint(
            "recovery-a",
            "root-1",
            0,
            SemanticCheckpoint::default(),
            "checkpoint-a",
        )
        .unwrap();

    let record = store
        .record_root_input(
            "recovery-a",
            "root-1",
            "turn-2",
            "Continue the investigation",
            "input-a",
        )
        .unwrap();

    assert_eq!(record.checkpoint_coverage, CheckpointCoverage::Stale);
    assert_eq!(
        record.latest_root_input.unwrap().text,
        "Continue the investigation"
    );
}

#[test]
fn corrupt_or_partial_newest_files_do_not_hide_last_valid_generation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let store = RecoveryStore::new(&root);
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();

    let recovery_dir = root.join("recoveries").join("recovery-a");
    std::fs::write(
        recovery_dir
            .join("snapshots")
            .join("99999999999999999999-corrupt.json"),
        "{not-json",
    )
    .unwrap();
    std::fs::write(
        recovery_dir
            .join("events")
            .join("99999999999999999999-partial.tmp"),
        "partial",
    )
    .unwrap();

    let loaded = store.load("recovery-a").unwrap().unwrap();
    assert_eq!(loaded.generation, 1);
    assert_eq!(loaded.initial_prompt, "Investigate Intake recovery");
}

#[test]
fn injected_publication_failures_recover_a_complete_generation() {
    for fault in [
        RecoveryStoreFaultPoint::AfterEventPublication,
        RecoveryStoreFaultPoint::AfterOperationReceiptPublication,
        RecoveryStoreFaultPoint::AfterSnapshotPublication,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("recovery");
        let faulted = RecoveryStore::new(&root).with_fault_injection_for_test(fault);
        let error = faulted
            .create(
                create_request("recovery-a", &temp.path().join("intake")),
                "create-a",
            )
            .unwrap_err();
        assert!(matches!(error, RecoveryStoreError::InjectedFault(actual) if actual == fault));

        let healthy = RecoveryStore::new(&root);
        let recovered = healthy.load("recovery-a").unwrap().unwrap();
        assert_eq!(recovered.generation, 1);
        assert_eq!(recovered.initial_prompt, "Investigate Intake recovery");
        let retry = healthy
            .create(
                create_request("recovery-a", &temp.path().join("intake")),
                "create-a",
            )
            .unwrap();
        assert_eq!(retry.generation, recovered.generation);
    }
}

#[test]
fn interrupted_snapshot_pruning_preserves_the_newest_generation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let healthy = RecoveryStore::new(&root);
    healthy
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    healthy
        .set_lifecycle("recovery-a", RecoveryLifecycle::Running, None, "running-a")
        .unwrap();

    let faulted = RecoveryStore::new(&root)
        .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterSnapshotPruneDeletion);
    let error = faulted
        .set_lifecycle(
            "recovery-a",
            RecoveryLifecycle::Interrupted,
            Some("simulated process crash".to_string()),
            "interrupt-a",
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RecoveryStoreError::InjectedFault(RecoveryStoreFaultPoint::AfterSnapshotPruneDeletion)
    ));

    let recovered = healthy.load("recovery-a").unwrap().unwrap();
    assert_eq!(recovered.generation, 3);
    assert_eq!(recovered.lifecycle, RecoveryLifecycle::Interrupted);
    assert_eq!(
        recovered.lifecycle_reason.as_deref(),
        Some("simulated process crash")
    );
}

#[test]
fn event_compaction_is_bounded_and_a_crash_after_deletion_replays_from_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let healthy = RecoveryStore::new(&root);
    healthy
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();

    let faulted = RecoveryStore::new(&root)
        .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterEventCompactionDeletion);
    let error = faulted
        .set_lifecycle(
            "recovery-a",
            RecoveryLifecycle::Running,
            Some("provider started".to_string()),
            "running-a",
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RecoveryStoreError::InjectedFault(RecoveryStoreFaultPoint::AfterEventCompactionDeletion)
    ));
    let recovered = healthy.load("recovery-a").unwrap().unwrap();
    assert_eq!(recovered.generation, 2);
    assert_eq!(recovered.lifecycle, RecoveryLifecycle::Running);
    let retry = healthy
        .set_lifecycle(
            "recovery-a",
            RecoveryLifecycle::Running,
            Some("provider started".to_string()),
            "running-a",
        )
        .unwrap();
    assert_eq!(retry.generation, recovered.generation);

    for index in 0..12 {
        healthy
            .set_lifecycle(
                "recovery-a",
                RecoveryLifecycle::Interrupted,
                Some(format!("checkpoint {index}")),
                format!("interrupt-{index}"),
            )
            .unwrap();
    }
    let events_dir = root.join("recoveries").join("recovery-a").join("events");
    let committed_events = std::fs::read_dir(events_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .count();
    assert!(
        committed_events <= 2,
        "event compaction must remain bounded, found {committed_events}"
    );
    assert_eq!(healthy.load("recovery-a").unwrap().unwrap().generation, 14);
}

#[test]
fn resolve_or_discard_purges_content_and_keeps_only_30_day_tombstone() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("recovery");
    let store = RecoveryStore::new(&root);
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    let purged_at = Utc.with_ymd_and_hms(2026, 7, 16, 4, 5, 6).unwrap();

    let tombstone = store
        .finalize_and_purge(
            "recovery-a",
            RecoveryLifecycle::Discarded,
            purged_at,
            "discard-a",
        )
        .unwrap();

    assert!(store.load("recovery-a").unwrap().is_none());
    assert!(!root.join("recoveries").join("recovery-a").exists());
    assert_eq!(tombstone.lifecycle, RecoveryLifecycle::Discarded);
    assert_eq!(
        tombstone.launch_base_oid.as_deref(),
        Some("1111111111111111111111111111111111111111")
    );
    assert_eq!(tombstone.expires_at, purged_at + Duration::days(30));
    assert!(tombstone.session_identity_hash.starts_with("sha256:"));
    let serialized = serde_json::to_string(&tombstone).unwrap();
    assert!(!serialized.contains("Investigate Intake recovery"));
    assert_eq!(
        store.load_tombstone("recovery-a").unwrap().unwrap(),
        tombstone
    );
}

#[test]
fn tombstone_retention_removes_only_expired_verified_entries() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let purged_at = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
    for recovery_id in ["expired", "retained"] {
        store
            .create(
                create_request(recovery_id, &temp.path().join(recovery_id)),
                format!("create-{recovery_id}"),
            )
            .unwrap();
        store
            .finalize_and_purge(
                recovery_id,
                RecoveryLifecycle::Resolved,
                if recovery_id == "expired" {
                    purged_at
                } else {
                    purged_at + Duration::days(20)
                },
                format!("resolve-{recovery_id}"),
            )
            .unwrap();
    }

    assert_eq!(
        store
            .remove_expired_tombstones(purged_at + Duration::days(31))
            .unwrap(),
        1
    );
    assert!(store.load_tombstone("expired").unwrap().is_none());
    assert!(store.load_tombstone("retained").unwrap().is_some());
}

#[test]
fn interrupted_purge_retries_from_the_tombstone_and_removes_all_gwt_content() {
    for fault in [
        RecoveryStoreFaultPoint::AfterTombstonePublication,
        RecoveryStoreFaultPoint::AfterRecoveryPayloadPurge,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("recovery");
        let healthy = RecoveryStore::new(&root);
        healthy
            .create(
                create_request("recovery-a", &temp.path().join("intake")),
                "create-a",
            )
            .unwrap();
        let attachment = healthy
            .copy_attachment_bytes("evidence.txt", b"purge after crash")
            .unwrap();
        healthy
            .bind_root(
                "recovery-a",
                ProviderRootBinding {
                    root_id: "root-1".to_string(),
                    session_tree_id: None,
                    quality: BindingQuality::Verified,
                    bound_at: Utc::now(),
                },
                "bind-a",
            )
            .unwrap();
        healthy
            .replace_checkpoint(
                "recovery-a",
                "root-1",
                0,
                SemanticCheckpoint {
                    attachment_refs: vec![attachment.clone()],
                    ..SemanticCheckpoint::default()
                },
                "checkpoint-a",
            )
            .unwrap();
        let durable = durable_attachment_path(&root, &attachment);
        let purged_at = Utc.with_ymd_and_hms(2026, 7, 16, 4, 5, 6).unwrap();
        let faulted = RecoveryStore::new(&root).with_fault_injection_for_test(fault);

        let error = faulted
            .finalize_and_purge(
                "recovery-a",
                RecoveryLifecycle::Discarded,
                purged_at,
                "discard-a",
            )
            .unwrap_err();
        assert!(matches!(error, RecoveryStoreError::InjectedFault(actual) if actual == fault));
        assert!(healthy.load_tombstone("recovery-a").unwrap().is_some());

        let tombstone = healthy
            .finalize_and_purge(
                "recovery-a",
                RecoveryLifecycle::Discarded,
                purged_at,
                "discard-a",
            )
            .unwrap();
        assert_eq!(tombstone.terminal_operation_id, "discard-a");
        assert!(healthy.load("recovery-a").unwrap().is_none());
        assert!(!root.join("recoveries").join("recovery-a").exists());
        assert!(!durable.exists());
    }
}

#[test]
fn project_store_uses_repo_hash_scope_and_lists_newest_recovery_first() {
    let temp = tempfile::tempdir().unwrap();
    let repo_hash = compute_repo_hash("https://github.com/example/recovery.git");
    let project_root = temp.path().join("projects").join(repo_hash.as_str());
    let store = RecoveryStore::for_project_dir(&project_root);

    let mut older = create_request("recovery-old", &temp.path().join("intake-old"));
    older.created_at = Utc.with_ymd_and_hms(2026, 7, 16, 1, 0, 0).unwrap();
    store.create(older, "create-old").unwrap();

    let mut newer = create_request("recovery-new", &temp.path().join("intake-new"));
    newer.created_at = Utc.with_ymd_and_hms(2026, 7, 16, 2, 0, 0).unwrap();
    store.create(newer, "create-new").unwrap();

    assert_eq!(store.root(), project_root);
    assert_eq!(
        store
            .list()
            .unwrap()
            .into_iter()
            .map(|record| record.recovery_id)
            .collect::<Vec<_>>(),
        ["recovery-new", "recovery-old"]
    );
}

#[test]
fn checkpoint_and_board_outbox_share_one_durable_generation_until_ack() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    store
        .bind_root(
            "recovery-a",
            ProviderRootBinding {
                root_id: "root-1".to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-a",
        )
        .unwrap();

    let checkpoint = SemanticCheckpoint {
        summary: "Recovery design is settled.".to_string(),
        board_intents: vec![BoardMilestoneIntent {
            entry_id: "recovery-a-checkpoint-1".to_string(),
            title: "Intake checkpoint".to_string(),
            body: "Decision: use exact resume with checkpoint fallback.".to_string(),
            queued_at: Utc::now(),
        }],
        ..SemanticCheckpoint::default()
    };
    let replaced = store
        .replace_checkpoint("recovery-a", "root-1", 0, checkpoint, "checkpoint-a")
        .unwrap();
    assert_eq!(replaced.board_outbox.len(), 1);
    assert!(replaced.board_entry_ids.is_empty());

    let acked = store
        .ack_board_entry("recovery-a", "recovery-a-checkpoint-1", "board-ack-a")
        .unwrap();
    assert!(acked.board_outbox.is_empty());
    assert_eq!(acked.board_entry_ids, ["recovery-a-checkpoint-1"]);
    let retry = store
        .ack_board_entry("recovery-a", "recovery-a-checkpoint-1", "board-ack-a")
        .unwrap();
    assert_eq!(retry.generation, acked.generation);
}

#[test]
fn checkpoint_continuation_prompt_is_bounded_and_never_starts_blank() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-a", &temp.path().join("intake")),
            "create-a",
        )
        .unwrap();
    store
        .bind_root(
            "recovery-a",
            ProviderRootBinding {
                root_id: "root-1".to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-a",
        )
        .unwrap();
    store
        .replace_checkpoint(
            "recovery-a",
            "root-1",
            0,
            SemanticCheckpoint {
                summary: "Use exact resume with a semantic fallback.".to_string(),
                confirmed_decisions: vec!["Keep recovery outside the worktree.".to_string()],
                open_questions: vec!["How should legacy ambiguity be confirmed?".to_string()],
                next_action: Some("Wire the Recovery Center.".to_string()),
                as_of_turn_id: Some("turn-2".to_string()),
                visible_items: vec![
                    VisibleDiscussionItem {
                        role: "user".to_string(),
                        kind: "message".to_string(),
                        text: "older visible item".to_string(),
                        partial: false,
                    },
                    VisibleDiscussionItem {
                        role: "assistant".to_string(),
                        kind: "message".to_string(),
                        text: "latest visible item".to_string(),
                        partial: false,
                    },
                ],
                attachment_refs: Vec::new(),
                board_intents: Vec::new(),
            },
            "checkpoint-a",
        )
        .unwrap();
    let stale = store
        .record_root_input(
            "recovery-a",
            "root-1",
            "turn-3",
            "Current user intent after the checkpoint.",
            "input-a",
        )
        .unwrap();

    let prompt = build_checkpoint_continuation_prompt(&stale, 1, 1_200).unwrap();
    assert!(prompt.contains("Use exact resume with a semantic fallback."));
    assert!(prompt.contains("Keep recovery outside the worktree."));
    assert!(prompt.contains("Wire the Recovery Center."));
    assert!(prompt.contains("Current user intent after the checkpoint."));
    assert!(prompt.contains("latest visible item"));
    assert!(!prompt.contains("older visible item"));
    assert!(prompt.chars().count() <= 1_200);
}

#[test]
fn continuation_without_checkpoint_uses_initial_prompt_or_requires_attention() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let with_initial = store
        .create(
            create_request("recovery-with-initial", &temp.path().join("intake")),
            "create-initial",
        )
        .unwrap();
    let prompt = build_checkpoint_continuation_prompt(&with_initial, 4, 800).unwrap();
    assert!(prompt.contains("Investigate Intake recovery"));

    let mut blank_request = create_request("recovery-blank", &temp.path().join("legacy"));
    blank_request.initial_prompt.clear();
    let blank = store.create(blank_request, "create-blank").unwrap();
    assert!(matches!(
        build_checkpoint_continuation_prompt(&blank, 4, 800),
        Err(RecoveryStoreError::NoContinuationContext { .. })
    ));
}

#[test]
fn continuation_link_records_source_target_revision_and_definitive_reason() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-source", &temp.path().join("intake")),
            "create-source",
        )
        .unwrap();
    let mut target = create_request("recovery-target", &temp.path().join("intake"));
    target.session_id = "session-target".to_string();
    store.create(target, "create-target").unwrap();
    let linked_at = Utc.with_ymd_and_hms(2026, 7, 16, 2, 3, 4).unwrap();
    let link = RecoveryContinuationLink {
        source_recovery_id: "recovery-source".to_string(),
        target_recovery_id: "recovery-target".to_string(),
        source_checkpoint_revision: 0,
        definitive_reason: "Exact provider resume rejected: No conversation found with session ID"
            .to_string(),
        linked_at,
    };

    let (source, target) = store
        .link_continuation(link.clone(), "link-source-target")
        .unwrap();

    assert_eq!(source.continuation_targets, vec![link.clone()]);
    assert_eq!(target.continuation_source, Some(link.clone()));
    let retry = store.link_continuation(link, "link-source-target").unwrap();
    assert_eq!(retry.0.generation, source.generation);
    assert_eq!(retry.1.generation, target.generation);
}

#[test]
fn successor_is_prepared_before_target_creation_and_semantic_retry_reuses_identity() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-source", &temp.path().join("intake")),
            "create-source",
        )
        .unwrap();
    let linked_at = Utc.with_ymd_and_hms(2026, 7, 16, 4, 0, 0).unwrap();
    let prepared = store
        .prepare_successor(
            RecoveryContinuationLink {
                source_recovery_id: "recovery-source".to_string(),
                target_recovery_id: "recovery-target".to_string(),
                source_checkpoint_revision: 0,
                definitive_reason: "manual:confirm_resume".to_string(),
                linked_at,
            },
            "prepare-successor",
        )
        .unwrap();
    assert_eq!(prepared.target_recovery_id, "recovery-target");
    assert!(store.load("recovery-source").unwrap().is_some());

    let semantic_retry = store
        .prepare_successor(
            RecoveryContinuationLink {
                source_recovery_id: "recovery-source".to_string(),
                target_recovery_id: "a-different-proposal".to_string(),
                source_checkpoint_revision: 0,
                definitive_reason: "manual:confirm_resume".to_string(),
                linked_at: linked_at + Duration::seconds(1),
            },
            "prepare-successor-retry",
        )
        .unwrap();
    assert_eq!(semantic_retry, prepared);
    assert_eq!(
        store
            .prepared_successor_for_source("recovery-source")
            .unwrap(),
        Some(prepared.clone())
    );

    assert!(matches!(
        store.prepare_successor(
            RecoveryContinuationLink {
                source_recovery_id: "recovery-source".to_string(),
                target_recovery_id: "conflicting-target".to_string(),
                source_checkpoint_revision: 0,
                definitive_reason: "manual:start_fresh".to_string(),
                linked_at,
            },
            "prepare-conflicting-successor",
        ),
        Err(RecoveryStoreError::ContinuationConflict)
    ));
}

#[test]
fn prepared_successor_repairs_orphan_target_link_after_target_creation() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-source", &temp.path().join("intake")),
            "create-source",
        )
        .unwrap();
    let link = RecoveryContinuationLink {
        source_recovery_id: "recovery-source".to_string(),
        target_recovery_id: "recovery-target".to_string(),
        source_checkpoint_revision: 0,
        definitive_reason: "manual:continue_checkpoint".to_string(),
        linked_at: Utc::now(),
    };
    store
        .prepare_successor(link.clone(), "prepare-successor")
        .unwrap();

    let mut target = create_request("recovery-target", &temp.path().join("intake"));
    target.session_id = "session-target".to_string();
    store.create(target, "create-target").unwrap();
    let records = store.list().unwrap();
    assert_eq!(records.len(), 2);
    let source = store.load("recovery-source").unwrap().unwrap();
    let target = store.load("recovery-target").unwrap().unwrap();
    assert_eq!(source.continuation_targets, vec![link.clone()]);
    assert_eq!(target.continuation_source, Some(link.clone()));
    assert_eq!(
        store
            .prepared_successor_for_source("recovery-source")
            .unwrap(),
        Some(link)
    );
}

#[test]
fn prepared_successor_repairs_every_link_publication_boundary() {
    for fault in [
        RecoveryStoreFaultPoint::AfterContinuationSourcePublication,
        RecoveryStoreFaultPoint::AfterContinuationTargetPublication,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("recovery");
        let healthy = RecoveryStore::new(&root);
        healthy
            .create(
                create_request("recovery-source", &temp.path().join("intake")),
                "create-source",
            )
            .unwrap();
        let link = RecoveryContinuationLink {
            source_recovery_id: "recovery-source".to_string(),
            target_recovery_id: "recovery-target".to_string(),
            source_checkpoint_revision: 0,
            definitive_reason: "startup:semantic".to_string(),
            linked_at: Utc::now(),
        };
        healthy
            .prepare_successor(link.clone(), "prepare-successor")
            .unwrap();
        let mut target = create_request("recovery-target", &temp.path().join("intake"));
        target.session_id = "session-target".to_string();
        healthy.create(target, "create-target").unwrap();

        let faulted = RecoveryStore::new(&root).with_fault_injection_for_test(fault);
        assert!(matches!(
            faulted.load("recovery-target"),
            Err(RecoveryStoreError::InjectedFault(actual)) if actual == fault
        ));
        let restarted = RecoveryStore::new(&root);
        let target = restarted.load("recovery-target").unwrap().unwrap();
        let source = restarted.load("recovery-source").unwrap().unwrap();
        assert_eq!(source.continuation_targets, vec![link.clone()]);
        assert_eq!(target.continuation_source, Some(link));
    }
}

#[test]
fn ready_successor_finalization_is_repaired_after_each_crash_boundary() {
    for fault in [
        RecoveryStoreFaultPoint::AfterSuccessorReadyObservation,
        RecoveryStoreFaultPoint::AfterTombstonePublication,
        RecoveryStoreFaultPoint::AfterRecoveryPayloadPurge,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("recovery");
        let healthy = RecoveryStore::new(&root);
        healthy
            .create(
                create_request("recovery-source", &temp.path().join("intake")),
                "create-source",
            )
            .unwrap();
        let link = RecoveryContinuationLink {
            source_recovery_id: "recovery-source".to_string(),
            target_recovery_id: "recovery-target".to_string(),
            source_checkpoint_revision: 0,
            definitive_reason: "manual:start_fresh".to_string(),
            linked_at: Utc::now(),
        };
        healthy
            .prepare_successor(link, "prepare-successor")
            .unwrap();
        let mut target = create_request("recovery-target", &temp.path().join("intake"));
        target.session_id = "session-target".to_string();
        healthy.create(target, "create-target").unwrap();
        healthy
            .bind_root_semantic(
                "recovery-target",
                "target-root",
                None,
                BindingQuality::Verified,
                "bind-target",
            )
            .unwrap();
        healthy
            .advance_launch_stage(
                "recovery-target",
                RecoveryLaunchStage::Ready,
                Some(ProviderRootRole::Root),
                "ready-target",
            )
            .unwrap();

        let faulted = RecoveryStore::new(&root).with_fault_injection_for_test(fault);
        assert!(matches!(
            faulted.list(),
            Err(RecoveryStoreError::InjectedFault(actual)) if actual == fault
        ));
        let restarted = RecoveryStore::new(&root);
        let records = restarted.list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].recovery_id, "recovery-target");
        assert!(restarted.load("recovery-source").unwrap().is_none());
        assert_eq!(
            restarted
                .load_tombstone("recovery-source")
                .unwrap()
                .unwrap()
                .lifecycle,
            RecoveryLifecycle::Resolved
        );
        assert!(restarted
            .prepared_successor_for_source("recovery-source")
            .unwrap()
            .is_none());
    }
}

#[test]
fn discarded_materialized_successor_is_cancelled_and_retry_gets_a_new_identity() {
    for fault in [
        RecoveryStoreFaultPoint::AfterContinuationSourcePublication,
        RecoveryStoreFaultPoint::AfterContinuationIntentCleanup,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("recovery");
        let healthy = RecoveryStore::new(&root);
        healthy
            .create(
                create_request("recovery-source", &temp.path().join("intake")),
                "create-source",
            )
            .unwrap();
        let linked_at = Utc.with_ymd_and_hms(2026, 7, 16, 6, 0, 0).unwrap();
        let discarded_link = RecoveryContinuationLink {
            source_recovery_id: "recovery-source".to_string(),
            target_recovery_id: "recovery-target-discarded".to_string(),
            source_checkpoint_revision: 0,
            definitive_reason: "startup:exact".to_string(),
            linked_at,
        };
        healthy
            .prepare_successor(discarded_link.clone(), "prepare-discarded-target")
            .unwrap();
        let mut discarded_target =
            create_request("recovery-target-discarded", &temp.path().join("intake"));
        discarded_target.session_id = "session-target-discarded".to_string();
        healthy
            .create(discarded_target.clone(), "create-discarded-target")
            .unwrap();
        let discard_faulted = RecoveryStore::new(&root)
            .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterTombstonePublication);
        assert!(matches!(
            discard_faulted.finalize_and_purge(
                "recovery-target-discarded",
                RecoveryLifecycle::Discarded,
                Utc::now(),
                "discard-materialized-target",
            ),
            Err(RecoveryStoreError::InjectedFault(
                RecoveryStoreFaultPoint::AfterTombstonePublication
            ))
        ));

        let faulted = RecoveryStore::new(&root).with_fault_injection_for_test(fault);
        assert!(matches!(
            faulted.prepared_successor_for_source("recovery-source"),
            Err(RecoveryStoreError::InjectedFault(actual)) if actual == fault
        ));

        let restarted = RecoveryStore::new(&root);
        assert!(restarted
            .prepared_successor_for_source("recovery-source")
            .unwrap()
            .is_none());
        assert!(restarted
            .load("recovery-source")
            .unwrap()
            .unwrap()
            .continuation_targets
            .is_empty());
        assert!(matches!(
            restarted.create(discarded_target, "create-discarded-target"),
            Err(RecoveryStoreError::TombstoneConflict(id))
                if id == "recovery-target-discarded"
        ));
        restarted
            .finalize_and_purge(
                "recovery-target-discarded",
                RecoveryLifecycle::Discarded,
                Utc::now(),
                "discard-materialized-target",
            )
            .unwrap();

        let replacement = restarted
            .prepare_successor(
                RecoveryContinuationLink {
                    source_recovery_id: "recovery-source".to_string(),
                    target_recovery_id: "recovery-target-replacement".to_string(),
                    source_checkpoint_revision: 0,
                    definitive_reason: "startup:exact".to_string(),
                    linked_at: linked_at + Duration::seconds(1),
                },
                "prepare-replacement-target",
            )
            .unwrap();
        assert_eq!(
            replacement.target_recovery_id,
            "recovery-target-replacement"
        );
    }
}

#[test]
fn continuation_link_repairs_every_transaction_publication_boundary() {
    for fault in [
        RecoveryStoreFaultPoint::AfterContinuationIntentPublication,
        RecoveryStoreFaultPoint::AfterEventPublication,
        RecoveryStoreFaultPoint::AfterOperationReceiptPublication,
        RecoveryStoreFaultPoint::AfterSnapshotPublication,
        RecoveryStoreFaultPoint::AfterContinuationSourcePublication,
        RecoveryStoreFaultPoint::AfterContinuationTargetPublication,
        RecoveryStoreFaultPoint::AfterContinuationIntentCleanup,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let store_root = temp.path().join("recovery");
        let healthy = RecoveryStore::new(&store_root);
        healthy
            .create(
                create_request("recovery-source", &temp.path().join("intake")),
                "create-source",
            )
            .unwrap();
        let mut target = create_request("recovery-target", &temp.path().join("intake"));
        target.session_id = "session-target".to_string();
        healthy.create(target, "create-target").unwrap();

        let linked_at = Utc.with_ymd_and_hms(2026, 7, 16, 2, 3, 4).unwrap();
        let link = RecoveryContinuationLink {
            source_recovery_id: "recovery-source".to_string(),
            target_recovery_id: "recovery-target".to_string(),
            source_checkpoint_revision: 0,
            definitive_reason: "Exact resume was definitively rejected".to_string(),
            linked_at,
        };
        let faulted = RecoveryStore::new(&store_root).with_fault_injection_for_test(fault);
        assert!(matches!(
            faulted.link_continuation(link.clone(), "link-source-target"),
            Err(RecoveryStoreError::InjectedFault(actual)) if actual == fault
        ));

        // A restarted reader repairs the durable intent before returning
        // either side, so callers never observe a permanently half-linked
        // continuation.
        let restarted = RecoveryStore::new(&store_root);
        let target = restarted.load("recovery-target").unwrap().unwrap();
        let source = restarted.load("recovery-source").unwrap().unwrap();
        assert_eq!(source.continuation_targets, vec![link.clone()]);
        assert_eq!(target.continuation_source, Some(link.clone()));
        assert!(
            std::fs::read_dir(store_root.join("continuations"))
                .unwrap()
                .all(|entry| entry
                    .unwrap()
                    .path()
                    .extension()
                    .and_then(|value| value.to_str())
                    != Some("json")),
            "completed continuation intent must be cleaned up after {fault:?}"
        );

        // linked_at is observation metadata. Reconstructing the semantic
        // retry after a process restart must not create a new generation.
        let mut semantic_retry = link;
        semantic_retry.linked_at += Duration::seconds(1);
        let retry = restarted
            .link_continuation(semantic_retry, "link-source-target")
            .unwrap();
        assert_eq!(retry.0.generation, source.generation);
        assert_eq!(retry.1.generation, target.generation);
    }
}

#[test]
fn continuation_target_retains_provider_root_provenance_after_source_purge() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-source", &temp.path().join("intake")),
            "create-source",
        )
        .unwrap();
    store
        .bind_root_semantic(
            "recovery-source",
            "provider-root-source",
            None,
            BindingQuality::Verified,
            "bind-source",
        )
        .unwrap();
    let mut target = create_request("recovery-target", &temp.path().join("intake"));
    target.session_id = "session-target".to_string();
    store.create(target, "create-target").unwrap();
    store
        .link_continuation(
            RecoveryContinuationLink {
                source_recovery_id: "recovery-source".to_string(),
                target_recovery_id: "recovery-target".to_string(),
                source_checkpoint_revision: 0,
                definitive_reason: "Exact resume was definitively rejected".to_string(),
                linked_at: Utc::now(),
            },
            "link-source-target",
        )
        .unwrap();

    let target = store
        .bind_root_semantic(
            "recovery-target",
            "provider-root-target",
            Some("provider-tree-target".to_string()),
            BindingQuality::Verified,
            "bind-target",
        )
        .unwrap();
    let provenance = target
        .continuation_root_provenance
        .as_ref()
        .expect("target continuation root provenance");
    assert_eq!(
        provenance.source_provider_root_id.as_deref(),
        Some("provider-root-source")
    );
    assert_eq!(
        provenance.target_provider_root_id.as_deref(),
        Some("provider-root-target")
    );

    store
        .finalize_and_purge(
            "recovery-source",
            RecoveryLifecycle::Resolved,
            Utc::now(),
            "purge-source",
        )
        .unwrap();
    let target = store.load("recovery-target").unwrap().unwrap();
    assert_eq!(
        target
            .continuation_root_provenance
            .as_ref()
            .and_then(|value| value.source_provider_root_id.as_deref()),
        Some("provider-root-source")
    );
    assert_eq!(
        target
            .continuation_root_provenance
            .as_ref()
            .and_then(|value| value.target_provider_root_id.as_deref()),
        Some("provider-root-target")
    );
}

#[test]
fn recovery_record_without_continuation_root_provenance_remains_deserializable() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    let record = store
        .create(
            create_request("recovery-source", &temp.path().join("intake")),
            "create-source",
        )
        .unwrap();
    let mut legacy_json = serde_json::to_value(record).unwrap();
    legacy_json
        .as_object_mut()
        .unwrap()
        .remove("continuation_root_provenance");

    let restored: gwt_core::recovery::RecoveryRecord = serde_json::from_value(legacy_json).unwrap();
    assert!(restored.continuation_root_provenance.is_none());
}

#[test]
fn continuation_link_inherits_semantic_context_and_keeps_shared_attachments_after_source_purge() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("recovery"));
    store
        .create(
            create_request("recovery-source", &temp.path().join("intake")),
            "create-source",
        )
        .unwrap();
    store
        .bind_root(
            "recovery-source",
            ProviderRootBinding {
                root_id: "root-source".to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-source",
        )
        .unwrap();
    let attachment = store
        .copy_attachment_bytes("board-gap.txt", b"board evidence")
        .unwrap();
    let source = store.load("recovery-source").unwrap().unwrap();
    let source = store
        .replace_checkpoint(
            "recovery-source",
            "root-source",
            source.checkpoint_revision,
            SemanticCheckpoint {
                summary: "Continue the Board recovery investigation.".to_string(),
                attachment_refs: vec![attachment.clone()],
                board_intents: vec![BoardMilestoneIntent {
                    entry_id: "source-board-entry".to_string(),
                    title: "Source only".to_string(),
                    body: "Do not republish from the target".to_string(),
                    queued_at: Utc::now(),
                }],
                ..SemanticCheckpoint::default()
            },
            "checkpoint-source",
        )
        .unwrap();
    let mut target = create_request("recovery-target", &temp.path().join("intake"));
    target.session_id = "session-target".to_string();
    target.initial_prompt = "stale container path /tmp/old-attachment".to_string();
    store.create(target, "create-target").unwrap();
    let link = RecoveryContinuationLink {
        source_recovery_id: "recovery-source".to_string(),
        target_recovery_id: "recovery-target".to_string(),
        source_checkpoint_revision: source.checkpoint_revision,
        definitive_reason: "Exact provider resume was definitively rejected".to_string(),
        linked_at: Utc::now(),
    };

    let (_, target) = store.link_continuation(link, "link-with-context").unwrap();
    let inherited = target.checkpoint.as_ref().expect("inherited checkpoint");
    assert_eq!(
        inherited.summary,
        "Continue the Board recovery investigation."
    );
    assert_eq!(inherited.attachment_refs, vec![attachment.clone()]);
    assert!(inherited.board_intents.is_empty());
    assert_eq!(target.checkpoint_revision, source.checkpoint_revision);
    let prompt = build_checkpoint_continuation_prompt_with_attachments(&store, &target, 4, 2_000)
        .expect("target prompt");
    assert!(prompt.contains("Continue the Board recovery investigation."));
    assert!(prompt.contains("board-gap.txt"));
    assert!(!prompt.contains("/tmp/old-attachment"));

    store
        .finalize_and_purge(
            "recovery-source",
            RecoveryLifecycle::Resolved,
            Utc::now(),
            "purge-source",
        )
        .unwrap();
    store.verify_attachment(&attachment).unwrap();
    assert!(
        build_checkpoint_continuation_prompt_with_attachments(&store, &target, 4, 2_000).is_ok()
    );
}
