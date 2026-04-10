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
    ReconcileOptions, RefreshDecision, RefreshIssuesOptions,
};
use gwt_core::index::watcher::{start_watcher, WatcherConfig};
use gwt_core::logging::{LogEvent as Notification, LogLevel as Severity};
use gwt_core::paths::{gwt_logs_dir, gwt_project_index_venv_dir, gwt_runtime_runner_path};
use gwt_core::repo_hash::{compute_repo_hash, RepoHash};
use gwt_core::worktree_hash::{compute_worktree_hash, WorktreeHash};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Semaphore;

/// Type alias for the in-process notification sender used by index_worker
/// (SPEC-6 Phase 5: replaces the deleted `gwt_notification::NotificationBus`).
pub type NotificationBus = UnboundedSender<Notification>;

const ISSUE_REFRESH_TTL_MINUTES: u64 = 15;
/// Maximum number of concurrent runner subprocesses (each loads e5-base
/// ~440 MB into RAM, so a hard cap is required to avoid overwhelming the
/// host when many worktrees need indexing).
const RUNNER_CONCURRENCY: usize = 1;

/// Global semaphore that throttles concurrent Python runner spawns.
fn runner_semaphore() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(RUNNER_CONCURRENCY))
}

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

/// Process-global notification bus handle so the index worker can publish
/// lifecycle events into the TUI Logs tab.
fn notification_bus() -> &'static OnceLock<NotificationBus> {
    static BUS: OnceLock<NotificationBus> = OnceLock::new();
    &BUS
}

/// Initialize the worker's notification bus handle. Called once from
/// `main.rs` after the Model is created. Subsequent calls are ignored.
pub fn init_notification_bus(bus: NotificationBus) {
    let _ = notification_bus().set(bus);
}

