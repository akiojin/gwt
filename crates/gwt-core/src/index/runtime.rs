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
    index::paths::gwt_index_worktree_dir,
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

/// Synchronously remove the index directory for a single Worktree (used by
/// the gwt TUI Worktree-remove handler).
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
        let mut cmd = std::process::Command::new(&self.python_executable);
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
        // Spawn-and-forget: the caller wraps this in a tokio task and we want
        // a non-blocking return.
        cmd.spawn().map(|_| ())
    }
}

// gwt_index_worktree_dir is re-exported from `crate::index::paths`; keep this
// path in scope so tests can verify the layout matches.
#[allow(dead_code)]
fn _layout_anchor(repo: &RepoHash, wt_hash: &str) -> PathBuf {
    let _ = (repo, wt_hash);
    let _ = gwt_index_worktree_dir;
    PathBuf::new()
}

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
            self.calls.lock().unwrap().push(format!(
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
        assert_eq!(spawner.calls.lock().unwrap().len(), 1);
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
            index_root: idx.clone(),
            repo_hash: repo,
            active_worktree_paths: Vec::new(),
            legacy_worktree_dirs: Vec::new(),
        };
        reconcile_repo(&opts).unwrap();
        assert!(!orphan.exists());
    }
}
