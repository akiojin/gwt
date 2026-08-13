//! Phase 70 T-IDX-388/389 (Issue #3264): single batch search + typed search
//! contract.
//!
//! AS-2 / SC-043: a default 8-scope search must use one runner tree, one
//! model load, and one query encode — no per-scope process fan-out. FR-387:
//! healthy-but-stale scopes surface `stale_scopes` + `refresh_queued` on the
//! success payload. FR-388: missing / corrupt scopes that do not repair
//! within the wait window return a typed retryable `INDEX_NOT_READY`
//! failure (exit code 75), never a silent empty success.
//!
//! Phase 70d T-IDX-416/417 (SPEC #1939, bundled-required by SPEC #3170
//! FR-097): a NON-blocking search (`auto_build = false`, the GUI/Knowledge
//! Bridge path) on a missing / corrupt scope must queue exactly one
//! host-coordinated repair before returning the prompt typed
//! `INDEX_NOT_READY`, and concurrent callers must coalesce into a single
//! repair admission. Stale verified-result behavior stays unchanged.

#![cfg(unix)]

use std::{
    fs,
    path::Path,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
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
    _release_env: ScopedEnvVar,
    _descendant_env: ScopedEnvVar,
    repo: std::path::PathBuf,
    runner_log: std::path::PathBuf,
    /// Creating this file unblocks a fixture whose fake runner parks on a
    /// rebuild action (see `setup_search_fixture_with_blocking_rebuild`).
    release: std::path::PathBuf,
    /// Pid file written by the deadline-tree fake runner's descendant.
    descendant: std::path::PathBuf,
}

/// Fake runner script: records each invocation and answers with the
/// configured payload (also satisfies the runtime probes).
const FAKE_RUNNER_PASSTHROUGH: &str = "#!/bin/sh\n\
echo \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\n\
printf '%s\\n' \"$GWT_FAKE_RUNNER_PAYLOAD\"\n";

/// Fake runner script that additionally parks any `index-issues` rebuild
/// until the release file exists, so concurrent repair admissions stay
/// observable while the first coordinated job is still in flight.
const FAKE_RUNNER_BLOCKING_ISSUE_REBUILD: &str = "#!/bin/sh\n\
echo \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\n\
case \"$*\" in\n\
  *\"--action index-issues\"*)\n\
    while [ ! -f \"$GWT_FAKE_RUNNER_RELEASE\" ]; do sleep 0.05; done\n\
    ;;\n\
esac\n\
printf '%s\\n' \"$GWT_FAKE_RUNNER_PAYLOAD\"\n";

/// Fake runner whose `search-multi` attempt terminates without a structured
/// payload: stderr noise, non-JSON stdout, non-zero exit (T-IDX-418
/// unstructured transport case). Non-search actions (runtime provisioning
/// probes, pip) keep succeeding so only the search attempt is at fault.
const FAKE_RUNNER_UNSTRUCTURED_FAILURE: &str = "#!/bin/sh\n\
echo \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\n\
case \"$*\" in\n\
  *\"--action search-multi\"*)\n\
    echo 'model warmup 42%' >&2\n\
    echo 'this is not a structured payload'\n\
    exit 3\n\
    ;;\n\
esac\n\
printf '%s\\n' \"$GWT_FAKE_RUNNER_PAYLOAD\"\n";

/// Fake runner whose `search-multi` attempt fails with a structured stdout
/// diagnostic while stderr still carries progress noise (T-IDX-418
/// structured-output precedence). Non-search actions keep succeeding.
const FAKE_RUNNER_STRUCTURED_FAILURE: &str = "#!/bin/sh\n\
echo \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\n\
case \"$*\" in\n\
  *\"--action search-multi\"*)\n\
    echo 'loading model 42%' >&2\n\
    printf '%s\\n' '{\"ok\": false, \"error_code\": \"SEARCH_FAILED\", \"retryable\": false, \"error\": \"issues query failed: bad hnsw segment\", \"affected_scopes\": [\"issues\"]}'\n\
    exit 2\n\
    ;;\n\
esac\n\
printf '%s\\n' \"$GWT_FAKE_RUNNER_PAYLOAD\"\n";

