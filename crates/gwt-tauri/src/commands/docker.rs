//! Docker context commands for GUI binding

use crate::commands::project::resolve_repo_path_for_project_root;
use gwt_core::config::{ProfilesConfig, Settings};
use gwt_core::docker::{
    compose_available, daemon_running, detect_docker_files, docker_available, ContainerStatus,
    DevContainerConfig, DockerFileType, DockerManager,
};
use gwt_core::git::Remote;
use gwt_core::worktree::WorktreeManager;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone)]
struct ComposeProbeTarget {
    docker_file_type: DockerFileType,
    compose_args: Vec<String>,
    compose_paths: Vec<PathBuf>,
}

fn compose_file_paths_from_args(compose_args: &[String]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut idx = 0usize;
    while idx + 1 < compose_args.len() {
        if compose_args[idx] == "-f" {
            let path = compose_args[idx + 1].trim();
            if !path.is_empty() {
                paths.push(PathBuf::from(path));
            }
            idx += 2;
            continue;
        }
        idx += 1;
    }
    paths
}

fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn merge_profile_env_for_detection(os_env: &HashMap<String, String>) -> HashMap<String, String> {
    let mut env_vars = os_env.clone();

    let Ok(config) = ProfilesConfig::load() else {
        return env_vars;
    };
    let Some(active) = config.active.as_ref() else {
        return env_vars;
    };
    let Some(profile) = config.profiles.get(active) else {
        return env_vars;
    };

    for key in &profile.disabled_env {
        env_vars.remove(key);
    }
    for (key, value) in &profile.env {
        env_vars.insert(key.clone(), value.clone());
    }
    env_vars
}

fn merge_compose_env_from_source(
    env: &mut HashMap<String, String>,
    compose_paths: &[PathBuf],
    source_env: &HashMap<String, String>,
) {
    for compose_path in compose_paths {
        let Ok(keys) = DockerManager::list_env_keys_from_compose_file(compose_path) else {
            continue;
        };
        for key in keys {
            let k = key.trim();
            if k.is_empty() || !is_valid_env_key(k) {
                continue;
            }
            if let Some(value) = source_env.get(k) {
                env.insert(k.to_string(), value.to_string());
            }
        }
    }
}

fn build_compose_command_args(compose_args: &[String], suffix: &[&str]) -> Vec<String> {
    let mut args = vec!["compose".to_string()];
    args.extend(compose_args.iter().cloned());
    args.extend(suffix.iter().map(|s| s.to_string()));
    args
}

fn resolve_compose_status(
    worktree_path: &Path,
    compose_args: &[String],
    container_name: &str,
    env: &HashMap<String, String>,
) -> ContainerStatus {
    let running_output = gwt_core::process::command("docker")
        .args(build_compose_command_args(compose_args, &["ps", "-q"]))
        .current_dir(worktree_path)
        .env("COMPOSE_PROJECT_NAME", container_name)
        .envs(env)
        .output();

    let all_output = gwt_core::process::command("docker")
        .args(build_compose_command_args(
            compose_args,
            &["ps", "-a", "-q"],
        ))
        .current_dir(worktree_path)
        .env("COMPOSE_PROJECT_NAME", container_name)
        .envs(env)
        .output();

    match (running_output, all_output) {
        (Ok(running), Ok(all)) if running.status.success() && all.status.success() => {
            if !running.stdout.is_empty() {
                ContainerStatus::Running
            } else if !all.stdout.is_empty() {
                ContainerStatus::Stopped
            } else {
                ContainerStatus::NotFound
            }
        }
        (Ok(running), _) if running.status.success() && !running.stdout.is_empty() => {
            ContainerStatus::Running
        }
        _ => ContainerStatus::NotFound,
    }
}

fn resolve_compose_images_exist(
    worktree_path: &Path,
    compose_args: &[String],
    container_name: &str,
    env: &HashMap<String, String>,
) -> bool {
    gwt_core::process::command("docker")
        .args(build_compose_command_args(compose_args, &["images", "-q"]))
        .current_dir(worktree_path)
        .env("COMPOSE_PROJECT_NAME", container_name)
        .envs(env)
        .output()
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false)
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

    let remotes = Remote::list(&repo_path).unwrap_or_default();
    let normalized_branch = strip_known_remote_prefix(branch_ref, &remotes);

    let (file_type, compose_services, compose_probe) = match worktree_path.as_ref() {
        Some(wt) => match detect_docker_files(wt) {
            Some(DockerFileType::Compose(compose_path)) => {
                let compose_args =
                    vec!["-f".to_string(), compose_path.to_string_lossy().to_string()];
                let services = DockerManager::list_services_from_compose_file(&compose_path)
                    .unwrap_or_default();
                let probe = ComposeProbeTarget {
                    docker_file_type: DockerFileType::Compose(compose_path.clone()),
                    compose_paths: vec![compose_path],
                    compose_args,
                };
                ("compose".to_string(), services, Some(probe))
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
                    ("devcontainer".to_string(), Vec::new(), None)
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
                    let compose_args = cfg
                        .as_ref()
                        .map(|c| c.to_compose_args(&devcontainer_dir))
                        .unwrap_or_default();
                    let compose_paths = compose_file_paths_from_args(&compose_args);
                    let probe = if compose_args.is_empty() {
                        None
                    } else {
                        Some(ComposeProbeTarget {
                            docker_file_type: DockerFileType::DevContainer(
                                devcontainer_path.clone(),
                            ),
                            compose_args,
                            compose_paths,
                        })
                    };
                    ("devcontainer".to_string(), services, probe)
                }
            }
            Some(DockerFileType::Dockerfile(_)) => ("dockerfile".to_string(), Vec::new(), None),
            _ => ("none".to_string(), Vec::new(), None),
        },
        None => ("none".to_string(), Vec::new(), None),
    };

    let process_env: HashMap<String, String> = std::env::vars().collect();
    let detection_env = merge_profile_env_for_detection(&process_env);

    // Detect container / image status when daemon is running and worktree exists.
    let (container_status, images_exist) = if daemon_ok && compose_ok {
        if let (Some(wt), Some(probe)) = (worktree_path.as_ref(), compose_probe.as_ref()) {
            if normalized_branch.is_empty() {
                (None, None)
            } else {
                let container_name = DockerManager::generate_container_name(normalized_branch);
                let mgr = DockerManager::new(wt, normalized_branch, probe.docker_file_type.clone());
                let mut env = mgr.collect_passthrough_env();
                merge_compose_env_from_source(&mut env, &probe.compose_paths, &detection_env);
                let status = resolve_compose_status(wt, &probe.compose_args, &container_name, &env);
                let status_str = match status {
                    ContainerStatus::Running => "running",
                    ContainerStatus::Stopped => "stopped",
                    ContainerStatus::NotFound => "not_found",
                };
                let img =
                    resolve_compose_images_exist(wt, &probe.compose_args, &container_name, &env);
                (Some(status_str.to_string()), Some(img))
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
