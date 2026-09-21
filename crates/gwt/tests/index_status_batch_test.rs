//! Phase 71 T-IDX-434 (Issue #3772): read-only index status projection.
//!
//! SPEC #1939 AS-32 / FR-416: every status entrypoint projects health from
//! disk without ensuring the runtime or starting a Python runner.

#![cfg(unix)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use gwt_core::test_support::ScopedEnvVar;
use serde_json::Value;
use sha2::{Digest, Sha256};

type ByteSnapshot = BTreeMap<PathBuf, Option<Vec<u8>>>;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn every_status_entrypoint_is_model_free_and_has_zero_side_effects() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("create home");
    let _home = ScopedEnvVar::set("HOME", &home);
    let _userprofile = ScopedEnvVar::set("USERPROFILE", &home);
    // The contract must exercise the real repository/disk projection path.
    // GWT_INDEX_TEST_FIXTURE would bypass the behavior under test.
    let _fixture = ScopedEnvVar::unset("GWT_INDEX_TEST_FIXTURE");
    let _batch_limit = ScopedEnvVar::unset("GWT_INDEX_STATUS_WORKTREE_BATCH_LIMIT");

    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    add_origin(&repo, "https://github.com/example/project.git");
    commit_file(&repo, "README.md", "# repo\n");
    let wt_a = tmp.path().join("wt-a");
    let wt_b = tmp.path().join("wt-b");
    let wt_c = tmp.path().join("wt-c");
    add_worktree(&repo, &wt_a, "feature/a");
    add_worktree(&repo, &wt_b, "feature/b");
    add_worktree(&repo, &wt_c, "feature/c");
    let repo_hash = gwt_core::repo_hash::detect_repo_hash(&repo).expect("repo hash");
    let hash_repo = worktree_hash(&repo);
    let hash_a = worktree_hash(&wt_a);
    let hash_b = worktree_hash(&wt_b);
    let hash_c = worktree_hash(&wt_c);

    // Preserve the existing executable fake-runner seam, but record every
    // argv rather than merely counting `--action status`. Read-only status
    // must leave this log byte-empty for all actions, including probes.
    let runner_log = tmp.path().join("runner-log.txt");
    fs::write(&runner_log, b"").expect("create runner log");
    let _log_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_LOG", &runner_log);
    let _payload_env = ScopedEnvVar::set(
        "GWT_FAKE_RUNNER_PAYLOAD",
        r#"{"ok": true, "runtime": {"healthy": true}, "status": {}, "worktrees": {}}"#,
    );
    let python = gwt_core::runtime::project_index_python_path();
    fs::create_dir_all(python.parent().expect("python parent")).expect("create venv dir");
    fs::write(
        &python,
        "#!/bin/sh\necho \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\nprintf '%s\\n' \"$GWT_FAKE_RUNNER_PAYLOAD\"\n",
    )
    .expect("write fake python");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    // Sentinels make accidental runtime/index recreation, installation, or
    // reconciliation observable as a path-or-byte change.
    let runtime_sentinel = home.join(".gwt/runtime/status-contract.bin");
    let index_sentinel = home.join(".gwt/index/status-contract.bin");
    fs::create_dir_all(runtime_sentinel.parent().expect("runtime parent"))
        .expect("create runtime root");
    fs::create_dir_all(index_sentinel.parent().expect("index parent")).expect("create index root");
    fs::write(&runtime_sentinel, b"runtime-before-status\0\xff").expect("write runtime sentinel");
    fs::write(&index_sentinel, b"index-before-status\0\xfe").expect("write index sentinel");
    let corrupt_head = home
        .join(".gwt/index")
        .join(repo_hash.as_str())
        .join("file-index-v2/worktrees")
        .join(&hash_a)
        .join("head.json");
    fs::create_dir_all(corrupt_head.parent().expect("corrupt head parent"))
        .expect("create corrupt view fixture");
    fs::write(&corrupt_head, b"{not-json").expect("write corrupt view head");
    let valid_view_id = seed_valid_file_index_view(
        &home
            .join(".gwt/index")
            .join(repo_hash.as_str())
            .join("file-index-v2"),
        repo_hash.as_str(),
        &hash_b,
        3,
        2,
    );
    let corrupt_component_view_id = seed_valid_file_index_view(
        &home
            .join(".gwt/index")
            .join(repo_hash.as_str())
            .join("file-index-v2"),
        repo_hash.as_str(),
        &hash_c,
        3,
        2,
    );
    let corrupt_component_root = home
        .join(".gwt/index")
        .join(repo_hash.as_str())
        .join("file-index-v2/worktrees")
        .join(&hash_c);
    let view_descriptor: Value = serde_json::from_slice(
        &fs::read(
            corrupt_component_root
                .join("views")
                .join(&corrupt_component_view_id)
                .join("descriptor.json"),
        )
        .expect("read component-corrupt view descriptor"),
    )
    .expect("parse component-corrupt view descriptor");
    let overlay_id = view_descriptor["overlay_generation_id"]
        .as_str()
        .expect("overlay generation id");
    let overlay_descriptor_path = corrupt_component_root
        .join("overlays")
        .join(overlay_id)
        .join("descriptor.json");
    let mut overlay_descriptor: Value = serde_json::from_slice(
        &fs::read(&overlay_descriptor_path).expect("read overlay descriptor"),
    )
    .expect("parse overlay descriptor");
    overlay_descriptor["base_generation_id"] = Value::String("wrong-base".to_string());
    fs::write(
        &overlay_descriptor_path,
        serde_json::to_vec(&overlay_descriptor).expect("serialize corrupt overlay descriptor"),
    )
    .expect("write corrupt overlay descriptor");
    for (worktree, marker) in [
        (&repo, b"repo".as_slice()),
        (&wt_a, b"a"),
        (&wt_b, b"b"),
        (&wt_c, b"c"),
    ] {
        let legacy_index = worktree.join(".gwt/index");
        fs::create_dir_all(&legacy_index).expect("create legacy worktree index");
        fs::write(legacy_index.join("status-contract.bin"), marker)
            .expect("write legacy worktree sentinel");
    }
    let before = status_surface_byte_snapshot(&home, [&repo, &wt_a, &wt_b, &wt_c]);

    let project = gwt::index_worker::project_index_status_for_path(&wt_a);
    let current = gwt::aggregate_current_worktree_index_status_for_path(&wt_a);
    let project_ready = gwt::index_worker::project_index_status_for_path(&wt_b);
    let current_ready = gwt::aggregate_current_worktree_index_status_for_path(&wt_b);
    let project_corrupt_component = gwt::index_worker::project_index_status_for_path(&wt_c);
    let current_corrupt_component = gwt::aggregate_current_worktree_index_status_for_path(&wt_c);
    let all = gwt::aggregate_project_index_status_for_path(&repo);

    let runner_argv = fs::read(&runner_log).expect("read runner log");
    assert!(
        runner_argv.is_empty(),
        "status/probe entrypoints must start zero runner processes and emit zero argv, got:\n{}",
        String::from_utf8_lossy(&runner_argv)
    );
    assert_eq!(
        status_surface_byte_snapshot(&home, [&repo, &wt_a, &wt_b, &wt_c]),
        before,
        "status/probe must not change any path or byte below isolated HOME runtime/index",
    );

    assert_model_free_projection(
        "project status",
        &project,
        gwt::index_worker::ProjectIndexStatusCoverageScope::CurrentWorktree,
        &[(&hash_a, wt_a.as_path())],
        4,
        &[(&hash_a, "view_head_invalid")],
        None,
    );
    assert_model_free_projection(
        "current-worktree aggregate",
        &current,
        gwt::index_worker::ProjectIndexStatusCoverageScope::CurrentWorktree,
        &[(&hash_a, wt_a.as_path())],
        4,
        &[(&hash_a, "view_head_invalid")],
        None,
    );
    assert_model_free_projection(
        "project status with valid view",
        &project_ready,
        gwt::index_worker::ProjectIndexStatusCoverageScope::CurrentWorktree,
        &[(&hash_b, wt_b.as_path())],
        4,
        &[],
        Some((&hash_b, &valid_view_id)),
    );
    assert_model_free_projection(
        "current-worktree aggregate with valid view",
        &current_ready,
        gwt::index_worker::ProjectIndexStatusCoverageScope::CurrentWorktree,
        &[(&hash_b, wt_b.as_path())],
        4,
        &[],
        Some((&hash_b, &valid_view_id)),
    );
    assert_model_free_projection(
        "project status with corrupt component",
        &project_corrupt_component,
        gwt::index_worker::ProjectIndexStatusCoverageScope::CurrentWorktree,
        &[(&hash_c, wt_c.as_path())],
        4,
        &[(&hash_c, "view_closure_invalid")],
        None,
    );
    assert_model_free_projection(
        "current-worktree aggregate with corrupt component",
        &current_corrupt_component,
        gwt::index_worker::ProjectIndexStatusCoverageScope::CurrentWorktree,
        &[(&hash_c, wt_c.as_path())],
        4,
        &[(&hash_c, "view_closure_invalid")],
        None,
    );
    assert_model_free_projection(
        "all-worktree aggregate",
        &all,
        gwt::index_worker::ProjectIndexStatusCoverageScope::AllWorktrees,
        &[
            (&hash_repo, repo.as_path()),
            (&hash_a, wt_a.as_path()),
            (&hash_b, wt_b.as_path()),
            (&hash_c, wt_c.as_path()),
        ],
        4,
        &[
            (&hash_a, "view_head_invalid"),
            (&hash_c, "view_closure_invalid"),
        ],
        Some((&hash_b, &valid_view_id)),
    );
}

