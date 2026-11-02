//! Phase 8: end-to-end test that spawns the real Python runner against the
//! real `intfloat/multilingual-e5-base` embedding model.
//!
//! This test is `#[ignore]` by default because it downloads the model on first
//! run (~440 MB) and runs much slower than unit tests. CI invokes it with
//! `cargo test -- --ignored` after restoring the HuggingFace cache.

use std::{fs, path::PathBuf};

use gwt_core::{repo_hash::compute_repo_hash, worktree_hash::compute_worktree_hash};

fn runner_python() -> PathBuf {
    gwt_core::runtime::project_index_python_path()
}

fn runner_script() -> PathBuf {
    gwt_core::runtime::project_index_runner_path()
}

fn assert_runtime_bootstrapped() {
    assert!(
        runner_python().is_file(),
        "runner Python must be bootstrapped at {}",
        runner_python().display()
    );
    assert!(
        runner_script().is_file(),
        "runner script must be bootstrapped at {}",
        runner_script().display()
    );
}

#[test]
#[ignore]
fn search_files_e2e_with_real_e5_auto_builds() {
    assert_runtime_bootstrapped();

    let tmp = tempfile::tempdir().unwrap();
    let repo_root = tmp.path().join("repo");
    fs::create_dir_all(repo_root.join("src")).unwrap();
    fs::write(
        repo_root.join("src/watcher.rs"),
        "//! filesystem watcher with debounce semantics\nfn main() {}\n",
    )
    .unwrap();
    fs::write(
        repo_root.join("src/lib.rs"),
        "//! library entrypoint\npub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();
    fs::write(repo_root.join("README.md"), "# project\n").unwrap();

    let repo = compute_repo_hash("https://github.com/example/test.git");
    let wt = compute_worktree_hash(&repo_root).unwrap();

    // Force a temporary HOME so the test does not pollute the user's index.
    let fake_home = tmp.path().join("fake_home");
    fs::create_dir_all(&fake_home).unwrap();

    let output = gwt_core::process::hidden_command(runner_python())
        .arg(runner_script())
        .arg("--action")
        .arg("search-files")
        .arg("--repo-hash")
        .arg(repo.as_str())
        .arg("--worktree-hash")
        .arg(wt.as_str())
        .arg("--project-root")
        .arg(&repo_root)
        .arg("--query")
        .arg("watcher debounce")
        .arg("--n-results")
        .arg("3")
        .env("HOME", &fake_home)
        .env("USERPROFILE", &fake_home)
        .env("GWT_INDEX_FAKE_EMBEDDING", "0")
        .output()
        .expect("runner spawn must succeed");

    assert!(
        output.status.success(),
        "runner exit={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"ok\": true") || stdout.contains("\"ok\":true"));
    assert!(
        stdout.contains("watcher.rs"),
        "expected watcher.rs in results, got: {stdout}"
    );
}

#[test]
#[ignore]
fn search_specs_e2e_with_real_e5_auto_builds() {
    assert_runtime_bootstrapped();

    let tmp = tempfile::tempdir().unwrap();
    let repo_root = tmp.path().join("repo");
    fs::create_dir_all(&repo_root).unwrap();

    let repo = compute_repo_hash("https://github.com/example/test.git");
    let fake_home = tmp.path().join("fake_home");
    fs::create_dir_all(&fake_home).unwrap();
    let cache_dir = fake_home
        .join(".gwt/cache/issues")
        .join(repo.as_str())
        .join("1939");
    fs::create_dir_all(cache_dir.join("sections")).unwrap();
    fs::write(
        cache_dir.join("meta.json"),
        r#"{"number":1939,"title":"gwt-spec: Watcher SPEC","labels":["gwt-spec","phase/review"],"state":"open","updated_at":"2026-04-14T00:00:00Z","comment_ids":[]}"#,
    )
    .unwrap();
    fs::write(
        cache_dir.join("body.md"),
        "<!-- gwt-spec id=1939 version=1 -->\nWatcher SPEC\n",
    )
    .unwrap();
    fs::write(
        cache_dir.join("sections/spec.md"),
        "# Watcher SPEC\nFilesystem watcher debounce semantics for indexing.\n",
    )
    .unwrap();

    let output = gwt_core::process::hidden_command(runner_python())
        .arg(runner_script())
        .arg("--action")
        .arg("search-specs")
        .arg("--repo-hash")
        .arg(repo.as_str())
        .arg("--project-root")
        .arg(&repo_root)
        .arg("--query")
        .arg("watcher debounce")
        .arg("--n-results")
        .arg("3")
        .env("HOME", &fake_home)
        .env("USERPROFILE", &fake_home)
        .env("GWT_INDEX_FAKE_EMBEDDING", "0")
        .output()
        .expect("runner spawn must succeed");

    assert!(
        output.status.success(),
        "runner exit={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload: serde_json::Value = serde_json::from_str(&stdout).expect("runner JSON response");
    assert_eq!(payload["ok"], true, "runner response: {stdout}");
    assert!(
        payload["specResults"]
            .as_array()
            .is_some_and(|results| results.iter().any(|result| result["spec_id"] == "1939")),
        "expected SPEC 1939 in results, got: {stdout}"
    );
}
