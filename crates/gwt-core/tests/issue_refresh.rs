//! Phase 8: integration tests for `gwt_core::index::runtime::refresh_issues_if_stale`.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gwt_core::{
    index::runtime::{
        refresh_issues_if_stale, PythonRunnerSpawner, RefreshIssuesOptions, RunnerSpawner,
    },
    repo_hash::compute_repo_hash,
};

/// A test double that records calls instead of actually spawning the python runner.
#[derive(Clone, Default)]
struct RecordingSpawner {
    calls: Arc<Mutex<Vec<String>>>,
}

impl RunnerSpawner for RecordingSpawner {
    fn spawn_index_issues(
        &self,
        repo_hash: &str,
        project_root: &std::path::Path,
        respect_ttl: bool,
    ) -> std::io::Result<()> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!(
                "{}|{}|{}",
                repo_hash,
                project_root.display(),
                respect_ttl
            ));
        Ok(())
    }
}

fn write_meta(index_root: &std::path::Path, repo_hash: &str, minutes_ago: i64) {
    let dir = index_root.join(repo_hash).join("issues");
    std::fs::create_dir_all(&dir).unwrap();
    let now = chrono::Utc::now() - chrono::Duration::minutes(minutes_ago);
    let meta = serde_json::json!({
        "schema_version": 1,
        "last_full_refresh": now.to_rfc3339(),
        "ttl_minutes": 15,
    });
    std::fs::write(dir.join("meta.json"), meta.to_string()).unwrap();
}

#[tokio::test]
async fn refresh_kicks_runner_when_ttl_expired() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    write_meta(&index_root, repo.as_str(), 20);

    let spawner = RecordingSpawner::default();
    let opts = RefreshIssuesOptions {
        index_root: index_root.clone(),
        repo_hash: repo,
        project_root: tmp.path().to_path_buf(),
        ttl: Duration::from_secs(15 * 60),
    };
    refresh_issues_if_stale(&opts, &spawner).await.unwrap();

    let calls = spawner
        .calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(calls.len(), 1, "expected one runner spawn call");
}

#[tokio::test]
async fn refresh_skipped_within_ttl() {
    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    write_meta(&index_root, repo.as_str(), 5);

    let spawner = RecordingSpawner::default();
    let opts = RefreshIssuesOptions {
        index_root,
        repo_hash: repo,
        project_root: tmp.path().to_path_buf(),
        ttl: Duration::from_secs(15 * 60),
    };
    refresh_issues_if_stale(&opts, &spawner).await.unwrap();

    let calls = spawner
        .calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(calls.len(), 0, "must not spawn runner within TTL window");
}

#[tokio::test]
async fn refresh_kicks_runner_when_meta_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");

    let spawner = RecordingSpawner::default();
    let opts = RefreshIssuesOptions {
        index_root: tmp.path().join("index"),
        repo_hash: repo,
        project_root: tmp.path().to_path_buf(),
        ttl: Duration::from_secs(15 * 60),
    };
    refresh_issues_if_stale(&opts, &spawner).await.unwrap();

    let calls = spawner
        .calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(calls.len(), 1, "missing meta means stale");
}

