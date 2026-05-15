use std::{
    path::{Path, PathBuf},
    process::Command,
};

use chrono::{TimeZone, Utc};
use tempfile::tempdir;

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git command");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_ref_exists(repo: &Path, refname: &str) -> bool {
    let status = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", refname])
        .current_dir(repo)
        .status()
        .expect("git show-ref");
    status.success()
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

fn init_git_clone_with_origin(repo: &Path) {
    let root = repo.parent().expect("repo parent");
    let seed = root.join("seed");
    let origin = root.join("origin.git");

    std::fs::create_dir_all(&seed).expect("create seed");
    run_git(root, &["init", "-q", "-b", "develop", "seed"]);
    run_git(&seed, &["config", "user.name", "Codex Test"]);
    run_git(&seed, &["config", "user.email", "codex@example.com"]);
    std::fs::write(seed.join("README.md"), "seed\n").expect("write seed");
    run_git(&seed, &["add", "README.md"]);
    run_git(&seed, &["commit", "-qm", "init"]);
    run_git(&seed, &["checkout", "-qb", "main"]);
    std::fs::write(seed.join("main-only.txt"), "main\n").expect("write main marker");
    run_git(&seed, &["add", "main-only.txt"]);
    run_git(&seed, &["commit", "-qm", "main marker"]);
    run_git(&seed, &["checkout", "develop"]);
    std::fs::write(seed.join("develop-only.txt"), "develop\n").expect("write develop marker");
    run_git(&seed, &["add", "develop-only.txt"]);
    run_git(&seed, &["commit", "-qm", "develop marker"]);
    run_git(&seed, &["checkout", "main"]);

    let status = Command::new("git")
        .args(["clone", "--bare"])
        .arg(&seed)
        .arg(&origin)
        .status()
        .expect("git clone --bare");
    assert!(status.success(), "git clone --bare failed");

    let status = Command::new("git")
        .args(["clone"])
        .arg(&origin)
        .arg(repo)
        .status()
        .expect("git clone repo");
    assert!(status.success(), "git clone repo failed");
    run_git(repo, &["config", "user.name", "Codex Test"]);
    run_git(repo, &["config", "user.email", "codex@example.com"]);
}

fn delete_origin_branch(repo: &Path, branch: &str) {
    let origin = repo.parent().expect("repo parent").join("origin.git");
    run_git(&origin, &["branch", "-D", branch]);
}

fn init_main_only_bare_workspace_home(workspace_home: &Path) -> (PathBuf, PathBuf) {
    let root = workspace_home.parent().expect("workspace parent");
    let seed = root.join("main-only-seed");
    let origin = root.join("main-only-origin.git");
    let bare_repo = workspace_home.join("sample.git");

    std::fs::create_dir_all(&seed).expect("create seed");
    run_git(root, &["init", "-q", "-b", "main", "main-only-seed"]);
    run_git(&seed, &["config", "user.name", "Codex Test"]);
    run_git(&seed, &["config", "user.email", "codex@example.com"]);
    std::fs::write(seed.join("main-only.txt"), "main\n").expect("write main marker");
    run_git(&seed, &["add", "main-only.txt"]);
    run_git(&seed, &["commit", "-qm", "init"]);

    let status = Command::new("git")
        .args(["clone", "--bare"])
        .arg(&seed)
        .arg(&origin)
        .status()
        .expect("git clone --bare origin");
    assert!(status.success(), "git clone --bare origin failed");

    std::fs::create_dir_all(workspace_home).expect("create workspace home");
    let status = Command::new("git")
        .args(["clone", "--bare"])
        .arg(&origin)
        .arg(&bare_repo)
        .status()
        .expect("git clone --bare workspace");
    assert!(status.success(), "git clone --bare workspace failed");

    let _ = Command::new("git")
        .args(["config", "--unset-all", "remote.origin.fetch"])
        .current_dir(&bare_repo)
        .status();

    (bare_repo, origin)
}

#[test]
fn start_work_launch_confirmation_materializes_reserved_work_branch_and_worktree() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);

    let base_branch =
        gwt::start_work::resolve_start_work_base_branch(&repo).expect("start work base branch");
    let reserved_branch = gwt::start_work::reserve_start_work_branch_name(
        &repo,
        Utc.with_ymd_and_hms(2026, 5, 4, 12, 34, 0)
            .single()
            .expect("timestamp"),
    )
    .expect("reserve start work branch name");
    let refs_before = git_output(&repo, &["for-each-ref", "refs/heads/work"]);
    assert!(
        refs_before.is_empty(),
        "Start Work open must not create refs"
    );

    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch(&reserved_branch)
        .base_branch(&base_branch)
        .build();
    assert!(
        config.working_dir.is_none(),
        "worktree materialization is deferred until launch confirmation"
    );

    gwt_agent::resolve_launch_worktree(&repo, &mut config).expect("materialize launch worktree");

    let worktree = config.working_dir.expect("materialized worktree");
    assert!(worktree.exists(), "worktree should exist after launch");
    assert_eq!(
        git_output(&worktree, &["branch", "--show-current"]),
        reserved_branch
    );
    assert_eq!(
        git_output(&repo, &["rev-parse", "--abbrev-ref", "origin/HEAD"]),
        "origin/main"
    );
    assert_eq!(base_branch, "origin/develop");
    assert!(
        worktree.join("develop-only.txt").is_file(),
        "Start Work branch should be created from origin/develop"
    );
    assert!(
        !worktree.join("main-only.txt").exists(),
        "Start Work branch must not inherit origin/HEAD content when HEAD points to main"
    );
    assert!(git_output(&repo, &["for-each-ref", "refs/heads/work"]).contains(&reserved_branch));
}

