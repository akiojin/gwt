use std::{path::Path, process::Command};

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
fn start_work_launch_confirmation_falls_back_when_develop_tracking_ref_is_stale() {
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
        "stale origin/develop should fall back to origin/HEAD"
    );
    assert!(
        !worktree.join("develop-only.txt").exists(),
        "deleted upstream develop must not be used as the Start Work base"
    );
}
