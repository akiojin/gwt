//! Git operations benchmarks

use criterion::{criterion_group, criterion_main, Criterion};
use gwt_core::git::Repository;
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

fn git_discover_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let path = temp.path().to_path_buf();

    c.bench_function("git_discover", |b| {
        b.iter(|| Repository::discover(&path).unwrap())
    });
}

fn git_open_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let path = temp.path().to_path_buf();

    c.bench_function("git_open", |b| b.iter(|| Repository::open(&path).unwrap()));
}

fn git_has_uncommitted_changes_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let repo = Repository::discover(temp.path()).unwrap();

    c.bench_function("git_has_uncommitted_changes", |b| {
        b.iter(|| repo.has_uncommitted_changes().unwrap())
    });
}

fn git_head_name_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let repo = Repository::discover(temp.path()).unwrap();

    c.bench_function("git_head_name", |b| b.iter(|| repo.head_name().unwrap()));
}

fn git_list_worktrees_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let repo = Repository::discover(temp.path()).unwrap();

    c.bench_function("git_list_worktrees", |b| {
        b.iter(|| repo.list_worktrees().unwrap())
    });
}

criterion_group!(
    benches,
    git_discover_benchmark,
    git_open_benchmark,
    git_has_uncommitted_changes_benchmark,
    git_head_name_benchmark,
    git_list_worktrees_benchmark,
);
criterion_main!(benches);