#[test]
fn status_on_a_clean_home_does_not_create_runtime_or_index_roots() {
    use std::os::unix::fs::PermissionsExt;

    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("read-only-home");
    fs::create_dir_all(&home).expect("create clean home");
    let _home = ScopedEnvVar::set("HOME", &home);
    let _userprofile = ScopedEnvVar::set("USERPROFILE", &home);
    let _fixture = ScopedEnvVar::unset("GWT_INDEX_TEST_FIXTURE");

    let repo = tmp.path().join("repo-clean-home");
    init_git_repo(&repo);
    add_origin(&repo, "https://github.com/example/clean-home.git");
    commit_file(&repo, "README.md", "# clean home\n");

    fs::set_permissions(&home, fs::Permissions::from_mode(0o555)).expect("lock clean home");
    let project = gwt::index_worker::project_index_status_for_path(&repo);
    let current = gwt::aggregate_current_worktree_index_status_for_path(&repo);
    let all = gwt::aggregate_project_index_status_for_path(&repo);
    fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).expect("unlock clean home");

    for (entrypoint, view) in [("project", project), ("current", current), ("all", all)] {
        assert_eq!(
            view.state,
            gwt::ProjectIndexStatusState::RepairRequired,
            "{entrypoint} must project missing disk state without runtime setup: {view:?}"
        );
    }
    assert!(
        !home.join(".gwt").exists(),
        "read-only status must not create a runtime or index root in a clean HOME"
    );
}

