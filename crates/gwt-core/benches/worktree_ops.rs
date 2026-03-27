//! Worktree operations benchmarks

use criterion::{criterion_group, criterion_main, Criterion};
use gwt_core::worktree::WorktreeManager;
use tempfile::TempDir;

fn run_git_checked(repo_path: &std::path::Path, args: &[&str]) {
    let output = gwt_core::process::git_command()
        .args(args)
        .current_dir(repo_path)
        .output()
        .expect("git fixture command should spawn");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn create_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    run_git_checked(temp.path(), &["init"]);
    run_git_checked(temp.path(), &["config", "user.email", "test@test.com"]);
    run_git_checked(temp.path(), &["config", "user.name", "Test User"]);
    // Create initial commit
    std::fs::write(temp.path().join("README.md"), "# Test").unwrap();
    run_git_checked(temp.path(), &["add", "."]);
    run_git_checked(temp.path(), &["commit", "-m", "Initial commit"]);
    temp
}

fn create_test_repo_with_worktrees() -> TempDir {
    let temp = create_test_repo();
    let repo_path = temp.path();

    for branch in ["feature/bench-a", "feature/bench-b", "feature/bench-c"] {
        run_git_checked(repo_path, &["branch", branch]);

        let worktree_path = repo_path.join(".worktrees").join(branch.replace('/', "-"));
        std::fs::create_dir_all(worktree_path.parent().unwrap()).unwrap();
        let output = gwt_core::process::git_command()
            .arg("worktree")
            .arg("add")
            .arg(worktree_path.to_string_lossy().into_owned())
            .arg(branch)
            .current_dir(repo_path)
            .output()
            .expect("git worktree add should spawn");
        assert!(
            output.status.success(),
            "git worktree add failed for {}: {}",
            branch,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    temp
}

fn worktree_list_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let manager = WorktreeManager::new(temp.path()).unwrap();

    c.bench_function("worktree_list", |b| b.iter(|| manager.list().unwrap()));
}

fn worktree_list_basic_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let manager = WorktreeManager::new(temp.path()).unwrap();

    c.bench_function("worktree_list_basic", |b| {
        b.iter(|| manager.list_basic().unwrap())
    });
}

fn worktree_list_vs_basic_multi_worktree_benchmark(c: &mut Criterion) {
    let temp = create_test_repo_with_worktrees();
    let manager = WorktreeManager::new(temp.path()).unwrap();

    c.bench_function("worktree_list_multi", |b| {
        b.iter(|| manager.list().unwrap())
    });
    c.bench_function("worktree_list_basic_multi", |b| {
        b.iter(|| manager.list_basic().unwrap())
    });
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
    worktree_list_basic_benchmark,
    worktree_list_vs_basic_multi_worktree_benchmark,
    worktree_manager_new_benchmark,
    worktree_get_by_branch_benchmark,
    worktree_active_count_benchmark,
);
criterion_main!(benches);
