use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use gwt::index_worker::bootstrap_project_index_for_path_with;
use gwt_core::{
    index::runtime::RunnerSpawner, repo_hash::detect_repo_hash,
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
            .unwrap_or_else(|p| p.into_inner())
            .push(format!(
                "{}|{}|{}",
                repo_hash,
                project_root.display(),
                respect_ttl
            ));
        Ok(())
    }
}

#[test]
fn bootstrap_helper_reconciles_index_layout_and_kicks_issue_refresh() {
    let tmp = tempfile::tempdir().expect("tempdir");
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
        })
        .to_string(),
    )
    .expect("write issues meta");

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

    let calls = spawner.calls.lock().unwrap_or_else(|p| p.into_inner());
    assert_eq!(
        calls.len(),
        1,
        "stale issue metadata should kick one refresh"
    );
    assert!(
        calls[0].contains(repo_hash.as_str()),
        "refresh must target the resolved repo hash, got {:?}",
        *calls
    );
    assert!(
        calls[0].contains(&wt.display().to_string()),
        "refresh should use the requested worktree path, got {:?}",
        *calls
    );
}

fn init_git_repo(path: &Path) {
    let output = std::process::Command::new("git")
        .args(["init", path.to_str().unwrap()])
        .output()
        .expect("git init");
    assert!(output.status.success(), "git init failed");

    let email = std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(path)
        .output()
        .expect("git config user.email");
    assert!(email.status.success(), "git config user.email failed");

    let name = std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(path)
        .output()
        .expect("git config user.name");
    assert!(name.status.success(), "git config user.name failed");
}

fn add_origin(path: &Path, url: &str) {
    let output = std::process::Command::new("git")
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
    let add = std::process::Command::new("git")
        .args(["add", name])
        .current_dir(path)
        .output()
        .expect("git add");
    assert!(add.status.success(), "git add failed");

    let commit = std::process::Command::new("git")
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
    let output = std::process::Command::new("git")
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
