use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use gwt::index_worker::bootstrap_project_index_for_path_with_broker;
#[cfg(unix)]
use gwt::index_worker::{
    bootstrap_project_index_for_path, execute_claimed_project_index_refresh_with,
};
use gwt_core::process::hidden_command;
#[cfg(unix)]
use gwt_core::{
    index::broker::{
        RefreshIntent, RefreshReason, RefreshResourceClass, RefreshScope, RefreshTarget,
        RefreshTargetState, REFRESH_INTENT_PROTOCOL_VERSION,
    },
    index_coordinator::JobPriority,
    paths::gwt_cache_dir,
    process_console::{ProcessConsoleHub, ProcessKind},
    worktree_hash::compute_worktree_hash,
};
use gwt_core::{
    index::{broker::RefreshBroker, runtime::RunnerSpawner},
    repo_hash::detect_repo_hash,
};

#[derive(Clone, Default)]
struct RecordingSpawner {
    calls: Arc<Mutex<Vec<String>>>,
}

impl RunnerSpawner for RecordingSpawner {
    fn spawn_index_issues(
        &self,
        repo_hash: &str,
        project_root: &Path,
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

#[cfg(unix)]
use gwt_core::test_support::ScopedEnvVar;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
#[cfg(unix)]
fn startup_and_frontend_ready_coalesce_one_canonical_base_without_direct_runner_spawn() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("create home");
    let _home = ScopedEnvVar::set("HOME", &home);
    let _userprofile = ScopedEnvVar::set("USERPROFILE", &home);
    let process_console = ProcessConsoleHub::new();
    assert!(
        gwt_core::process_console::set_global(process_console.clone()),
        "index_bootstrap_test must install the process-global observation hub exactly once"
    );

    let repo = tmp.path().join("repo");
    let wt_a = tmp.path().join("wt-feature-a");
    let wt_b = tmp.path().join("wt-feature-b");
    init_git_repo(&repo);
    add_origin(&repo, "https://github.com/example/project.git");
    commit_file(&repo, "README.md", "# repo\n");
    add_worktree(&repo, &wt_a, "feature/a");
    add_worktree(&repo, &wt_b, "feature/b");

    let repo_hash = detect_repo_hash(&repo).expect("repo hash");
    let wt_hash = compute_worktree_hash(&wt_a).expect("worktree hash");
    let index_root = home.join(".gwt").join("index");
    let worktree_root = index_root
        .join(repo_hash.as_str())
        .join("worktrees")
        .join(wt_hash.as_str());
    fs::create_dir_all(worktree_root.join("specs")).expect("create legacy specs dir");
    fs::write(worktree_root.join("specs").join("chroma.sqlite3"), "legacy")
        .expect("write legacy sqlite");
    fs::write(worktree_root.join("manifest-specs.json"), "[]").expect("write legacy manifest");
    fs::write(worktree_root.join("meta.json"), r#"{"schema_version":1}"#)
        .expect("write worktree meta");

    let legacy_worktree_index = wt_a.join(".gwt").join("index");
    fs::create_dir_all(&legacy_worktree_index).expect("create legacy worktree dir");
    fs::write(legacy_worktree_index.join("stale"), "data").expect("write legacy worktree file");

    let orphan = index_root
        .join(repo_hash.as_str())
        .join("worktrees")
        .join("deadbeefdeadbeef");
    fs::create_dir_all(&orphan).expect("create orphan");
    fs::write(orphan.join("marker"), "data").expect("write orphan file");

    // Activate the legacy direct issue-refresh branch. A partial GREEN that
    // removes runtime ensure but leaves this direct spawn behind must still
    // fail the synchronous RecordingSpawner assertion below.
    let issues_dir = index_root.join(repo_hash.as_str()).join("issues");
    fs::create_dir_all(&issues_dir).expect("create issues dir");
    fs::write(
        issues_dir.join("meta.json"),
        serde_json::json!({
            "schema_version": 1,
            "last_full_refresh": (chrono::Utc::now() - chrono::Duration::minutes(20)).to_rfc3339(),
            "ttl_minutes": 15,
            "source_cache_fingerprint": "stale",
            "source_document_count": 1,
        })
        .to_string(),
    )
    .expect("write stale issues meta");
    let cache_root = gwt_cache_dir().join("issues").join(repo_hash.as_str());
    gwt_github::Cache::new(cache_root)
        .write_snapshot(&gwt_github::IssueSnapshot {
            number: gwt_github::IssueNumber(3772),
            title: "Refresh Broker admission storm".to_string(),
            body: "Startup submits one canonical base intent.".to_string(),
            labels: vec!["perf".to_string()],
            state: gwt_github::IssueState::Open,
            updated_at: gwt_github::UpdatedAt::new("2026-08-29T00:00:00Z"),
            comments: vec![],
        })
        .expect("write issue cache snapshot");

    let runner_log = tmp.path().join("runner-log.txt");
    fs::write(&runner_log, b"").expect("create runner log");
    let _runner_log = ScopedEnvVar::set("GWT_FAKE_RUNNER_LOG", &runner_log);
    let python = gwt_core::runtime::project_index_python_path();
    fs::create_dir_all(python.parent().expect("python parent")).expect("create runtime dir");
    fs::write(
        &python,
        "#!/bin/sh\necho \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\nprintf '{\"ok\":true}\\n'\n",
    )
    .expect("write poison runner");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).expect("chmod runner");
    }