/// Fake runner whose `search-multi` attempt spawns a descendant, records its
/// pid, then outlives any reasonable deadline (T-IDX-418 deadline + tree
/// reaping case). Non-search actions answer instantly with the payload.
// The descendant fork and its pid file come first: the deadline reaper races
// this script's cold start, and a reap that lands before the pid file exists
// starves `wait_for_pid_file` (observed as a 1-in-3 flake under load).
const FAKE_RUNNER_DEADLINE_TREE: &str = "#!/bin/sh\n\
case \"$*\" in\n\
  *\"--action search-multi\"*)\n\
    sleep 60 &\n\
    echo $! > \"$GWT_FAKE_RUNNER_DESCENDANT\"\n\
    echo \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\n\
    sleep 20\n\
    ;;\n\
  *)\n\
    echo \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\n\
    ;;\n\
esac\n\
printf '%s\\n' \"$GWT_FAKE_RUNNER_PAYLOAD\"\n";

fn setup_search_fixture(payload: &str) -> SearchFixture {
    setup_search_fixture_with_script(payload, FAKE_RUNNER_PASSTHROUGH)
}

fn setup_search_fixture_with_blocking_rebuild(payload: &str) -> SearchFixture {
    setup_search_fixture_with_script(payload, FAKE_RUNNER_BLOCKING_ISSUE_REBUILD)
}

fn setup_search_fixture_with_script(payload: &str, script: &str) -> SearchFixture {
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
    let release = tmp.path().join("runner-release.marker");
    let descendant = tmp.path().join("runner-descendant.pid");
    let log_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_LOG", &runner_log);
    let payload_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_PAYLOAD", payload);
    let release_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_RELEASE", &release);
    let descendant_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_DESCENDANT", &descendant);

    let python = gwt_core::runtime::project_index_python_path();
    fs::create_dir_all(python.parent().expect("python parent")).expect("create venv dir");
    fs::write(&python, script).expect("write fake python");
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
        _release_env: release_env,
        _descendant_env: descendant_env,
        repo,
        runner_log,
        release,
        descendant,
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

/// Runner invocations for one rebuild action (e.g. `--action index-issues`).
fn rebuild_invocations(log: &Path, action_marker: &str) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains(action_marker))
        .map(str::to_string)
        .collect()
}

/// Poll the runner log until at least `minimum` invocations of the rebuild
/// action appear; the queued repair runs on a detached coordinator thread,
/// so the log is the only observable completion signal.
fn wait_for_rebuild_invocations(
    log: &Path,
    action_marker: &str,
    minimum: usize,
    deadline: Duration,
) -> Vec<String> {
    let started = Instant::now();
    loop {
        let invocations = rebuild_invocations(log, action_marker);
        if invocations.len() >= minimum {
            return invocations;
        }
        assert!(
            started.elapsed() < deadline,
            "expected at least {minimum} queued coordinated repair invocation(s) \
             of {action_marker} within {deadline:?} (FR-097 repair admission \
             before typed not-ready), got {invocations:#?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
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
        other => panic!("expected typed INDEX_NOT_READY, got {other:?}"),
    }
    assert_eq!(INDEX_NOT_READY_EXIT_CODE, 75);
}

#[test]
fn healthy_query_failure_returns_typed_non_retryable_error_without_repair_wait() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = setup_search_fixture(
        r#"{"ok": false, "error_code": "SEARCH_FAILED", "retryable": false, "error": "issues query failed: query embedding rejected", "affected_scopes": ["issues"]}"#,
    );

    let error = gwt::search_project_index(
        &fixture.repo,
        "healthy query contract failure",
        &[IndexSearchScope::Issues],
        None,
        IndexSearchMatchMode::Semantic,
        true,
    )
    .expect_err("a healthy query failure must surface immediately");

    assert_eq!(error.error_code(), Some("SEARCH_FAILED"));
    assert!(!error.retryable());
    match error {
        IndexSearchError::SearchFailed(failed) => {
            assert_eq!(failed.affected_scopes, vec!["issues".to_string()]);
            assert!(failed.reason.contains("query embedding rejected"));
        }
        other => panic!("expected typed SEARCH_FAILED, got {other:?}"),
    }
    assert_eq!(
        search_invocations(&fixture.runner_log).len(),
        1,
        "healthy query failure must not enter status polling or repair wait"
    );
}

