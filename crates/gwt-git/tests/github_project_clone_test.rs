use std::path::{Path, PathBuf};

use gwt_core::config::BareProjectConfig;
use gwt_git::{clone_project_as_nested_bare, detect_repo_type, RepoType};
use tempfile::TempDir;

fn git(args: &[&str], cwd: &Path) {
    let output = gwt_core::process::hidden_command("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("git {args:?}: {error}"));
    assert!(
        output.status.success(),
        "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn remote_repo(default_branch: &str, include_develop: bool) -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("remote tempdir");
    let source = temp.path().join("source");
    let bare = temp.path().join("sample.git");
    std::fs::create_dir_all(&source).expect("source dir");
    git(&["init", "-b", default_branch], &source);
    git(&["config", "user.email", "gwt@example.com"], &source);
    git(&["config", "user.name", "GWT Test"], &source);
    std::fs::write(source.join("README.md"), "# Sample\n").expect("readme");
    git(&["add", "README.md"], &source);
    git(&["commit", "-m", "initial"], &source);
    if include_develop {
        git(&["checkout", "-b", "develop"], &source);
        std::fs::write(source.join("develop.txt"), "develop\n").expect("develop");
        git(&["add", "develop.txt"], &source);
        git(&["commit", "-m", "develop"], &source);
        git(&["checkout", default_branch], &source);
    }
    git(
        &[
            "clone",
            "--bare",
            source.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
        temp.path(),
    );
    (temp, bare)
}

#[test]
fn clone_project_as_nested_bare_creates_develop_worktree_and_project_config() {
    let (_remote_temp, remote) = remote_repo("main", true);
    let parent = tempfile::tempdir().expect("parent tempdir");

    let outcome = clone_project_as_nested_bare(remote.to_str().unwrap(), parent.path())
        .expect("clone project");

    assert_eq!(outcome.workspace_home, parent.path().join("sample"));
    assert_eq!(
        outcome.bare_repo_path,
        outcome.workspace_home.join("sample.git")
    );
    assert_eq!(outcome.initial_branch, "develop");
    assert_eq!(
        outcome.initial_worktree_path,
        outcome.workspace_home.join("develop")
    );
    assert!(outcome.bare_repo_path.join("HEAD").is_file());
    assert!(outcome.initial_worktree_path.join(".git").is_file());
    assert!(matches!(
        detect_repo_type(&outcome.workspace_home),
        RepoType::Bare {
            develop_worktree: Some(path)
        } if path == outcome.initial_worktree_path
    ));

    let config = BareProjectConfig::load(&outcome.workspace_home)
        .expect("load project config")
        .expect("project config exists");
    assert_eq!(config.bare_repo_name, "sample.git");
    assert_eq!(config.remote_url, Some(remote.display().to_string()));
    assert_eq!(config.migrated_from, None);
}

#[test]
fn clone_project_as_nested_bare_falls_back_to_default_branch_without_develop() {
    let (_remote_temp, remote) = remote_repo("main", false);
    let parent = tempfile::tempdir().expect("parent tempdir");

    let outcome = clone_project_as_nested_bare(remote.to_str().unwrap(), parent.path())
        .expect("clone project");

    assert_eq!(outcome.initial_branch, "main");
    assert_eq!(
        outcome.initial_worktree_path,
        outcome.workspace_home.join("main")
    );
    assert!(outcome.initial_worktree_path.join(".git").is_file());
}

#[test]
fn clone_project_as_nested_bare_refuses_existing_target_directory() {
    let (_remote_temp, remote) = remote_repo("main", true);
    let parent = tempfile::tempdir().expect("parent tempdir");
    std::fs::create_dir_all(parent.path().join("sample")).expect("existing target");

    let error = clone_project_as_nested_bare(remote.to_str().unwrap(), parent.path())
        .expect_err("existing target must fail");

    assert!(
        error.to_string().contains("already exists"),
        "unexpected error: {error}"
    );
    assert!(
        !parent.path().join("sample").join("sample.git").exists(),
        "clone must not write into an existing target"
    );
}