fn assert_model_free_projection(
    entrypoint: &str,
    view: &gwt::ProjectIndexStatusView,
    expected_scope: gwt::index_worker::ProjectIndexStatusCoverageScope,
    expected_worktrees: &[(&str, &Path)],
    total_worktrees: usize,
    corrupt_worktrees: &[(&str, &str)],
    ready_worktree: Option<(&str, &str)>,
) {
    // The isolated index has no repo artifacts, so a successful disk-only
    // projection reports repair required; Error/Skipped would hide the
    // model-free health result behind an execution failure.
    assert_eq!(
        view.state,
        gwt::ProjectIndexStatusState::RepairRequired,
        "{entrypoint} must return the missing-on-disk projection: {view:?}",
    );
    let coverage = view
        .coverage
        .as_ref()
        .unwrap_or_else(|| panic!("{entrypoint} must report projection coverage: {view:?}"));
    assert_eq!(coverage.scope, expected_scope, "{entrypoint}: {view:?}");
    assert_eq!(
        coverage.probed_worktrees,
        expected_worktrees.len(),
        "{entrypoint}: {view:?}",
    );
    assert_eq!(
        coverage.total_worktrees, total_worktrees,
        "{entrypoint}: {view:?}",
    );
    assert!(!coverage.truncated, "{entrypoint}: {view:?}");
    assert_eq!(
        view.worktrees.len(),
        expected_worktrees.len(),
        "{entrypoint} must project exactly the covered worktrees: {view:?}",
    );
    for (hash, expected_path) in expected_worktrees {
        let meta = view
            .worktrees
            .get(*hash)
            .unwrap_or_else(|| panic!("{entrypoint} omitted covered worktree {hash}: {view:?}"));
        let actual_path =
            dunce::canonicalize(&meta.path).unwrap_or_else(|_| meta.path.clone().into());
        let expected_path =
            dunce::canonicalize(expected_path).unwrap_or_else(|_| (*expected_path).to_path_buf());
        assert_eq!(actual_path, expected_path, "{entrypoint}: {meta:?}");
        assert!(
            !meta.branch.is_empty(),
            "{entrypoint} must project a branch label for {hash}: {meta:?}"
        );
        for (scope, health) in [
            ("files", view.scopes.files.get(*hash)),
            ("files-docs", view.scopes.files_docs.get(*hash)),
        ] {
            let health = health.unwrap_or_else(|| {
                panic!("{entrypoint} omitted {scope} health for {hash}: {view:?}")
            });
            if let Some((_, expected_reason)) = corrupt_worktrees
                .iter()
                .find(|(corrupt_hash, _)| *hash == *corrupt_hash)
            {
                assert!(
                    !health.healthy
                        && health.repair_required
                        && health.reason == *expected_reason
                        && health.view_id.is_none(),
                    "{entrypoint} must project the corrupt v2 {scope} closure for {hash}: {health:?}"
                );
            } else if ready_worktree.is_some_and(|(ready_hash, _)| *hash == ready_hash) {
                let (_, expected_view_id) = ready_worktree.expect("ready worktree tuple");
                let expected_count = if scope == "files" { 3 } else { 2 };
                assert!(
                    health.healthy
                        && !health.repair_required
                        && health.reason == "ready"
                        && health.document_count == expected_count
                        && health.view_id.as_deref() == Some(expected_view_id),
                    "{entrypoint} must project the validated v2 {scope} view for {hash}: {health:?}"
                );
            } else {
                assert!(
                    !health.healthy && health.repair_required && !health.reason.is_empty(),
                    "{entrypoint} must project missing {scope} health for {hash}: {health:?}"
                );
            }
        }
    }
    for (scope, health) in [
        ("issues", view.scopes.issues.as_ref()),
        ("specs", view.scopes.specs.as_ref()),
        ("memory", view.scopes.memory.as_ref()),
        ("discussions", view.scopes.discussions.as_ref()),
        ("board", view.scopes.board.as_ref()),
    ] {
        let health = health
            .unwrap_or_else(|| panic!("{entrypoint} omitted repo-shared {scope} health: {view:?}"));
        assert!(
            !health.healthy && health.repair_required && !health.reason.is_empty(),
            "{entrypoint} must project missing repo-shared {scope} health: {health:?}"
        );
    }
}

