//! Phase 8: integration tests for `gwt_core::index::runtime::reconcile_repo`.

use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use fs2::FileExt;
use gwt_core::{
    index::runtime::{
        reconcile_repo, remove_worktree_index, sweep_file_index_v2,
        sweep_file_index_v2_with_remover, FileIndexGcOptions, FileIndexGcPin, FileIndexGcPinKind,
        ReconcileOptions,
    },
    repo_hash::compute_repo_hash,
    worktree_hash::compute_worktree_hash,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

const NOW_NS: u64 = 2_000_000_000_000_000_000;
const HOUR_NS: u64 = 3_600_000_000_000;

#[derive(Clone)]
struct ViewClosure {
    view: PathBuf,
    base: PathBuf,
    overlay: PathBuf,
    view_id: String,
}

fn canonical_sha256(value: &Value) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}

fn compatibility_json() -> Value {
    serde_json::json!({
        "layout_version": 2,
        "index_schema_version": 1,
        "scope_set": ["files", "files-docs"],
        "model_id": "intfloat/multilingual-e5-base",
        "model_revision": "revision",
        "dimension": 768,
        "normalization": "none",
        "metric": "cosine",
        "query_prefix": "query: ",
        "passage_prefix": "passage: ",
        "document_contract": {
            "payload_builder_version": 1,
            "decode": "utf-8-replace",
            "content_limit": 2000
        },
        "path_policy_hash": "policy",
        "writer_protocol": "file-index-v2",
        "runner_hash": "runner"
    })
}

fn seed_view_closure(
    v2_root: &Path,
    repo_hash: &str,
    worktree_hash: &str,
    base_id: &str,
    overlay_id: &str,
    snapshot_id: &str,
) -> ViewClosure {
    let compatibility = compatibility_json();
    let mut semantic = compatibility.clone();
    semantic.as_object_mut().unwrap().remove("runner_hash");
    let identity = serde_json::json!({
        "schema_version": 1,
        "repo_hash": repo_hash,
        "worktree_hash": worktree_hash,
        "base_generation_id": base_id,
        "overlay_generation_id": overlay_id,
        "compatibility": semantic,
        "visible_counts": {"files": 1, "files-docs": 0},
        "source_snapshot_id": snapshot_id,
    });
    let view_id = canonical_sha256(&identity);
    let mut descriptor = serde_json::json!({
        "schema_version": 1,
        "view_id": view_id,
        "repo_hash": repo_hash,
        "worktree_hash": worktree_hash,
        "base_generation_id": base_id,
        "overlay_generation_id": overlay_id,
        "compatibility": compatibility,
        "visible_counts": {"files": 1, "files-docs": 0},
        "source_snapshot_id": snapshot_id,
        "verified_at": "2026-08-29T00:00:00+00:00",
    });
    descriptor["descriptor_checksum"] = Value::String(canonical_sha256(&descriptor));

    let base = v2_root.join("bases").join(base_id);
    let worktree_root = v2_root.join("worktrees").join(worktree_hash);
    let overlay = worktree_root.join("overlays").join(overlay_id);
    let view = worktree_root.join("views").join(&view_id);
    fs::create_dir_all(&base).unwrap();
    fs::create_dir_all(&overlay).unwrap();
    fs::create_dir_all(&view).unwrap();
    fs::write(base.join("sentinel"), b"base").unwrap();
    fs::write(overlay.join("sentinel"), b"overlay").unwrap();
    fs::write(
        view.join("descriptor.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    ViewClosure {
        view,
        base,
        overlay,
        view_id,
    }
}

fn write_head(path: &Path, active: &str, previous: Option<&str>, sequence: u64) {
    let mut head = serde_json::json!({
        "schema_version": 1,
        "active_view_id": active,
        "previous_view_id": previous,
        "sequence": sequence,
    });
    head["checksum"] = Value::String(canonical_sha256(&head));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(&head).unwrap()).unwrap();
}

fn gc_options(
    index_root: &Path,
    repo_hash: gwt_core::repo_hash::RepoHash,
    active_worktree_hashes: Vec<String>,
    now_unix_nanos: u64,
) -> FileIndexGcOptions {
    FileIndexGcOptions {
        index_root: index_root.to_path_buf(),
        repo_hash,
        active_worktree_hashes,
        now_unix_nanos,
        artifact_ttl: Duration::from_secs(60 * 60),
        worktree_grace: Duration::from_secs(60 * 60),
    }
}

