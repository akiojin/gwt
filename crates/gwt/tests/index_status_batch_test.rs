//! Phase 70 T-IDX-394/395 (Issue #3264): all-worktree status batching.
//!
//! FR-393 / AS-13: the explicit all-worktree health aggregation runs ONE
//! batch runner process covering every selected worktree instead of one
//! serial Python spawn per worktree (previously up to 32).

#![cfg(unix)]

use std::{
    fs,
    path::Path,
    sync::{Mutex, OnceLock},
};

use gwt_core::test_support::ScopedEnvVar;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn all_worktree_status_uses_one_batch_runner_process() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("create home");
    let _home = ScopedEnvVar::set("HOME", &home);
    let _userprofile = ScopedEnvVar::set("USERPROFILE", &home);

    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    add_origin(&repo, "https://github.com/example/project.git");
    commit_file(&repo, "README.md", "# repo\n");
    let wt_a = tmp.path().join("wt-a");
    let wt_b = tmp.path().join("wt-b");
    add_worktree(&repo, &wt_a, "feature/a");
    add_worktree(&repo, &wt_b, "feature/b");

    let runner_log = tmp.path().join("runner-log.txt");
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

    let view = gwt::aggregate_project_index_status_for_path(&repo);
    // The fake payload reports no scopes; only the process accounting is
    // under test here.
    let _ = view;

    let status_calls: Vec<String> = fs::read_to_string(&runner_log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("--action status"))
        .map(str::to_string)
        .collect();
    assert_eq!(
        status_calls.len(),
        1,
        "all-worktree status must run one batch process, got {status_calls:#?}"
    );
    let call = &status_calls[0];
    assert!(
        call.contains("--worktree-hashes"),
        "batch status must pass every selected worktree hash: {call}"
    );
    let hash_a = gwt_core::worktree_hash::compute_worktree_hash(&wt_a)
        .expect("hash a")
        .to_string();
    let hash_b = gwt_core::worktree_hash::compute_worktree_hash(&wt_b)
        .expect("hash b")
        .to_string();
    for hash in [&hash_a, &hash_b] {
        assert!(
            call.contains(hash.as_str()),
            "batch status must cover worktree {hash}: {call}"
        );
    }
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

#[test]
fn batch_status_merges_worktree_scopes_and_survives_runner_failure() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("create home");
    let _home = ScopedEnvVar::set("HOME", &home);
    let _userprofile = ScopedEnvVar::set("USERPROFILE", &home);

    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    add_origin(&repo, "https://github.com/example/project.git");
    commit_file(&repo, "README.md", "# repo\n");
    let hash = gwt_core::worktree_hash::compute_worktree_hash(&repo)
        .expect("hash")
        .to_string();

    let runner_log = tmp.path().join("runner-log.txt");
    let _log_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_LOG", &runner_log);
    // Per-worktree files status merges over the repo-shared scopes.
    let payload = format!(
        r#"{{"ok": true, "runtime": {{"healthy": true}}, "status": {{"issues": {{"healthy": true, "exists": true, "repair_required": false, "document_count": 1, "reason": "ready"}}}}, "worktrees": {{"{hash}": {{"files": {{"healthy": true, "exists": true, "repair_required": false, "document_count": 2, "reason": "ready"}}}}}}}}"#
    );
    let _payload_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_PAYLOAD", &payload);
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

    let view = gwt::aggregate_project_index_status_for_path(&repo);
    assert!(
        view.worktrees.contains_key(&hash),
        "merged worktree status must reach the aggregated view: {view:?}"
    );

    // A failing batch runner degrades every probed worktree, never panics,
    // and invalidates the probe cache (FR-393).
    // Only the status action fails; probes stay healthy so the runtime
    // ensure path does not rebuild a real venv.
    fs::write(
        &python,
        "#!/bin/sh\necho \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\ncase \"$*\" in *\"--action status\"*) echo broken >&2; exit 3;; *) printf '{\"ok\": true}\\n';; esac\n",
    )
    .expect("write failing python");
    gwt::global_aggregated_status_cache().invalidate(&repo);
    let view = gwt::aggregate_project_index_status_for_path(&repo);
    assert!(
        matches!(
            view.state,
            gwt::ProjectIndexStatusState::Error | gwt::ProjectIndexStatusState::RepairRequired
        ),
        "failed batch status must surface an error state: {view:?}"
    );
}
