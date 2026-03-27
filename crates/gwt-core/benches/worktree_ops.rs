//! Worktree operations benchmarks

use criterion::{criterion_group, criterion_main, Criterion};
use gwt_core::worktree::WorktreeManager;
use tempfile::TempDir;

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let output = gwt_core::process::git_command()
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command should spawn");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn create_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "test@test.com"]);
    run_git(temp.path(), &["config", "user.name", "Test User"]);
    std::fs::write(temp.path().join("README.md"), "# Test").unwrap();
    run_git(temp.path(), &["add", "."]);
    run_git(temp.path(), &["commit", "-m", "Initial commit"]);
    temp
}

fn worktree_list_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let manager = WorktreeManager::new(temp.path()).unwrap();

    c.bench_function("worktree_list", |b| b.iter(|| manager.list().unwrap()));
}

fn worktree_manager_new_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let path = temp.path().to_path_buf();

    c.bench_function("worktree_manager_new", |b| {
        b.iter(|| WorktreeManager::new(&path).unwrap())
    });
}

fn worktree_get_by_branch_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let manager = WorktreeManager::new(temp.path()).unwrap();

    c.bench_function("worktree_get_by_branch", |b| {
        b.iter(|| manager.get_by_branch("main").unwrap())
    });
}

fn worktree_active_count_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let manager = WorktreeManager::new(temp.path()).unwrap();

    c.bench_function("worktree_active_count", |b| {
        b.iter(|| manager.active_count().unwrap())
    });
}

criterion_group!(
    benches,
    worktree_list_benchmark,
    worktree_manager_new_benchmark,
    worktree_get_by_branch_benchmark,
    worktree_active_count_benchmark,
);
criterion_main!(benches);
