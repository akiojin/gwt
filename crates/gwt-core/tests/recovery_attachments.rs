use chrono::Utc;
use gwt_core::recovery::{
    build_checkpoint_continuation_prompt_with_attachments, read_recovery_attachment_bytes,
    BindingQuality, CreateRecovery, ProviderRootBinding, RecoveryLifecycle, RecoverySessionKind,
    RecoveryStore, RecoveryStoreError, RecoveryStoreFaultPoint, SemanticCheckpoint,
    MAX_RECOVERY_ATTACHMENT_BYTES,
};

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

fn create_bound_recovery(store: &RecoveryStore, recovery_id: &str, worktree: &std::path::Path) {
    store
        .create(
            CreateRecovery {
                recovery_id: recovery_id.to_string(),
                session_id: format!("session-{recovery_id}"),
                repo_id: "repo-attachment-test".to_string(),
                session_kind: RecoverySessionKind::Intake,
                worktree_path: worktree.to_path_buf(),
                launch_base_ref: Some("origin/develop".to_string()),
                launch_base_oid: "1".repeat(40),
                launch_head_oid: "1".repeat(40),
                provider: "codex".to_string(),
                model: None,
                runtime: "host".to_string(),
                initial_prompt: "Inspect the attached design evidence".to_string(),
                created_at: Utc::now(),
            },
            format!("create-{recovery_id}"),
        )
        .unwrap();
    store
        .bind_root(
            recovery_id,
            ProviderRootBinding {
                root_id: format!("root-{recovery_id}"),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            format!("bind-{recovery_id}"),
        )
        .unwrap();
}

#[test]
fn external_attachment_is_copied_by_content_without_retaining_source_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project-recovery");
    let source_dir = temp.path().join("outside-project");
    std::fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join("Recovery design.png");
    std::fs::write(&source, b"durable attachment bytes").unwrap();
    let store = RecoveryStore::new(&root);

    let first = store.copy_attachment(&source).unwrap();
    let second = store.copy_attachment(&source).unwrap();
    let from_sidecar = store
        .copy_attachment_bytes("container-image.png", b"durable attachment bytes")
        .unwrap();

    assert_eq!(first, second, "identical bytes must deduplicate");
    assert_eq!(first.content_id, from_sidecar.content_id);
    assert!(first.content_id.starts_with("sha256:"));
    assert_eq!(first.file_name, "Recovery design.png");
    assert_eq!(first.byte_len, 24);
    let metadata = serde_json::to_string(&first).unwrap();
    assert!(!metadata.contains(&source_dir.to_string_lossy().to_string()));

    std::fs::remove_file(&source).unwrap();
    let durable = durable_attachment_path(&root, &first);
    assert!(durable.starts_with(&root));
    assert_eq!(
        store
            .read_attachment_bytes(&first, MAX_RECOVERY_ATTACHMENT_BYTES)
            .unwrap(),
        b"durable attachment bytes"
    );
}

#[test]
fn purge_removes_unshared_attachment_content_only_after_last_reference() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project-recovery");
    let store = RecoveryStore::new(&root);
    let source = temp.path().join("evidence.txt");
    std::fs::write(&source, b"shared evidence").unwrap();
    let attachment = store.copy_attachment(&source).unwrap();

    for recovery_id in ["recovery-a", "recovery-b"] {
        create_bound_recovery(&store, recovery_id, temp.path());
        store
            .replace_checkpoint(
                recovery_id,
                &format!("root-{recovery_id}"),
                0,
                SemanticCheckpoint {
                    summary: "Use the attached evidence.".to_string(),
                    attachment_refs: vec![attachment.clone()],
                    ..SemanticCheckpoint::default()
                },
                format!("checkpoint-{recovery_id}"),
            )
            .unwrap();
    }
    let durable = durable_attachment_path(&root, &attachment);
    assert!(durable.is_file());

    store
        .finalize_and_purge(
            "recovery-a",
            RecoveryLifecycle::Resolved,
            Utc::now(),
            "resolve-a",
        )
        .unwrap();
    assert!(durable.is_file(), "the second recovery still references it");

    store
        .finalize_and_purge(
            "recovery-b",
            RecoveryLifecycle::Discarded,
            Utc::now(),
            "discard-b",
        )
        .unwrap();
    assert!(!durable.exists(), "last-reference purge removes the blob");
}

#[test]
fn attachment_copy_rejects_non_file_sources() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("project-recovery"));
    let error = store.copy_attachment(temp.path()).unwrap_err();
    assert!(error.to_string().contains("regular file"), "{error}");
    let error = store
        .copy_attachment_bytes("../secret.txt", b"secret")
        .unwrap_err();
    assert!(error.to_string().contains("basename"), "{error}");
}

#[test]
fn attachment_copy_rejects_content_over_the_durable_size_bound() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::new(temp.path().join("project-recovery"));
    let oversized = vec![0_u8; MAX_RECOVERY_ATTACHMENT_BYTES as usize + 1];
    let error = store
        .copy_attachment_bytes("oversized.bin", &oversized)
        .unwrap_err();
    assert!(error.to_string().contains("size limit"), "{error}");
}