/// Publish an index lifecycle event.
///
/// SPEC-6 Phase 5: this used to write to a dedicated `~/.gwt/logs/index.log`
/// file and push to the notification bus. Both have been replaced by a
/// `tracing::debug!(target: "gwt_tui::index", ...)` call so the event lands
/// in the unified `~/.gwt/logs/gwt.log.YYYY-MM-DD` JSONL file alongside
/// every other tracing event. The Logs tab picks it up via the file
/// watcher.
///
/// The legacy `~/.gwt/logs/index.log` writer is preserved as a best-effort
/// secondary sink for shell-friendly tail-ability; it can be removed once
/// downstream tooling is migrated.
pub fn log_event(message: &str) {
    tracing::debug!(target: "gwt_tui::index", "{}", message);

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

    // Best-effort notification bus push for the legacy in-memory mirror
    // (only useful in tests where the file watcher is not running).
    if let Some(bus) = notification_bus().get() {
        let _ = bus.send(Notification::new(
            Severity::Debug,
            "index",
            message.to_string(),
        ));
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

/// Per-worktree build state used to coalesce concurrent rebuild requests.
#[derive(Default)]
struct WorktreeBuildState {
    in_flight: bool,
    pending_scopes: ScopeMask,
}

fn build_states() -> &'static Mutex<HashMap<String, WorktreeBuildState>> {
    static STATES: OnceLock<Mutex<HashMap<String, WorktreeBuildState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

type ScopeMask = u8;

const SCOPE_FILES: ScopeMask = 1 << 0;
const SCOPE_FILES_DOCS: ScopeMask = 1 << 1;
// SPEC-12: SCOPE_SPECS removed. SPECs are now GitHub Issues cached at
// ~/.gwt/cache/issues/ and indexed through the Issue search scope, not
// the local specs/ directory watcher.
const SCOPE_ALL: ScopeMask = SCOPE_FILES | SCOPE_FILES_DOCS;

const DOC_FILE_EXTENSIONS: &[&str] = &["md", "mdx", "rst", "adoc", "txt"];
const SKIP_FILE_EXTENSIONS: &[&str] = &["snap"];
const SCOPE_SKIP_PREFIXES: &[&str] = &[
    ".git",
    ".claude",
    ".codex",
    ".gemini",
    ".gwt",
    "specs-archive",
    "tasks",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".nuxt",
];

fn scope_names_from_mask(mask: ScopeMask) -> Vec<&'static str> {
    let mut scopes = Vec::new();
    if mask & SCOPE_FILES != 0 {
        scopes.push("files");
    }
    if mask & SCOPE_FILES_DOCS != 0 {
        scopes.push("files-docs");
    }
    scopes
}

fn scopes_for_changed_paths(project_root: &Path, changed_paths: &[PathBuf]) -> ScopeMask {
    if changed_paths.is_empty() {
        return SCOPE_ALL;
    }

    changed_paths.iter().fold(0, |mask, path| {
        mask | scope_for_changed_path(project_root, path)
    })
}

fn scope_for_changed_path(project_root: &Path, path: &Path) -> ScopeMask {
    let rel = path.strip_prefix(project_root).unwrap_or(path);
    let first = rel
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str());
    if let Some(name) = first {
        // SPEC-12: specs/ is no longer a local directory. Treat it as
        // a skip prefix if it somehow reappears.
        if SCOPE_SKIP_PREFIXES.contains(&name) || name == "specs" {
            return 0;
        }
        if name == "docs" {
            return SCOPE_FILES_DOCS;
        }
    }

    let file_name = rel
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase());
    if let Some(file_name) = file_name.as_deref() {
        if file_name.starts_with("readme") {
            return SCOPE_FILES_DOCS;
        }
    }

    let extension = rel
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    if let Some(extension) = extension.as_deref() {
        if SKIP_FILE_EXTENSIONS.contains(&extension) {
            return 0;
        }
        if DOC_FILE_EXTENSIONS.contains(&extension) {
            return SCOPE_FILES_DOCS;
        }
    }

    SCOPE_FILES
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
            Ok(RefreshDecision::Spawned) => {
                log_event("issue refresh: runner spawned (TTL expired or meta missing)");
            }
            Ok(RefreshDecision::SkippedWithinTtl { remaining_seconds }) => {
                log_event(&format!(
                    "issue refresh: skipped (TTL valid, {}s remaining)",
                    remaining_seconds
                ));
            }
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
                    let sample: Vec<String> = batch
                        .changed_paths
                        .iter()
                        .take(3)
                        .map(|p| p.display().to_string())
                        .collect();
                    let scopes = scopes_for_changed_paths(&worktree_path, &batch.changed_paths);
                    log_event(&format!(
                        "watcher batch: wt_hash={} paths={} scopes={:?} sample={:?}",
                        wt_hash,
                        batch.changed_paths.len(),
                        scope_names_from_mask(scopes),
                        sample
                    ));
                    schedule_incremental_index(
                        repo_hash.clone(),
                        wt_hash.clone(),
                        worktree_path.clone(),
                        scopes,
                    );
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

/// Kick a background full/incremental rebuild for the requested scopes on
/// the worker runtime. Returns immediately; the actual subprocess execution
/// is throttled by `runner_semaphore` and coalesced per-worktree: if a build
/// is already in flight for the worktree, newly requested scopes are merged
/// into the pending follow-up pass.
fn schedule_incremental_index(
    repo_hash: RepoHash,
    wt_hash: WorktreeHash,
    project_root: PathBuf,
    scopes: ScopeMask,
) {
    if scopes == 0 {
        log_event(&format!(
            "schedule_incremental_index: skipped (no relevant scopes) wt_hash={}",
            wt_hash
        ));
        return;
    }

    let python = gwt_project_index_venv_dir().join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python3"
    });
    let runner = gwt_runtime_runner_path();
    if !python.exists() || !runner.exists() {
        log_event(&format!(
            "schedule_incremental_index: runtime missing (python={} runner={})",
            python.display(),
            runner.display()
        ));
        return;
    }

    // Coalesce: if a build is already running for this worktree, set dirty
    // and return without queueing another.
    let wt_key = wt_hash.as_str().to_string();
    {
        let mut states = build_states().lock().unwrap();
        let state = states.entry(wt_key.clone()).or_default();
        if state.in_flight {
            state.pending_scopes |= scopes;
            log_event(&format!(
                "schedule: coalesced wt_hash={} pending_scopes={:?}",
                wt_hash,
                scope_names_from_mask(state.pending_scopes)
            ));
            return;
        }
        state.in_flight = true;
        state.pending_scopes = 0;
    }

    let wt_hash_loop = wt_hash.clone();
    worker_runtime().spawn(async move {
        let mut next_scopes = scopes;
        loop {
            run_scopes(
                &python,
                &runner,
                &repo_hash,
                &wt_hash_loop,
                &project_root,
                next_scopes,
            )
            .await;

            let maybe_follow_up = {
                let mut states = build_states().lock().unwrap();
                let state = states.entry(wt_key.clone()).or_default();
                if state.pending_scopes != 0 {
                    let pending_scopes = state.pending_scopes;
                    state.pending_scopes = 0;
                    Some(pending_scopes)
                } else {
                    state.in_flight = false;
                    None
                }
            };
            if let Some(pending_scopes) = maybe_follow_up {
                log_event(&format!(
                    "schedule: running coalesced follow-up pass wt_hash={} scopes={:?}",
                    wt_hash_loop,
                    scope_names_from_mask(pending_scopes)
                ));
                next_scopes = pending_scopes;
                continue;
            }
            break;
        }
    });
}