    // Exercise the production wrapper used by startup / FrontendReady, not
    // only the lower-level injectable helper. Main plus linked worktrees and
    // a replay must all open the same default durable broker. The Process
    // Console banner is emitted synchronously before a detached runner
    // process starts, so its exact count avoids a shell-log timing race.
    let runner_banners_before = process_console
        .snapshot_kind(ProcessKind::IndexRunner)
        .len();
    for project_root in [&repo, &wt_a, &wt_b, &wt_a] {
        bootstrap_project_index_for_path(project_root).expect("bootstrap index");
    }
    assert_eq!(
        process_console
            .snapshot_kind(ProcessKind::IndexRunner)
            .len(),
        runner_banners_before,
        "startup / FrontendReady admission must synchronously emit zero runner spawn banners"
    );

    assert!(
        worktree_root.join("meta.json").exists(),
        "startup admission must not mutate worktree metadata"
    );
    assert!(
        worktree_root.join("specs").exists(),
        "startup admission must not mutate legacy worktree specs before the broker owns a job"
    );
    assert!(
        worktree_root.join("manifest-specs.json").exists(),
        "startup admission must not mutate the legacy specs manifest"
    );
    assert!(
        legacy_worktree_index.exists(),
        "startup admission must not remove legacy $WORKTREE/.gwt/index"
    );
    assert!(
        orphan.exists(),
        "startup admission must leave orphan cleanup to the admitted broker owner"
    );

    let runner_argv = fs::read(&runner_log).expect("read runner log");
    assert!(
        runner_argv.is_empty(),
        "startup must submit to the refresh broker instead of spawning any runner: {}",
        String::from_utf8_lossy(&runner_argv)
    );

    let broker = RefreshBroker::open_default().expect("open default refresh broker");
    let expected_base = RefreshTarget::base(
        repo_hash.as_str(),
        [RefreshScope::Files, RefreshScope::FilesDocs],
    );
    let snapshot = broker.inspect().expect("inspect refresh broker");
    let targets = snapshot
        .targets()
        .iter()
        .map(|state| state.target())
        .collect::<Vec<_>>();
    assert_eq!(
        targets,
        vec![&expected_base],
        "main plus N linked worktrees must coalesce to one canonical base target and no overlays"
    );
    assert_eq!(snapshot.target_count(), 1, "distinct target count");
    assert!(
        snapshot.queue_depth() <= 1,
        "startup queue must remain bounded by the one distinct base target"
    );
    assert_eq!(
        snapshot.running_count(),
        0,
        "startup admission must not start model work synchronously"
    );
    let pending = snapshot
        .target(&expected_base)
        .expect("startup base target snapshot");
    assert_eq!(pending.state(), RefreshTargetState::Quiet);
    assert_eq!(pending.priority(), JobPriority::Background);
    assert!(
        pending.quiet_deadline_millis().is_some(),
        "startup admission must retain a background quiet deadline"
    );
    assert!(
        broker
            .claim_next()
            .expect("probe quiet startup target")
            .is_none(),
        "startup target must not be claimable before the quiet period expires"
    );