#[test]
fn attachment_path_reads_reject_oversized_sparse_files_before_allocating() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("oversized-sparse.bin");
    std::fs::File::create(&source)
        .unwrap()
        .set_len(MAX_RECOVERY_ATTACHMENT_BYTES + 1)
        .unwrap();
    let store = RecoveryStore::new(temp.path().join("project-recovery"));

    let read_error = read_recovery_attachment_bytes(&source).unwrap_err();
    let copy_error = store.copy_attachment(&source).unwrap_err();

    assert!(
        read_error.to_string().contains("size limit"),
        "{read_error}"
    );
    assert!(
        copy_error.to_string().contains("size limit"),
        "{copy_error}"
    );
}

#[test]
fn attachment_path_reads_never_follow_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("private.txt");
    let source = temp.path().join("attachment.txt");
    std::fs::write(&target, b"must not be copied through a symlink").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &source).unwrap();
    #[cfg(windows)]
    if let Err(error) = std::os::windows::fs::symlink_file(&target, &source) {
        if error.kind() == std::io::ErrorKind::PermissionDenied
            || error.raw_os_error() == Some(1314)
        {
            // Windows requires Developer Mode or SeCreateSymbolicLinkPrivilege.
            // Production still uses FILE_FLAG_OPEN_REPARSE_POINT; other tests
            // cover the shared handle validation path on every platform.
            return;
        }
        panic!("create attachment symlink: {error}");
    }
    let store = RecoveryStore::new(temp.path().join("project-recovery"));

    assert!(read_recovery_attachment_bytes(&source).is_err());
    assert!(store.copy_attachment(&source).is_err());
}

#[test]
fn continuation_prompt_resolves_only_durable_attachment_paths_within_the_bound() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project-recovery");
    let store = RecoveryStore::new(&root);
    let source_dir = temp.path().join("external-source");
    std::fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join("design evidence.png");
    std::fs::write(&source, b"durable image bytes").unwrap();
    let attachment = store.copy_attachment(&source).unwrap();
    create_bound_recovery(&store, "prompt-recovery", temp.path());
    let record = store
        .replace_checkpoint(
            "prompt-recovery",
            "root-prompt-recovery",
            0,
            SemanticCheckpoint {
                summary: "Continue from the reviewed screenshot.".repeat(40),
                attachment_refs: vec![attachment.clone()],
                ..SemanticCheckpoint::default()
            },
            "checkpoint-prompt-recovery",
        )
        .unwrap();
    std::fs::remove_file(&source).unwrap();

    let prompt =
        build_checkpoint_continuation_prompt_with_attachments(&store, &record, 12, 900).unwrap();
    let durable = durable_attachment_path(&root, &attachment);

    assert!(prompt.contains("Recovered attachments"));
    assert!(prompt.contains("design evidence.png"));
    assert!(prompt.contains(&durable.to_string_lossy().to_string()));
    assert!(!prompt.contains(&source_dir.to_string_lossy().to_string()));
    assert!(prompt.chars().count() <= 900);
}

#[test]
fn continuation_prompt_fails_closed_when_a_durable_attachment_is_corrupt() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project-recovery");
    let store = RecoveryStore::new(&root);
    let attachment = store
        .copy_attachment_bytes("evidence.txt", b"trusted evidence")
        .unwrap();
    create_bound_recovery(&store, "corrupt-recovery", temp.path());
    let record = store
        .replace_checkpoint(
            "corrupt-recovery",
            "root-corrupt-recovery",
            0,
            SemanticCheckpoint {
                summary: "Use the evidence.".to_string(),
                attachment_refs: vec![attachment.clone()],
                ..SemanticCheckpoint::default()
            },
            "checkpoint-corrupt-recovery",
        )
        .unwrap();
    let durable = durable_attachment_path(&root, &attachment);
    std::fs::write(durable, b"tampered").unwrap();

    let error = build_checkpoint_continuation_prompt_with_attachments(&store, &record, 12, 900)
        .unwrap_err();
    assert!(error.to_string().contains("digest verification"), "{error}");
}

#[test]
fn interrupted_attachment_publication_is_idempotently_reusable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project-recovery");
    let faulted = RecoveryStore::new(&root)
        .with_fault_injection_for_test(RecoveryStoreFaultPoint::AfterAttachmentPublication);
    let error = faulted
        .copy_attachment_bytes("evidence.txt", b"published before crash")
        .unwrap_err();
    assert!(matches!(
        error,
        RecoveryStoreError::InjectedFault(RecoveryStoreFaultPoint::AfterAttachmentPublication)
    ));

    let healthy = RecoveryStore::new(&root);
    let attachment = healthy
        .copy_attachment_bytes("evidence.txt", b"published before crash")
        .unwrap();
    assert_eq!(
        healthy
            .read_attachment_bytes(&attachment, MAX_RECOVERY_ATTACHMENT_BYTES)
            .unwrap(),
        b"published before crash"
    );
}
