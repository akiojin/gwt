//! Phase 70 T-IDX-388/389 (Issue #3264): single batch search + typed search
//! contract.
//!
//! AS-2 / SC-043: a default 8-scope search must use one runner tree, one
//! model load, and one query encode — no per-scope process fan-out. FR-387:
//! healthy-but-stale scopes surface `stale_scopes` + `refresh_queued` on the
//! success payload. FR-388: missing / corrupt scopes that do not repair
//! within the wait window return a typed retryable `INDEX_NOT_READY`
//! failure (exit code 75), never a silent empty success.

#![cfg(unix)]

use std::{
    fs,
    path::Path,
    sync::{Mutex, OnceLock},
};

use gwt::index_search::{IndexSearchError, INDEX_NOT_READY_EXIT_CODE};
use gwt::protocol::{IndexSearchMatchMode, IndexSearchScope};
use gwt_core::test_support::ScopedEnvVar;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct SearchFixture {
    _tmp: tempfile::TempDir,
    _home: ScopedEnvVar,
    _userprofile: ScopedEnvVar,
    _log_env: ScopedEnvVar,
    _payload_env: ScopedEnvVar,
    repo: std::path::PathBuf,
    runner_log: std::path::PathBuf,
}

fn setup_search_fixture(payload: &str) -> SearchFixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("create home");
    let home_env = ScopedEnvVar::set("HOME", &home);
    let userprofile_env = ScopedEnvVar::set("USERPROFILE", &home);

    let repo = tmp.path().join("repo");
    init_git_repo(&repo);
    add_origin(&repo, "https://github.com/example/project.git");
    commit_file(&repo, "README.md", "# repo\n");

    let runner_log = tmp.path().join("runner-log.txt");
    let log_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_LOG", &runner_log);
    let payload_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_PAYLOAD", payload);

    // Fake runner python: records each invocation and answers with the
    // configured payload (also satisfies the runtime probes).
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

    SearchFixture {
        _tmp: tmp,
        _home: home_env,
        _userprofile: userprofile_env,
        _log_env: log_env,
        _payload_env: payload_env,
        repo,
        runner_log,
    }
}

fn search_invocations(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("--action search"))
        .map(str::to_string)
        .collect()
}

#[test]
fn default_eight_scope_search_uses_one_batch_runner_process() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = setup_search_fixture(r#"{"ok": true}"#);

    let outcome = gwt::search_project_index(
        &fixture.repo,
        "coordinator design",
        &[],
        None,
        IndexSearchMatchMode::Semantic,
        true,
    )
    .expect("batch search succeeds");
    assert!(outcome.results.is_empty());

    let invocations = search_invocations(&fixture.runner_log);
    assert_eq!(
        invocations.len(),
        1,
        "default 8-scope search must spawn exactly one runner process \
         (one model load / one query encode), got {invocations:#?}"
    );
    let call = &invocations[0];
    assert!(
        call.contains("--action search-multi"),
        "batch search must use the versioned search-multi action: {call}"
    );
    for scope in [
        "issues",
        "specs",
        "memory",
        "discussions",
        "board",
        "works",
        "files-docs",
        "files",
    ] {
        assert!(
            call.contains(scope),
            "batch search must cover scope {scope}: {call}"
        );
    }
    assert!(
        call.contains("--worktree-hash"),
        "file scopes require the worktree hash in the batch request: {call}"
    );
}

#[test]
fn stale_scopes_surface_on_success_payload_with_refresh_marker() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = setup_search_fixture(
        r#"{"ok": true, "scopes": {"issues": {"state": "stale"}}, "stale_scopes": ["issues"]}"#,
    );

    let outcome = gwt::search_project_index(
        &fixture.repo,
        "stale issue lookup",
        &[IndexSearchScope::Issues],
        None,
        IndexSearchMatchMode::Semantic,
        true,
    )
    .expect("stale scopes still return verified results");

    assert_eq!(
        outcome.stale_scopes,
        vec!["issues".to_string()],
        "healthy-but-stale scopes must be reported additively (FR-387)"
    );
    assert!(
        outcome.refresh_queued,
        "a single-flight refresh must be queued for stale scopes (FR-387)"
    );
}

#[test]
fn missing_scope_returns_typed_not_ready_instead_of_silent_empty_success() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture =
        setup_search_fixture(r#"{"ok": true, "scopes": {"files": {"state": "missing"}}}"#);
    // Keep the repair wait short for the test; production default is 30s
    // (FR-388).
    let _wait_env = ScopedEnvVar::set("GWT_INDEX_SEARCH_REPAIR_WAIT_MS", "200");

    let error = gwt::search_project_index(
        &fixture.repo,
        "missing files scope",
        &[IndexSearchScope::Files],
        None,
        IndexSearchMatchMode::Semantic,
        true,
    )
    .expect_err("missing scope must not degrade into a silent empty success");

    match error {
        IndexSearchError::NotReady(not_ready) => {
            assert!(
                not_ready
                    .affected_scopes
                    .iter()
                    .any(|scope| scope == "files"),
                "affected scopes must name the missing scope: {not_ready:?}"
            );
            assert!(
                not_ready.waited_ms >= 200,
                "the caller must have waited for repair before failing: {not_ready:?}"
            );
            assert!(
                not_ready.retry_after_ms > 0,
                "retry information is mandatory: {not_ready:?}"
            );
        }
        IndexSearchError::Other(other) => {
            panic!("expected typed INDEX_NOT_READY, got untyped error: {other}")
        }
    }
    assert_eq!(INDEX_NOT_READY_EXIT_CODE, 75);
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
