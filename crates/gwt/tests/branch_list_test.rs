use gwt::{list_branch_entries, BranchScope};
use tempfile::tempdir;

#[test]
fn list_branch_entries_marks_head_and_returns_local_branches() {
    let dir = tempdir().expect("tempdir");

    run_git(dir.path(), &["init", "-q"]);
    run_git(dir.path(), &["config", "user.name", "PoC Tester"]);
    run_git(dir.path(), &["config", "user.email", "poc@example.com"]);
    std::fs::write(dir.path().join("README.md"), "# demo\n").expect("write readme");
    run_git(dir.path(), &["add", "README.md"]);
    run_git(dir.path(), &["commit", "-qm", "init"]);
    run_git(dir.path(), &["branch", "-M", "main"]);
    run_git(dir.path(), &["branch", "feature/alpha"]);

    let branches = list_branch_entries(dir.path()).expect("branch entries");

    assert!(branches.iter().any(|branch| {
        branch.name == "main" && branch.is_head && branch.scope == BranchScope::Local
    }));
    assert!(branches.iter().any(|branch| {
        branch.name == "feature/alpha" && !branch.is_head && branch.scope == BranchScope::Local
    }));
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