    // Elevate the quiet startup intent, acquire its real durable claim, and
    // pass ownership into the maintenance boundary. Cleanup must happen only
    // after this owner transition, and stale issue state must not escape the
    // broker by invoking RunnerSpawner directly.
    broker
        .submit(RefreshIntent {
            protocol_version: REFRESH_INTENT_PROTOCOL_VERSION,
            target: expected_base.clone(),
            desired_epoch: pending.desired_epoch(),
            desired_snapshot: pending.desired_snapshot().to_string(),
            priority: JobPriority::ManualRebuild,
            reason: RefreshReason::Manual,
            resource_class: RefreshResourceClass::Embedding,
        })
        .expect("promote startup target for owner execution");
    let claim = broker
        .claim_next()
        .expect("claim promoted startup target")
        .expect("promoted startup target must have one owner");
    let spawner = RecordingSpawner::default();
    execute_claimed_project_index_refresh_with(claim, &wt_a, &index_root, &spawner)
        .expect("execute claimed index maintenance");
    let completed = broker.inspect().expect("inspect completed startup owner");
    assert_eq!(completed.running_count(), 0);
    let completed_target = completed
        .target(&expected_base)
        .expect("completed startup base target");
    assert_eq!(completed_target.state(), RefreshTargetState::Ready);
    assert_eq!(completed_target.follow_up_count(), 0);
    assert!(
        spawner
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty(),
        "broker-owned maintenance must not retain a direct issue runner escape hatch"
    );
    assert!(!worktree_root.join("specs").exists());
    assert!(!worktree_root.join("manifest-specs.json").exists());
    assert!(!legacy_worktree_index.exists());
    assert!(!orphan.exists());
}

#[test]
fn bootstrap_preserves_repo_scoped_memory_index_directory() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // SPEC-2805: memory is repo-scoped at ~/.gwt/index/<repo>/memory/. Bootstrap
    // must not treat it as an orphan worktree dir or as legacy worktree-scoped
    // state, regardless of whether a current worktree exists.
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    let wt = tmp.path().join("wt-feature");
    init_git_repo(&repo);
    add_origin(&repo, "https://github.com/example/project.git");
    commit_file(&repo, "README.md", "# repo\n");
    add_worktree(&repo, &wt, "feature/shared");

    let repo_hash = detect_repo_hash(&repo).expect("repo hash");
    let index_root = tmp.path().join("index");
    let memory_dir = index_root.join(repo_hash.as_str()).join("memory");
    fs::create_dir_all(&memory_dir).expect("create repo-scoped memory dir");
    fs::write(memory_dir.join("chroma.sqlite3"), "fake-db").expect("write memory db");
    fs::write(memory_dir.join("meta.json"), r#"{"schema_version":1}"#).expect("write memory meta");

    let broker = RefreshBroker::open(index_root.join("refresh-broker"), Duration::from_secs(30))
        .expect("open refresh broker");
    let spawner = RecordingSpawner::default();
    bootstrap_project_index_for_path_with_broker(&wt, &index_root, &broker, &spawner)
        .expect("bootstrap index");

    assert!(
        memory_dir.join("chroma.sqlite3").exists(),
        "bootstrap must preserve the repo-scoped memory chroma.sqlite3"
    );
    assert!(
        memory_dir.join("meta.json").exists(),
        "bootstrap must preserve the repo-scoped memory meta.json"
    );
}

