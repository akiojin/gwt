//! Tokio job spawning and lifecycle reconciliation for the index runner.
//!
//! This module owns the Rust side of:
//! - Reconciling `~/.gwt/index/<repo-hash>/worktrees/` against `git worktree list`
//!   and removing orphans + legacy `$WORKTREE/.gwt/index/` directories
//! - Cleaning up legacy worktree-scoped SPEC index artifacts after SPEC index
//!   moved to the repo root
//! - Refreshing the Issue index according to a TTL window
//! - Spawning the Python runner as background tokio tasks
//!
//! The actual ChromaDB writes happen inside the Python runner. This module
//! never touches sqlite directly.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    error::{GwtError, Result},
    repo_hash::RepoHash,
    worktree_hash::compute_worktree_hash,
};

// =====================================================================
// reconcile_repo
// =====================================================================

/// Inputs needed to reconcile the index storage layout for a single repo.
#[derive(Debug, Clone)]
pub struct ReconcileOptions {
    /// Override of `~/.gwt/index/`. Tests inject a tempdir here.
    pub index_root: PathBuf,
    pub repo_hash: RepoHash,
    /// Absolute paths of every worktree currently registered with git for
    /// this repository.
    pub active_worktree_paths: Vec<PathBuf>,
    /// Absolute paths of every worktree where a legacy `$WORKTREE/.gwt/index/`
    /// directory should be removed.
    pub legacy_worktree_dirs: Vec<PathBuf>,
}

/// Reconcile orphan worktree-hash directories under
/// `<index_root>/<repo>/worktrees/` and delete legacy `$WORKTREE/.gwt/index/`.
pub fn reconcile_repo(opts: &ReconcileOptions) -> Result<()> {
    // 1. Compute the set of valid wt-hashes from the active worktree paths.
    let mut valid_hashes = std::collections::HashSet::new();
    for path in &opts.active_worktree_paths {
        if let Ok(h) = compute_worktree_hash(path) {
            let hash = h.as_str().to_string();
            remove_legacy_worktree_specs_artifacts(&opts.index_root, &opts.repo_hash, &hash)?;
            valid_hashes.insert(hash);
        }
    }

    // 2. Walk <index_root>/<repo>/worktrees/ and remove orphans.
    let worktrees_dir = opts
        .index_root
        .join(opts.repo_hash.as_str())
        .join("worktrees");
    if worktrees_dir.is_dir() {
        for entry in std::fs::read_dir(&worktrees_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let hash = name.to_string_lossy().to_string();
            if !valid_hashes.contains(&hash) {
                let path = entry.path();
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                }
            }
        }
    }

    // 3. Remove legacy $WORKTREE/.gwt/index/ directories.
    for wt in &opts.legacy_worktree_dirs {
        let legacy = wt.join(".gwt").join("index");
        if legacy.exists() {
            let _ = std::fs::remove_dir_all(&legacy);
        }
    }

    Ok(())
}

fn remove_legacy_worktree_specs_artifacts(
    index_root: &Path,
    repo: &RepoHash,
    worktree_hash: &str,
) -> Result<()> {
    let worktree_dir = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(worktree_hash);
    let legacy_specs = worktree_dir.join("specs");
    if legacy_specs.exists() {
        std::fs::remove_dir_all(&legacy_specs)?;
    }
    let legacy_manifest = worktree_dir.join("manifest-specs.json");
    if legacy_manifest.exists() {
        std::fs::remove_file(&legacy_manifest)?;
    }
    Ok(())
}

/// Synchronously remove the index directory for a single worktree.
///
/// Called by non-interactive worktree lifecycle handlers when a worktree is
/// removed and its per-worktree file indexes should be deleted eagerly.
pub fn remove_worktree_index(
    index_root: &Path,
    repo: &RepoHash,
    worktree_hash: &str,
) -> Result<()> {
    let _ = repo;
    let target = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(worktree_hash);
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    Ok(())
}

// =====================================================================
// refresh_issues_if_stale
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IssueMetadata {
    schema_version: u32,
    last_full_refresh: String,
    ttl_minutes: u64,
}

/// Trait abstraction over the Python runner spawn so tests can substitute a
/// recording double.
pub trait RunnerSpawner: Send + Sync {
    fn spawn_index_issues(
        &self,
        repo_hash: &str,
        project_root: &Path,
        respect_ttl: bool,
    ) -> std::io::Result<()>;
}

#[derive(Debug, Clone)]
pub struct RefreshIssuesOptions {
    pub index_root: PathBuf,
    pub repo_hash: RepoHash,
    pub project_root: PathBuf,
    pub ttl: Duration,
}

