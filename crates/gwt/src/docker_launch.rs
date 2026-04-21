use super::*;

pub(crate) fn detect_wizard_docker_context_and_status(
    project_root: &Path,
) -> (
    Option<DockerWizardContext>,
    gwt_docker::ComposeServiceStatus,
) {
    let files = gwt_docker::detect_docker_files(project_root);
    let Some(compose_file) = docker_compose_file_for_launch(project_root, &files)
        .ok()
        .flatten()
    else {
        return (None, gwt_docker::ComposeServiceStatus::NotFound);
    };

    let Ok(services) = gwt_docker::parse_compose_file(&compose_file) else {
        return (None, gwt_docker::ComposeServiceStatus::NotFound);
    };
    if services.is_empty() {
        return (None, gwt_docker::ComposeServiceStatus::NotFound);
    }

    let suggested_service = docker_devcontainer_defaults(project_root, &files)
        .and_then(|defaults| defaults.service)
        .or_else(|| services.first().map(|service| service.name.clone()));
    let status = suggested_service
        .as_deref()
        .map(|service| {
            gwt_docker::compose_service_status(&compose_file, service)
                .unwrap_or(gwt_docker::ComposeServiceStatus::NotFound)
        })
        .unwrap_or(gwt_docker::ComposeServiceStatus::NotFound);

    (
        Some(DockerWizardContext {
            services: services.into_iter().map(|service| service.name).collect(),
            suggested_service,
        }),
        status,
    )
}