#[test]
fn non_blocking_missing_issue_scope_fails_promptly_and_queues_coordinated_repair() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture =
        setup_search_fixture(r#"{"ok": true, "scopes": {"issues": {"state": "missing"}}}"#);

    let started = Instant::now();
    let error = gwt::search_project_index(
        &fixture.repo,
        "missing issues scope",
        &[IndexSearchScope::Issues],
        None,
        IndexSearchMatchMode::Semantic,
        false,
    )
    .expect_err("a missing scope must fail typed, never silently succeed (FR-097)");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(5),
        "the non-blocking path must return promptly instead of joining the \
         repair wait window, took {elapsed:?}"
    );
    match error {
        IndexSearchError::NotReady(not_ready) => {
            assert!(
                not_ready
                    .affected_scopes
                    .iter()
                    .any(|scope| scope == "issues"),
                "affected scopes must name the missing issues scope: {not_ready:?}"
            );
            assert_eq!(
                not_ready.waited_ms, 0,
                "the non-blocking path must not report a repair wait: {not_ready:?}"
            );
            assert!(
                not_ready.retry_after_ms > 0,
                "a positive retry delay is mandatory: {not_ready:?}"
            );
        }
        other => panic!("expected typed INDEX_NOT_READY, got {other:?}"),
    }

    // T-IDX-416/417 (FR-097): the typed not-ready must be preceded by a
    // queued coordinated repair for the broken scope.
    wait_for_rebuild_invocations(
        &fixture.runner_log,
        "--action index-issues",
        1,
        Duration::from_secs(15),
    );
    // A single caller must admit exactly one repair job — never a fan-out.
    std::thread::sleep(Duration::from_millis(500));
    let repairs = rebuild_invocations(&fixture.runner_log, "--action index-issues");
    assert_eq!(
        repairs.len(),
        1,
        "one non-blocking search must queue exactly one coordinated repair: {repairs:#?}"
    );
}

#[test]
fn non_blocking_corrupt_spec_scope_fails_promptly_and_queues_coordinated_repair() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture =
        setup_search_fixture(r#"{"ok": true, "scopes": {"specs": {"state": "corrupt"}}}"#);

    let error = gwt::search_project_index(
        &fixture.repo,
        "corrupt specs scope",
        &[IndexSearchScope::Specs],
        None,
        IndexSearchMatchMode::Semantic,
        false,
    )
    .expect_err("a corrupt scope must fail typed, never silently succeed (FR-097)");

    match error {
        IndexSearchError::NotReady(not_ready) => {
            assert!(
                not_ready
                    .affected_scopes
                    .iter()
                    .any(|scope| scope == "specs"),
                "affected scopes must name the corrupt specs scope: {not_ready:?}"
            );
            assert!(
                not_ready.reason.contains("corrupt"),
                "the typed reason must carry the corrupt state: {not_ready:?}"
            );
            assert!(
                not_ready.retry_after_ms > 0,
                "a positive retry delay is mandatory: {not_ready:?}"
            );
        }
        other => panic!("expected typed INDEX_NOT_READY, got {other:?}"),
    }

    wait_for_rebuild_invocations(
        &fixture.runner_log,
        "--action index-specs",
        1,
        Duration::from_secs(15),
    );
}