fn canonical_sha256(value: &Value) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}

fn seed_valid_file_index_view(
    v2_root: &Path,
    repo_hash: &str,
    worktree_hash: &str,
    files: usize,
    files_docs: usize,
) -> String {
    assert_eq!((files, files_docs), (3, 2), "fixture manifest shape");
    let gwt_crate = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let runner_path = gwt_crate.join("../gwt-core/runtime/chroma_index_runner.py");
    let policy_path = gwt_crate.join("../gwt-core/runtime/index_path_policy.json");
    let policy: Value = serde_json::from_slice(&fs::read(policy_path).expect("read path policy"))
        .expect("parse path policy");
    let compatibility = serde_json::json!({
        "layout_version": 2,
        "index_schema_version": 1,
        "scope_set": ["files", "files-docs"],
        "model_id": "intfloat/multilingual-e5-base",
        "model_revision": "d128750597153bb5987e10b1c3493a34e5a4502a",
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
        "path_policy_hash": canonical_sha256(&policy),
        "writer_protocol": "file-index-v2",
        "runner_hash": format!("{:x}", Sha256::digest(fs::read(runner_path).expect("read runner")))
    });
    let mut semantic_compatibility = compatibility.clone();
    semantic_compatibility
        .as_object_mut()
        .expect("compatibility object")
        .remove("runner_hash");
    let manifest_entries = [
        ("README.md", "files-docs"),
        ("docs/guide.md", "files-docs"),
        ("src/lib.rs", "files"),
        ("src/main.rs", "files"),
        ("src/status.rs", "files"),
    ]
    .into_iter()
    .map(|(path, scope)| {
        serde_json::json!({
            "path": path,
            "source_object": format!("object:{path}"),
            "source_digest": sha256_text(&format!("source:{path}")),
            "scope": scope,
            "payload_digest": sha256_text(&format!("payload:{path}")),
            "metadata_digest": sha256_text(&format!("metadata:{path}")),
            "cas_key": sha256_text(&format!("cas:{path}")),
            "input_digest": sha256_text(&format!("input:{path}")),
            "vector_checksum": sha256_text(&format!("vector:{path}")),
            "dimension": 768,
        })
    })
    .collect::<Vec<_>>();
    let manifest = serde_json::json!({"entries": manifest_entries, "schema_version": 1});
    let manifest_digest = canonical_sha256(&manifest["entries"]);
    let record_keys = manifest["entries"]
        .as_array()
        .expect("manifest entries")
        .iter()
        .map(|entry| serde_json::json!([entry["path"], entry["cas_key"]]))
        .collect::<Vec<_>>();
    let base_id = canonical_sha256(&serde_json::json!({
        "compatibility": semantic_compatibility,
        "records": record_keys,
        "root_tree_oid": null,
    }));
    let source_snapshot_id = canonical_sha256(&Value::Array(
        manifest["entries"]
            .as_array()
            .expect("manifest entries")
            .iter()
            .map(|entry| {
                serde_json::json!([
                    entry["path"],
                    entry["source_digest"],
                    if entry["scope"] == "files" {
                        "code"
                    } else {
                        "docs"
                    },
                    entry["payload_digest"],
                    entry["metadata_digest"],
                ])
            })
            .collect(),
    ));
    let overlay_id = canonical_sha256(&serde_json::json!({
        "compatibility": semantic_compatibility,
        "records": [],
        "source_snapshot_id": source_snapshot_id,
        "base_generation_id": base_id,
    }));
    let identity = serde_json::json!({
        "schema_version": 1,
        "repo_hash": repo_hash,
        "worktree_hash": worktree_hash,
        "base_generation_id": base_id,
        "overlay_generation_id": overlay_id,
        "compatibility": semantic_compatibility,
        "visible_counts": {"files": files, "files-docs": files_docs},
        "source_snapshot_id": source_snapshot_id,
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
        "visible_counts": {"files": files, "files-docs": files_docs},
        "source_snapshot_id": source_snapshot_id,
        "verified_at": "2026-08-29T00:00:00+00:00",
    });
    descriptor["descriptor_checksum"] = Value::String(canonical_sha256(&descriptor));

    let base_root = v2_root.join("bases").join(&base_id);
    let worktree_root = v2_root.join("worktrees").join(worktree_hash);
    let overlay_root = worktree_root.join("overlays").join(&overlay_id);
    let view_root = worktree_root.join("views").join(&view_id);
    for root in [&base_root, &overlay_root] {
        fs::create_dir_all(root.join("store")).expect("create generation store");
        fs::write(root.join("store/chroma.sqlite3"), b"SQLite format 3\0")
            .expect("write immutable store marker");
    }
    let base_descriptor = serde_json::json!({
        "schema_version": 1,
        "kind": "base",
        "base_generation_id": base_id,
        "repo_hash": repo_hash,
        "root_tree_oid": null,
        "canonical_ref": null,
        "compatibility": compatibility,
        "files_generation": {"store": "store", "collection": "files_code"},
        "files_docs_generation": {"store": "store", "collection": "files_docs"},
        "manifest_digest": manifest_digest,
        "document_counts": {"files": files, "files-docs": files_docs, "total": files + files_docs},
        "build_state": "verified",
        "created_at": "2026-08-29T00:00:00+00:00",
        "verified_at": "2026-08-29T00:00:00+00:00",
    });
    let overlay_manifest = serde_json::json!({"entries": [], "schema_version": 1});
    let overlay_descriptor = serde_json::json!({
        "schema_version": 1,
        "kind": "overlay",
        "overlay_generation_id": overlay_id,
        "repo_hash": repo_hash,
        "worktree_hash": worktree_hash,
        "base_generation_id": base_id,
        "source_snapshot_id": source_snapshot_id,
        "compatibility": compatibility,
        "files_generation": {"store": "store", "collection": "files_code"},
        "files_docs_generation": {"store": "store", "collection": "files_docs"},
        "files_shadow": [],
        "files_docs_shadow": [],
        "tombstones": [],
        "manifest_digest": canonical_sha256(&overlay_manifest["entries"]),
        "build_state": "verified",
        "created_at": "2026-08-29T00:00:00+00:00",
        "verified_at": "2026-08-29T00:00:00+00:00",
    });
    for (root, artifact_descriptor, artifact_manifest) in [
        (&base_root, &base_descriptor, &manifest),
        (&overlay_root, &overlay_descriptor, &overlay_manifest),
    ] {
        fs::write(
            root.join("descriptor.json"),
            serde_json::to_vec(artifact_descriptor).expect("serialize generation descriptor"),
        )
        .expect("write generation descriptor");
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(artifact_manifest).expect("serialize generation manifest"),
        )
        .expect("write generation manifest");
    }
    fs::create_dir_all(&view_root).expect("create view directory");
    fs::write(
        view_root.join("descriptor.json"),
        serde_json::to_vec(&descriptor).expect("serialize view descriptor"),
    )
    .expect("write view descriptor");
    let mut head = serde_json::json!({
        "schema_version": 1,
        "active_view_id": view_id,
        "previous_view_id": null,
        "sequence": 1,
    });
    head["checksum"] = Value::String(canonical_sha256(&head));
    fs::write(
        worktree_root.join("head.json"),
        serde_json::to_vec(&head).expect("serialize view head"),
    )
    .expect("write view head");
    view_id
}

