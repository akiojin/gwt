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
//! - Capture every runner spawn into `~/.gwt/logs/index.log` so the user (and
//!   any helper agent) can audit the lifecycle without console noise.
//!
//! Where ChromaDB writes happen: `crates/gwt-core/runtime/chroma_index_runner.py`.
//! This module never touches sqlite directly.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gwt_core::error::Result;
use gwt_core::index::paths::gwt_index_root;
use gwt_core::index::runtime::{
    reconcile_repo, refresh_issues_if_stale, remove_worktree_index, PythonRunnerSpawner,
    ReconcileOptions, RefreshIssuesOptions,
};
use gwt_core::index::watcher::{start_watcher, WatcherConfig};
use gwt_core::paths::{gwt_logs_dir, gwt_project_index_venv_dir, gwt_runtime_runner_path};
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

// =====================================================================
// Logging — `~/.gwt/logs/index.log`
// =====================================================================

fn index_log_path() -> PathBuf {
    gwt_logs_dir().join("index.log")
}

/// Append a single timestamped line to `~/.gwt/logs/index.log`. Failures are
/// silently ignored — logging must never block index lifecycle work.
pub fn log_event(message: &str) {
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
    let line = format!("[{ts}] {message}\n");
    if let Some(parent) = index_log_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(index_log_path())
    {
        let _ = f.write_all(line.as_bytes());
    }
}

fn open_runner_log_file(action: &str) -> Option<std::fs::File> {
    let logs_dir = gwt_logs_dir().join("index");
    let _ = std::fs::create_dir_all(&logs_dir);
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = logs_dir.join(format!("runner-{unix}-{action}.log"));
    OpenOptions::new().create(true).append(true).open(path).ok()
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
pub fn bootstrap(repo_root: &Path, active_worktrees: &[PathBuf]) {
    log_event(&format!(
        "bootstrap start: repo_root={} active_worktrees={}",
        repo_root.display(),
        active_worktrees.len()
    ));
    let Some(repo_hash) = detect_repo_hash(repo_root) else {
        log_event("bootstrap skipped: no origin remote configured");
        return;
    };
    log_event(&format!("bootstrap repo_hash={}", repo_hash));

    // 1) Reconcile orphans + legacy directories — synchronous, fast.
    let opts = ReconcileOptions {
        index_root: gwt_index_root(),
        repo_hash: repo_hash.clone(),
        active_worktree_paths: active_worktrees.to_vec(),
        legacy_worktree_dirs: active_worktrees.to_vec(),
    };
    match reconcile_repo(&opts) {
        Ok(()) => log_event("reconcile_repo done"),
        Err(e) => log_event(&format!("reconcile_repo failed: {e}")),
    }

    // 2) Background Issue refresh.
    let project_root = repo_root.to_path_buf();
    let repo_hash_for_issues = repo_hash.clone();
    log_event(&format!(
        "spawning issue refresh task (ttl={}min)",
        ISSUE_REFRESH_TTL_MINUTES
    ));
    worker_runtime().spawn(async move {
        let opts = RefreshIssuesOptions {
            index_root: gwt_index_root(),
            repo_hash: repo_hash_for_issues,
            project_root,
            ttl: Duration::from_secs(ISSUE_REFRESH_TTL_MINUTES * 60),
        };
        let spawner = LoggingRunnerSpawner::wrap(make_runner_spawner());
        match refresh_issues_if_stale(&opts, &spawner).await {
            Ok(()) => log_event("issue refresh evaluation completed"),
            Err(e) => log_event(&format!("issue refresh evaluation failed: {e}")),
        }
    });

    // 3) Start a watcher per active Worktree.
    for wt in active_worktrees {
        ensure_watcher(repo_root, wt);
    }
    log_event(&format!(
        "bootstrap done: launched watchers for {} worktree(s)",
        active_worktrees.len()
    ));
}

/// Idempotently ensure that a watcher is running for the given Worktree.
pub fn ensure_watcher(repo_root: &Path, worktree_path: &Path) {
    let Some(repo_hash) = detect_repo_hash(repo_root) else {
        return;
    };
    let Ok(wt_hash) = compute_worktree_hash(worktree_path) else {
        log_event(&format!(
            "ensure_watcher: failed to compute worktree hash for {}",
            worktree_path.display()
        ));
        return;
    };
    let key = wt_hash.as_str().to_string();

    {
        let reg = registry().lock().unwrap();
        if reg.handles.contains_key(&key) {
            log_event(&format!(
                "ensure_watcher: already running for wt_hash={}",
                wt_hash
            ));
            return;
        }
    }

    log_event(&format!(
        "ensure_watcher: starting watcher for wt_hash={} path={}",
        wt_hash,
        worktree_path.display()
    ));
    let worktree_path = worktree_path.to_path_buf();
    let repo_root = repo_root.to_path_buf();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    let handle = worker_runtime().spawn(async move {
        let cfg = WatcherConfig::default();
        let mut watcher = match start_watcher(&worktree_path, cfg) {
            Ok(w) => w,
            Err(e) => {
                log_event(&format!(
                    "watcher start failed for {}: {e}",
                    worktree_path.display()
                ));
                return;
            }
        };
        log_event(&format!(
            "watcher running for wt_hash={} path={}",
            wt_hash,
            worktree_path.display()
        ));
        let mut shutdown_rx = rx;
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                batch = watcher.recv_batch() => {
                    let Some(batch) = batch else { break };
                    log_event(&format!(
                        "watcher batch: wt_hash={} paths={}",
                        wt_hash,
                        batch.changed_paths.len()
                    ));
                    if let Err(e) = run_incremental_index(&repo_hash, &wt_hash, &repo_root) {
                        log_event(&format!("incremental index spawn failed: {e}"));
                    }
                }
            }
        }
        log_event(&format!("watcher shutdown for wt_hash={}", wt_hash));
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

    log_event(&format!(
        "shutdown_and_remove: wt_hash={} path={}",
        wt_hash,
        worktree_path.display()
    ));

    {
        let mut reg = registry().lock().unwrap();
        if let Some(tx) = reg.shutdown.remove(&key) {
            let _ = tx.send(());
        }
        reg.handles.remove(&key);
    }

    if let Some(repo_hash) = detect_repo_hash(repo_root) {
        match remove_worktree_index(&gwt_index_root(), &repo_hash, wt_hash.as_str()) {
            Ok(()) => log_event(&format!("removed index dir for wt_hash={}", wt_hash)),
            Err(ref e) => log_event(&format!("remove_worktree_index failed: {e}")),
        }
    }

    Ok(())
}

fn run_incremental_index(
    repo_hash: &RepoHash,
    wt_hash: &WorktreeHash,
    project_root: &Path,
) -> std::io::Result<()> {
    let python = gwt_project_index_venv_dir().join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python3"
    });
    let runner = gwt_runtime_runner_path();
    if !python.exists() || !runner.exists() {
        log_event(&format!(
            "run_incremental_index: runtime missing (python={} runner={})",
            python.display(),
            runner.display()
        ));
        return Ok(());
    }

    for scope in ["files", "files-docs", "specs"] {
        let action = if scope == "specs" {
            "index-specs"
        } else {
            "index-files"
        };
        let log_file = open_runner_log_file(&format!("{action}-{scope}-incremental"));
        log_event(&format!(
            "spawn runner: action={} scope={} repo_hash={} wt_hash={}",
            action, scope, repo_hash, wt_hash
        ));
        let mut cmd = std::process::Command::new(&python);
        cmd.arg(&runner)
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
            .arg(scope);
        if let Some(file) = log_file.as_ref().and_then(|f| f.try_clone().ok()) {
            cmd.stdout(file);
        } else {
            cmd.stdout(std::process::Stdio::null());
        }
        if let Some(file) = log_file.and_then(|f| f.try_clone().ok()) {
            cmd.stderr(file);
        } else {
            cmd.stderr(std::process::Stdio::null());
        }
        cmd.stdin(std::process::Stdio::null());
        let _ = cmd.spawn()?;
    }
    Ok(())
}

