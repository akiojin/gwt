//! End-to-end tests for the SPEC-1934 US-6 migration orchestrator.

use std::path::Path;

use gwt::migration::execute_migration;
use gwt_core::config::BareProjectConfig;
use gwt_core::migration::{MigrationOptions, MigrationPhase};

fn run_git(dir: &Path, args: &[&str]) {
    let output = gwt_core::process::hidden_command("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo_with_commit(path: &Path) {
    run_git(path, &["init", "."]);
    // Configure a deterministic identity so commits succeed in CI sandboxes.
    run_git(path, &["config", "user.email", "test@example.com"]);
    run_git(path, &["config", "user.name", "Test"]);
    std::fs::write(path.join("README.md"), "# sample\n").unwrap();
    run_git(path, &["add", "README.md"]);
    run_git(path, &["commit", "-m", "init"]);
}

fn current_branch(path: &Path) -> String {
    let output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(path)
        .output()
        .expect("symbolic-ref");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn t100_e2e_normal_to_nested_bare_worktree_layout() {
    let project = tempfile::tempdir().unwrap();
    init_repo_with_commit(project.path());
    let branch = current_branch(project.path());
    let project_dir_name = project
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo")
        .to_string();

    let outcome = execute_migration(
        project.path(),
        MigrationOptions::default(),
        |_phase, _pct| {},
    )
    .expect("execute_migration");

    let bare = project.path().join(format!("{project_dir_name}.git"));
    let worktree = project.path().join(&branch);

    assert!(bare.is_dir(), "bare repo must exist at {}", bare.display());
    assert!(
        worktree.is_dir(),
        "branch worktree must exist at {}",
        worktree.display()
    );
    assert!(
        worktree.join(".git").exists(),
        "worktree must carry a .git marker"
    );
    assert!(
        worktree.join("README.md").is_file(),
        "tracked content must land in the worktree"
    );

    // Old `.git` directory is gone, replaced by the bare repo.
    assert!(
        !project.path().join(".git").exists(),
        "old .git directory must be removed"
    );

    // .gwt/project.toml is written.
    let cfg = BareProjectConfig::load(project.path())
        .expect("load project.toml")
        .expect("project.toml exists");
    assert_eq!(cfg.bare_repo_name, format!("{project_dir_name}.git"));
    assert_eq!(cfg.migrated_from.as_deref(), Some("normal"));

    // Outcome reflects the layout.
    assert_eq!(outcome.bare_repo_path, bare);
    assert_eq!(outcome.branch_worktree_path, worktree);
}

#[test]
fn t100_e2e_progress_callback_walks_phases() {
    let project = tempfile::tempdir().unwrap();
    init_repo_with_commit(project.path());

    let mut seen: Vec<MigrationPhase> = Vec::new();
    execute_migration(project.path(), MigrationOptions::default(), |phase, _| {
        seen.push(phase)
    })
    .expect("execute_migration");

    // The orchestrator must advertise at minimum these milestones.
    for required in [
        MigrationPhase::Validate,
        MigrationPhase::Backup,
        MigrationPhase::Bareify,
        MigrationPhase::Worktrees,
        MigrationPhase::Submodules,
        MigrationPhase::Tracking,
        MigrationPhase::Cleanup,
        MigrationPhase::Done,
    ] {
        assert!(
            seen.contains(&required),
            "progress callback missing phase {required:?}, saw {seen:?}"
        );
    }
}