/// SPEC #1939 Phase 70 T-IDX-382 (Issue #3264): every rebuild entry must
/// funnel through the host-wide index coordinator. While another process
/// holds the heavy lease (a model-loaded runner), `default_rebuild_runner`
/// must queue instead of spawning a concurrent runner Python.
#[cfg(unix)]
#[test]
fn default_rebuild_runner_waits_for_host_wide_heavy_lease() {
    use gwt_core::index_coordinator::{IndexCoordinator, JobAdmission, JobPriority, TargetKey};
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

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

    // Fake runner python: records each invocation and reports success.
    let runner_log = tmp.path().join("runner-log.txt");
    let _runner_log_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_LOG", &runner_log);
    let python = gwt_core::runtime::project_index_python_path();
    fs::create_dir_all(python.parent().expect("python parent")).expect("create venv dir");
    fs::write(
        &python,
        "#!/bin/sh\necho \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\nprintf '{\"ok\":true}\\n'\n",
    )
    .expect("write fake python");
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).expect("chmod fake python");

    // Another process' embedding build holds the host-wide heavy lease.
    let coordinator = IndexCoordinator::open(gwt_core::index_coordinator::coordinator_root())
        .expect("open coordinator");
    let admission = coordinator
        .request_job(
            &TargetKey::repo_shared("unrelated-repo", "files"),
            JobPriority::Background,
            Duration::from_secs(5),
        )
        .expect("request unrelated job");
    let holder = match admission {
        JobAdmission::Owner(guard) => guard,
        JobAdmission::Joined(_) => panic!("expected to own the unrelated target"),
    };
    let heavy = holder
        .acquire_heavy(Duration::from_secs(5))
        .expect("acquire heavy lease");

    let rebuild_repo = repo.clone();
    let rebuild = std::thread::spawn(move || {
        gwt::default_rebuild_runner(
            &rebuild_repo,
            gwt::index_worker::IndexRebuildScope::Issues,
            None,
        )
    });

    // While the heavy lease is held elsewhere the rebuild must not spawn the
    // runner Python (FR-379 host-wide exclusion).
    let held_window = Instant::now();
    while held_window.elapsed() < Duration::from_millis(500) {
        assert!(
            !runner_log.exists()
                || fs::read_to_string(&runner_log)
                    .unwrap_or_default()
                    .is_empty(),
            "rebuild spawned a runner while the host-wide heavy lease was held"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    drop(heavy);
    drop(holder);

    let join_deadline = Instant::now();
    while !rebuild.is_finished() {
        assert!(
            join_deadline.elapsed() < Duration::from_secs(20),
            "rebuild did not proceed after the heavy lease was released"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    rebuild
        .join()
        .expect("join rebuild thread")
        .expect("rebuild succeeds after lease release");
    let log = fs::read_to_string(&runner_log).expect("runner log after release");
    assert!(
        log.contains("index-issues"),
        "rebuild must run the issues indexer after acquiring the lease, got {log:?}"
    );
}

/// T-IDX-431 review regression: file-index-v2 reports action failures in its
/// JSON payload while intentionally retaining exit status 0. The owner must
/// publish that structured failure through the coordinator so an equivalent
/// joined rebuild cannot observe a false `Completed` outcome.
#[cfg(unix)]
#[test]
fn default_rebuild_runner_propagates_structured_file_failure_to_joiner() {
    use gwt_core::index_coordinator::{IndexCoordinator, TargetKey};
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

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
    add_origin(&repo, "https://github.com/example/structured-failure.git");
    commit_file(&repo, "README.md", "# repo\n");

    let runner_log = tmp.path().join("runner-log.txt");
    let runner_started = tmp.path().join("runner-started");
    let runner_release = tmp.path().join("runner-release");
    let _runner_log_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_LOG", &runner_log);
    let _runner_started_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_STARTED", &runner_started);
    let _runner_release_env = ScopedEnvVar::set("GWT_FAKE_RUNNER_RELEASE", &runner_release);
    let python = gwt_core::runtime::project_index_python_path();
    fs::create_dir_all(python.parent().expect("python parent")).expect("create venv dir");
    fs::write(
        &python,
        "#!/bin/sh\n\
echo \"$@\" >> \"$GWT_FAKE_RUNNER_LOG\"\n\
: > \"$GWT_FAKE_RUNNER_STARTED\"\n\
while [ ! -f \"$GWT_FAKE_RUNNER_RELEASE\" ]; do sleep 0.01; done\n\
printf '{\"ok\":false,\"error_code\":\"PUBLISH_FAILED\",\"error\":\"marker fsync failed\"}\\n'\n",
    )
    .expect("write fake python");
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).expect("chmod fake python");

    let repo_hash = detect_repo_hash(&repo).expect("repo hash");
    let worktree_hash = compute_worktree_hash(&repo).expect("worktree hash");
    let coordinator = IndexCoordinator::open(gwt_core::index_coordinator::coordinator_root())
        .expect("open coordinator");
    let key = TargetKey::worktree(repo_hash.as_str(), "files", worktree_hash.as_str());

    let owner_repo = repo.clone();
    let owner = std::thread::spawn(move || {
        gwt::default_rebuild_runner(
            &owner_repo,
            gwt::index_worker::IndexRebuildScope::Files,
            None,
        )
    });
    let started_deadline = Instant::now() + Duration::from_secs(10);
    while !runner_started.exists() {
        assert!(
            Instant::now() < started_deadline,
            "owner runner never started"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let joiner_repo = repo.clone();
    let joiner = std::thread::spawn(move || {
        gwt::default_rebuild_runner(
            &joiner_repo,
            gwt::index_worker::IndexRebuildScope::Files,
            None,
        )
    });
    let waiter_deadline = Instant::now() + Duration::from_secs(10);
    while fs::read_dir(coordinator.target_waiters_dir(&key))
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
        == 0
    {
        assert!(
            Instant::now() < waiter_deadline,
            "equivalent rebuild never joined the owner"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    fs::write(&runner_release, b"release").expect("release fake runner");

    let owner_error = owner
        .join()
        .expect("join owner")
        .expect_err("structured owner failure must propagate");
    let joiner_error = joiner
        .join()
        .expect("join joiner")
        .expect_err("joined caller must observe failed outcome");
    for error in [&owner_error, &joiner_error] {
        assert!(
            error.contains("PUBLISH_FAILED"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("marker fsync failed"),
            "unexpected error: {error}"
        );
    }
    assert_eq!(
        fs::read_to_string(&runner_log)
            .expect("runner log")
            .lines()
            .count(),
        1,
        "equivalent rebuilds must share one runner invocation"
    );
}

fn init_git_repo(path: &Path) {
    let output = hidden_command("git")
        .args(["init", path.to_str().unwrap()])
        .output()
        .expect("git init");
    assert!(output.status.success(), "git init failed");

    let email = hidden_command("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(path)
        .output()
        .expect("git config user.email");
    assert!(email.status.success(), "git config user.email failed");

    let name = hidden_command("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(path)
        .output()
        .expect("git config user.name");
    assert!(name.status.success(), "git config user.name failed");
}

fn add_origin(path: &Path, url: &str) {
    let output = hidden_command("git")
        .args(["remote", "add", "origin", url])
        .current_dir(path)
        .output()
        .expect("git remote add origin");
    assert!(
        output.status.success(),
        "git remote add origin failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit_file(path: &Path, name: &str, body: &str) {
    fs::write(path.join(name), body).expect("write commit file");
    let add = hidden_command("git")
        .args(["add", name])
        .current_dir(path)
        .output()
        .expect("git add");
    assert!(add.status.success(), "git add failed");

    let commit = hidden_command("git")
        .args(["commit", "-m", "init"])
        .current_dir(path)
        .output()
        .expect("git commit");
    assert!(
        commit.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&commit.stderr)
    );
}

fn add_worktree(repo: &Path, worktree: &Path, branch: &str) {
    let output = hidden_command("git")
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

/// Phase 70 T-IDX-401 (Issue #3264): the manual rebuild entry coordinates,
/// passes interactive QoS, resumes after a runner yield, and surfaces
/// runner failures.
#[cfg(unix)]
#[test]
fn manual_rebuild_runner_resumes_yields_and_surfaces_failures() {
    use std::os::unix::fs::PermissionsExt;

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

    let runner_log = tmp.path().join("runner-log.txt");
    let yield_flag = tmp.path().join("yielded-once");
    let python = gwt_core::runtime::project_index_python_path();
    fs::create_dir_all(python.parent().expect("python parent")).expect("create venv dir");
    // First index invocation reports a cooperative yield; the retry
    // completes. Probe-style invocations always succeed.
    fs::write(
        &python,
        format!(
            "#!/bin/sh\necho \"$@\" >> \"{log}\"\nif [ ! -f \"{flag}\" ]; then\n  touch \"{flag}\"\n  printf '{{\"ok\": true, \"yielded\": true, \"resumable\": true}}\\n'\nelse\n  printf '{{\"ok\": true, \"indexed\": 1}}\\n'\nfi\n",
            log = runner_log.display(),
            flag = yield_flag.display(),
        ),
    )
    .expect("write fake python");
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).expect("chmod");

    gwt::manual_rebuild_runner(&repo, gwt::index_worker::IndexRebuildScope::Issues, None)
        .expect("yielded rebuild resumes and completes");
    let calls = fs::read_to_string(&runner_log).expect("runner log");
    let rebuild_calls = calls
        .lines()
        .filter(|line| line.contains("--action index-issues"))
        .count();
    assert_eq!(
        rebuild_calls, 2,
        "a yielded rebuild must re-run after releasing the heavy lease: {calls}"
    );
    assert!(
        calls.contains("--qos interactive"),
        "manual rebuilds run at interactive QoS: {calls}"
    );

    // Runner failure propagates as an error instead of silent success.
    fs::write(
        &python,
        "#!/bin/sh\ncase \"$*\" in *\"--action index-\"*) echo broken >&2; exit 3;; *) printf '{\"ok\": true}\\n';; esac\n",
    )
    .expect("write failing python");
    let error =
        gwt::manual_rebuild_runner(&repo, gwt::index_worker::IndexRebuildScope::Issues, None)
            .expect_err("runner failure must propagate");
    assert!(error.contains("broken"), "{error}");
}
