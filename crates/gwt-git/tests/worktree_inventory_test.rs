use std::{path::Path, process::Command, sync::Barrier, time::Instant};

use gwt_git::WorktreeManager;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repository() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-b", "main"]);
    git(
        root.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    root
}

#[test]
fn inventory_collapses_callers_and_ticks() {
    let fixture = repository();
    let root = fixture.path();
    let baseline_before = gwt_core::process::thread_git_spawn_count();
    let baseline_start = Instant::now();
    for _tick in 0..5 {
        for _caller in 0..12 {
            let output = gwt_core::process::run_git_logged(
                &["worktree", "list", "--porcelain"],
                Some(&root),
            )
            .unwrap();
            assert!(output.status.success());
        }
    }
    println!(
        "baseline: callers=12 ticks=5 requests=60 git_spawns={} elapsed_ms={:.3}",
        gwt_core::process::thread_git_spawn_count() - baseline_before,
        baseline_start.elapsed().as_secs_f64() * 1000.0
    );
    let before = gwt_core::process::thread_git_spawn_count();
    let start = Instant::now();
    let mut count = None;
    for _tick in 0..5 {
        for _caller in 0..12 {
            let entries = WorktreeManager::new(&root).list().unwrap();
            assert!(!entries.is_empty());
            assert_eq!(*count.get_or_insert(entries.len()), entries.len());
        }
    }
    let spawns = gwt_core::process::thread_git_spawn_count() - before;
    println!(
        "inventory: worktrees={count:?} callers=12 ticks=5 requests=60 git_spawns={spawns} elapsed_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(
        spawns, 1,
        "unchanged repository has one inventory generation"
    );
}

#[test]
fn inventory_shares_concurrent_requests_and_refreshes_before_next_read() {
    let root = repository();
    let barrier = Barrier::new(8);
    let spawns = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    let before = gwt_core::process::thread_git_spawn_count();
                    WorktreeManager::new(root.path()).list().unwrap();
                    gwt_core::process::thread_git_spawn_count() - before
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum::<u64>()
    });
    assert_eq!(
        spawns, 1,
        "simultaneous cold callers share the Git execution"
    );

    let linked = root.path().join("linked");
    git(
        root.path(),
        &["config", "worktree.useRelativePaths", "true"],
    );
    git(
        root.path(),
        &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
    );
    let entries = WorktreeManager::new(root.path()).list().unwrap();
    assert_eq!(
        entries.len(),
        2,
        "no sleep between mutation and next reference"
    );
    let before = gwt_core::process::thread_git_spawn_count();
    assert_eq!(WorktreeManager::new(&linked).list().unwrap().len(), 2);
    assert_eq!(
        gwt_core::process::thread_git_spawn_count(),
        before,
        "linked and main roots share a repository key"
    );
    git(root.path(), &["worktree", "lock", linked.to_str().unwrap()]);
    assert!(WorktreeManager::new(&linked)
        .list()
        .unwrap()
        .iter()
        .any(|entry| entry.locked));
    git(
        root.path(),
        &["worktree", "unlock", linked.to_str().unwrap()],
    );
    git(&linked, &["checkout", "--detach"]);
    assert!(WorktreeManager::new(root.path())
        .list()
        .unwrap()
        .iter()
        .any(|entry| entry.branch.is_none()));
    let git_file = std::fs::read(linked.join(".git")).unwrap();
    std::fs::remove_file(linked.join(".git")).unwrap();
    assert!(
        WorktreeManager::new(root.path())
            .list()
            .unwrap()
            .iter()
            .any(|entry| entry.prunable),
        "external removal is visible even with relative gitdir paths"
    );
    std::fs::write(linked.join(".git"), git_file).unwrap();
    git(
        root.path(),
        &["worktree", "remove", linked.to_str().unwrap()],
    );
    assert_eq!(WorktreeManager::new(root.path()).list().unwrap().len(), 1);

    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(Instant::now());
    assert!(
        WorktreeManager::new(root.path())
            .list()
            .unwrap_err()
            .to_string()
            .contains("deadline"),
        "a cache hit cannot bypass the caller deadline"
    );
}

#[test]
fn inventory_is_shared_between_processes() {
    if let Some(root) = std::env::var_os("GWT_INVENTORY_TEST_REPOSITORY") {
        let before = gwt_core::process::thread_git_spawn_count();
        assert_eq!(WorktreeManager::new(root).list().unwrap().len(), 1);
        assert_eq!(
            gwt_core::process::thread_git_spawn_count(),
            before,
            "another process already published this generation"
        );
        return;
    }
    let root = repository();
    WorktreeManager::new(root.path()).list().unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "inventory_is_shared_between_processes",
            "--nocapture",
        ])
        .env("GWT_INVENTORY_TEST_REPOSITORY", root.path())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
}
