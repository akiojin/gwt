use std::{
    fmt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
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
    worktree_hash::compute_worktree_hash,
};
use serde::Serialize;

/// Determine `RepoHash` for the given repository root by shelling out to
/// `git remote get-url origin`. Returns `None` if no origin is configured.
pub fn detect_repo_hash(repo_root: &Path) -> Option<RepoHash> {
    gwt_core::repo_hash::detect_repo_hash(repo_root)
}

pub fn bootstrap_project_index_for_path(project_root: &Path) -> Result<(), String> {
    let runtime_started = Instant::now();
    gwt_core::runtime::ensure_project_index_runtime().map_err(|err| err.to_string())?;
    tracing::info!(
        target: "gwt::index",
        project_root = %project_root.display(),
        elapsed_ms = runtime_started.elapsed().as_millis() as u64,
        "project index runtime ensured for bootstrap"
    );
    let spawner = PythonRunnerSpawner {
        python_executable: project_index_python_path(),
        runner_script: gwt_runtime_runner_path(),
    };
    bootstrap_project_index_for_path_with(project_root, &gwt_index_root(), &spawner)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectIndexStatusState {
    Ready,
    Skipped,
    Error,
    RepairRequired,
    Repairing,
}

impl ProjectIndexStatusState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Skipped => "skipped",
            Self::Error => "error",
            Self::RepairRequired => "repair_required",
            Self::Repairing => "repairing",
        }
    }
}

impl fmt::Display for ProjectIndexStatusState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RebuildProgress {
    pub scopes_done: u32,
    pub scopes_total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectIndexStatusView {
    pub state: ProjectIndexStatusState,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair_started_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<RebuildProgress>,
}

impl ProjectIndexStatusView {
    pub fn new(state: ProjectIndexStatusState, detail: impl Into<String>) -> Self {
        Self {
            state,
            detail: detail.into(),
            repair_started_at: None,
            progress: None,
        }
    }
}

pub fn project_index_status_for_path(project_root: &Path) -> ProjectIndexStatusView {
    match project_index_status_for_path_inner(project_root) {
        Ok(status) => status,
        Err(error) => ProjectIndexStatusView::new(ProjectIndexStatusState::Error, error),
    }
}

fn project_index_status_for_path_inner(
    project_root: &Path,
) -> Result<ProjectIndexStatusView, String> {
    let Some(repo_root) = resolve_git_worktree_root(project_root) else {
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Skipped,
            "No git worktree detected",
        ));
    };
    let Some(repo_hash) = detect_repo_hash(&repo_root) else {
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Skipped,
            "No origin remote configured",
        ));
    };
    let worktree_hash =
        compute_worktree_hash(&repo_root).map_err(|err| format!("compute worktree hash: {err}"))?;
    let runtime_started = Instant::now();
    let report =
        gwt_core::runtime::ensure_project_index_runtime().map_err(|err| err.to_string())?;
    tracing::info!(
        target: "gwt::index",
        project_root = %project_root.display(),
        elapsed_ms = runtime_started.elapsed().as_millis() as u64,
        "project index runtime ensured for status"
    );
    let runner_started = Instant::now();
    let output = gwt_core::process::hidden_command(project_index_python_path())
        .arg(gwt_runtime_runner_path())
        .arg("--action")
        .arg("status")
        .arg("--repo-hash")
        .arg(repo_hash.as_str())
        .arg("--worktree-hash")
        .arg(worktree_hash.as_str())
        .current_dir(&repo_root)
        .output()
        .map_err(|err| format!("run project index status: {err}"))?;
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        elapsed_ms = runner_started.elapsed().as_millis() as u64,
        exit_status = %output.status,
        "project index status runner completed"
    );
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Error,
            format!("runner exit {}: {detail}", output.status),
        ));
    }
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("parse project index status: {err}"))?;
    let unhealthy = payload
        .get("status")
        .and_then(serde_json::Value::as_object)
        .map(|status| {
            status
                .values()
                .filter(|scope| {
                    !scope
                        .get("healthy")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    if unhealthy == 0 {
        Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::Ready,
            format!("Runtime ready; asset {}", report.runner_hash),
        ))
    } else {
        Ok(ProjectIndexStatusView::new(
            ProjectIndexStatusState::RepairRequired,
            format!("{unhealthy} index scope(s) require repair"),
        ))
    }
}

pub fn bootstrap_project_index_for_path_with<S: RunnerSpawner + ?Sized>(
    project_root: &Path,
    index_root: &Path,
    spawner: &S,
) -> Result<(), String> {
    let bootstrap_started = Instant::now();
    let Some(repo_root) = resolve_git_worktree_root(project_root) else {
        return Ok(());
    };
    let Some(repo_hash) = detect_repo_hash(&repo_root) else {
        return Ok(());
    };

    let worktree_list_started = Instant::now();
    let active_worktrees =
        list_git_worktree_paths(&repo_root).unwrap_or_else(|_| vec![repo_root.clone()]);
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        elapsed_ms = worktree_list_started.elapsed().as_millis() as u64,
        worktree_count = active_worktrees.len(),
        "project index active worktrees listed"
    );
    let reconcile_started = Instant::now();
    reconcile_repo(&ReconcileOptions {
        index_root: index_root.to_path_buf(),
        repo_hash: repo_hash.clone(),
        active_worktree_paths: active_worktrees.clone(),
        legacy_worktree_dirs: active_worktrees,
    })
    .map_err(|err| err.to_string())?;
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        elapsed_ms = reconcile_started.elapsed().as_millis() as u64,
        "project index repository reconciled"
    );

    let refresh = RefreshIssuesOptions {
        index_root: index_root.to_path_buf(),
        repo_hash,
        project_root: repo_root.clone(),
        ttl: Duration::from_secs(15 * 60),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;
    let refresh_started = Instant::now();
    runtime
        .block_on(refresh_issues_if_stale(&refresh, spawner))
        .map(|decision| {
            tracing::info!(
                target: "gwt::index",
                project_root = %repo_root.display(),
                elapsed_ms = refresh_started.elapsed().as_millis() as u64,
                decision = ?decision,
                "project index issue refresh checked"
            );
        })
        .map_err(|err| err.to_string())?;
    tracing::info!(
        target: "gwt::index",
        project_root = %repo_root.display(),
        elapsed_ms = bootstrap_started.elapsed().as_millis() as u64,
        "project index bootstrap helper completed"
    );

    Ok(())
}

