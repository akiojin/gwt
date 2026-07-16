use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use gwt::index_worker::bootstrap_project_index_for_path_with;
use gwt_core::process::hidden_command;
use gwt_core::{
    index::runtime::RunnerSpawner, paths::gwt_cache_dir, repo_hash::detect_repo_hash,
    worktree_hash::compute_worktree_hash,
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

use gwt_core::test_support::ScopedEnvVar;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn bootstrap_helper_reconciles_index_layout_and_kicks_issue_refresh() {
    let _env_lock = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).expect("create home");
    let _home = ScopedEnvVar::set("HOME", &home);
    let _userprofile = ScopedEnvVar::set("USERPROFILE", &home);

    let repo = tmp.path().join("repo");
    let wt = tmp.path().join("wt-feature");
    init_git_repo(&repo);
    add_origin(&repo, "https://github.com/example/project.git");
    commit_file(&repo, "README.md", "# repo\n");
    add_worktree(&repo, &wt);

    let repo_hash = detect_repo_hash(&repo).expect("repo hash");
    let wt_hash = compute_worktree_hash(&wt).expect("worktree hash");
    let index_root = tmp.path().join("index");
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

    let legacy_worktree_index = wt.join(".gwt").join("index");
    fs::create_dir_all(&legacy_worktree_index).expect("create legacy worktree dir");
    fs::write(legacy_worktree_index.join("stale"), "data").expect("write legacy worktree file");

    let orphan = index_root
        .join(repo_hash.as_str())
        .join("worktrees")
        .join("deadbeefdeadbeef");
    fs::create_dir_all(&orphan).expect("create orphan");
    fs::write(orphan.join("marker"), "data").expect("write orphan file");

    let issues_dir = index_root.join(repo_hash.as_str()).join("issues");
    fs::create_dir_all(&issues_dir).expect("create issues dir");
    let stale = chrono::Utc::now() - chrono::Duration::minutes(20);
    fs::write(
        issues_dir.join("meta.json"),
        serde_json::json!({
            "schema_version": 1,
            "last_full_refresh": stale.to_rfc3339(),
            "ttl_minutes": 15,
            "source_cache_fingerprint": "stale",
            "source_document_count": 1,
        })
        .to_string(),
    )
    .expect("write issues meta");
    let cache_root = gwt_cache_dir().join("issues").join(repo_hash.as_str());
    gwt_github::Cache::new(cache_root)
        .write_snapshot(&gwt_github::IssueSnapshot {
            number: gwt_github::IssueNumber(2867),
            title: "Recent Projects cache freshness".to_string(),
            body: "Closed state should reach Issue search at startup.".to_string(),
            labels: vec!["bug".to_string()],
            state: gwt_github::IssueState::Closed,
            updated_at: gwt_github::UpdatedAt::new("2026-05-23T00:00:00Z"),
            comments: vec![],
        })
        .expect("write issue cache snapshot");

    let spawner = RecordingSpawner::default();
    bootstrap_project_index_for_path_with(&wt, &index_root, &spawner).expect("bootstrap index");

    assert!(
        worktree_root.join("meta.json").exists(),
        "worktree meta should survive bootstrap cleanup"
    );
    assert!(
        !worktree_root.join("specs").exists(),
        "bootstrap should remove legacy worktree specs dir"
    );
    assert!(
        !worktree_root.join("manifest-specs.json").exists(),
        "bootstrap should remove legacy worktree specs manifest"
    );
    assert!(
        !legacy_worktree_index.exists(),
        "bootstrap should remove legacy $WORKTREE/.gwt/index"
    );
    assert!(
        !orphan.exists(),
        "bootstrap should remove orphan worktree index"
    );

    let calls = spawner
        .calls
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(
        calls.len(),
        1,
        "source cache fingerprint mismatch should kick one issue rebuild"
    );
    assert!(
        calls[0].contains(repo_hash.as_str()),
        "rebuild must target the resolved repo hash, got {:?}",
        *calls
    );
    assert!(
        call_project_root_matches(&calls[0], &wt),
        "rebuild should use the requested worktree path, got {:?}",
        *calls
    );
    assert!(
        calls[0].ends_with("|false"),
        "startup rebuild should bypass index TTL after cache source mismatch, got {:?}",
        *calls
    );
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
    add_worktree(&repo, &wt);

    let repo_hash = detect_repo_hash(&repo).expect("repo hash");
    let index_root = tmp.path().join("index");
    let memory_dir = index_root.join(repo_hash.as_str()).join("memory");
    fs::create_dir_all(&memory_dir).expect("create repo-scoped memory dir");
    fs::write(memory_dir.join("chroma.sqlite3"), "fake-db").expect("write memory db");
    fs::write(memory_dir.join("meta.json"), r#"{"schema_version":1}"#).expect("write memory meta");

    let spawner = RecordingSpawner::default();
    bootstrap_project_index_for_path_with(&wt, &index_root, &spawner).expect("bootstrap index");

    assert!(
        memory_dir.join("chroma.sqlite3").exists(),
        "bootstrap must preserve the repo-scoped memory chroma.sqlite3"
    );
    assert!(
        memory_dir.join("meta.json").exists(),
        "bootstrap must preserve the repo-scoped memory meta.json"
    );
}

fn call_project_root_matches(call: &str, expected: &Path) -> bool {
    let Some(actual) = call.split('|').nth(1) else {
        return false;
    };
    normalize_path(actual) == normalize_path(expected)
}

fn normalize_path(path: impl AsRef<Path>) -> PathBuf {
    dunce::canonicalize(path.as_ref()).unwrap_or_else(|_| path.as_ref().to_path_buf())
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

fn add_worktree(repo: &Path, worktree: &Path) {
    let output = hidden_command("git")
        .args([
            "worktree",
            "add",
            "-b",
            "feature/shared",
            worktree.to_str().unwrap(),
        ])
        .current_dir(repo)
        .output()
        .expect("git worktree add");
    assert!(
        output.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