#[test]
fn orphan_worktree_directory_is_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");

    let live_wt = tmp.path().join("live");
    fs::create_dir(&live_wt).unwrap();
    let live_hash = compute_worktree_hash(&live_wt).unwrap();

    let orphan_dir = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join("deadbeefdeadbeef");
    fs::create_dir_all(&orphan_dir).unwrap();
    fs::write(orphan_dir.join("manifest.json"), "[]").unwrap();

    let live_dir = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(live_hash.as_str());
    fs::create_dir_all(&live_dir).unwrap();

    let opts = ReconcileOptions {
        index_root,
        repo_hash: repo,
        active_worktree_paths: vec![live_wt],
        legacy_worktree_dirs: Vec::new(),
    };
    reconcile_repo(&opts).unwrap();

    assert!(!orphan_dir.exists(), "orphan dir should be removed");
    assert!(live_dir.exists(), "live dir must be preserved");
}

#[test]
fn legacy_dotgwt_index_directory_is_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let worktree = tmp.path().join("wt");
    fs::create_dir(&worktree).unwrap();
    let legacy = worktree.join(".gwt").join("index");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("dummy"), "data").unwrap();

    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let opts = ReconcileOptions {
        index_root: tmp.path().join("index"),
        repo_hash: repo,
        active_worktree_paths: vec![worktree.clone()],
        legacy_worktree_dirs: vec![worktree],
    };
    reconcile_repo(&opts).unwrap();

    assert!(
        !legacy.exists(),
        "legacy $WORKTREE/.gwt/index/ must be removed"
    );
}

#[test]
fn reconcile_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let opts = ReconcileOptions {
        index_root: tmp.path().join("index"),
        repo_hash: repo,
        active_worktree_paths: Vec::new(),
        legacy_worktree_dirs: Vec::new(),
    };
    reconcile_repo(&opts).unwrap();
    reconcile_repo(&opts).unwrap();
}

#[test]
fn legacy_worktree_scoped_specs_directory_is_removed_for_live_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");

    let live_wt = tmp.path().join("live");
    fs::create_dir(&live_wt).unwrap();
    let live_hash = compute_worktree_hash(&live_wt).unwrap();

    let legacy_specs = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(live_hash.as_str())
        .join("specs");
    fs::create_dir_all(&legacy_specs).unwrap();
    fs::write(legacy_specs.join("chroma.sqlite3"), "data").unwrap();
    let legacy_manifest = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(live_hash.as_str())
        .join("manifest-specs.json");
    fs::write(&legacy_manifest, "[]").unwrap();

    let live_files = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(live_hash.as_str())
        .join("files");
    fs::create_dir_all(&live_files).unwrap();

    let opts = ReconcileOptions {
        index_root,
        repo_hash: repo,
        active_worktree_paths: vec![live_wt],
        legacy_worktree_dirs: Vec::new(),
    };
    reconcile_repo(&opts).unwrap();

    assert!(
        !legacy_specs.exists(),
        "legacy worktree-scoped specs dir should be removed"
    );
    assert!(
        !legacy_manifest.exists(),
        "legacy worktree-scoped specs manifest should be removed"
    );
    assert!(live_files.exists(), "live files dir must be preserved");
}

#[test]
fn legacy_specs_cleanup_preserves_worktree_meta_file() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");

    let live_wt = tmp.path().join("live");
    fs::create_dir(&live_wt).unwrap();
    let live_hash = compute_worktree_hash(&live_wt).unwrap();

    let worktree_root = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(live_hash.as_str());
    fs::create_dir_all(worktree_root.join("specs")).unwrap();
    fs::write(worktree_root.join("specs").join("chroma.sqlite3"), "data").unwrap();
    fs::write(worktree_root.join("manifest-specs.json"), "[]").unwrap();
    fs::write(worktree_root.join("meta.json"), r#"{"schema_version":1}"#).unwrap();

    let opts = ReconcileOptions {
        index_root,
        repo_hash: repo,
        active_worktree_paths: vec![live_wt],
        legacy_worktree_dirs: Vec::new(),
    };
    reconcile_repo(&opts).unwrap();

    assert!(
        worktree_root.join("meta.json").exists(),
        "phase 6 worktree meta should survive legacy cleanup"
    );
    assert!(
        !worktree_root.join("specs").exists(),
        "legacy worktree-scoped specs dir should still be removed"
    );
    assert!(
        !worktree_root.join("manifest-specs.json").exists(),
        "legacy worktree-scoped specs manifest should still be removed"
    );
}