#[test]
fn concurrent_non_blocking_searches_coalesce_into_one_repair_admission() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = setup_search_fixture_with_blocking_rebuild(
        r#"{"ok": true, "scopes": {"issues": {"state": "missing"}}}"#,
    );

    let handles: Vec<_> = (0..4)
        .map(|caller| {
            let repo = fixture.repo.clone();
            std::thread::spawn(move || {
                gwt::search_project_index(
                    &repo,
                    &format!("missing issues scope caller {caller}"),
                    &[IndexSearchScope::Issues],
                    None,
                    IndexSearchMatchMode::Semantic,
                    false,
                )
            })
        })
        .collect();
    for handle in handles {
        let error = handle
            .join()
            .expect("search caller thread")
            .expect_err("every concurrent caller gets the typed not-ready");
        assert!(
            matches!(error, IndexSearchError::NotReady(_)),
            "expected typed INDEX_NOT_READY, got {error:?}"
        );
    }

    // The first queued repair becomes the coordinator job owner and parks on
    // the blocked fake runner; every other caller's repair request must
    // coalesce into that in-flight job instead of admitting a second runner.
    wait_for_rebuild_invocations(
        &fixture.runner_log,
        "--action index-issues",
        1,
        Duration::from_secs(15),
    );
    // Give the remaining detached repair threads time to reach admission
    // while the owner is still parked, then release the runner.
    std::thread::sleep(Duration::from_secs(3));
    fs::write(&fixture.release, b"go").expect("release parked rebuild");
    std::thread::sleep(Duration::from_secs(2));

    let repairs = rebuild_invocations(&fixture.runner_log, "--action index-issues");
    assert_eq!(
        repairs.len(),
        1,
        "concurrent non-blocking searches must share exactly one \
         host-coordinated repair admission (FR-097): {repairs:#?}"
    );
}

#[test]
fn non_blocking_stale_scope_keeps_verified_results_and_refresh_contract() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = setup_search_fixture(
        r#"{"ok": true, "scopes": {"issues": {"state": "stale"}}, "stale_scopes": ["issues"], "scope_results": {"issues": {"issueResults": [{"number": 42, "title": "Search index", "state": "open"}]}}}"#,
    );

    let outcome = gwt::search_project_index(
        &fixture.repo,
        "stale issue lookup",
        &[IndexSearchScope::Issues],
        None,
        IndexSearchMatchMode::Semantic,
        false,
    )
    .expect("stale verified results must keep flowing on the non-blocking path");

    assert_eq!(
        outcome.results.len(),
        1,
        "verified stale results must be returned, not dropped: {outcome:?}"
    );
    assert_eq!(outcome.results[0].title, "#42 Search index");
    assert_eq!(
        outcome.stale_scopes,
        vec!["issues".to_string()],
        "stale scopes stay reported additively (FR-387)"
    );
    assert!(
        outcome.refresh_queued,
        "the single-flight stale refresh contract stays unchanged (FR-387)"
    );
}

fn assert_safe_public_unavailable(error: &IndexSearchError) {
    assert_eq!(
        error,
        &IndexSearchError::Other("project index search is temporarily unavailable".to_string())
    );
    assert_eq!(error.error_code(), None);
    assert!(!error.retryable());
}

#[test]
fn spawn_failure_maps_to_safe_legacy_public_error_without_repair() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = setup_search_fixture(r#"{"ok": true}"#);
    // Prime the runtime while the fake python is executable so the manifest
    // is current, then break only the spawn: the venv python still exists
    // (runtime checks stay satisfied) but can no longer execute (T-IDX-418
    // spawn failure case).
    gwt_core::runtime::ensure_project_index_runtime().expect("prime project index runtime");
    {
        use std::os::unix::fs::PermissionsExt;
        let python = gwt_core::runtime::project_index_python_path();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o644)).expect("chmod 644");
    }

    let error = gwt::search_project_index(
        &fixture.repo,
        "spawn failure query",
        &[IndexSearchScope::Issues],
        None,
        IndexSearchMatchMode::Semantic,
        false,
    )
    .expect_err("a spawn failure must surface through the legacy public error");

    assert_safe_public_unavailable(&error);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        rebuild_invocations(&fixture.runner_log, "--action index-").is_empty(),
        "an infrastructure failure must queue zero repair (repair is only \
         for classified missing/corrupt scopes)"
    );
}

