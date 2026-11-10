//! Existing durable launch-recovery receipt regressions through the shared library.

use std::path::{Path, PathBuf};

use gwt::session_launch::recovery::{
    durable_launch_recovery_records, DurableLaunchRecoveryKind, DurableLaunchRecoveryRecord,
    DURABLE_LAUNCH_RECOVERY_SCHEMA_VERSION,
};
use gwt::session_launch::{durable_launch_recovery_dir, durable_launch_recovery_path};

fn recovery_fixture(root: &Path) -> (PathBuf, PathBuf, Vec<u8>) {
    let repo = root.join("repo");
    init_repo(&repo);
    let sessions = root.join("sessions");
    let record = DurableLaunchRecoveryRecord {
        schema_version: DURABLE_LAUNCH_RECOVERY_SCHEMA_VERSION,
        kind: DurableLaunchRecoveryKind::Genesis,
        session_id: "archive-test".to_string(),
        project_root: repo.clone(),
        worktree_path: repo,
        repo_hash: "different-repository".to_string(),
        owner_kind: "issue".to_string(),
        owner_number: 4803,
        expected_binding: None,
        expected_agent_id: None,
        expected_session_identity: None,
    };
    let path = durable_launch_recovery_path(&sessions, &record.session_id).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = serde_json::to_vec_pretty(&record).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    (sessions, path, bytes)
}

#[test]
fn recovery_repository_mismatch_archives_original_receipt_once() {
    let temp = tempfile::tempdir().unwrap();
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let (sessions, path, bytes) = recovery_fixture(temp.path());
    assert!(durable_launch_recovery_records(&sessions).is_empty());
    assert!(!path.exists());
    let archive = durable_launch_recovery_dir(&sessions).join("archive");
    let files: Vec<_> = std::fs::read_dir(&archive)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read(files[0].path()).unwrap(), bytes);
    assert!(durable_launch_recovery_records(&sessions).is_empty());
    assert_eq!(std::fs::read_dir(archive).unwrap().count(), 1);
}

#[test]
fn recovery_repository_unavailable_retains_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let (sessions, path, _) = recovery_fixture(temp.path());
    let mut record: DurableLaunchRecoveryRecord =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record.worktree_path = temp.path().join("unavailable");
    let bytes = serde_json::to_vec_pretty(&record).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert!(durable_launch_recovery_records(&sessions).is_empty());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert!(!durable_launch_recovery_dir(&sessions)
        .join("archive")
        .exists());
}

#[test]
fn recovery_repository_archive_failure_retains_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
    let (sessions, path, bytes) = recovery_fixture(temp.path());
    std::fs::write(
        durable_launch_recovery_dir(&sessions).join("archive"),
        b"blocked",
    )
    .unwrap();
    assert!(durable_launch_recovery_records(&sessions).is_empty());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

fn init_repo(path: &Path) {
    std::fs::create_dir_all(path).expect("create repository directory");
    let status = gwt_core::process::hidden_command("git")
        .args(["init", "--quiet"])
        .current_dir(path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init failed");
    let status = gwt_core::process::hidden_command("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://example.com/owner/binding-repair.git",
        ])
        .current_dir(path)
        .status()
        .expect("git remote add origin");
    assert!(status.success(), "git remote add origin failed");
}