#[test]
fn remove_worktree_index_deletes_existing_worktree_scope_and_ignores_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let worktree_hash = "0123456789abcdef";
    let worktree_index = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(worktree_hash);
    fs::create_dir_all(&worktree_index).unwrap();
    fs::write(worktree_index.join("manifest-code.json"), "[]").unwrap();

    remove_worktree_index(&index_root, &repo, worktree_hash).unwrap();
    assert!(!worktree_index.exists());

    remove_worktree_index(&index_root, &repo, worktree_hash).unwrap();
}

#[test]
fn v2_gc_preserves_active_previous_and_journal_head_closures() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let v2_root = index_root.join(repo.as_str()).join("file-index-v2");
    let worktree_hash = "111122223333ffff";
    let active = seed_view_closure(
        &v2_root,
        repo.as_str(),
        worktree_hash,
        "base-active",
        "overlay-active",
        "snapshot-active",
    );
    let previous = seed_view_closure(
        &v2_root,
        repo.as_str(),
        worktree_hash,
        "base-previous",
        "overlay-previous",
        "snapshot-previous",
    );
    let journal = seed_view_closure(
        &v2_root,
        repo.as_str(),
        worktree_hash,
        "base-journal",
        "overlay-journal",
        "snapshot-journal",
    );
    let journal_previous = seed_view_closure(
        &v2_root,
        repo.as_str(),
        worktree_hash,
        "base-journal-previous",
        "overlay-journal-previous",
        "snapshot-journal-previous",
    );
    let worktree_root = v2_root.join("worktrees").join(worktree_hash);
    write_head(
        &worktree_root.join("head.json"),
        &active.view_id,
        Some(&previous.view_id),
        3,
    );
    write_head(
        &worktree_root.join("head.previous.json"),
        &journal.view_id,
        Some(&journal_previous.view_id),
        2,
    );
    let old_staging = v2_root
        .join("bases")
        .join(format!(".unused.staging-{}-1", NOW_NS - 2 * HOUR_NS));
    let old_quarantine = worktree_root
        .join("overlays")
        .join(format!(".unused.quarantine-{}-1", NOW_NS - 2 * HOUR_NS));
    fs::create_dir_all(&old_staging).unwrap();
    fs::create_dir_all(&old_quarantine).unwrap();

    let report = sweep_file_index_v2(&gc_options(
        &index_root,
        repo,
        vec![worktree_hash.to_string()],
        NOW_NS,
    ))
    .unwrap();

    for closure in [&active, &previous, &journal, &journal_previous] {
        assert!(closure.view.is_dir(), "head-rooted View was deleted");
        assert!(closure.base.is_dir(), "head-rooted Base was deleted");
        assert!(closure.overlay.is_dir(), "head-rooted Overlay was deleted");
        assert!(!report.deleted.contains(&closure.view));
        assert!(!report.deleted.contains(&closure.base));
        assert!(!report.deleted.contains(&closure.overlay));
    }
    assert!(
        !old_staging.exists(),
        "expired unpinned staging must be swept"
    );
    assert!(
        !old_quarantine.exists(),
        "expired unpinned quarantine must be swept"
    );
}

#[test]
fn live_migration_and_continuation_pins_override_artifact_ttl() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let v2_root = index_root.join(repo.as_str()).join("file-index-v2");
    let worktree_hash = "111122223333ffff";
    let migration_staging = v2_root
        .join("bases")
        .join(format!(".migration.staging-{}-1", NOW_NS - 2 * HOUR_NS));
    let continuation_staging = v2_root
        .join("cas")
        .join(format!(".continuation.staging-{}-1", NOW_NS - 2 * HOUR_NS));
    let unpinned_staging = v2_root
        .join("bases")
        .join(format!(".unpinned.staging-{}-1", NOW_NS - 2 * HOUR_NS));
    for path in [&migration_staging, &continuation_staging, &unpinned_staging] {
        fs::create_dir_all(path).unwrap();
    }
    let migration = FileIndexGcPin::acquire(
        &v2_root,
        FileIndexGcPinKind::Migration,
        repo.as_str(),
        Some(worktree_hash),
        vec![migration_staging
            .strip_prefix(&v2_root)
            .unwrap()
            .to_path_buf()],
    )
    .unwrap();
    let continuation = FileIndexGcPin::acquire(
        &v2_root,
        FileIndexGcPinKind::Continuation,
        repo.as_str(),
        Some(worktree_hash),
        vec![continuation_staging
            .strip_prefix(&v2_root)
            .unwrap()
            .to_path_buf()],
    )
    .unwrap();

    sweep_file_index_v2(&gc_options(&index_root, repo.clone(), vec![], NOW_NS)).unwrap();
    assert!(migration_staging.is_dir());
    assert!(continuation_staging.is_dir());
    assert!(!unpinned_staging.exists());

    drop(migration);
    drop(continuation);
    sweep_file_index_v2(&gc_options(&index_root, repo, vec![], NOW_NS)).unwrap();
    assert!(!migration_staging.exists());
    assert!(!continuation_staging.exists());
}

