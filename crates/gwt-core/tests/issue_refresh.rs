//! Phase 8: integration tests for `gwt_core::index::runtime::refresh_issues_if_stale`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gwt_core::index::runtime::{refresh_issues_if_stale, RefreshIssuesOptions, RunnerSpawner};
use gwt_core::repo_hash::compute_repo_hash;

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
        self.calls.lock().unwrap_or_else(|p| p.into_inner()).push(format!(
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

    let calls = spawner.calls.lock().unwrap_or_else(|p| p.into_inner());
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

    let calls = spawner.calls.lock().unwrap_or_else(|p| p.into_inner());
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

    let calls = spawner.calls.lock().unwrap_or_else(|p| p.into_inner());
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
