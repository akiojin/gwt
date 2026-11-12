//! Phase 8: integration tests for `gwt_core::index::runtime::refresh_issues_if_stale`.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gwt_core::{
    index::runtime::{
        issue_rebuild_mode, refresh_issues_if_stale, PythonRunnerSpawner, RefreshIssuesOptions,
        RunnerSpawner,
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
        "document_count": 1,
    });
    std::fs::write(dir.join("meta.json"), meta.to_string()).unwrap();
    std::fs::write(dir.join("chroma.sqlite3"), b"fixture").unwrap();
}

fn write_pooled_generation(index_root: &std::path::Path, repo_hash: &str) -> std::path::PathBuf {
    use gwt_core::index::view::canonical_json_sha256;
    use serde_json::json;

    write_meta(index_root, repo_hash, 5);
    let repo = index_root.join(repo_hash);
    std::fs::remove_file(repo.join("issues/chroma.sqlite3")).unwrap();
    let generation = repo.join("issues.gen/gen-1-1");
    let store = repo.join("issues.gen/record-stores/issues-records-v1/store");
    std::fs::create_dir_all(&generation).unwrap();
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("chroma.sqlite3"), b"fixture").unwrap();
    let members = json!({"1": "a".repeat(64)});
    let digest = canonical_json_sha256(&members);
    std::fs::write(
        generation.join("membership.json"),
        json!({
            "schema_version": 1,
            "writer_protocol": "issues-records-v1",
            "record_layout": "issue-spec-chunks-v1",
            "members": members,
            "members_digest": digest,
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        repo.join("issues.gen/active.json"),
        json!({
            "schema_version": 2,
            "writer_protocol": "issues-records-v1",
            "scope": "issues",
            "generation": "gen-1-1",
            "document_count": 1,
            "membership_digest": digest,
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        repo.join("manifest-issues.json"),
        json!([{"path":"1", "content_hash":"a".repeat(64)}]).to_string(),
    )
    .unwrap();
    repo
}

#[tokio::test]
async fn pooled_issue_generation_keeps_incremental_mode_and_fresh_ttl() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = compute_repo_hash("https://github.com/example/pooled-issues.git");
    let repo_dir = write_pooled_generation(tmp.path(), repo.as_str());
    assert_eq!(issue_rebuild_mode(tmp.path(), repo.as_str()), "incremental");
    let spawner = RecordingSpawner::default();
    let opts = RefreshIssuesOptions {
        index_root: tmp.path().to_path_buf(),
        repo_hash: repo,
        project_root: tmp.path().to_path_buf(),
        ttl: Duration::from_secs(900),
    };
    refresh_issues_if_stale(&opts, &spawner).await.unwrap();
    assert!(spawner.calls.lock().unwrap().is_empty());

    std::fs::remove_file(
        repo_dir.join("issues.gen/record-stores/issues-records-v1/store/chroma.sqlite3"),
    )
    .unwrap();
    assert_eq!(
        issue_rebuild_mode(tmp.path(), opts.repo_hash.as_str()),
        "full"
    );
    refresh_issues_if_stale(&opts, &spawner).await.unwrap();
    assert_eq!(spawner.calls.lock().unwrap().len(), 1);
}

#[test]
fn pooled_issue_generation_rejects_unknown_contract_and_inexact_membership() {
    use gwt_core::index::view::canonical_json_sha256;
    use serde_json::{json, Value};

    let tmp = tempfile::tempdir().unwrap();
    let repo = "abc1234567890def";
    let repo_dir = write_pooled_generation(tmp.path(), repo);
    let pointer_path = repo_dir.join("issues.gen/active.json");
    let membership_path = repo_dir.join("issues.gen/gen-1-1/membership.json");
    let original: Value = serde_json::from_slice(&std::fs::read(&pointer_path).unwrap()).unwrap();
    for (field, value) in [
        ("schema_version", json!(3)),
        ("writer_protocol", json!("future-issues-records")),
        ("scope", json!("memory")),
        ("delta", json!("gen-2-2")),
        ("generation", json!("../gen-1-1")),
        ("document_count", json!(2)),
        ("membership_digest", json!("b".repeat(64))),
    ] {
        let mut pointer = original.clone();
        pointer[field] = value;
        std::fs::write(&pointer_path, pointer.to_string()).unwrap();
        assert_eq!(issue_rebuild_mode(tmp.path(), repo), "full", "{field}");
    }
    std::fs::write(&pointer_path, original.to_string()).unwrap();
    let mut membership: Value =
        serde_json::from_slice(&std::fs::read(&membership_path).unwrap()).unwrap();
    membership["members"]["1"] = json!("c".repeat(64));
    std::fs::write(&membership_path, membership.to_string()).unwrap();
    assert_eq!(issue_rebuild_mode(tmp.path(), repo), "full");

    // A self-consistent checksum cannot authorize two logical IDs sharing
    // one storage key; generation membership remains one-to-one.
    membership["members"]["2"] = membership["members"]["1"].clone();
    let digest = canonical_json_sha256(&membership["members"]);
    membership["members_digest"] = json!(digest);
    let mut pointer = original;
    pointer["document_count"] = json!(2);
    pointer["membership_digest"] = json!(digest);
    std::fs::write(&pointer_path, pointer.to_string()).unwrap();
    std::fs::write(&membership_path, membership.to_string()).unwrap();
    std::fs::write(
        repo_dir.join("manifest-issues.json"),
        json!([
            {"path":"1", "content_hash":"c".repeat(64)},
            {"path":"2", "content_hash":"c".repeat(64)}
        ])
        .to_string(),
    )
    .unwrap();
    let meta_path = repo_dir.join("issues/meta.json");
    let mut meta: Value = serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    meta["document_count"] = json!(2);
    std::fs::write(meta_path, meta.to_string()).unwrap();
    assert_eq!(issue_rebuild_mode(tmp.path(), repo), "full");
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
    assert_eq!(
        calls.as_slice(),
        [format!(
            "{}|{}|false",
            opts.repo_hash.as_str(),
            opts.project_root.display()
        )],
        "expired TTL must issue exactly one spawn with the requested repository and project root"
    );
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

#[test]
fn python_runner_spawner_builds_issue_index_command_and_surfaces_spawn_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let coordinator_root = tmp.path().join("coordinator");
    let spawner = PythonRunnerSpawner {
        python_executable: tmp.path().join("missing-python.exe"),
        runner_script: tmp.path().join("runner.py"),
    };

    let error = spawner
        .spawn_index_issues_with_coordinator_root("repo-hash", tmp.path(), true, &coordinator_root)
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
            "document_count": 1,
        })
        .to_string(),
    )
    .unwrap();

    std::fs::write(issues_dir.join("chroma.sqlite3"), b"fixture").unwrap();
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
    let coordinator_root = tmp.path().join("coordinator");
    let log = tmp.path().join("runner-log.txt");
    let python = tmp.path().join("fake-python.sh");
    std::fs::write(
        &python,
        format!(
            "#!/bin/sh\ncp \"{}\" \"{}\"\necho \"$@\" >> \"{}\"\nexit 0\n",
            coordinator_root.join("heavy.ticket.json").display(),
            tmp.path().join("runner-ticket.json").display(),
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();

    let spawner = PythonRunnerSpawner {
        python_executable: python,
        runner_script: tmp.path().join("runner.py"),
    };
    spawner
        .spawn_index_issues_with_coordinator_root(
            "cafe0123cafe0123",
            tmp.path(),
            false,
            &coordinator_root,
        )
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
    let ticket: serde_json::Value = serde_json::from_slice(
        &std::fs::read(tmp.path().join("runner-ticket.json")).expect("holder ticket recorded"),
    )
    .unwrap();
    assert_eq!(
        ticket["holder_context"]["project_root"],
        tmp.path().to_string_lossy().as_ref()
    );
    assert_eq!(ticket["holder_context"]["action"], "index-issues");
    assert_eq!(ticket["holder_context"]["qos"], "background");
}

/// Issue #4140 AC-5: while the issues index runner is still running, a
/// `verify.run`-shaped claimant must get the host-wide heavy lease within a
/// bounded time. The reported failure held it for the whole 30-minute runner,
/// so every agent's verification was refused with `host busy`.
#[cfg(unix)]
#[test]
fn coordinated_issue_index_hands_the_heavy_lease_to_a_waiting_verification_run() {
    use gwt_core::index_coordinator::{
        IndexCoordinator, JobAdmission, JobOutcome, JobPriority, TargetKey,
    };
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    let tmp = tempfile::tempdir().unwrap();
    let coordinator_root = tmp.path().join("coordinator");
    let started = tmp.path().join("runner-started");
    let stop = tmp.path().join("runner-stop");
    let python = tmp.path().join("fake-python.sh");
    std::fs::write(
        &python,
        format!(
            // Self-limiting: the arena disappears when the test ends, and a
            // runner that only watched for the stop file would then loop
            // forever on a path that can never appear.
            "#!/bin/sh\ntouch \"{}\"\ni=0\nwhile [ ! -f \"{}\" ] && [ $i -lt 600 ]; do sleep 0.05; \
             i=$((i+1)); done\nexit 0\n",
            started.display(),
            stop.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();

    let spawner = PythonRunnerSpawner {
        python_executable: python,
        runner_script: tmp.path().join("runner.py"),
    };
    spawner
        .spawn_index_issues_with_coordinator_root(
            "cafe0123cafe0123",
            tmp.path(),
            false,
            &coordinator_root,
        )
        .expect("spawn detaches");

    let deadline = Instant::now() + Duration::from_secs(20);
    while !started.exists() {
        assert!(Instant::now() < deadline, "the fake runner never started");
        std::thread::sleep(Duration::from_millis(25));
    }

    let coordinator = IndexCoordinator::open(&coordinator_root).expect("open coordinator");
    let key = TargetKey::verification("cafe0123cafe0123", "wt-1");
    let guard = match coordinator
        .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(10))
        .expect("request verification job")
    {
        JobAdmission::Owner(guard) => guard,
        JobAdmission::Joined(_) => panic!("the index job must not own the verification target"),
    };
    let lease = guard
        .acquire_heavy_with_ttl(Duration::from_secs(20), Duration::from_secs(60))
        .unwrap_or_else(|err| {
            let _ = std::fs::write(&stop, b"stop");
            panic!("the running issue index runner must yield the heavy lease: {err}")
        });

    assert!(
        !stop.exists(),
        "the runner must still be running — the lease was handed over, not waited out"
    );

    lease.release().expect("release verification lease");
    guard
        .complete(JobOutcome::Completed)
        .expect("complete verification job");
    std::fs::write(&stop, b"stop").expect("let the fake runner exit");
}

/// A runner failure is drained and logged without crashing the caller.
#[cfg(unix)]
#[test]
fn python_runner_spawner_survives_runner_failure() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    let tmp = tempfile::tempdir().unwrap();
    let coordinator_root = tmp.path().join("coordinator");
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
        .spawn_index_issues_with_coordinator_root(
            "dead0123dead0123",
            tmp.path(),
            true,
            &coordinator_root,
        )
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
    let coordinator_root = tmp.path().join("coordinator");
    let log = tmp.path().join("runner-log.txt");
    let python = tmp.path().join("fake-python.sh");
    std::fs::write(
        &python,
        format!("#!/bin/sh\necho \"$@\" >> \"{}\"\nexit 0\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();

    let coordinator = IndexCoordinator::open(&coordinator_root).expect("open isolated coordinator");
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
        .spawn_index_issues_with_coordinator_root(
            "beef0123beef0123",
            tmp.path(),
            false,
            &coordinator_root,
        )
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
    let coordinator_root = tmp.path().join("coordinator");
    // Present but not executable: passes the is_file precheck, fails spawn.
    let python = tmp.path().join("fake-python.sh");
    std::fs::write(&python, "#!/bin/sh\nexit 0\n").unwrap();

    let spawner = PythonRunnerSpawner {
        python_executable: python,
        runner_script: tmp.path().join("runner.py"),
    };
    spawner
        .spawn_index_issues_with_coordinator_root(
            "f00d0123f00d0123",
            tmp.path(),
            false,
            &coordinator_root,
        )
        .expect("spawn detaches");

    let coordinator = IndexCoordinator::open(&coordinator_root).expect("open isolated coordinator");
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