#[test]
fn unstructured_runner_termination_maps_to_safe_legacy_public_error() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture =
        setup_search_fixture_with_script(r#"{"ok": true}"#, FAKE_RUNNER_UNSTRUCTURED_FAILURE);

    let error = gwt::search_project_index(
        &fixture.repo,
        "unstructured failure query",
        &[IndexSearchScope::Issues],
        None,
        IndexSearchMatchMode::Semantic,
        false,
    )
    .expect_err("an unstructured termination must surface through the public error");

    assert_safe_public_unavailable(&error);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        rebuild_invocations(&fixture.runner_log, "--action index-").is_empty(),
        "an unstructured termination must queue zero repair"
    );
}

#[test]
fn structured_stdout_failure_takes_precedence_over_stderr_progress() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture =
        setup_search_fixture_with_script(r#"{"ok": true}"#, FAKE_RUNNER_STRUCTURED_FAILURE);

    let error = gwt::search_project_index(
        &fixture.repo,
        "structured failure query",
        &[IndexSearchScope::Issues],
        None,
        IndexSearchMatchMode::Semantic,
        false,
    )
    .expect_err("a structured runner failure must surface as a typed error");

    assert_eq!(
        error.error_code(),
        Some("SEARCH_FAILED"),
        "the structured stdout diagnostic must win over stderr progress \
         (T-IDX-418 structured-output precedence): {error:?}"
    );
    assert!(
        !error.retryable(),
        "a healthy-store query failure is non-retryable: {error:?}"
    );
    match &error {
        IndexSearchError::SearchFailed(failed) => {
            assert!(
                failed.reason.contains("bad hnsw segment"),
                "the reason must come from structured stdout: {failed:?}"
            );
            assert!(
                !failed.reason.contains("loading model"),
                "stderr progress noise must not mask the structured \
                 diagnostic: {failed:?}"
            );
            assert_eq!(failed.affected_scopes, vec!["issues".to_string()]);
        }
        other => panic!("expected typed SEARCH_FAILED, got {other:?}"),
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        rebuild_invocations(&fixture.runner_log, "--action index-").is_empty(),
        "a healthy query failure must queue zero repair (FR-097)"
    );
}

#[test]
fn runner_deadline_expiry_maps_to_search_unavailable_and_reaps_process_tree() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = setup_search_fixture_with_script(r#"{"ok": true}"#, FAKE_RUNNER_DEADLINE_TREE);
    // Production default stays 30 seconds (FR-103); the test bounds it tight
    // but leaves the fake runner's cold start room to write the descendant
    // pid file first — 500ms lost that race about once in three under load.
    let _deadline_env = ScopedEnvVar::set("GWT_INDEX_SEARCH_RUNNER_DEADLINE_MS", "2000");

    let started = Instant::now();
    let error = gwt::search_project_index(
        &fixture.repo,
        "deadline expiry query",
        &[IndexSearchScope::Issues],
        None,
        IndexSearchMatchMode::Semantic,
        false,
    )
    .expect_err("a runner that outlives its deadline must surface as a public error");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(10),
        "the interactive attempt must stop at the deadline instead of \
         waiting for the runner (FR-103), took {elapsed:?}"
    );
    assert_safe_public_unavailable(&error);

    // FR-103: the complete process tree — including the descendant the
    // runner backgrounded — must be terminated and reaped by the deadline.
    let descendant = wait_for_pid_file(&fixture.descendant, Duration::from_secs(5));
    wait_for_process_exit(descendant, Duration::from_secs(5));
}

/// Poll for the descendant pid file the deadline-tree fake runner writes.
fn wait_for_pid_file(path: &Path, deadline: Duration) -> u32 {
    let started = Instant::now();
    loop {
        if let Some(pid) = fs::read_to_string(path)
            .ok()
            .and_then(|raw| raw.trim().parse::<u32>().ok())
        {
            return pid;
        }
        assert!(
            started.elapsed() < deadline,
            "descendant pid file was not written at {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Poll `kill -0` until the process is gone.
fn wait_for_process_exit(pid: u32, deadline: Duration) {
    let started = Instant::now();
    loop {
        let alive = gwt_core::process::hidden_command("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if !alive {
            return;
        }
        assert!(
            started.elapsed() < deadline,
            "process {pid} survived the deadline-driven tree termination"
        );
        std::thread::sleep(Duration::from_millis(50));
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