#[tokio::test]
async fn refresh_returns_quickly_even_when_runner_runs_long() {
    use std::time::Instant;

    #[derive(Clone)]
    struct SlowSpawner;

    impl RunnerSpawner for SlowSpawner {
        fn spawn_index_issues(
            &self,
            _repo_hash: &str,
            _project_root: &std::path::Path,
            _respect_ttl: bool,
        ) -> std::io::Result<()> {
            // Simulate immediate-return spawn (background tokio task elsewhere).
            Ok(())
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let repo = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let opts = RefreshIssuesOptions {
        index_root: tmp.path().join("index"),
        repo_hash: repo,
        project_root: tmp.path().to_path_buf(),
        ttl: Duration::from_secs(15 * 60),
    };

    let start = Instant::now();
    refresh_issues_if_stale(&opts, &SlowSpawner).await.unwrap();
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "refresh must not block on runner work"
    );
}

#[test]
fn python_runner_spawner_builds_issue_index_command_and_surfaces_spawn_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let spawner = PythonRunnerSpawner {
        python_executable: tmp.path().join("missing-python.exe"),
        runner_script: tmp.path().join("runner.py"),
    };

    let error = spawner
        .spawn_index_issues("repo-hash", tmp.path(), true)
        .expect_err("missing executable should surface the spawn error");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

/// Phase 70 T-IDX-394 (Issue #3264 FR-394): after taking the coordinator
/// lock, a queued issue refresh must detect that an equivalent refresh
/// already completed and skip the duplicate run.
#[test]
fn issue_index_refreshed_since_detects_completed_duplicate() {
    use gwt_core::index::runtime::issue_index_refreshed_since;

    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let issues_dir = index_root.join("abc1234567890def").join("issues");
    std::fs::create_dir_all(&issues_dir).unwrap();

    // No meta yet: nothing was refreshed.
    let now = chrono::Utc::now();
    assert!(!issue_index_refreshed_since(
        &index_root,
        "abc1234567890def",
        now
    ));

    std::fs::write(
        issues_dir.join("meta.json"),
        serde_json::json!({
            "schema_version": 1,
            "last_full_refresh": now.to_rfc3339(),
            "ttl_minutes": 15,
        })
        .to_string(),
    )
    .unwrap();

    assert!(
        issue_index_refreshed_since(
            &index_root,
            "abc1234567890def",
            now - chrono::Duration::minutes(1)
        ),
        "a refresh completed after our request must be detected"
    );
    assert!(
        !issue_index_refreshed_since(
            &index_root,
            "abc1234567890def",
            now + chrono::Duration::minutes(1)
        ),
        "an older refresh must not satisfy a newer request"
    );
}

/// Phase 70 T-IDX-401 (Issue #3264): the production issue index spawn runs
/// the runner through the host-wide coordinator (FR-379/FR-382), passing the
/// background QoS profile and draining the child.
#[cfg(unix)]
#[test]
fn python_runner_spawner_runs_issue_index_through_the_coordinator() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("runner-log.txt");
    let python = tmp.path().join("fake-python.sh");
    std::fs::write(
        &python,
        format!("#!/bin/sh\necho \"$@\" >> \"{}\"\nexit 0\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();

    let spawner = PythonRunnerSpawner {
        python_executable: python,
        runner_script: tmp.path().join("runner.py"),
    };
    spawner
        .spawn_index_issues("cafe0123cafe0123", tmp.path(), false)
        .expect("spawn detaches");

    // The detached worker acquires the coordinator lease, runs the runner
    // with background QoS, and drains it. Poll for its completion.
    let deadline = Instant::now() + Duration::from_secs(20);
    let contents = loop {
        let contents = std::fs::read_to_string(&log).unwrap_or_default();
        if !contents.is_empty() {
            break contents;
        }
        assert!(
            Instant::now() < deadline,
            "coordinated issue index runner did not run"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(contents.contains("--action index-issues"), "{contents}");
    assert!(contents.contains("--qos background"), "{contents}");
    assert!(contents.contains("cafe0123cafe0123"), "{contents}");
}

/// A runner failure is drained and logged without crashing the caller.
#[cfg(unix)]
#[test]
fn python_runner_spawner_survives_runner_failure() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("ran");
    let python = tmp.path().join("fake-python.sh");
    std::fs::write(
        &python,
        format!(
            "#!/bin/sh\ntouch \"{}\"\necho boom >&2\nexit 3\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();

    let spawner = PythonRunnerSpawner {
        python_executable: python,
        runner_script: tmp.path().join("runner.py"),
    };
    spawner
        .spawn_index_issues("dead0123dead0123", tmp.path(), true)
        .expect("spawn detaches");

    let deadline = Instant::now() + Duration::from_secs(20);
    while !marker.exists() {
        assert!(
            Instant::now() < deadline,
            "failing runner must still be executed and drained"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A concurrent equivalent refresh coalesces instead of double-spawning
/// (FR-382): the joined worker leaves once the owner publishes.
#[cfg(unix)]
#[test]
fn python_runner_spawner_coalesces_into_a_running_issue_job() {
    use gwt_core::index_coordinator::{
        IndexCoordinator, JobAdmission, JobOutcome, JobPriority, TargetKey,
    };
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("runner-log.txt");
    let python = tmp.path().join("fake-python.sh");
    std::fs::write(
        &python,
        format!("#!/bin/sh\necho \"$@\" >> \"{}\"\nexit 0\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();

    let coordinator = IndexCoordinator::open_default().expect("open coordinator");
    let key = TargetKey::repo_shared("beef0123beef0123", "issues");
    let guard = match coordinator
        .request_job(&key, JobPriority::Background, Duration::from_secs(5))
        .expect("own the issues target")
    {
        JobAdmission::Owner(guard) => guard,
        JobAdmission::Joined(_) => panic!("test must own the target first"),
    };

    let spawner = PythonRunnerSpawner {
        python_executable: python,
        runner_script: tmp.path().join("runner.py"),
    };
    spawner
        .spawn_index_issues("beef0123beef0123", tmp.path(), false)
        .expect("spawn detaches");

    // The worker joins the running job instead of starting a second runner.
    let deadline = Instant::now() + Duration::from_secs(20);
    while guard.waiter_count().expect("waiter count") == 0 {
        assert!(
            Instant::now() < deadline,
            "detached worker must join the running issue job"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    guard.complete(JobOutcome::Completed).expect("complete");

    // The joined worker leaves after observing the shared completion...
    let waiters_dir = coordinator.target_waiters_dir(&key);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let live = std::fs::read_dir(&waiters_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        if live == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "coalesced worker must deregister after the shared outcome"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    // ...and never spawned its own runner.
    assert!(
        std::fs::read_to_string(&log).unwrap_or_default().is_empty(),
        "coalesced refresh must not spawn a duplicate runner"
    );
}

/// A runner that cannot even spawn publishes a failed outcome instead of
/// leaving the job dangling.
#[cfg(unix)]
#[test]
fn python_runner_spawner_publishes_failure_when_spawn_is_impossible() {
    use gwt_core::index_coordinator::IndexCoordinator;
    use std::time::Instant;

    let tmp = tempfile::tempdir().unwrap();
    // Present but not executable: passes the is_file precheck, fails spawn.
    let python = tmp.path().join("fake-python.sh");
    std::fs::write(&python, "#!/bin/sh\nexit 0\n").unwrap();

    let spawner = PythonRunnerSpawner {
        python_executable: python,
        runner_script: tmp.path().join("runner.py"),
    };
    spawner
        .spawn_index_issues("f00d0123f00d0123", tmp.path(), false)
        .expect("spawn detaches");

    let coordinator = IndexCoordinator::open_default().expect("open coordinator");
    let key = gwt_core::index_coordinator::TargetKey::repo_shared("f00d0123f00d0123", "issues");
    let state_path = coordinator.target_state_path(&key);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let raw = std::fs::read_to_string(&state_path).unwrap_or_default();
        if raw.contains("failed") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "spawn failure must publish a failed outcome, state: {raw}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn issue_index_refreshed_since_rejects_unparseable_timestamps() {
    use gwt_core::index::runtime::issue_index_refreshed_since;

    let tmp = tempfile::tempdir().unwrap();
    let index_root = tmp.path().join("index");
    let issues_dir = index_root.join("baadf00dbaadf00d").join("issues");
    std::fs::create_dir_all(&issues_dir).unwrap();
    std::fs::write(
        issues_dir.join("meta.json"),
        serde_json::json!({
            "schema_version": 1,
            "last_full_refresh": "not-a-timestamp",
            "ttl_minutes": 15,
        })
        .to_string(),
    )
    .unwrap();
    assert!(!issue_index_refreshed_since(
        &index_root,
        "baadf00dbaadf00d",
        chrono::Utc::now()
    ));
}
