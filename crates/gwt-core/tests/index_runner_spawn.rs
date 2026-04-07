//! Phase 8: end-to-end test that spawns the real Python runner against the
//! real `intfloat/multilingual-e5-base` embedding model.
//!
//! This test is `#[ignore]` by default because it downloads the model on first
//! run (~440 MB) and runs much slower than unit tests. CI invokes it with
//! `cargo test -- --ignored` after restoring the HuggingFace cache.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use gwt_core::repo_hash::compute_repo_hash;
use gwt_core::worktree_hash::compute_worktree_hash;

fn runner_python() -> PathBuf {
    let home = dirs::home_dir().expect("home");
    if cfg!(windows) {
        home.join(".gwt/runtime/chroma-venv/Scripts/python.exe")
    } else {
        home.join(".gwt/runtime/chroma-venv/bin/python3")
    }
}

fn runner_script() -> PathBuf {
    let home = dirs::home_dir().expect("home");
    home.join(".gwt/runtime/chroma_index_runner.py")
}

#[test]
#[ignore]
fn search_files_e2e_with_real_e5_auto_builds() {
    if !runner_python().exists() || !runner_script().exists() {
        eprintln!("skipping: runner not bootstrapped");
        return;
    }

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

    let output = Command::new(runner_python())
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
    if !runner_python().exists() || !runner_script().exists() {
        eprintln!("skipping: runner not bootstrapped");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let repo_root = tmp.path().join("repo");
    let spec_dir = repo_root.join("specs/SPEC-1");
    fs::create_dir_all(&spec_dir).unwrap();
    fs::write(
        spec_dir.join("spec.md"),
        "# Watcher SPEC\nFilesystem watcher debounce semantics for indexing.\n",
    )
    .unwrap();
    fs::write(
        spec_dir.join("metadata.json"),
        r#"{"id":"1","title":"Watcher SPEC","status":"open","phase":"draft"}"#,
    )
    .unwrap();

    let repo = compute_repo_hash("https://github.com/example/test.git");
    let wt = compute_worktree_hash(&repo_root).unwrap();
    let fake_home = tmp.path().join("fake_home");
    fs::create_dir_all(&fake_home).unwrap();

    let output = Command::new(runner_python())
        .arg(runner_script())
        .arg("--action")
        .arg("search-specs")
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
        .output()
        .expect("runner spawn must succeed");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Watcher SPEC") || stdout.contains("SPEC-1"));
}