#[derive(Debug, Clone)]
pub(crate) struct DockerLaunchPlan {
    pub(crate) compose_file: PathBuf,
    pub(crate) service: String,
    pub(crate) container_cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerExecProgram {
    executable: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerPackageRunnerCandidate {
    executable: &'static str,
    base_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageRunnerProgram {
    pub(crate) executable: String,
    pub(crate) args: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DevContainerLaunchDefaults {
    pub(crate) service: Option<String>,
    pub(crate) workspace_folder: Option<String>,
    pub(crate) compose_file: Option<PathBuf>,
}

pub(crate) fn apply_docker_runtime_to_launch_config(
    repo_path: &Path,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Docker {
        return Ok(());
    }

    let worktree = config
        .working_dir
        .clone()
        .unwrap_or_else(|| repo_path.to_path_buf());
    let launch = resolve_docker_launch_plan(&worktree, config.docker_service.as_deref())?;
    ensure_docker_launch_runtime_ready()?;
    ensure_docker_launch_service_ready(&launch, config.docker_lifecycle_intent)?;
    ensure_docker_gwt_binary_setup(&worktree, &launch.service)?;
    maybe_inject_docker_sandbox_env(&launch, config)?;
    install_launch_gwt_bin_env(&mut config.env_vars, gwt_agent::LaunchRuntimeTarget::Docker)?;
    let runtime_program = resolve_docker_exec_program(&launch, config)?;
    config.command = runtime_program.executable;
    config.args = runtime_program.args;
    config
        .env_vars
        .insert("GWT_PROJECT_ROOT".to_string(), launch.container_cwd.clone());
    config.docker_service = Some(launch.service);
    Ok(())
}

pub(crate) fn finalize_docker_agent_launch_config(
    repo_path: &Path,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Docker {
        return Ok(());
    }

    let worktree = config
        .working_dir
        .clone()
        .unwrap_or_else(|| repo_path.to_path_buf());
    let launch = resolve_docker_launch_plan(&worktree, config.docker_service.as_deref())?;
    let runtime_program = PackageRunnerProgram {
        executable: config.command.clone(),
        args: config.args.clone(),
    };

    let mut args = vec![
        "compose".to_string(),
        "-f".to_string(),
        launch.compose_file.display().to_string(),
        "exec".to_string(),
        "-w".to_string(),
        launch.container_cwd,
    ];
    args.extend(docker_compose_exec_env_args(&config.env_vars));
    args.push(launch.service);
    args.push(runtime_program.executable);
    args.extend(runtime_program.args);

    config.command = docker_binary_for_launch();
    config.args = args;
    Ok(())
}

fn resolve_user_home_dir() -> Result<PathBuf, String> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE")
    } else {
        std::env::var("HOME")
    }
    .map(PathBuf::from)
    .map_err(|_| "Could not determine home directory".to_string())?;
    Ok(home)
}

pub(crate) fn docker_bundle_mounts_for_home(home: &Path) -> DockerBundleMounts {
    let gwt_bin_dir = home.join(".gwt").join("bin");
    DockerBundleMounts {
        host_gwt: gwt_bin_dir.join(DOCKER_HOST_GWT_BIN_NAME),
        host_gwtd: gwt_bin_dir.join(DOCKER_HOST_GWTD_BIN_NAME),
    }
}

fn docker_compose_mount_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn docker_bundle_override_content(service: &str, bundle: &DockerBundleMounts) -> String {
    let host_gwt = docker_compose_mount_path(&bundle.host_gwt);
    let host_gwtd = docker_compose_mount_path(&bundle.host_gwtd);
    format!(
        "# Auto-generated docker-compose override for gwt bundle mounting\n\
         version: '3.8'\n\
         services:\n\
           {service}:\n\
             volumes:\n\
               - \"{host_gwt}:{DOCKER_GWT_BIN_PATH}:ro\"\n\
               - \"{host_gwtd}:{DOCKER_GWTD_BIN_PATH}:ro\"\n"
    )
}

pub(crate) fn ensure_docker_gwt_binary_setup(
    repo_path: &Path,
    service: &str,
) -> Result<(), String> {
    use std::fs;

    let home = resolve_user_home_dir()?;
    let bundle = docker_bundle_mounts_for_home(&home);

    if !bundle.host_gwt.exists() || !bundle.host_gwtd.exists() {
        let override_path = repo_path.join("docker-compose.override.yml");
        if !override_path.exists() {
            eprintln!(
                "Note: Linux gwt bundle not found at {} and {}\n\
                 This is required for Docker agent support.\n\
                 You can either:\n\
                 1. Download the Linux release bundle and place the extracted binaries at these paths\n\
                 2. Run 'gwt setup docker' to set up Docker integration automatically"
                ,
                bundle.host_gwt.display(),
                bundle.host_gwtd.display()
            );
        }
    }

    let override_path = repo_path.join("docker-compose.override.yml");
    if !override_path.exists() {
        let override_content = docker_bundle_override_content(service, &bundle);
        fs::write(&override_path, override_content).map_err(|err| {
            format!(
                "Failed to create docker-compose.override.yml: {err}\n\
                 Manually create {} with gwt/gwtd bundle mounts",
                override_path.display()
            )
        })?;
    }

    Ok(())
}

fn maybe_inject_docker_sandbox_env(
    launch: &DockerLaunchPlan,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    if cfg!(windows)
        || !matches!(config.agent_id, gwt_agent::AgentId::ClaudeCode)
        || !config.skip_permissions
    {
        return Ok(());
    }

    let is_root = gwt_docker::compose_service_user_is_root(&launch.compose_file, &launch.service)
        .map_err(|err| {
        format!(
            "Failed to determine Docker user for service '{}': {err}",
            launch.service
        )
    })?;
    if is_root {
        config
            .env_vars
            .insert("IS_SANDBOX".to_string(), "1".to_string());
    }
    Ok(())
}

pub(crate) fn docker_compose_exec_env_args(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut keys = env_vars.keys().collect::<Vec<_>>();
    keys.sort();

    let mut args = Vec::new();
    for key in keys {
        let key = key.trim();
        if key.is_empty() || !is_valid_docker_env_key(key) {
            continue;
        }
        let value = env_vars.get(key).map(String::as_str).unwrap_or_default();
        args.push("-e".to_string());
        args.push(format!("{key}={value}"));
    }
    args
}

pub(crate) fn is_valid_docker_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn resolve_docker_exec_program(
    launch: &DockerLaunchPlan,
    config: &gwt_agent::LaunchConfig,
) -> Result<DockerExecProgram, String> {
    let Some(version_spec) = package_runner_version_spec(config) else {
        ensure_docker_launch_command_ready(launch, &config.command)?;
        return Ok(DockerExecProgram {
            executable: config.command.clone(),
            args: config.args.clone(),
        });
    };
    resolve_docker_package_runner(launch, config, &version_spec)
}

pub(crate) fn package_runner_version_spec(config: &gwt_agent::LaunchConfig) -> Option<String> {
    let package = config.agent_id.package_name()?;
    let version = config.tool_version.as_deref()?;
    if version == "installed" || version.is_empty() {
        return None;
    }
    Some(if version == "latest" {
        format!("{package}@latest")
    } else {
        format!("{package}@{version}")
    })
}

fn resolve_docker_package_runner(
    launch: &DockerLaunchPlan,
    config: &gwt_agent::LaunchConfig,
    version_spec: &str,
) -> Result<DockerExecProgram, String> {
    let agent_args = strip_package_runner_args(&config.args, version_spec);
    let candidates = vec![
        DockerPackageRunnerCandidate {
            executable: "bunx",
            base_args: vec![version_spec.to_string()],
        },
        DockerPackageRunnerCandidate {
            executable: "npx",
            base_args: vec!["--yes".to_string(), version_spec.to_string()],
        },
    ];

    for candidate in candidates {
        let output = gwt_docker::compose_service_exec_capture(
            &launch.compose_file,
            &launch.service,
            Some(&launch.container_cwd),
            &candidate.probe_args(),
        )
        .map_err(|err| err.to_string())?;
        if output.status.success() {
            return Ok(candidate.into_exec_program(agent_args.clone()));
        }
    }

    Err(format!(
        "Selected Docker runtime cannot launch {version_spec} in service '{}'",
        launch.service
    ))
}

pub(crate) fn strip_package_runner_args(args: &[String], version_spec: &str) -> Vec<String> {
    if args.first().is_some_and(|first| first == "--yes")
        && args.get(1).is_some_and(|arg| arg == version_spec)
    {
        return args[2..].to_vec();
    }
    if args.first().is_some_and(|arg| arg == version_spec) {
        return args[1..].to_vec();
    }
    args.to_vec()
}

pub(crate) fn resolve_docker_shell_command(launch: &DockerLaunchPlan) -> Result<String, String> {
    for candidate in ["bash", "sh"] {
        let available = gwt_docker::compose_service_has_command(
            &launch.compose_file,
            &launch.service,
            candidate,
        )
        .map_err(|err| err.to_string())?;
        if available {
            return Ok(candidate.to_string());
        }
    }

    Err(format!(
        "Selected Docker runtime has no interactive shell in service '{}'",
        launch.service
    ))
}

fn ensure_docker_launch_command_ready(
    launch: &DockerLaunchPlan,
    command: &str,
) -> Result<(), String> {
    let available =
        gwt_docker::compose_service_has_command(&launch.compose_file, &launch.service, command)
            .map_err(|err| err.to_string())?;
    if available {
        Ok(())
    } else {
        Err(format!(
            "Command '{command}' is not available in Docker service '{}'",
            launch.service
        ))
    }
}

impl DockerPackageRunnerCandidate {
    fn probe_args(&self) -> Vec<String> {
        let mut args = vec![self.executable.to_string()];
        args.extend(self.base_args.clone());
        args.push("--version".to_string());
        args
    }

    fn into_exec_program(self, mut agent_args: Vec<String>) -> DockerExecProgram {
        let mut args = self.base_args;
        args.append(&mut agent_args);
        DockerExecProgram {
            executable: self.executable.to_string(),
            args,
        }
    }
}

pub(crate) fn ensure_docker_launch_service_ready(
    launch: &DockerLaunchPlan,
    intent: gwt_agent::DockerLifecycleIntent,
) -> Result<(), String> {
    let status = gwt_docker::compose_service_status(&launch.compose_file, &launch.service)
        .map_err(|err| err.to_string())?;
    match normalize_docker_launch_action(intent, status) {
        DockerLaunchServiceAction::Connect => Ok(()),
        DockerLaunchServiceAction::Start => {
            gwt_docker::compose_up(&launch.compose_file, &launch.service)
                .map_err(|err| err.to_string())?;
            Ok(())
        }
        DockerLaunchServiceAction::Restart => {
            gwt_docker::compose_restart(&launch.compose_file, &launch.service)
                .map_err(|err| err.to_string())
        }
        DockerLaunchServiceAction::Recreate => {
            gwt_docker::compose_up_force_recreate(&launch.compose_file, &launch.service)
                .map_err(|err| err.to_string())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DockerLaunchServiceAction {
    Connect,
    Start,
    Restart,
    Recreate,
}

pub(crate) fn normalize_docker_launch_action(
    intent: gwt_agent::DockerLifecycleIntent,
    status: gwt_docker::ComposeServiceStatus,
) -> DockerLaunchServiceAction {
    use gwt_agent::DockerLifecycleIntent;
    use gwt_docker::ComposeServiceStatus;

    match intent {
        DockerLifecycleIntent::Recreate => DockerLaunchServiceAction::Recreate,
        DockerLifecycleIntent::Restart if status == ComposeServiceStatus::Running => {
            DockerLaunchServiceAction::Restart
        }
        DockerLifecycleIntent::Connect
        | DockerLifecycleIntent::Start
        | DockerLifecycleIntent::Restart
        | DockerLifecycleIntent::CreateAndStart => match status {
            ComposeServiceStatus::Running => DockerLaunchServiceAction::Connect,
            ComposeServiceStatus::Stopped
            | ComposeServiceStatus::Exited
            | ComposeServiceStatus::NotFound => DockerLaunchServiceAction::Start,
        },
    }
}

pub(crate) fn resolve_docker_launch_plan(
    worktree: &Path,
    selected_service: Option<&str>,
) -> Result<DockerLaunchPlan, String> {
    let files = gwt_docker::detect_docker_files(worktree);
    let compose_file = docker_compose_file_for_launch(worktree, &files)?.ok_or_else(|| {
        "Docker launch requires a docker-compose.yml or devcontainer compose target".to_string()
    })?;
    let services = gwt_docker::parse_compose_file(&compose_file).map_err(|err| err.to_string())?;
    if services.is_empty() {
        return Err("Docker launch requires at least one compose service".to_string());
    }

    let devcontainer_defaults = docker_devcontainer_defaults(worktree, &files);
    let service_name = selected_service
        .map(str::to_string)
        .or_else(|| {
            devcontainer_defaults
                .as_ref()
                .and_then(|defaults| defaults.service.clone())
        })
        .or_else(|| {
            if services.len() == 1 {
                services.first().map(|service| service.name.clone())
            } else {
                None
            }
        })
        .ok_or_else(|| {
            "Multiple Docker services detected; select a Docker service in Launch Agent Wizard"
                .to_string()
        })?;

    let service = services
        .iter()
        .find(|service| service.name == service_name)
        .ok_or_else(|| {
            format!("Selected Docker service was not found in compose file: {service_name}")
        })?;

    let container_cwd = devcontainer_defaults
        .as_ref()
        .and_then(|defaults| defaults.workspace_folder.clone())
        .or_else(|| service.working_dir.clone())
        .or_else(|| compose_workspace_mount_target(worktree, service))
        .ok_or_else(|| {
            format!(
                "Docker service {} is missing working_dir/workspaceFolder and no project-root volume mount was detected",
                service.name
            )
        })?;

    Ok(DockerLaunchPlan {
        compose_file,
        service: service.name.clone(),
        container_cwd,
    })
}

pub(crate) fn docker_binary_for_launch() -> String {
    std::env::var("GWT_DOCKER_BIN").unwrap_or_else(|_| "docker".to_string())
}

pub(crate) fn docker_compose_file_for_launch(
    project_root: &Path,
    files: &gwt_docker::DockerFiles,
) -> Result<Option<PathBuf>, String> {
    Ok(docker_devcontainer_defaults(project_root, files)
        .and_then(|defaults| defaults.compose_file)
        .or_else(|| files.compose_file.clone()))
}

pub(crate) fn docker_devcontainer_defaults(
    project_root: &Path,
    files: &gwt_docker::DockerFiles,
) -> Option<DevContainerLaunchDefaults> {
    let devcontainer_dir = files.devcontainer_dir.as_ref()?;
    let path = devcontainer_dir.join("devcontainer.json");
    if !path.is_file() {
        return None;
    }

    let config = gwt_docker::DevContainerConfig::load(&path).ok()?;
    let compose_file = config
        .docker_compose_file
        .as_ref()
        .and_then(|value| {
            value
                .to_vec()
                .into_iter()
                .map(|candidate| devcontainer_dir.join(candidate))
                .find(|path| path.is_file())
        })
        .or_else(|| files.compose_file.clone())
        .or_else(|| {
            let fallback = project_root.join("docker-compose.yml");
            fallback.is_file().then_some(fallback)
        });

    Some(DevContainerLaunchDefaults {
        service: config.service,
        workspace_folder: config.workspace_folder,
        compose_file,
    })
}

pub(crate) fn compose_workspace_mount_target(
    project_root: &Path,
    service: &gwt_docker::ComposeService,
) -> Option<String> {
    service
        .volumes
        .iter()
        .find(|mount| mount_source_matches_project_root(&mount.source, project_root))
        .map(|mount| mount.target.clone())
}

pub(crate) fn mount_source_matches_project_root(source: &str, project_root: &Path) -> bool {
    let normalized = source
        .trim()
        .trim_end_matches(['/', '\\'])
        .trim_end_matches("/.");

    if matches!(normalized, "." | "$PWD" | "${PWD}") {
        return true;
    }

    let source_path = Path::new(normalized);
    source_path.is_absolute() && same_worktree_path(source_path, project_root)
}