#[test]
fn start_work_launch_confirmation_recreates_remote_develop_when_tracking_ref_is_stale() {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    init_git_clone_with_origin(&repo);
    delete_origin_branch(&repo, "develop");

    let base_branch =
        gwt::start_work::resolve_start_work_base_branch(&repo).expect("start work base branch");
    let reserved_branch = "work/stale-develop";
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch(reserved_branch)
        .base_branch(&base_branch)
        .build();

    gwt_agent::resolve_launch_worktree(&repo, &mut config).expect("materialize launch worktree");

    let worktree = config.working_dir.expect("materialized worktree");
    assert_eq!(base_branch, "origin/develop");
    assert_eq!(
        git_output(&worktree, &["branch", "--show-current"]),
        reserved_branch
    );
    assert!(
        worktree.join("main-only.txt").is_file(),
        "stale origin/develop should be recreated from the remote default branch"
    );
    assert!(
        !worktree.join("develop-only.txt").exists(),
        "deleted upstream develop must not be used as the Start Work base"
    );
    assert!(
        !git_output(
            &repo,
            &["show-ref", "--verify", "refs/remotes/origin/develop"]
        )
        .is_empty(),
        "origin/develop tracking ref must be restored"
    );
}

#[test]
fn start_work_resolves_main_only_bare_workspace_home_by_creating_remote_develop() {
    let temp = tempdir().expect("tempdir");
    let workspace_home = temp.path().join("sample");
    let (bare_repo, origin) = init_main_only_bare_workspace_home(&workspace_home);

    let base_branch = gwt::start_work::resolve_start_work_base_branch(&workspace_home)
        .expect("start work base branch");

    assert_eq!(base_branch, "origin/develop");
    assert!(
        !workspace_home.join("main").exists(),
        "Start Work base resolution must not create a local main worktree"
    );
    assert!(
        !workspace_home.join("develop").exists(),
        "Start Work base resolution must not create a local develop worktree"
    );
    assert_eq!(
        git_output(&origin, &["show-ref", "--verify", "refs/heads/develop"])
            .split_whitespace()
            .last(),
        Some("refs/heads/develop"),
        "remote develop must be created from the remote default branch"
    );
    assert_eq!(
        git_output(
            &bare_repo,
            &["show-ref", "--verify", "refs/remotes/origin/develop"]
        )
        .split_whitespace()
        .last(),
        Some("refs/remotes/origin/develop"),
        "local origin/develop tracking ref must be fetched"
    );

    let reserved_branch = "work/main-only";
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch(reserved_branch)
        .base_branch(&base_branch)
        .build();

    gwt_agent::resolve_launch_worktree(&workspace_home, &mut config)
        .expect("materialize launch worktree");

    let worktree = config.working_dir.expect("materialized worktree");
    assert_eq!(
        git_output(&worktree, &["branch", "--show-current"]),
        reserved_branch
    );
    assert!(
        worktree.join("main-only.txt").is_file(),
        "Start Work branch should be created from the newly-created origin/develop"
    );
    assert_eq!(
        git_output(
            &origin,
            &["show-ref", "--verify", "refs/heads/work/main-only"]
        )
        .split_whitespace()
        .last(),
        Some("refs/heads/work/main-only"),
        "remote work branch must be pushed from origin/develop"
    );
}

#[test]
fn start_work_launch_fails_without_work_branch_when_remote_develop_creation_fails() {
    let temp = tempdir().expect("tempdir");
    let workspace_home = temp.path().join("sample");
    let (bare_repo, origin) = init_main_only_bare_workspace_home(&workspace_home);
    install_reject_develop_hook(&origin);
    let reserved_branch = "work/rejected-develop";
    let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
        .branch(reserved_branch)
        .base_branch("origin/develop")
        .build();

    let error = gwt_agent::resolve_launch_worktree(&workspace_home, &mut config)
        .expect_err("remote develop creation must fail");

    assert!(
        error.contains("origin/develop") || error.contains("develop rejected by test hook"),
        "error should explain the failed develop preparation: {error}"
    );
    assert!(
        config.working_dir.is_none(),
        "failed launch materialization must not assign a worktree"
    );
    assert!(
        !workspace_home
            .join("work")
            .join("rejected-develop")
            .exists(),
        "failed launch materialization must not leave a local worktree"
    );
    assert!(
        !git_ref_exists(&origin, "refs/heads/work/rejected-develop"),
        "failed develop preparation must not create a remote work branch"
    );
    assert!(
        !git_ref_exists(&bare_repo, "refs/remotes/origin/work/rejected-develop"),
        "failed develop preparation must not fetch a remote work tracking ref"
    );
}