async fn run_scopes(
    python: &Path,
    runner: &Path,
    repo_hash: &RepoHash,
    wt_hash: &WorktreeHash,
    project_root: &Path,
    scopes: ScopeMask,
) {
    for scope in scope_names_from_mask(scopes) {
        let action = "index-files";
        let log_file = open_runner_log_file(&format!("{action}-{scope}-incremental"));

        let permit = match runner_semaphore().acquire().await {
            Ok(p) => p,
            Err(_) => return,
        };

        log_event(&format!(
            "spawn runner: action={} scope={} repo_hash={} wt_hash={}",
            action, scope, repo_hash, wt_hash
        ));

        let mut cmd = tokio::process::Command::new(python);
        cmd.arg(runner)
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

        match cmd.spawn() {
            Ok(mut child) => match child.wait().await {
                Ok(status) => log_event(&format!(
                    "runner exit: action={} scope={} status={} wt_hash={}",
                    action, scope, status, wt_hash
                )),
                Err(e) => log_event(&format!(
                    "runner wait failed: action={} scope={} err={}",
                    action, scope, e
                )),
            },
            Err(e) => log_event(&format!(
                "runner spawn failed: action={} scope={} err={}",
                action, scope, e
            )),
        }

        drop(permit);
    }
}

/// Kick an initial integrity-check build for a single Worktree. Used by
/// the pane spawn site (`materialize_pending_launch_with`) to ensure the
/// index reflects the current on-disk state when the user actually starts
/// working in that Worktree. Bootstrap-time eager builds across all 9
/// worktrees were too expensive (each runner loads ~440 MB e5 model), so
/// we defer this to per-pane spawn instead.
pub fn kick_initial_build_for_worktree(repo_root: &Path, worktree_path: &Path) {
    let Some(repo_hash) = detect_repo_hash(repo_root) else {
        return;
    };
    let Ok(wt_hash) = compute_worktree_hash(worktree_path) else {
        log_event(&format!(
            "kick_initial_build: failed to compute worktree hash for {}",
            worktree_path.display()
        ));
        return;
    };
    log_event(&format!(
        "kick_initial_build: queueing integrity build for wt_hash={}",
        wt_hash
    ));
    schedule_incremental_index(repo_hash, wt_hash, worktree_path.to_path_buf(), SCOPE_ALL);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_paths_classify_code_without_docs_or_specs() {
        let root = Path::new("/repo");
        let mask = scopes_for_changed_paths(root, &[root.join("crates/gwt-tui/src/app.rs")]);

        assert_eq!(scope_names_from_mask(mask), vec!["files"]);
    }

    // SPEC-12: specs/ is no longer indexed locally. Changes under
    // specs/ are now skipped (treated like .git or node_modules).
    #[test]
    fn changed_paths_classify_docs_and_skip_specs() {
        let root = Path::new("/repo");
        let mask = scopes_for_changed_paths(
            root,
            &[
                root.join("README.md"),
                root.join("docs/guide/overview.md"),
                root.join("specs/SPEC-10/spec.md"),
            ],
        );

        assert_eq!(scope_names_from_mask(mask), vec!["files-docs"]);
    }

    #[test]
    fn changed_paths_skip_snapshot_noise() {
        let root = Path::new("/repo");
        let mask = scopes_for_changed_paths(root, &[root.join("crates/gwt-tui/tests/ui.snap")]);

        assert!(scope_names_from_mask(mask).is_empty());
    }
}
