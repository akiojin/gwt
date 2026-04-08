//! Background worker that owns vector index lifecycle for the TUI.
//!
//! Phase 8 / SPEC-10 FR-017〜FR-029. This module wraps a multi-thread tokio
//! runtime that lives for the entire TUI process and exposes synchronous
//! entrypoints the existing `app.rs` callers can use without having to
//! become async themselves.
//!
//! Responsibilities:
//! - Reconcile orphan worktree-hash directories on startup
//! - Refresh the Issue index according to a TTL window (background)
//! - Spawn / track / shut down per-Worktree filesystem watchers
//! - Trigger incremental index runs when watcher batches arrive
//!
//! Where ChromaDB writes happen: `crates/gwt-core/runtime/chroma_index_runner.py`.
//! This module never touches sqlite directly.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;

use gwt_core::error::Result;
use gwt_core::index::paths::gwt_index_root;
use gwt_core::index::runtime::{
    reconcile_repo, refresh_issues_if_stale, remove_worktree_index, PythonRunnerSpawner,
    ReconcileOptions, RefreshIssuesOptions,
};
use gwt_core::index::watcher::{start_watcher, WatcherConfig};
use gwt_core::paths::{gwt_project_index_venv_dir, gwt_runtime_runner_path};
use gwt_core::repo_hash::{compute_repo_hash, RepoHash};
use gwt_core::worktree_hash::{compute_worktree_hash, WorktreeHash};
use tokio::runtime::Runtime;

const ISSUE_REFRESH_TTL_MINUTES: u64 = 15;

/// Process-global tokio runtime owned by the worker. Lazily initialized.
fn worker_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("gwt-index-worker")
            .build()
            .expect("gwt index worker runtime")
    })
}

/// Tracks active watcher handles keyed by `worktree_hash`. Held inside the
/// global Mutex.
#[derive(Default)]
struct WatcherRegistry {
    handles: HashMap<String, tokio::task::JoinHandle<()>>,
    shutdown: HashMap<String, tokio::sync::oneshot::Sender<()>>,
}

fn registry() -> &'static Mutex<WatcherRegistry> {
    static REG: OnceLock<Mutex<WatcherRegistry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(WatcherRegistry::default()))
}

fn make_runner_spawner() -> PythonRunnerSpawner {
    PythonRunnerSpawner {
        python_executable: gwt_project_index_venv_dir().join(if cfg!(windows) {
            "Scripts/python.exe"
        } else {
            "bin/python3"
        }),
        runner_script: gwt_runtime_runner_path(),
    }
}

/// Determine `RepoHash` for the given repository root by shelling out to
/// `git remote get-url origin`. Returns `None` if no origin is configured.
pub fn detect_repo_hash(repo_root: &Path) -> Option<RepoHash> {
    let output = std::process::Command::new("git")
        .arg("remote")
        .arg("get-url")
        .arg("origin")
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        return None;
    }
    Some(compute_repo_hash(&url))
}

/// Reconcile + start background Issue refresh + start watchers for the
/// active worktrees of `repo_root`. Called once at TUI startup.
#[tracing::instrument(
    name = "index_worker_bootstrap",
    skip(active_worktrees),
    fields(repo_root = %repo_root.display(), worktrees = active_worktrees.len())
)]
pub fn bootstrap(repo_root: &Path, active_worktrees: &[PathBuf]) {
    let Some(repo_hash) = detect_repo_hash(repo_root) else {
        tracing::debug!("no origin remote configured; skipping index bootstrap");
        return;
    };

    // 1) Reconcile orphans + legacy directories — synchronous, fast.
    let opts = ReconcileOptions {
        index_root: gwt_index_root(),
        repo_hash: repo_hash.clone(),
        active_worktree_paths: active_worktrees.to_vec(),
        legacy_worktree_dirs: active_worktrees.to_vec(),
    };
    if let Err(e) = reconcile_repo(&opts) {
        tracing::warn!("index reconcile failed: {e}");
    }

    // 2) Background Issue refresh.
    let project_root = repo_root.to_path_buf();
    let repo_hash_for_issues = repo_hash.clone();
    worker_runtime().spawn(async move {
        let opts = RefreshIssuesOptions {
            index_root: gwt_index_root(),
            repo_hash: repo_hash_for_issues,
            project_root,
            ttl: Duration::from_secs(ISSUE_REFRESH_TTL_MINUTES * 60),
        };
        let spawner = make_runner_spawner();
        if let Err(e) = refresh_issues_if_stale(&opts, &spawner).await {
            tracing::warn!("issue refresh kick failed: {e}");
        }
    });

    // 3) Start a watcher per active Worktree.
    for wt in active_worktrees {
        ensure_watcher(repo_root, wt);
    }
}