#[test]
fn rust_gc_honors_the_python_pin_file_schema_and_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let v2_root = index_root.join(repo.as_str()).join("file-index-v2");
    let worktree_hash = "111122223333ffff";
    let protected_base = v2_root
        .join("bases")
        .join(format!(".python.staging-{}-1", NOW_NS - 2 * HOUR_NS));
    let protected_cas = v2_root
        .join("cas")
        .join(format!(".python.staging-{}-1", NOW_NS - 2 * HOUR_NS));
    let protected_worktree = v2_root
        .join("worktrees")
        .join(worktree_hash)
        .join("views")
        .join(format!(".python.staging-{}-1", NOW_NS - 2 * HOUR_NS));
    for path in [&protected_base, &protected_cas, &protected_worktree] {
        fs::create_dir_all(path).unwrap();
    }

    let pin_root = v2_root.join("leases").join("python-reader");
    fs::create_dir_all(&pin_root).unwrap();
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(pin_root.join(".lock"))
        .unwrap();
    lock.lock_shared().unwrap();
    let mut marker = serde_json::json!({
        "schema_version": 1,
        "pin_id": "python-reader",
        "kind": "reader",
        "repo_hash": repo.as_str(),
        "worktree_hash": worktree_hash,
        "protected_paths": ["bases", "cas", format!("worktrees/{worktree_hash}")],
        "owner_pid": 123,
        "created_at": "2026-08-29T00:00:00+00:00",
    });
    marker["checksum"] = Value::String(canonical_sha256(&marker));
    fs::write(
        pin_root.join("pin.json"),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();

    let options = gc_options(&index_root, repo, vec![], NOW_NS);
    sweep_file_index_v2(&options).unwrap();
    for path in [&protected_base, &protected_cas, &protected_worktree] {
        assert!(
            path.is_dir(),
            "Rust GC must honor each broad Python lease root while its shared lock is live"
        );
    }

    FileExt::unlock(&lock).unwrap();
    drop(lock);
    let after_grace = gc_options(
        &index_root,
        compute_repo_hash("https://github.com/akiojin/gwt.git"),
        vec![],
        NOW_NS + HOUR_NS + 1,
    );
    sweep_file_index_v2(&after_grace).unwrap();
    for path in [&protected_base, &protected_cas, &protected_worktree] {
        assert!(
            !path.exists(),
            "an unlocked Python lease must not pin the next eligible sweep"
        );
    }
}