fn resolve_git_worktree_root(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    let output = gwt_core::process::hidden_command("git")
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
    let output = gwt_core::process::hidden_command("git")
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

pub(crate) fn project_index_python_path() -> PathBuf {
    let venv = gwt_project_index_venv_dir();
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python3")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_index_status_state_serializes_stable_protocol_values() {
        let status = ProjectIndexStatusView::new(
            ProjectIndexStatusState::RepairRequired,
            "1 scope requires repair",
        );

        let payload = serde_json::to_value(status).expect("serialize status");

        assert_eq!(payload["state"], "repair_required");
        assert_eq!(payload["detail"], "1 scope requires repair");
    }

    #[test]
    fn project_index_status_state_serializes_repairing_variant() {
        let status = ProjectIndexStatusView::new(
            ProjectIndexStatusState::Repairing,
            "rebuilding 1/4: issues",
        );

        let payload = serde_json::to_value(&status).expect("serialize status");

        assert_eq!(payload["state"], "repairing");
        assert_eq!(payload["detail"], "rebuilding 1/4: issues");
        assert_eq!(ProjectIndexStatusState::Repairing.as_str(), "repairing");
        assert_eq!(
            format!("{}", ProjectIndexStatusState::Repairing),
            "repairing"
        );
    }

    #[test]
    fn project_index_status_view_omits_repair_progress_when_absent() {
        let view = ProjectIndexStatusView {
            state: ProjectIndexStatusState::Ready,
            detail: "Runtime ready".to_string(),
            repair_started_at: None,
            progress: None,
        };

        let payload = serde_json::to_value(&view).expect("serialize ready view");

        assert_eq!(payload["state"], "ready");
        assert!(
            payload.get("repair_started_at").is_none(),
            "repair_started_at should be omitted when None: {payload:?}"
        );
        assert!(
            payload.get("progress").is_none(),
            "progress should be omitted when None: {payload:?}"
        );
    }

    #[test]
    fn project_index_status_view_emits_repair_progress_when_present() {
        let started = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            chrono::NaiveDateTime::parse_from_str("2026-05-07T01:23:45", "%Y-%m-%dT%H:%M:%S")
                .expect("parse fixed timestamp"),
            chrono::Utc,
        );
        let view = ProjectIndexStatusView {
            state: ProjectIndexStatusState::Repairing,
            detail: "rebuilding 1/4: issues".to_string(),
            repair_started_at: Some(started),
            progress: Some(RebuildProgress {
                scopes_done: 1,
                scopes_total: 4,
            }),
        };

        let payload = serde_json::to_value(&view).expect("serialize repairing view");

        assert_eq!(payload["state"], "repairing");
        assert_eq!(payload["repair_started_at"], "2026-05-07T01:23:45Z");
        assert_eq!(payload["progress"]["scopes_done"], 1);
        assert_eq!(payload["progress"]["scopes_total"], 4);
    }

    #[test]
    fn project_index_status_state_variant_set_is_complete() {
        let variants = [
            ProjectIndexStatusState::Ready,
            ProjectIndexStatusState::Skipped,
            ProjectIndexStatusState::Error,
            ProjectIndexStatusState::RepairRequired,
            ProjectIndexStatusState::Repairing,
        ];
        let serialized: Vec<&'static str> = variants.iter().map(|state| state.as_str()).collect();
        assert_eq!(
            serialized,
            vec!["ready", "skipped", "error", "repair_required", "repairing",]
        );
    }
}