/// Outcome of a single `refresh_issues_if_stale` invocation. Lets callers
/// distinguish "actually spawned a runner" from "TTL still valid, skipped".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshDecision {
    /// Index was missing or stale; the spawner was invoked.
    Spawned,
    /// TTL has not expired yet. `remaining_seconds` is how long until the
    /// next refresh becomes due.
    SkippedWithinTtl { remaining_seconds: u64 },
}

/// Refresh the Issue index if (a) no metadata exists, or (b) the recorded
/// `last_full_refresh` is older than `ttl`. Returns immediately after
/// dispatching to the spawner — the spawner is responsible for any
/// background work.
pub async fn refresh_issues_if_stale<S: RunnerSpawner + ?Sized>(
    opts: &RefreshIssuesOptions,
    spawner: &S,
) -> Result<RefreshDecision> {
    let issues_dir = gwt_index_repo_dir_under(&opts.index_root, &opts.repo_hash).join("issues");
    let meta_path = issues_dir.join("meta.json");
    let mut remaining_seconds: u64 = 0;
    let stale = if meta_path.is_file() {
        match read_issue_meta(&meta_path) {
            Some(meta) => match DateTime::parse_from_rfc3339(&meta.last_full_refresh) {
                Ok(dt) => {
                    let age = Utc::now().signed_duration_since(dt.with_timezone(&Utc));
                    let age_std = age.to_std().unwrap_or(Duration::MAX);
                    if age_std >= opts.ttl {
                        true
                    } else {
                        remaining_seconds = (opts.ttl - age_std).as_secs();
                        false
                    }
                }
                Err(_) => true,
            },
            None => true,
        }
    } else {
        true
    };

    if stale {
        spawner
            .spawn_index_issues(opts.repo_hash.as_str(), &opts.project_root, false)
            .map_err(|e| GwtError::Other(format!("spawn issue index: {e}")))?;
        Ok(RefreshDecision::Spawned)
    } else {
        Ok(RefreshDecision::SkippedWithinTtl { remaining_seconds })
    }
}

fn gwt_index_repo_dir_under(index_root: &Path, repo: &RepoHash) -> PathBuf {
    index_root.join(repo.as_str())
}

fn read_issue_meta(path: &Path) -> Option<IssueMetadata> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

// =====================================================================
// Default RunnerSpawner: spawns the actual Python runner via tokio
// =====================================================================

/// Default `RunnerSpawner` that fires the real Python runner in a detached
/// tokio task. Used by the desktop app in production; tests prefer a recording
/// double.
#[derive(Debug, Clone)]
pub struct PythonRunnerSpawner {
    pub python_executable: PathBuf,
    pub runner_script: PathBuf,
}

impl RunnerSpawner for PythonRunnerSpawner {
    fn spawn_index_issues(
        &self,
        repo_hash: &str,
        project_root: &Path,
        respect_ttl: bool,
    ) -> std::io::Result<()> {
        // A missing venv python must surface synchronously to the caller;
        // once the job is detached behind the coordinator only logs would
        // see it.
        if !self.python_executable.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "index runner python not found: {}",
                    self.python_executable.display()
                ),
            ));
        }
        // SPEC-1924 FR-039 / SPEC-2809 Phase D-runner — emit a
        // `gwt.process.summary` start event so the Console window's
        // runner tab and the Logs Process facet observe the Python
        // chroma index runner spawn.
        let spawn_id = RUNNER_SPAWN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let label = format!(
            "{} {} --action index-issues",
            self.python_executable.display(),
            self.runner_script.display(),
        );
        tracing::info!(
            target: "gwt.process.summary",
            kind = "runner",
            spawn_id = spawn_id,
            label = %label,
            phase = "start",
            respect_ttl = respect_ttl,
            "process start",
        );
        crate::process::push_command_banner_to_hub(
            crate::process_console::ProcessKind::IndexRunner,
            spawn_id,
            &label,
            None,
        );

        let mut cmd = crate::process::hidden_command(&self.python_executable);
        cmd.arg(&self.runner_script)
            .arg("--action")
            .arg("index-issues")
            .arg("--repo-hash")
            .arg(repo_hash)
            .arg("--project-root")
            .arg(project_root);
        if respect_ttl {
            cmd.arg("--respect-ttl");
        }
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // Fire-and-forget for the caller, but the detached thread routes the
        // heavy index build through the host-wide coordinator (SPEC #1939
        // Phase 70 FR-379/FR-382) and drains the child while it runs.
        let repo_hash = repo_hash.to_string();
        std::thread::Builder::new()
            .name("gwt-index-issues".to_string())
            .spawn(move || run_coordinated_issue_index(&repo_hash, cmd, spawn_id, &label))
            .map(|_| ())
    }
}

/// Timeouts for the coordinated background issue index build.
const ISSUE_INDEX_ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);
const ISSUE_INDEX_HEAVY_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const ISSUE_INDEX_SHARED_WAIT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

