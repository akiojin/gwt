//! Docker context commands for GUI binding

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::config::Settings;
use gwt_core::docker::{
    compose_available, daemon_running, detect_docker_files, docker_available, DevContainerConfig,
    DockerFileType, DockerManager,
};
use gwt_core::git::Remote;
use gwt_core::worktree::WorktreeManager;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct DockerContext {
    pub worktree_path: Option<String>,
    /// "compose" | "devcontainer" | "dockerfile" | "none"
    pub file_type: String,
    pub compose_services: Vec<String>,
    pub docker_available: bool,
    pub compose_available: bool,
    pub daemon_running: bool,
    pub force_host: bool,
    /// "running" | "stopped" | "not_found" | null (when detection is not possible)
    pub container_status: Option<String>,
    /// Whether Docker images exist for this compose project (null when detection is not possible)
    pub images_exist: Option<bool>,
}

fn strip_known_remote_prefix<'a>(branch: &'a str, remotes: &[Remote]) -> &'a str {
    let Some((first, rest)) = branch.split_once('/') else {
        return branch;
    };
    if remotes.iter().any(|r| r.name == first) {
        return rest;
    }
    branch
}

fn resolve_existing_worktree_path(
    repo_path: &std::path::Path,
    branch_ref: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    let manager = WorktreeManager::new(repo_path).map_err(|e| e.to_string())?;
    let remotes = Remote::list(repo_path).unwrap_or_default();
    let normalized = strip_known_remote_prefix(branch_ref, &remotes);

    if let Ok(Some(wt)) = manager.get_by_branch_basic(normalized) {
        return Ok(Some(wt.path));
    }
    if normalized != branch_ref {
        if let Ok(Some(wt)) = manager.get_by_branch_basic(branch_ref) {
            return Ok(Some(wt.path));
        }
    }
    Ok(None)
}

/// Detect docker compose context for a branch (best-effort, read-only).
#[tauri::command]
pub fn detect_docker_context(
    project_path: String,
    branch: String,
) -> Result<DockerContext, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let settings = Settings::load(project_root).unwrap_or_default();
    let force_host = settings.docker.force_host;

    let docker_ok = docker_available();
    let compose_ok = compose_available();
    let daemon_ok = daemon_running();

    let branch_ref = branch.trim();
    let worktree_path = if branch_ref.is_empty() {
        None
    } else {
        resolve_existing_worktree_path(&repo_path, branch_ref)?
    };

    if force_host {
        return Ok(DockerContext {
            worktree_path: worktree_path.map(|p| p.to_string_lossy().to_string()),
            file_type: "none".to_string(),
            compose_services: Vec::new(),
            docker_available: docker_ok,
            compose_available: compose_ok,
            daemon_running: daemon_ok,
            force_host,
            container_status: None,
            images_exist: None,
        });
    }

    let (file_type, compose_services) = match worktree_path.as_ref() {
        Some(wt) => match detect_docker_files(wt) {
            Some(DockerFileType::Compose(compose_path)) => {
                let services = DockerManager::list_services_from_compose_file(&compose_path)
                    .unwrap_or_default();
                ("compose".to_string(), services)
            }
            Some(DockerFileType::DevContainer(devcontainer_path)) => {
                let devcontainer_dir = devcontainer_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| wt.to_path_buf());
                let cfg = DevContainerConfig::load(&devcontainer_path).ok();

                let compose_files = cfg
                    .as_ref()
                    .map(|c| c.get_compose_files())
                    .unwrap_or_default();

                if compose_files.is_empty() {
                    ("devcontainer".to_string(), Vec::new())
                } else {
                    let mut services: Vec<String> = Vec::new();
                    for file in compose_files {
                        let compose_path = devcontainer_dir.join(file);
                        let mut s = DockerManager::list_services_from_compose_file(&compose_path)
                            .unwrap_or_default();
                        services.append(&mut s);
                    }
                    services.sort();
                    services.dedup();
                    ("devcontainer".to_string(), services)
                }
            }
            Some(DockerFileType::Dockerfile(_)) => ("dockerfile".to_string(), Vec::new()),
            _ => ("none".to_string(), Vec::new()),
        },
        None => ("none".to_string(), Vec::new()),
    };

    // Detect container / image status when daemon is running and worktree exists.
    let (container_status, images_exist) = if daemon_ok && compose_ok {
        if let Some(wt) = worktree_path.as_ref() {
            if file_type == "compose"
                || (file_type == "devcontainer" && !compose_services.is_empty())
            {
                let wt_name = wt
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                // Detect the docker file type at worktree path for the manager.
                let docker_file_type = detect_docker_files(wt);
                if let Some(dft) = docker_file_type {
                    let mgr = DockerManager::new(wt, &wt_name, dft);
                    let status = mgr.get_status();
                    let status_str = match status {
                        gwt_core::docker::ContainerStatus::Running => "running",
                        gwt_core::docker::ContainerStatus::Stopped => "stopped",
                        gwt_core::docker::ContainerStatus::NotFound => "not_found",
                    };
                    let img = mgr.images_exist();
                    (Some(status_str.to_string()), Some(img))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    Ok(DockerContext {
        worktree_path: worktree_path.map(|p| p.to_string_lossy().to_string()),
        file_type,
        compose_services,
        docker_available: docker_ok,
        compose_available: compose_ok,
        daemon_running: daemon_ok,
        force_host,
        container_status,
        images_exist,
    })
}
