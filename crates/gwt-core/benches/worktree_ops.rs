//! Worktree operations benchmarks

use criterion::{criterion_group, criterion_main, Criterion};
use gwt_core::worktree::WorktreeManager;
use std::process::Command;
use tempfile::TempDir;

fn create_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new("git")
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    // Create initial commit
    std::fs::write(temp.path().join("README.md"), "# Test").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(temp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(temp.path())
        .output()
        .unwrap();
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
