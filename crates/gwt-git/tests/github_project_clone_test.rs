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

fn git_output(args: &[&str], cwd: &Path) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn install_reject_develop_hook(remote: &Path) {
    let hook = remote.join("hooks").join("pre-receive");
    std::fs::write(
        &hook,
        "#!/bin/sh\nwhile read _old _new ref; do\n  if [ \"$ref\" = \"refs/heads/develop\" ]; then\n    echo 'develop rejected by test hook' >&2\n    exit 1\n  fi\ndone\nexit 0\n",
    )
    .expect("write pre-receive hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
            .expect("chmod pre-receive hook");
    }
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
fn clone_project_as_nested_bare_creates_workspace_home_without_default_worktree() {
    let (_remote_temp, remote) = remote_repo("main", true);
    let parent = tempfile::tempdir().expect("parent tempdir");

    let outcome = clone_project_as_nested_bare(remote.to_str().unwrap(), parent.path())
        .expect("clone project");

    assert_eq!(outcome.workspace_home, parent.path().join("sample"));
    assert_eq!(
        outcome.bare_repo_path,
        outcome.workspace_home.join("sample.git")
    );
    assert!(outcome.bare_repo_path.join("HEAD").is_file());
    assert!(
        !outcome.workspace_home.join("develop").exists(),
        "Clone Project must not create a local develop worktree"
    );
    assert!(
        !outcome.workspace_home.join("main").exists(),
        "Clone Project must not create a local main worktree"
    );
    assert!(matches!(
        detect_repo_type(&outcome.workspace_home),
        RepoType::Bare {
            develop_worktree: None
        }
    ));
    assert_eq!(
        git_output(
            &["config", "--get", "remote.origin.fetch"],
            &outcome.bare_repo_path
        ),
        "+refs/heads/*:refs/remotes/origin/*"
    );
    assert_eq!(
        git_output(
            &["show-ref", "--verify", "refs/remotes/origin/develop"],
            &outcome.bare_repo_path
        )
        .split_whitespace()
        .last(),
        Some("refs/remotes/origin/develop")
    );

    let config = BareProjectConfig::load(&outcome.workspace_home)
        .expect("load project config")
        .expect("project config exists");
    assert_eq!(config.bare_repo_name, "sample.git");
    assert_eq!(config.remote_url, Some(remote.display().to_string()));
    assert_eq!(config.migrated_from, None);
}

#[test]
fn clone_project_as_nested_bare_creates_remote_develop_without_local_default_worktree() {
    let (_remote_temp, remote) = remote_repo("main", false);
    let parent = tempfile::tempdir().expect("parent tempdir");

    let outcome = clone_project_as_nested_bare(remote.to_str().unwrap(), parent.path())
        .expect("clone project");

    assert!(
        !outcome.workspace_home.join("main").exists(),
        "Clone Project must not create a local main worktree"
    );
    assert!(
        !outcome.workspace_home.join("develop").exists(),
        "Clone Project must not create a local develop worktree"
    );
    assert_eq!(
        git_output(&["show-ref", "--verify", "refs/heads/develop"], &remote)
            .split_whitespace()
            .last(),
        Some("refs/heads/develop"),
        "remote develop must be created from the remote default branch"
    );
    assert_eq!(
        git_output(
            &["show-ref", "--verify", "refs/remotes/origin/develop"],
            &outcome.bare_repo_path
        )
        .split_whitespace()
        .last(),
        Some("refs/remotes/origin/develop")
    );
}

#[test]
fn clone_project_as_nested_bare_cleans_up_when_remote_develop_creation_fails() {
    let (_remote_temp, remote) = remote_repo("main", false);
    install_reject_develop_hook(&remote);
    let parent = tempfile::tempdir().expect("parent tempdir");
    let workspace_home = parent.path().join("sample");

    let error = clone_project_as_nested_bare(remote.to_str().unwrap(), parent.path())
        .expect_err("remote develop creation must fail");

    assert!(
        error.to_string().contains("origin/develop")
            || error.to_string().contains("develop rejected by test hook"),
        "error should explain the failed develop preparation: {error}"
    );
    assert!(
        !workspace_home.exists(),
        "failed Clone Project must clean up the partial Workspace Home"
    );
    assert!(
        git_output(&["for-each-ref", "refs/heads/work"], &remote).is_empty(),
        "failed develop preparation must not create remote work branches"
    );
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
