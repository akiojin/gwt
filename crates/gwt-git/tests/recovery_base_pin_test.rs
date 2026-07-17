use std::{fs, path::Path};

use gwt_core::process::hidden_command;
use gwt_git::recovery::{
    ensure_recovery_base_pin, recovery_base_ref_name, recreate_missing_intake_worktree,
    remove_recovery_base_pin, verify_recovery_base_pin, verify_recovery_intake_worktree,
};

fn git(repo: &Path, args: &[&str]) -> String {
    let output = hidden_command("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8 git output")
        .trim()
        .to_string()
}

fn init_repo(root: &Path) -> (String, String) {
    fs::create_dir_all(root).expect("repo dir");
    git(root, &["init", "-q", "-b", "develop"]);
    git(root, &["config", "user.name", "Gwt Test"]);
    git(root, &["config", "user.email", "gwt@example.com"]);
    fs::write(root.join("README.md"), "first\n").expect("first fixture");
    git(root, &["add", "README.md"]);
    git(root, &["commit", "-qm", "first"]);
    let first = git(root, &["rev-parse", "HEAD"]);
    fs::write(root.join("README.md"), "second\n").expect("second fixture");
    git(root, &["add", "README.md"]);
    git(root, &["commit", "-qm", "second"]);
    let second = git(root, &["rev-parse", "HEAD"]);
    (first, second)
}

#[test]
fn recovery_base_pin_is_idempotent_and_rejects_conflicting_oid_or_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let (first, second) = init_repo(&repo);
    let recovery_id = "intake-safe_01";

    let reference = ensure_recovery_base_pin(&repo, recovery_id, &first).expect("create pin");
    assert_eq!(reference, recovery_base_ref_name(recovery_id).unwrap());
    assert_eq!(git(&repo, &["rev-parse", &reference]), first);
    assert_eq!(
        ensure_recovery_base_pin(&repo, recovery_id, &first).expect("repeat pin"),
        reference
    );
    assert!(
        ensure_recovery_base_pin(&repo, recovery_id, &second).is_err(),
        "an existing gwt recovery ref must never move to another OID"
    );
    for invalid in ["../escape", "a/b", ".", "", "id.lock"] {
        assert!(
            recovery_base_ref_name(invalid).is_err(),
            "invalid id {invalid:?}"
        );
    }
}

#[test]
fn missing_intake_worktree_is_recreated_at_the_pinned_commit_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let (first, _) = init_repo(&repo);
    let recovery_id = "intake-recreate";
    ensure_recovery_base_pin(&repo, recovery_id, &first).expect("pin base");
    let target = temp.path().join(".intake-5");

    recreate_missing_intake_worktree(&repo, &target, recovery_id, &first)
        .expect("recreate detached Intake");
    recreate_missing_intake_worktree(&repo, &target, recovery_id, &first)
        .expect("repeat recreation adopts the already-verified result");

    assert!(target.is_dir());
    assert_eq!(git(&target, &["rev-parse", "HEAD"]), first);
    assert!(git(&target, &["branch", "--show-current"]).is_empty());
    verify_recovery_intake_worktree(&repo, &target, recovery_id, &first)
        .expect("verify recreated Intake");
    assert_eq!(
        verify_recovery_base_pin(&repo, recovery_id, &first).expect("verify pin"),
        recovery_base_ref_name(recovery_id).unwrap()
    );
}

#[test]
fn existing_intake_verification_rejects_an_unrelated_repository_collision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let (first, _) = init_repo(&repo);
    let recovery_id = "existing-collision";
    ensure_recovery_base_pin(&repo, recovery_id, &first).expect("pin base");

    let collision = temp.path().join(".intake-6");
    let _ = init_repo(&collision);
    assert!(
        verify_recovery_intake_worktree(&repo, &collision, recovery_id, &first).is_err(),
        "a repository merely occupying the expected path must never be adopted"
    );
    assert!(collision.join(".git").is_dir());
}

#[test]
fn intake_recreation_fails_closed_for_unpinned_mismatch_and_path_collisions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let (first, second) = init_repo(&repo);

    let unpinned = temp.path().join(".intake-2");
    assert!(recreate_missing_intake_worktree(&repo, &unpinned, "unpinned", &first).is_err());
    assert!(!unpinned.exists());

    ensure_recovery_base_pin(&repo, "mismatch", &first).expect("pin first");
    let mismatch = temp.path().join(".intake-3");
    assert!(recreate_missing_intake_worktree(&repo, &mismatch, "mismatch", &second).is_err());
    assert!(!mismatch.exists());

    ensure_recovery_base_pin(&repo, "collision", &first).expect("pin collision");
    let collision = temp.path().join(".intake-4");
    fs::create_dir_all(&collision).expect("collision dir");
    fs::write(collision.join("keep.txt"), "user data\n").expect("collision data");
    assert!(recreate_missing_intake_worktree(&repo, &collision, "collision", &first).is_err());
    assert_eq!(
        fs::read_to_string(collision.join("keep.txt")).unwrap(),
        "user data\n"
    );

    let outside = temp.path().join("nested").join(".intake-5");
    fs::create_dir_all(outside.parent().unwrap()).expect("outside parent");
    assert!(recreate_missing_intake_worktree(&repo, &outside, "collision", &first).is_err());
    assert!(!outside.exists());

    let execution_like = temp.path().join("work").join("20260716-0100");
    fs::create_dir_all(execution_like.parent().unwrap()).expect("work parent");
    assert!(recreate_missing_intake_worktree(&repo, &execution_like, "collision", &first).is_err());
    assert!(!execution_like.exists());
}

#[test]
fn recovery_base_pin_cleanup_is_idempotent_and_never_deletes_a_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    let (first, second) = init_repo(&repo);
    let recovery_id = "cleanup";
    ensure_recovery_base_pin(&repo, recovery_id, &first).expect("pin first");

    assert!(remove_recovery_base_pin(&repo, recovery_id, &second).is_err());
    verify_recovery_base_pin(&repo, recovery_id, &first).expect("mismatched cleanup kept pin");
    remove_recovery_base_pin(&repo, recovery_id, &first).expect("remove pin");
    remove_recovery_base_pin(&repo, recovery_id, &first).expect("repeat cleanup");
    assert!(verify_recovery_base_pin(&repo, recovery_id, &first).is_err());
}