#[test]
fn removed_worktree_waits_for_grace_and_reader_pin_but_keeps_shared_base() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let v2_root = index_root.join(repo.as_str()).join("file-index-v2");
    let removed_hash = "111122223333ffff";
    let live_hash = "aaaabbbbccccdddd";
    let removed = seed_view_closure(
        &v2_root,
        repo.as_str(),
        removed_hash,
        "shared-base",
        "removed-overlay",
        "removed-snapshot",
    );
    let live = seed_view_closure(
        &v2_root,
        repo.as_str(),
        live_hash,
        "shared-base",
        "live-overlay",
        "live-snapshot",
    );
    let reader_only = seed_view_closure(
        &v2_root,
        repo.as_str(),
        removed_hash,
        "reader-base",
        "reader-overlay",
        "reader-snapshot",
    );
    let reader_cas = v2_root.join("cas").join("reader-cas");
    fs::create_dir_all(&reader_cas).unwrap();
    fs::write(reader_cas.join("sentinel"), b"cas").unwrap();
    write_head(
        &v2_root
            .join("worktrees")
            .join(removed_hash)
            .join("head.json"),
        &removed.view_id,
        None,
        1,
    );
    write_head(
        &v2_root.join("worktrees").join(live_hash).join("head.json"),
        &live.view_id,
        None,
        1,
    );
    let live_legacy = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(live_hash)
        .join("files");
    fs::create_dir_all(&live_legacy).unwrap();
    fs::write(live_legacy.join("sentinel"), b"legacy").unwrap();

    let first = gc_options(
        &index_root,
        repo.clone(),
        vec![live_hash.to_string()],
        NOW_NS,
    );
    sweep_file_index_v2(&first).unwrap();
    assert!(removed.view.is_dir(), "first absence only starts grace");
    let before_grace = gc_options(
        &index_root,
        repo.clone(),
        vec![live_hash.to_string()],
        NOW_NS + HOUR_NS - 1,
    );
    sweep_file_index_v2(&before_grace).unwrap();
    assert!(removed.view.is_dir(), "worktree grace is inclusive");

    let reader = FileIndexGcPin::acquire(
        &v2_root,
        FileIndexGcPinKind::Reader,
        repo.as_str(),
        Some(removed_hash),
        vec![
            removed.view.strip_prefix(&v2_root).unwrap().to_path_buf(),
            reader_only
                .view
                .strip_prefix(&v2_root)
                .unwrap()
                .to_path_buf(),
            reader_only
                .overlay
                .strip_prefix(&v2_root)
                .unwrap()
                .to_path_buf(),
            reader_only
                .base
                .strip_prefix(&v2_root)
                .unwrap()
                .to_path_buf(),
            reader_cas.strip_prefix(&v2_root).unwrap().to_path_buf(),
        ],
    )
    .unwrap();
    let after_grace = gc_options(
        &index_root,
        repo.clone(),
        vec![live_hash.to_string()],
        NOW_NS + HOUR_NS + 1,
    );
    sweep_file_index_v2(&after_grace).unwrap();
    assert!(
        removed.view.is_dir(),
        "live reader pin overrides worktree grace"
    );
    assert!(reader_only.view.is_dir(), "reader View must stay reachable");
    assert!(
        reader_only.overlay.is_dir(),
        "reader Overlay must stay reachable"
    );
    assert!(reader_only.base.is_dir(), "reader Base must stay reachable");
    assert!(reader_cas.is_dir(), "reader CAS must stay reachable");

    drop(reader);
    sweep_file_index_v2(&after_grace).unwrap();
    assert!(!removed.view.exists());
    assert!(!removed.overlay.exists());
    assert!(live.view.is_dir());
    assert!(live.overlay.is_dir());
    assert!(
        live.base.is_dir(),
        "shared Base is rooted by the live worktree"
    );
    assert!(
        live_legacy.is_dir(),
        "live legacy index needs an explicit purge gate"
    );
}

#[test]
fn gc_deletes_only_expired_unreferenced_staging_and_quarantine() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let v2_root = index_root.join(repo.as_str()).join("file-index-v2");
    let old_staging = v2_root
        .join("bases")
        .join(format!(".old.staging-{}-7", NOW_NS - 2 * HOUR_NS));
    let fresh_staging = v2_root
        .join("bases")
        .join(format!(".fresh.staging-{}-7", NOW_NS - HOUR_NS + 1));
    let old_quarantine = v2_root
        .join("worktrees/wt/overlays")
        .join(format!(".old.quarantine-{}-7", NOW_NS - 2 * HOUR_NS));
    let fresh_quarantine = v2_root
        .join("worktrees/wt/overlays")
        .join(format!(".fresh.quarantine-{}-7", NOW_NS - HOUR_NS + 1));
    for path in [
        &old_staging,
        &fresh_staging,
        &old_quarantine,
        &fresh_quarantine,
    ] {
        fs::create_dir_all(path).unwrap();
    }

    sweep_file_index_v2(&gc_options(&index_root, repo, vec![], NOW_NS)).unwrap();

    assert!(!old_staging.exists());
    assert!(!old_quarantine.exists());
    assert!(fresh_staging.is_dir());
    assert!(fresh_quarantine.is_dir());
}

#[test]
fn delete_failure_is_left_for_the_next_sweep() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let v2_root = index_root.join(repo.as_str()).join("file-index-v2");
    let quarantine = v2_root
        .join("worktrees/wt/views")
        .join(format!(".broken.quarantine-{}-9", NOW_NS - 2 * HOUR_NS));
    fs::create_dir_all(&quarantine).unwrap();
    let options = gc_options(&index_root, repo, vec![], NOW_NS);
    let mut deny_once = true;

    let first = sweep_file_index_v2_with_remover(&options, |path| {
        if path == quarantine && deny_once {
            deny_once = false;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "simulated Windows sharing violation",
            ));
        }
        fs::remove_dir_all(path)
    })
    .unwrap();
    assert!(quarantine.is_dir());
    assert_eq!(first.retry_pending, vec![quarantine.clone()]);

    let second = sweep_file_index_v2_with_remover(&options, fs::remove_dir_all).unwrap();
    assert!(!quarantine.exists());
    assert!(second.retry_pending.is_empty());
    assert!(second.deleted.contains(&quarantine));
}