fn run_coordinated_issue_index(
    repo_hash: &str,
    mut cmd: std::process::Command,
    spawn_id: u64,
    label: &str,
) {
    use crate::index_coordinator::{
        IndexCoordinator, JobAdmission, JobOutcome, JobPriority, TargetKey,
    };

    let coordinator = match IndexCoordinator::open_default() {
        Ok(coordinator) => coordinator,
        Err(err) => {
            tracing::warn!(
                target: "gwt::index",
                spawn_id = spawn_id,
                error = %err,
                "issue index skipped: coordinator unavailable"
            );
            return;
        }
    };
    let key = TargetKey::repo_shared(repo_hash, "issues");
    match coordinator.request_job(&key, JobPriority::Background, ISSUE_INDEX_ADMISSION_TIMEOUT) {
        Ok(JobAdmission::Owner(guard)) => {
            let heavy = match guard.acquire_heavy(ISSUE_INDEX_HEAVY_TIMEOUT) {
                Ok(heavy) => heavy,
                Err(err) => {
                    tracing::warn!(
                        target: "gwt::index",
                        spawn_id = spawn_id,
                        error = %err,
                        "issue index skipped: heavy lease unavailable"
                    );
                    let _ = guard.complete(JobOutcome::Failed {
                        message: format!("heavy lease unavailable: {err}"),
                    });
                    return;
                }
            };
            let outcome = match cmd.spawn().and_then(|child| child.wait_with_output()) {
                Ok(output) if output.status.success() => JobOutcome::Completed,
                Ok(output) => {
                    tracing::warn!(
                        target: "gwt::index",
                        spawn_id = spawn_id,
                        exit_status = %output.status,
                        stderr = %String::from_utf8_lossy(&output.stderr),
                        "issue index runner failed"
                    );
                    JobOutcome::Failed {
                        message: format!("issue index runner exited with {}", output.status),
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        target: "gwt::index",
                        spawn_id = spawn_id,
                        error = %err,
                        "issue index runner spawn failed"
                    );
                    JobOutcome::Failed {
                        message: err.to_string(),
                    }
                }
            };
            drop(heavy);
            let completed = matches!(outcome, JobOutcome::Completed);
            let _ = guard.complete(outcome);
            tracing::info!(
                target: "gwt.process.summary",
                kind = "runner",
                spawn_id = spawn_id,
                label = %label,
                phase = "end",
                success = completed,
                "process end",
            );
        }
        Ok(JobAdmission::Joined(waiter)) => {
            // An equivalent issue index build is already running host-wide;
            // coalesce instead of spawning a duplicate model load (FR-382).
            let _ = waiter.wait(ISSUE_INDEX_SHARED_WAIT_TIMEOUT);
            tracing::info!(
                target: "gwt::index",
                spawn_id = spawn_id,
                "issue index coalesced into a concurrent equivalent job"
            );
        }
        Err(err) => {
            tracing::warn!(
                target: "gwt::index",
                spawn_id = spawn_id,
                error = %err,
                "issue index skipped: job admission failed"
            );
        }
    }
}

static RUNNER_SPAWN_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::repo_hash::compute_repo_hash;

    #[derive(Default, Clone)]
    struct RecordingSpawner {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl RunnerSpawner for RecordingSpawner {
        fn spawn_index_issues(
            &self,
            repo_hash: &str,
            project_root: &Path,
            respect_ttl: bool,
        ) -> std::io::Result<()> {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(format!(
                    "{}|{}|{}",
                    repo_hash,
                    project_root.display(),
                    respect_ttl
                ));
            Ok(())
        }
    }

    #[tokio::test]
    async fn refresh_kicks_when_meta_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = compute_repo_hash("https://github.com/example/repo.git");
        let spawner = RecordingSpawner::default();
        let opts = RefreshIssuesOptions {
            index_root: tmp.path().join("idx"),
            repo_hash: repo,
            project_root: tmp.path().to_path_buf(),
            ttl: Duration::from_secs(15 * 60),
        };
        refresh_issues_if_stale(&opts, &spawner).await.unwrap();
        assert_eq!(
            spawner
                .calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            1
        );
    }

    #[test]
    fn reconcile_removes_orphan_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let idx = tmp.path().join("idx");
        let repo = compute_repo_hash("https://github.com/example/repo.git");
        let orphan = idx
            .join(repo.as_str())
            .join("worktrees")
            .join("deadbeefdeadbeef");
        std::fs::create_dir_all(&orphan).unwrap();

        let opts = ReconcileOptions {
            index_root: idx,
            repo_hash: repo,
            active_worktree_paths: Vec::new(),
            legacy_worktree_dirs: Vec::new(),
        };
        reconcile_repo(&opts).unwrap();
        assert!(!orphan.exists());
    }
}