fn sha256_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn status_surface_byte_snapshot<'a>(
    home: &Path,
    worktrees: impl IntoIterator<Item = &'a PathBuf>,
) -> ByteSnapshot {
    let mut snapshot = BTreeMap::new();
    for relative in [Path::new(".gwt/runtime"), Path::new(".gwt/index")] {
        collect_byte_snapshot(&home.join(relative), &mut snapshot);
    }
    for worktree in worktrees {
        let legacy_index = worktree.join(".gwt/index");
        collect_byte_snapshot(&legacy_index, &mut snapshot);
    }
    snapshot
}

fn collect_byte_snapshot(path: &Path, snapshot: &mut ByteSnapshot) {
    if path.is_dir() {
        snapshot.insert(path.to_path_buf(), None);
        let entries = fs::read_dir(path)
            .unwrap_or_else(|error| panic!("read snapshot directory {}: {error}", path.display()));
        for entry in entries {
            let entry = entry.expect("read snapshot entry");
            collect_byte_snapshot(&entry.path(), snapshot);
        }
    } else {
        let bytes = fs::read(path)
            .unwrap_or_else(|error| panic!("read snapshot file {}: {error}", path.display()));
        snapshot.insert(path.to_path_buf(), Some(bytes));
    }
}

fn worktree_hash(path: &Path) -> String {
    gwt_core::worktree_hash::compute_worktree_hash(path)
        .expect("worktree hash")
        .to_string()
}

