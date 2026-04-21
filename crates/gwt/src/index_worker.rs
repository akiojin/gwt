use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use gwt_core::{
    index::{
        paths::gwt_index_root,
        runtime::{
            reconcile_repo, refresh_issues_if_stale, PythonRunnerSpawner, ReconcileOptions,
            RefreshIssuesOptions, RunnerSpawner,
        },
    },
    paths::{gwt_project_index_venv_dir, gwt_runtime_runner_path},
    repo_hash::RepoHash,
};

/// Determine `RepoHash` for the given repository root by shelling out to
/// `git remote get-url origin`. Returns `None` if no origin is configured.
pub fn detect_repo_hash(repo_root: &Path) -> Option<RepoHash> {
    gwt_core::repo_hash::detect_repo_hash(repo_root)
}

pub fn bootstrap_project_index_for_path(project_root: &Path) -> Result<(), String> {
    gwt_core::runtime::ensure_project_index_runtime().map_err(|err| err.to_string())?;
    let spawner = PythonRunnerSpawner {
        python_executable: project_index_python_path(),
        runner_script: gwt_runtime_runner_path(),
    };
    bootstrap_project_index_for_path_with(project_root, &gwt_index_root(), &spawner)
}

pub fn bootstrap_project_index_for_path_with<S: RunnerSpawner + ?Sized>(
    project_root: &Path,
    index_root: &Path,
    spawner: &S,
) -> Result<(), String> {
    let Some(repo_root) = resolve_git_worktree_root(project_root) else {
        return Ok(());
    };
    let Some(repo_hash) = detect_repo_hash(&repo_root) else {
        return Ok(());
    };

    let active_worktrees =
        list_git_worktree_paths(&repo_root).unwrap_or_else(|_| vec![repo_root.clone()]);
    reconcile_repo(&ReconcileOptions {
        index_root: index_root.to_path_buf(),
        repo_hash: repo_hash.clone(),
        active_worktree_paths: active_worktrees.clone(),
        legacy_worktree_dirs: active_worktrees,
    })
    .map_err(|err| err.to_string())?;

    let refresh = RefreshIssuesOptions {
        index_root: index_root.to_path_buf(),
        repo_hash,
        project_root: repo_root,
        ttl: Duration::from_secs(15 * 60),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;
    runtime
        .block_on(refresh_issues_if_stale(&refresh, spawner))
        .map_err(|err| err.to_string())?;

    Ok(())
}

fn resolve_git_worktree_root(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        return None;
    }
    Some(canonicalize_path(PathBuf::from(root)))
}

fn list_git_worktree_paths(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(project_root)
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let mut worktrees = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            worktrees.push(canonicalize_path(PathBuf::from(path)));
        }
    }

    if worktrees.is_empty() {
        worktrees.push(canonicalize_path(project_root.to_path_buf()));
    }
    Ok(worktrees)
}

fn canonicalize_path(path: PathBuf) -> PathBuf {
    dunce::canonicalize(&path).unwrap_or(path)
}

fn project_index_python_path() -> PathBuf {
    let venv = gwt_project_index_venv_dir();
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python3")
    }
}