/// Idempotently ensure that a watcher is running for the given Worktree.
pub fn ensure_watcher(repo_root: &Path, worktree_path: &Path) {
    let Some(repo_hash) = detect_repo_hash(repo_root) else {
        return;
    };
    let Ok(wt_hash) = compute_worktree_hash(worktree_path) else {
        return;
    };
    let key = wt_hash.as_str().to_string();

    {
        let reg = registry().lock().unwrap();
        if reg.handles.contains_key(&key) {
            return;
        }
    }

    let worktree_path = worktree_path.to_path_buf();
    let repo_root = repo_root.to_path_buf();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    let handle = worker_runtime().spawn(async move {
        let cfg = WatcherConfig::default();
        let mut watcher = match start_watcher(&worktree_path, cfg) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("watcher start failed for {}: {e}", worktree_path.display());
                return;
            }
        };
        let spawner = make_runner_spawner();
        let mut shutdown_rx = rx;
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                batch = watcher.recv_batch() => {
                    let Some(_batch) = batch else { break };
                    if let Err(e) = run_incremental_index(&spawner, &repo_hash, &wt_hash, &repo_root) {
                        tracing::warn!("incremental index spawn failed: {e}");
                    }
                }
            }
        }
        watcher.shutdown().await;
    });

    let mut reg = registry().lock().unwrap();
    reg.handles.insert(key.clone(), handle);
    reg.shutdown.insert(key, tx);
}

/// Stop the watcher for `worktree_path` (if running) and remove its on-disk
/// index directory. Called by the gwt TUI Worktree-remove handler.
pub fn shutdown_and_remove(repo_root: &Path, worktree_path: &Path) -> Result<()> {
    let Ok(wt_hash) = compute_worktree_hash(worktree_path) else {
        return Ok(());
    };
    let key = wt_hash.as_str().to_string();

    {
        let mut reg = registry().lock().unwrap();
        if let Some(tx) = reg.shutdown.remove(&key) {
            let _ = tx.send(());
        }
        reg.handles.remove(&key);
    }

    if let Some(repo_hash) = detect_repo_hash(repo_root) {
        remove_worktree_index(&gwt_index_root(), &repo_hash, wt_hash.as_str())?;
    }

    Ok(())
}

fn run_incremental_index(
    _spawner: &PythonRunnerSpawner,
    repo_hash: &RepoHash,
    wt_hash: &WorktreeHash,
    project_root: &Path,
) -> std::io::Result<()> {
    // We piggy-back on the same runner script with --action index-files
    // --mode incremental. The spawner doesn't currently expose this so do
    // a direct std::process::Command spawn here.
    let python = gwt_project_index_venv_dir().join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python3"
    });
    let runner = gwt_runtime_runner_path();
    if !python.exists() || !runner.exists() {
        return Ok(());
    }

    for scope in ["files", "files-docs", "specs"] {
        let action = if scope == "specs" {
            "index-specs"
        } else {
            "index-files"
        };
        let _ = std::process::Command::new(&python)
            .arg(&runner)
            .arg("--action")
            .arg(action)
            .arg("--repo-hash")
            .arg(repo_hash.as_str())
            .arg("--worktree-hash")
            .arg(wt_hash.as_str())
            .arg("--project-root")
            .arg(project_root)
            .arg("--mode")
            .arg("incremental")
            .arg("--scope")
            .arg(scope)
            .spawn()?;
    }
    Ok(())
}