fn init_git_repo(path: &Path) {
    let output = gwt_core::process::hidden_command("git")
        .args(["init", path.to_str().unwrap()])
        .output()
        .expect("git init");
    assert!(output.status.success(), "git init failed");
    for (key, value) in [
        ("user.email", "test@example.com"),
        ("user.name", "Test User"),
    ] {
        let output = gwt_core::process::hidden_command("git")
            .args(["config", key, value])
            .current_dir(path)
            .output()
            .expect("git config");
        assert!(output.status.success(), "git config {key} failed");
    }
}

fn add_origin(path: &Path, url: &str) {
    let output = gwt_core::process::hidden_command("git")
        .args(["remote", "add", "origin", url])
        .current_dir(path)
        .output()
        .expect("git remote add origin");
    assert!(output.status.success(), "git remote add origin failed");
}

fn commit_file(path: &Path, name: &str, body: &str) {
    fs::write(path.join(name), body).expect("write commit file");
    let add = gwt_core::process::hidden_command("git")
        .args(["add", name])
        .current_dir(path)
        .output()
        .expect("git add");
    assert!(add.status.success(), "git add failed");
    let commit = gwt_core::process::hidden_command("git")
        .args(["commit", "-m", "init"])
        .current_dir(path)
        .output()
        .expect("git commit");
    assert!(commit.status.success(), "git commit failed");
}

fn add_worktree(repo: &Path, worktree: &Path, branch: &str) {
    let output = gwt_core::process::hidden_command("git")
        .args(["worktree", "add", "-b", branch, worktree.to_str().unwrap()])
        .current_dir(repo)
        .output()
        .expect("git worktree add");
    assert!(
        output.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