// =====================================================================
// LoggingRunnerSpawner — wraps PythonRunnerSpawner to log + redirect stdio
// =====================================================================

struct LoggingRunnerSpawner {
    inner: PythonRunnerSpawner,
}

impl LoggingRunnerSpawner {
    fn wrap(inner: PythonRunnerSpawner) -> Self {
        Self { inner }
    }
}

impl gwt_core::index::runtime::RunnerSpawner for LoggingRunnerSpawner {
    fn spawn_index_issues(
        &self,
        repo_hash: &str,
        project_root: &Path,
        respect_ttl: bool,
    ) -> std::io::Result<()> {
        log_event(&format!(
            "spawn runner: action=index-issues repo_hash={} respect_ttl={} project_root={}",
            repo_hash,
            respect_ttl,
            project_root.display()
        ));
        let log_file = open_runner_log_file("index-issues");
        let mut cmd = std::process::Command::new(&self.inner.python_executable);
        cmd.arg(&self.inner.runner_script)
            .arg("--action")
            .arg("index-issues")
            .arg("--repo-hash")
            .arg(repo_hash)
            .arg("--project-root")
            .arg(project_root);
        if respect_ttl {
            cmd.arg("--respect-ttl");
        }
        if let Some(file) = log_file.as_ref().and_then(|f| f.try_clone().ok()) {
            cmd.stdout(file);
        } else {
            cmd.stdout(std::process::Stdio::null());
        }
        if let Some(file) = log_file.and_then(|f| f.try_clone().ok()) {
            cmd.stderr(file);
        } else {
            cmd.stderr(std::process::Stdio::null());
        }
        cmd.stdin(std::process::Stdio::null());
        cmd.spawn().map(|_| ())
    }
}
